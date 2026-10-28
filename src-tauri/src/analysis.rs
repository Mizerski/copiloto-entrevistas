//! Análise ao vivo do lado do candidato. Agrupa as falas em turnos (pergunta do
//! entrevistador + sua resposta) e faz duas chamadas à LLM:
//!
//! - Resposta: uma fala pronta por pergunta, tirada só do perfil da pessoa, em streaming
//!   (a primeira frase aparece antes do resto chegar). Começa enquanto o entrevistador
//!   ainda pergunta (a cada `ANSWER_EVERY_WORDS` palavras novas, contando o parcial da
//!   transcrição) e é refeita na hora em que ele para de falar, cancelando a versão que
//!   estava no ar. Depois que a pessoa começa a responder, a fala daquela pergunta não
//!   muda mais.
//! - Memória: a cada resposta fechada, o resumo da conversa e os fatos que a pessoa
//!   afirmou, pra as próximas respostas não contradizerem as anteriores.
//!
//! O microfone é o candidato; a chamada é o entrevistador. Uma resposta fecha quando o
//! entrevistador volta a falar (mais que um "uhum") ou depois de
//! `CLOSE_AFTER_SILENCE_MS` de silêncio. Um "uhum" do candidato no meio da pergunta
//! também não conta como resposta.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::audio::Source;
use crate::db::{Db, Segment};
use crate::kit::fold;
use crate::llm::{LlmClient, Prompt, TextPrompt};
use crate::session::Heard;

const CLOSE_AFTER_SILENCE_MS: u64 = 3_000;
/// Turnos anteriores mandados junto com o atual, pra dar contexto.
const CONTEXT_TURNS: usize = 3;
/// Fala com até tantas palavras no meio da fala do outro é só "uhum", "entendi", "sim":
/// não fecha a resposta (do entrevistador) nem começa uma (do candidato).
const BACKCHANNEL_WORDS: usize = 3;
/// Palavras novas da pergunta que disparam outra sugestão.
const ANSWER_EVERY_WORDS: usize = 6;
/// Silêncio do entrevistador que fecha a pergunta quando a transcrição não avisa antes.
const QUESTION_DONE_MS: u64 = 1_200;
/// Um "sim" curto do candidato vira resposta se o entrevistador não voltar a falar nesse tempo.
const SHORT_ANSWER_MS: u64 = 1_500;
/// Uma versão nova da resposta só substitui a da tela quando já tem esse começo.
const REWRITE_MIN_CHARS: usize = 60;
/// Intervalo mínimo entre dois pedaços da resposta mandados pra tela.
const STREAM_EVERY: Duration = Duration::from_millis(60);
/// O que o modelo devolve quando a fala na tela ainda serve.
const KEEP: &str = "MESMA";
/// Fatos guardados na memória da entrevista (os mais recentes).
const MAX_FACTS: usize = 60;

const SYSTEM: &str = "\
Você acompanha, ao vivo, uma entrevista de emprego do lado de quem está sendo entrevistado, \
em português do Brasil. Você recebe o perfil da pessoa e as trocas recentes. Olhe só a última \
resposta dela e devolva:

1. resumo: o resumo atualizado da conversa inteira até agora, em até 100 palavras.
2. fatos: até 5 afirmações concretas que a pessoa fez nesta resposta e que daria pra \
contradizer depois (cargo, empresa, tempo, números, datas, tecnologias, o que ela fez \
pessoalmente e o que foi do time). Cada uma resumida em até 12 palavras, com o trecho copiado \
exatamente da resposta.

