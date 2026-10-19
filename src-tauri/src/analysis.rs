//! Análise ao vivo do lado do candidato. Agrupa as falas em turnos (pergunta do
//! entrevistador + sua resposta) e faz duas chamadas à LLM:
//!
//! - Resposta: uma fala pronta por pergunta, tirada só do perfil da pessoa. Começa
//!   enquanto o entrevistador ainda pergunta (a cada `ANSWER_EVERY_WORDS` palavras novas)
//!   e é refeita quando ele para de falar. Depois que a pessoa começa a responder, a fala
//!   daquela pergunta não muda mais.
//! - Memória: a cada resposta fechada, o resumo da conversa e os fatos que a pessoa
//!   afirmou, pra as próximas respostas não contradizerem as anteriores.
//!
//! O microfone é o candidato; a chamada é o entrevistador. Uma resposta fecha quando o
//! entrevistador volta a falar (mais que um "uhum") ou depois de
//! `CLOSE_AFTER_SILENCE_MS` de silêncio. Resultado que chega depois de um mais novo é
//! descartado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::audio::Source;
use crate::db::{Db, Segment};
use crate::kit::fold;
use crate::llm::{LlmClient, Prompt};

const CLOSE_AFTER_SILENCE_MS: u64 = 3_000;
/// Turnos anteriores mandados junto com o atual, pra dar contexto.
const CONTEXT_TURNS: usize = 3;
/// Fala do entrevistador com até tantas palavras no meio da resposta é só "uhum",
/// "entendi": não fecha a resposta.
const BACKCHANNEL_WORDS: usize = 3;
/// Palavras novas da pergunta que disparam outra sugestão.
const ANSWER_EVERY_WORDS: usize = 6;
/// Silêncio do entrevistador que fecha a pergunta.
const QUESTION_DONE_MS: u64 = 1_200;
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
    /// A fala sugerida pra uma pergunta. Uma versão nova da mesma pergunta (mesmo
    /// `turn`) substitui a anterior.
    Answer {
        answer: Answer,
    },
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
    /// A fala pronta.
    pub text: String,
    pub latency_ms: u64,
}

/// Uma afirmação da pessoa, guardada pra comparar com o que perguntarem depois.
struct Fact {
    at_ms: u64,
    claim: String,
    quote: String,
}

#[derive(Default, Clone)]
struct Turn {
    question: Vec<Segment>,
    answer: Vec<Segment>,
    /// Quantas falas da resposta já foram analisadas.
    analyzed: usize,
}

struct State {
    turns: Vec<Turn>,
    summary: String,
    /// A última fala sugerida de cada pergunta (pelo índice do turno).
    answers: HashMap<usize, String>,
    facts: Vec<Fact>,
    suggest: SuggestState,
}