A transcrição é automática e pode ter palavras erradas.";

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AnalysisEvent {
    /// O entrevistador está fazendo uma pergunta nova (ou ela cresceu). Chega antes da
    /// resposta, pra tela não deixar a resposta da pergunta anterior à vista.
    Question { turn: usize, question: String },
    /// A fala sugerida pra uma pergunta, inteira ou ainda chegando (`done`). Uma versão
    /// nova da mesma pergunta (mesmo `turn`) substitui a anterior.
    Answer { answer: Answer },
    /// `analisando`, `ok` ou `erro`.
    Status {
        state: String,
        message: Option<String>,
        latency_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// A pergunta, na ordem da entrevista (0 = primeira).
    pub turn: usize,
    /// Quando a pergunta terminou (relógio da sessão).
    pub t_ms: u64,
    /// O que o entrevistador disse até agora nesta pergunta.
    pub question: String,
    /// A fala pronta (ou o começo dela, se `done` for falso).
    pub text: String,
    /// Do pedido até o primeiro pedaço do texto.
    pub latency_ms: u64,
    pub done: bool,
}

/// Uma afirmação da pessoa, guardada pra comparar com o que perguntarem depois.
struct Fact {
    at_ms: u64,
    claim: String,
    quote: String,
}

#[derive(Default, Clone)]
struct Turn {
    /// Quando o entrevistador começou a perguntar (0 se o candidato falou primeiro).
    start_ms: u64,
    /// O turno começou por uma pergunta do entrevistador.
    asked: bool,
    question: Vec<Segment>,
    answer: Vec<Segment>,
    /// Falas curtas do candidato no meio da pergunta ("uhum", "sim"): só viram resposta
    /// se o entrevistador não continuar.
    held: Vec<Segment>,
    /// Quantas falas da resposta já foram analisadas.
    analyzed: usize,
}

impl Turn {
    fn asked_at(start_ms: u64) -> Self {
        Turn {
            start_ms,
            asked: true,
            ..Default::default()
        }
    }

    /// O entrevistador perguntou e o candidato ainda não começou a responder.
    fn asking(&self) -> bool {
        self.asked && self.answer.is_empty()
    }
}

struct State {
    turns: Vec<Turn>,
    /// O que o entrevistador está falando agora e ainda não virou fala final.
    partial: String,
    summary: String,
    /// A última fala mostrada de cada pergunta (pelo índice do turno).
    answers: HashMap<usize, Shown>,
    /// A última pergunta mandada pra tela, pra não repetir.
    shown_question: (usize, String),
    facts: Vec<Fact>,
    suggest: SuggestState,
}

#[derive(Clone)]
struct Shown {
    text: String,
    /// Chegou inteira (e não só o começo de um pedido cancelado).
    done: bool,
}

#[derive(Default)]
struct SuggestState {
    /// O pedido no ar, pra cancelar quando chega uma versão melhor da pergunta.
    in_flight: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Identifica o pedido mais novo; resultado de outro é descartado.
    generation: u64,
    /// O pedido mais novo já mostrou texto na tela.
    emitted: bool,
    /// Palavras da pergunta atual quando a última sugestão foi pedida.
    words_at_request: usize,
    /// Turno da última sugestão pedida.
    turn: usize,
}

pub struct Analyzer {
    session_id: i64,
    db: Arc<Db>,
    llm: Arc<LlmClient>,
    stable_context: String,
    state: Mutex<State>,
    generation: AtomicU64,
    emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
}

/// Quem pergunta: a outra ponta da chamada.
const INTERVIEWER: Source = Source::Sistema;

impl Analyzer {
    /// `profile`: o perfil em Markdown; `job`: a descrição da vaga, se houver.
    pub fn new(
        session_id: i64,
        db: Arc<Db>,
        llm: LlmClient,
        profile: &str,
        job: &str,
        emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
    ) -> Arc<Self> {
        let mut stable_context = String::new();
        if !job.trim().is_empty() {
            stable_context.push_str(&format!("# Vaga\n{}\n\n", job.trim()));
        }
        stable_context.push_str(&format!(
            "# Perfil de quem está sendo entrevistado (escrito pela própria pessoa)\n{profile}\n"
        ));
        Arc::new(Analyzer {
            session_id,
            db,
            llm: Arc::new(llm),
            stable_context,
            state: Mutex::new(State {
                turns: Vec::new(),
                partial: String::new(),
                summary: String::new(),
                answers: HashMap::new(),
                shown_question: (usize::MAX, String::new()),
                facts: Vec::new(),
                suggest: SuggestState::default(),
            }),
            generation: AtomicU64::new(0),
            emit,
        })
    }

    /// Um pedido mínimo com o mesmo começo de prompt das respostas: abre a conexão e
    /// grava o perfil no cache do provedor antes da primeira pergunta.
    pub fn warm_up(self: &Arc<Self>) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let prompt = TextPrompt {
                system: crate::profile::ANSWER_SYSTEM,
                stable_context: &this.stable_context,
                user: "A entrevista vai começar. Responda só: ok.",
                max_tokens: 5,
            };
            if let Ok((_, usage)) = this.llm.stream_text(&prompt, |_| {}).await {
                let _ = this.db.insert_usage(this.session_id, "aquecimento", &usage);
            }
        });
    }

    /// Uma fala confirmada (já sem eco).
    pub fn on_segment(self: &Arc<Self>, segment: &Segment) {
        if segment.echo {
            return;
        }
        let from_interviewer = segment.source == INTERVIEWER;
        let close = {
            let mut st = self.state.lock().unwrap();
            if from_interviewer {
                st.partial.clear();
                let previous_open = st.turns.last().is_some_and(|t| !t.answer.is_empty());
                if previous_open && word_count(&segment.text) <= BACKCHANNEL_WORDS {
                    st.turns.last_mut().unwrap().answer.push(segment.clone());
                    return;
                }
                if previous_open || st.turns.is_empty() {
                    st.turns.push(Turn::asked_at(segment.start_ms));
                }
                let turn = st.turns.last_mut().unwrap();
                // O entrevistador continuou depois do "uhum": não era resposta.
                turn.held.retain(|h| h.end_ms > segment.end_ms);
                turn.question.push(segment.clone());
                // O entrevistador voltou a falar: a resposta anterior fechou.
                previous_open && needs_analysis(&st.turns[st.turns.len() - 2])
            } else {
                add_answer(&mut st, segment.clone());
                false
            }
        };
        if from_interviewer {
            self.show_question();
            self.suggest_answer(segment.end_ms, false);
        }
        if close {
            self.analyze(segment.end_ms);
        }
    }

    /// `heard`: até quando cada lado foi ouvido, contando os parciais (as falas finais
    /// chegam a cada poucos segundos e com atraso; olhar só pra elas confunde pausa de
    /// transcrição com fim de fala), o parcial do entrevistador e se ele já parou.
    pub fn tick(self: &Arc<Self>, now_ms: u64, heard: &Heard) {
        let (close_previous, question_done, silent_answer) = {
            let mut st = self.state.lock().unwrap();
            st.partial = heard.interviewer_partial.trim().to_string();

            // Pergunta nova pelo parcial: a tela troca antes da primeira fala final.
            let previous_open = st.turns.last().is_none_or(|t| !t.answer.is_empty());
            let opened = previous_open && word_count(&st.partial) > BACKCHANNEL_WORDS;
            let close_previous = opened && st.turns.last().is_some_and(needs_analysis);
            if opened {
                st.turns
                    .push(Turn::asked_at(heard.interviewer_partial_start));
            }

            if let Some(turn) = st.turns.last_mut().filter(|t| t.asking()) {
                // O entrevistador voltou a falar depois do "uhum": era só reação.
                turn.held
                    .retain(|h| heard.interviewer_until <= h.end_ms + 300);
                let ready = turn
                    .held
                    .first()
                    .is_some_and(|h| now_ms >= h.end_ms + SHORT_ANSWER_MS);
                if ready {
                    let held = std::mem::take(&mut turn.held);
                    turn.answer.extend(held);
                }
            }

            let question_done = st.turns.last().is_some_and(|t| {
                let last = t
                    .question
                    .last()
                    .map_or(0, |s| s.end_ms)
                    .max(heard.interviewer_until);
                let stopped = (heard.interviewer_ended && st.partial.is_empty())
                    || now_ms >= last + QUESTION_DONE_MS;
                t.asking() && (!t.question.is_empty() || !st.partial.is_empty()) && stopped
            });
            let silent_answer = st.turns.last().is_some_and(|t| {
                let last_heard = t.answer.last().map_or(0, |s| s.end_ms).max(heard.mic_until);
                needs_analysis(t) && now_ms >= last_heard + CLOSE_AFTER_SILENCE_MS
            });
            (close_previous, question_done, silent_answer)
        };
        if close_previous {
            self.analyze(now_ms);
        }
        self.show_question();
        self.suggest_answer(now_ms, question_done);
        if silent_answer {
            self.analyze(now_ms);
        }
    }

    /// Fim da sessão: analisa a última resposta, se ainda não foi.
    pub fn flush(self: &Arc<Self>, now_ms: u64) {
        let pending = self
            .state
            .lock()
            .unwrap()
            .turns
            .last()
            .is_some_and(needs_analysis);
        if pending {
            self.analyze(now_ms);
        }
    }

    /// Avisa a tela da pergunta em andamento, quando ela mudou.
    fn show_question(&self) {
        let event = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.len().checked_sub(1) else {
                return;
            };
            if !st.turns[index].asking() {
                return;
            }
            let question = question_text(&st, index);
            if question.is_empty() || st.shown_question == (index, question.clone()) {
                return;
            }
            st.shown_question = (index, question.clone());
            AnalysisEvent::Question {
                turn: index,
                question,
            }
        };
        (self.emit)(event);
    }

    fn analyze(self: &Arc<Self>, now_ms: u64) {
        let llm = self.llm.clone();
        let (user, answer) = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.iter().rposition(|t| !t.answer.is_empty()) else {
                return;
            };
            let turn = &mut st.turns[index];
            turn.analyzed = turn.answer.len();
            let answer = turn.answer.clone();
            (self.user_prompt(&st, index, now_ms), answer)
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.status("analisando", None, None);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let schema = schema();
            let prompt = Prompt {
                system: SYSTEM,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                max_tokens: 4000,
            };
            let result = llm.complete_json(&prompt).await;
            // Chegou depois da análise de uma resposta mais nova: os fatos desta resposta
            // continuam valendo, só o resumo é que ficou velho.
            let current = this.generation.load(Ordering::SeqCst) == generation;
            match result {
                Ok((value, usage)) => {
                    let _ = this.db.insert_usage(this.session_id, "analise", &usage);
                    this.apply(&value, &answer, current);
                    this.status("ok", None, Some(usage.latency_ms));
                }
                Err(e) => this.status("erro", Some(e), None),
            }
        });
    }

    /// O que responder à pergunta do entrevistador. Começa enquanto ele ainda pergunta
    /// (a cada `ANSWER_EVERY_WORDS` palavras novas) e de novo quando ele para de falar
    /// (`closing`), cancelando o pedido que ainda estiver no ar: a versão com a pergunta
    /// inteira é a que importa e não pode esperar a anterior terminar.
    fn suggest_answer(self: &Arc<Self>, now_ms: u64, closing: bool) {
        let mut st = self.state.lock().unwrap();
        let Some(index) = st.turns.len().checked_sub(1) else {
            return;
        };
        if !st.turns[index].asked {
            return;
        }
        let question = question_text(&st, index);
        if question.is_empty() {
            return;
        }
        // Você já começou a responder com a fala na tela: ela não muda mais.
        if !st.turns[index].answer.is_empty() && st.answers.contains_key(&index) {
            return;
        }
        let words = word_count(&question);
        let seen = if st.suggest.turn == index {
            st.suggest.words_at_request
        } else {
            0
        };
        let enough = if closing {
            words > seen
        } else {
            words >= seen + ANSWER_EVERY_WORDS || (seen == 0 && words >= 3)
        };
        if !enough {
            return;
        }
        if let Some(task) = &st.suggest.in_flight {
            // No meio da pergunta, a versão no ar segue; quando ela terminar, pede de novo.
            if !closing && st.suggest.turn == index {
                return;
            }
            task.abort();
        }
        let generation = st.suggest.generation + 1;
        let user = self.answer_prompt(&st, index, now_ms);
        let this = self.clone();
        let task = tauri::async_runtime::spawn(async move {
            this.stream_answer(user, index, generation, now_ms).await;
        });
        st.suggest = SuggestState {
            in_flight: Some(task),
            generation,
            emitted: false,
            words_at_request: words,
            turn: index,
        };
    }

    async fn stream_answer(
        self: Arc<Self>,
        user: String,
        turn: usize,
        generation: u64,
        now_ms: u64,
    ) {
        let prompt = TextPrompt {
            system: crate::profile::ANSWER_SYSTEM,
            stable_context: &self.stable_context,
            user: &user,
            max_tokens: 1200,
        };
        let started = Instant::now();
        let mut first_text_ms = None;
        let mut last_sent: Option<Instant> = None;
        let result = self
            .llm
            .stream_text(&prompt, |text| {
                let latency = *first_text_ms.get_or_insert(started.elapsed().as_millis() as u64);
                if last_sent.is_none_or(|t| t.elapsed() >= STREAM_EVERY) {
                    last_sent = Some(Instant::now());
                    self.show_answer(turn, generation, text, false, latency);
                }
            })
            .await;
        let latency = first_text_ms.unwrap_or(started.elapsed().as_millis() as u64);
        match result {
            Ok((text, usage)) => {
                let _ = self.db.insert_usage(self.session_id, "resposta", &usage);
                self.show_answer(turn, generation, &text, true, latency);
            }
            Err(e) => self.status("erro", Some(e), None),
        }
        let at_ms = {
            let mut st = self.state.lock().unwrap();
            if st.suggest.generation != generation || st.turns.len() != turn + 1 {
                // Outro pedido já passou na frente, ou o entrevistador mudou de pergunta.
                return;
            }
            st.suggest.in_flight = None;
            st.turns[turn].question.last().map_or(now_ms, |s| s.end_ms)
        };
        // A pergunta continuou enquanto esta estava no ar.
        self.suggest_answer(at_ms, false);
    }

    fn answer_prompt(&self, st: &State, index: usize, now_ms: u64) -> String {
        let mut out = format!("Tempo de entrevista: {}.\n", clock(now_ms));
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        if !st.facts.is_empty() {
            out.push_str("\nO que você já disse nesta entrevista:\n");
            for f in &st.facts {
                out.push_str(&format!(
                    "- [{}] {} (\"{}\")\n",
                    clock(f.at_ms),
                    f.claim,
                    f.quote
                ));
            }
        }
        // Só a fala inteira pode ser mantida; a que ficou pela metade é refeita.
        if let Some(shown) = st.answers.get(&index).filter(|s| s.done) {
            out.push_str(&format!(
                "\nFala na tela agora pra esta pergunta (se ainda serve, responda só {KEEP}):\n{}\n",
                shown.text
            ));
        }
        out.push_str("\nConversa recente (a última pergunta pode estar em andamento):\n");
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index],
        );
        if st.turns[index].asking() && !st.partial.is_empty() {
            out.push_str(&format!(
                "[agora] Entrevistador (ainda falando): {}\n",
                st.partial
            ));
        }
        out
    }

    /// Mostra a fala da pergunta, ainda chegando ou pronta. Se você já começou a
    /// responder, a que está na tela fica (trocar o texto no meio da fala atrapalha); a
    /// que já estava entrando continua até o fim.
    fn show_answer(
        &self,
        turn_index: usize,
        generation: u64,
        text: &str,
        done: bool,
        latency_ms: u64,
    ) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        // "MESMA" (ou o começo dela, ainda chegando): a fala na tela fica.
        let keep = text
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_uppercase();
        let keep = !keep.is_empty() && KEEP.starts_with(&keep);
        let answer = {
            let mut st = self.state.lock().unwrap();
            if st.suggest.generation != generation || st.turns.len() != turn_index + 1 {
                return;
            }
            let turn = &st.turns[turn_index];
            let started = !turn.answer.is_empty();
            let t_ms = turn.question.last().map_or(turn.start_ms, |s| s.end_ms);
            let question = question_text(&st, turn_index);
            let emitted = st.suggest.emitted;
            let shown = st.answers.get(&turn_index).cloned();
            let text = match (&shown, keep) {
                // A da tela ficou pela metade (o pedido dela foi cancelado): ela vira a final.
                (Some(s), true) if done && !s.done => s.text.clone(),
                (_, true) => return,
                (Some(_), false) if started && !emitted => return,
                // Uma reescrita só toma o lugar quando já tem um começo pra ler.
                (Some(_), false)
                    if !emitted && !done && text.chars().count() < REWRITE_MIN_CHARS =>
                {
                    return;
                }
                (Some(s), false) if s.text == text && s.done == done => return,
                _ => text.to_string(),
            };
            st.suggest.emitted = true;
            st.answers.insert(
                turn_index,
                Shown {
                    text: text.clone(),
                    done,
                },
            );
            Answer {
                turn: turn_index,
                t_ms,
                question,
                text,
                latency_ms,
                done,
            }
        };
        if done {
            let _ = self.db.insert_card(
                self.session_id,
                answer.t_ms,
                "resposta",
                &answer.text,
                "media",
            );
        }
        (self.emit)(AnalysisEvent::Answer { answer });
    }

    fn apply(&self, value: &Value, answer: &[Segment], current: bool) {
        let answer_words = fold(&join(answer));
        let mut st = self.state.lock().unwrap();
        let segments: Vec<&Segment> = answer.iter().collect();
        for fact in value["fatos"].as_array().into_iter().flatten() {
            let claim = fact["afirmacao"].as_str().unwrap_or_default().trim();
            let quote = fact["trecho"].as_str().unwrap_or_default().trim();
            if claim.is_empty() || !quote_in(quote, &answer_words) {
                continue;
            }
            let at_ms = find_quote(quote, &segments)
                .unwrap_or_else(|| answer.first().map_or(0, |s| s.start_ms));
            st.facts.push(Fact {
                at_ms,
                claim: claim.into(),
                quote: quote.into(),
            });
        }
        let excess = st.facts.len().saturating_sub(MAX_FACTS);
        st.facts.drain(..excess);
        if let Some(summary) = value["resumo"]
            .as_str()
            .filter(|s| current && !s.trim().is_empty())
        {
            st.summary = summary.trim().to_string();
            let _ = self.db.save_coverage(self.session_id, "[]", &st.summary);
        }
    }

    fn status(&self, state: &str, message: Option<String>, latency_ms: Option<u64>) {
        (self.emit)(AnalysisEvent::Status {
            state: state.into(),
            message,
            latency_ms,
        });
    }

    fn user_prompt(&self, st: &State, index: usize, now_ms: u64) -> String {
        let mut out = format!("Tempo de entrevista: {}.\n", clock(now_ms));
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        out.push_str("\nTrocas recentes (olhe a última resposta):\n");
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index],
        );
        out
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["resumo", "fatos"],
        "properties": {
            "resumo": { "type": "string" },
            "fatos": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["afirmacao", "trecho"],
                    "properties": {
                        "afirmacao": { "type": "string" },
                        "trecho": { "type": "string" }
                    }
                }
            }
        }
    })
}

/// Onde o trecho foi dito: o começo da fala em que ele aparece. Procura em janelas de
/// três falas seguidas (o trecho pode atravessar a divisão da transcrição) e fica com a
/// última janela que ainda contém o trecho, que começa mais perto dele.
fn find_quote(quote: &str, segments: &[&Segment]) -> Option<u64> {
    let window = |i: usize| -> String {
        let end = (i + 3).min(segments.len());
        fold(
            &segments[i..end]
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        )
    };
    let first = (0..segments.len()).find(|&i| quote_in(quote, &window(i)))?;
    let mut at = first;
    while at + 1 < segments.len() && quote_in(quote, &window(at + 1)) {
        at += 1;
    }
    Some(segments[at].start_ms)
}

/// O trecho está num texto longo (currículo)? Compara com janelas do tamanho do trecho,
/// pra não aceitar palavras soltas espalhadas pelo texto todo.
pub fn quote_in_text(quote: &str, text: &str) -> bool {
    let folded = fold(text);
    let words: Vec<&str> = folded.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return false;
    }
    let span = (word_count(quote) + 6).min(words.len());
    words.windows(span).any(|w| quote_in(quote, &w.join(" ")))
}

fn push_turns(out: &mut String, turns: &[Turn]) {
    let mut segments: Vec<&Segment> = turns
        .iter()
        .flat_map(|t| t.question.iter().chain(&t.answer))
        .collect();
    // O "uhum" do entrevistador entra na resposta; na ordem do tempo fica legível.
    segments.sort_by_key(|s| s.start_ms);
    for s in segments {
        let who = if s.source == INTERVIEWER {
            "Entrevistador"
        } else {
            "Você"
        };
        out.push_str(&format!("[{}] {who}: {}\n", clock(s.start_ms), s.text));
    }
}