#[derive(Default)]
struct SuggestState {
    in_flight: bool,
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
                summary: String::new(),
                answers: HashMap::new(),
                facts: Vec::new(),
                suggest: SuggestState::default(),
            }),
            generation: AtomicU64::new(0),
            emit,
        })
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
                let previous_open = st.turns.last().is_some_and(|t| !t.answer.is_empty());
                if previous_open && word_count(&segment.text) <= BACKCHANNEL_WORDS {
                    st.turns.last_mut().unwrap().answer.push(segment.clone());
                    return;
                }
                if previous_open || st.turns.is_empty() {
                    st.turns.push(Turn {
                        question: vec![segment.clone()],
                        ..Default::default()
                    });
                } else {
                    st.turns.last_mut().unwrap().question.push(segment.clone());
                }
                // O entrevistador voltou a falar: a resposta anterior fechou.
                previous_open && needs_analysis(&st.turns[st.turns.len() - 2])
            } else {
                if st.turns.is_empty() {
                    st.turns.push(Turn::default());
                }
                st.turns.last_mut().unwrap().answer.push(segment.clone());
                false
            }
        };
        if from_interviewer {
            self.suggest_answer(segment.end_ms, false);
        }
        if close {
            self.analyze(segment.end_ms);
        }
    }

    /// `heard_until_ms`: até quando o candidato foi ouvido, contando os parciais. As
    /// falas finais chegam a cada poucos segundos e com atraso; olhar só pra elas
    /// confunde pausa de transcrição com fim de resposta.
    /// `interviewer_heard_ms`: o mesmo pra quem pergunta (fecha a pergunta).
    pub fn tick(self: &Arc<Self>, now_ms: u64, heard_until_ms: u64, interviewer_heard_ms: u64) {
        let question_done = {
            let st = self.state.lock().unwrap();
            st.turns.last().is_some_and(|t| {
                let last = t
                    .question
                    .last()
                    .map_or(0, |s| s.end_ms)
                    .max(interviewer_heard_ms);
                !t.question.is_empty() && t.answer.is_empty() && now_ms >= last + QUESTION_DONE_MS
            })
        };
        if question_done {
            self.suggest_answer(now_ms, true);
        }
        let silent_answer = {
            let st = self.state.lock().unwrap();
            st.turns.last().is_some_and(|t| {
                let last_heard = t.answer.last().map_or(0, |s| s.end_ms).max(heard_until_ms);
                needs_analysis(t) && now_ms >= last_heard + CLOSE_AFTER_SILENCE_MS
            })
        };
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

    /// O que responder à pergunta do entrevistador. Começa enquanto ele
    /// ainda pergunta (a cada `ANSWER_EVERY_WORDS` palavras novas) e de novo quando ele
    /// para de falar, pra estar na tela quando for a sua vez.
    fn suggest_answer(self: &Arc<Self>, now_ms: u64, closing: bool) {
        let llm = self.llm.clone();
        let (user, turn_index) = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.len().checked_sub(1) else {
                return;
            };
            if st.turns[index].question.is_empty() {
                return;
            }
            // Você já começou a responder com a fala na tela: ela não muda mais.
            if !st.turns[index].answer.is_empty() && st.answers.contains_key(&index) {
                return;
            }
            let words = word_count(&join(&st.turns[index].question));
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
            if st.suggest.in_flight || !enough {
                return;
            }
            st.suggest = SuggestState {
                in_flight: true,
                words_at_request: words,
                turn: index,
            };
            (self.answer_prompt(&st, index, now_ms), index)
        };

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let schema = crate::profile::answer_schema();
            let prompt = Prompt {
                system: crate::profile::ANSWER_SYSTEM,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                max_tokens: 2500,
            };
            let result = llm.complete_json(&prompt).await;
            let (current, at_ms) = {
                let mut st = this.state.lock().unwrap();
                st.suggest.in_flight = false;
                let at = st
                    .turns
                    .get(turn_index)
                    .and_then(|t| t.question.last())
                    .map_or(now_ms, |s| s.end_ms);
                // O entrevistador já passou pra outra pergunta: esta perdeu a hora.
                (st.turns.len() == turn_index + 1, at)
            };
            match result {
                Ok((value, usage)) => {
                    let _ = this.db.insert_usage(this.session_id, "resposta", &usage);
                    if current {
                        this.show_answer(&value, turn_index, at_ms, usage.latency_ms);
                    }
                }
                Err(e) => this.status("erro", Some(e), None),
            }
            if current {
                // A pergunta continuou enquanto esta estava no ar.
                this.suggest_answer(at_ms, false);
            }
        });
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
        if let Some(text) = st.answers.get(&index) {
            out.push_str(&format!("\nFala na tela agora pra esta pergunta:\n{text}\n"));
        }
        out.push_str("\nConversa recente (a última pergunta pode estar em andamento):\n");
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index],
        );
        out
    }

    /// Mostra a fala da pergunta. Se você já começou a responder, a que está na tela
    /// fica (trocar o texto no meio da fala atrapalha).
    fn show_answer(&self, value: &Value, turn_index: usize, t_ms: u64, latency_ms: u64) {
        let text = value["fala"].as_str().unwrap_or_default().trim().to_string();
        if text.is_empty() {
            return;
        }
        let answer = {
            let mut st = self.state.lock().unwrap();
            let Some(turn) = st.turns.get(turn_index) else {
                return;
            };
            let started = !turn.answer.is_empty();
            let question = join(&turn.question);
            match st.answers.get(&turn_index) {
                Some(_) if started => return,
                Some(shown) if *shown == text => return,
                _ => {}
            }
            st.answers.insert(turn_index, text.clone());
            Answer {
                turn: turn_index,
                t_ms,
                question,
                text,
                latency_ms,
            }
        };
        let _ = self
            .db
            .insert_card(self.session_id, t_ms, "resposta", &answer.text, "media");
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
        push_turns(&mut out, &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index]);
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