/// Uma fala final do candidato.
fn add_answer(st: &mut State, segment: Segment) {
    if st.turns.is_empty() {
        st.turns.push(Turn::default());
    }
    let n = st.turns.len();
    // Começou antes da pergunta atual: é o fim da resposta anterior, que a transcrição
    // do microfone entregou atrasada.
    if n >= 2 && st.turns[n - 1].asking() && segment.start_ms < st.turns[n - 1].start_ms {
        st.turns[n - 2].answer.push(segment);
        return;
    }
    let turn = &mut st.turns[n - 1];
    if turn.asking() && word_count(&segment.text) <= BACKCHANNEL_WORDS {
        turn.held.push(segment);
        return;
    }
    let held = std::mem::take(&mut turn.held);
    turn.answer.extend(held);
    turn.answer.push(segment);
}

/// A pergunta do turno, com o parcial que ainda está chegando se for a atual.
fn question_text(st: &State, index: usize) -> String {
    let turn = &st.turns[index];
    let mut text = join(&turn.question);
    if index + 1 == st.turns.len() && turn.asking() && !st.partial.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&st.partial);
    }
    text
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn needs_analysis(turn: &Turn) -> bool {
    turn.answer.len() > turn.analyzed
}

fn join(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// O trecho citado está na resposta? Tolera pequenas diferenças: 80% das palavras do
/// trecho precisam aparecer na resposta.
pub fn quote_in(quote: &str, answer_folded: &str) -> bool {
    let quote = fold(quote);
    if quote.is_empty() {
        return false;
    }
    if answer_folded.contains(&quote) {
        return true;
    }
    let answer: HashMap<&str, ()> = answer_folded.split(' ').map(|w| (w, ())).collect();
    let words: Vec<&str> = quote.split(' ').collect();
    let found = words.iter().filter(|w| answer.contains_key(*w)).count();
    found as f32 / words.len() as f32 >= 0.8
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
