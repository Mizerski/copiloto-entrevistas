//! Análise ao vivo. Agrupa as falas em turnos (pergunta do entrevistador + resposta do
//! candidato) e faz duas chamadas à LLM:
//!
//! - Sugestão: a próxima pergunta, pronta pra ler em voz alta. Roda enquanto o candidato
//!   ainda fala (a cada `SUGGEST_EVERY_WORDS` palavras novas), com resposta curta, pra
//!   já estar na tela quando ele parar.
//! - Análise: cobertura das competências e resumo, a cada resposta fechada. Não tem
//!   pressa.
//!
//! Perguntas proibidas são checadas localmente, sem LLM, na hora.
//!
//! Uma resposta fecha quando o entrevistador volta a falar (mais que um "uhum") ou quando
//! o candidato fica em silêncio por `CLOSE_AFTER_SILENCE_MS`. Resultado que chega depois
//! de um mais novo é descartado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::audio::Source;
use crate::db::{Db, Segment};
use crate::kit::{Kit, fold};
use crate::llm::{LlmClient, Prompt};

const CLOSE_AFTER_SILENCE_MS: u64 = 3_000;
/// Turnos anteriores mandados junto com o atual, pra dar contexto.
const CONTEXT_TURNS: usize = 3;
const MAX_CARDS_PER_ANSWER: usize = 1;
/// Palavras novas da resposta que disparam uma sugestão enquanto o candidato fala.
const SUGGEST_EVERY_WORDS: usize = 12;
/// Fala do entrevistador com até tantas palavras no meio da resposta é só "uhum",
/// "entendi": não fecha a resposta.
const BACKCHANNEL_WORDS: usize = 3;

const SUGGEST_SYSTEM: &str = "\
Você sopra a próxima pergunta pra quem está conduzindo uma entrevista, em português do Brasil, \
ao vivo. Você recebe o kit da entrevista (vaga, competências, roteiro), o currículo e as trocas \
recentes. A última resposta do candidato pode estar no meio.

Devolva a melhor pergunta pra fazer assim que o candidato parar de falar:
- Escrita exatamente como o entrevistador falaria em voz alta, na segunda pessoa, em tom de \
conversa, com até 25 palavras. Nada de instrução (\"pergunte sobre...\"): a pergunta em si.
- Ancorada no que o candidato acabou de dizer, de preferência retomando as palavras dele, e no \
que a vaga precisa avaliar.
- Prefira, nesta ordem: pedir um exemplo concreto ou a parte da pessoa quando a resposta foi \
vaga ou no plural; checar algo que contradiz o currículo ou o que foi dito antes; levar pra uma \
competência importante ainda sem evidência; seguir o roteiro.
- Pergunta aberta, que não sugere a resposta.
- motivo: até 8 palavras dizendo o que a pergunta busca.
- Não repita uma pergunta da lista de já sugeridas, nem com outras palavras.
- Se a resposta ainda está no começo e não dá pra saber o que perguntar, devolva pergunta vazia.

Avalie só o conteúdo. Nunca pergunte sobre idade, família, religião, saúde, origem ou outra \
característica pessoal. A transcrição é automática e pode ter palavras erradas.";

const SYSTEM: &str = "\
Você é o copiloto de quem está conduzindo uma entrevista de emprego, em português do Brasil. \
Você recebe o kit da entrevista, o currículo do candidato e as trocas recentes. Analise só a \
última resposta do candidato e devolva:

1. cobertura: as competências do kit que essa resposta tocou, com o nível de evidência \
(parcial ou forte, comparando com os sinais do kit) e um trecho curto copiado exatamente da \
resposta que justifica. Sem trecho, não inclua a competência.
2. cards: a próxima pergunta já é sugerida em outro lugar; aqui vai no máximo 1 observação \
curta (até 15 palavras) sobre a condução. Tipos: indutiva (a pergunta do entrevistador \
sugeria a resposta ou era fechada demais), positivo (evidência forte que vale registrar; use \
pouco). Não repita uma observação da lista de cards mostrados. Quase sempre, devolva cards \
vazio.
3. resumo: o resumo atualizado da conversa inteira até agora, em até 100 palavras.

Regras:
- Avalie só o conteúdo do que foi dito. Nunca comente sotaque, jeito de falar, nervosismo, \
gênero, idade, aparência ou qualquer característica pessoal.
- Não recomende contratar ou não contratar.
- Use os nomes das competências exatamente como estão no kit.
- A transcrição é automática e pode ter palavras erradas; não trate erro de transcrição como \
erro do candidato.";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    None,
    Partial,
    Strong,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageItem {
    pub competency: String,
    pub level: Level,
    /// Trecho da resposta que sustenta o nível.
    pub evidence: String,
    /// Quando a evidência foi dita (relógio da sessão).
    pub at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveCard {
    pub id: i64,
    pub t_ms: u64,
    pub kind: String,
    pub text: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AnalysisEvent {
    Card {
        card: LiveCard,
    },
    Coverage {
        items: Vec<CoverageItem>,
    },
    /// A próxima pergunta sugerida. Substitui a anterior na tela.
    Suggestion {
        suggestion: Suggestion,
    },
    /// `analisando`, `ok`, `descartada` ou `erro`.
    Status {
        state: String,
        message: Option<String>,
        latency_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: i64,
    pub t_ms: u64,
    pub kind: String,
    pub question: String,
    pub why: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub name: String,
    pub resume: String,
    pub checkpoints: String,
}

#[derive(Default, Clone)]
struct Turn {
    question: Vec<Segment>,
    answer: Vec<Segment>,
    /// Quantas falas da resposta já foram analisadas.
    analyzed: usize,
}

#[derive(Clone)]
struct CurrentBlock {
    name: String,
    minutes: Option<u32>,
    started_ms: u64,
}

struct State {
    turns: Vec<Turn>,
    coverage: Vec<CoverageItem>,
    summary: String,
    block: Option<CurrentBlock>,
    /// Textos dos últimos cards mostrados, pra não repetir.
    shown_cards: Vec<String>,
    /// Perguntas já sugeridas, pra não repetir.
    suggested: Vec<String>,
    suggest: SuggestState,
}

#[derive(Default)]
struct SuggestState {
    in_flight: bool,
    /// Palavras da resposta atual quando a última sugestão foi pedida.
    words_at_request: usize,
    /// Turno da última sugestão pedida.
    turn: usize,
}

/// Quantos cards recentes entram no prompt e na checagem de repetição.
const RECENT_CARDS: usize = 8;

pub struct Analyzer {
    session_id: i64,
    db: Arc<Db>,
    llm: Option<Arc<LlmClient>>,
    kit: Kit,
    stable_context: String,
    state: Mutex<State>,
    generation: AtomicU64,
    emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
}

impl Analyzer {
    /// Sem `llm`, só faz a checagem local de perguntas proibidas.
    pub fn new(
        session_id: i64,
        db: Arc<Db>,
        llm: Option<LlmClient>,
        kit: Kit,
        candidate: &Candidate,
        emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
    ) -> Arc<Self> {
        let mut stable_context = kit.to_prompt();
        stable_context.push_str("\n## Candidato\n");
        if !candidate.name.is_empty() {
            stable_context.push_str(&format!("Nome: {}\n", candidate.name));
        }
        if !candidate.resume.is_empty() {
            stable_context.push_str(&format!("Currículo:\n{}\n", candidate.resume));
        }
        if !candidate.checkpoints.is_empty() {
            stable_context.push_str(&format!("Pontos a checar:\n{}\n", candidate.checkpoints));
        }
        let coverage = kit
            .competencies
            .iter()
            .map(|c| CoverageItem {
                competency: c.name.clone(),
                level: Level::None,
                evidence: String::new(),
                at_ms: 0,
            })
            .collect();
        Arc::new(Analyzer {
            session_id,
            db,
            llm: llm.map(Arc::new),
            kit,
            stable_context,
            state: Mutex::new(State {
                turns: Vec::new(),
                coverage,
                summary: String::new(),
                block: None,
                shown_cards: Vec::new(),
                suggested: Vec::new(),
                suggest: SuggestState::default(),
            }),
            generation: AtomicU64::new(0),
            emit,
        })
    }

    pub fn coverage(&self) -> Vec<CoverageItem> {
        self.state.lock().unwrap().coverage.clone()
    }

    pub fn set_block(&self, name: String, minutes: Option<u32>, started_ms: u64) {
        self.state.lock().unwrap().block = Some(CurrentBlock {
            name,
            minutes,
            started_ms,
        });
    }

    /// Uma fala confirmada (já sem eco).
    pub fn on_segment(self: &Arc<Self>, segment: &Segment) {
        if segment.echo {
            return;
        }
        if segment.source == Source::Mic
            && let Some(phrase) = self.kit.forbidden_in(&segment.text)
        {
            self.card(
                segment.end_ms,
                "proibida",
                &format!("Pergunta proibida (\"{phrase}\"). Mude de assunto."),
                "alta",
            );
        }
        let close = {
            let mut st = self.state.lock().unwrap();
            match segment.source {
                Source::Mic => {
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
                }
                Source::Sistema => {
                    if st.turns.is_empty() {
                        st.turns.push(Turn::default());
                    }
                    st.turns.last_mut().unwrap().answer.push(segment.clone());
                    false
                }
            }
        };
        if close {
            self.analyze(segment.end_ms);
        } else if segment.source == Source::Sistema {
            self.suggest(segment.end_ms, false);
        }
    }

    /// `heard_until_ms`: até quando o candidato foi ouvido, contando os parciais. As
    /// falas finais chegam a cada poucos segundos e com atraso; olhar só pra elas
    /// confunde pausa de transcrição com fim de resposta.
    pub fn tick(self: &Arc<Self>, now_ms: u64, heard_until_ms: u64) {
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
        let Some(llm) = self.llm.clone() else { return };
        self.suggest(now_ms, true);
        let (user, answer_text, answer_end) = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.iter().rposition(|t| !t.answer.is_empty()) else {
                return;
            };
            let turn = &mut st.turns[index];
            turn.analyzed = turn.answer.len();
            let answer_text = join(&turn.answer);
            let answer_end = turn.answer.last().map(|s| s.end_ms).unwrap_or(now_ms);
            (
                self.user_prompt(&st, index, now_ms),
                answer_text,
                answer_end,
            )
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.status("analisando", None, None);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let schema = this.schema();
            let prompt = Prompt {
                system: SYSTEM,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                max_tokens: 4000,
            };
            let result = llm.complete_json(&prompt).await;
            if this.generation.load(Ordering::SeqCst) != generation {
                this.status(
                    "descartada",
                    Some("chegou depois de uma resposta mais nova".into()),
                    None,
                );
                return;
            }
            match result {
                Ok((value, usage)) => {
                    let _ = this.db.insert_usage(this.session_id, "analise", &usage);
                    this.apply(&value, &answer_text, answer_end);
                    this.status("ok", None, Some(usage.latency_ms));
                }
                Err(e) => this.status("erro", Some(e), None),
            }
        });
    }

    /// Pede a próxima pergunta. Durante a resposta, só a cada `SUGGEST_EVERY_WORDS`
    /// palavras novas; no fim (`closing`), sempre que houver algo novo. Uma de cada vez:
    /// se chegar fala nova enquanto uma está no ar, outra sai quando ela voltar.
    fn suggest(self: &Arc<Self>, now_ms: u64, closing: bool) {
        let Some(llm) = self.llm.clone() else { return };
        let (user, turn_index) = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.iter().rposition(|t| !t.answer.is_empty()) else {
                return;
            };
            let words = word_count(&join(&st.turns[index].answer));
            let seen = if st.suggest.turn == index {
                st.suggest.words_at_request
            } else {
                0
            };
            let enough = if closing {
                words > seen
            } else {
                words >= seen + SUGGEST_EVERY_WORDS
            };
            if st.suggest.in_flight || !enough {
                return;
            }
            st.suggest = SuggestState {
                in_flight: true,
                words_at_request: words,
                turn: index,
            };
            (self.suggest_prompt(&st, index, now_ms), index)
        };

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let schema = suggest_schema();
            let prompt = Prompt {
                system: SUGGEST_SYSTEM,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                max_tokens: 400,
            };
            let result = llm.complete_json(&prompt).await;
            let (current, at_ms) = {
                let mut st = this.state.lock().unwrap();
                st.suggest.in_flight = false;
                let at = st
                    .turns
                    .get(turn_index)
                    .and_then(|t| t.answer.last())
                    .map_or(now_ms, |s| s.end_ms);
                // Se o entrevistador já começou outra pergunta, a sugestão perdeu a hora.
                (st.turns.len() == turn_index + 1, at)
            };
            match result {
                Ok((value, usage)) => {
                    let _ = this.db.insert_usage(this.session_id, "sugestao", &usage);
                    if current {
                        this.show_suggestion(&value, at_ms, usage.latency_ms);
                    }
                }
                Err(e) => this.status("erro", Some(e), None),
            }
            if current {
                // Chegou mais fala enquanto esta estava no ar.
                this.suggest(at_ms, false);
            }
        });
    }

    fn show_suggestion(&self, value: &Value, t_ms: u64, latency_ms: u64) {
        let question = value["pergunta"].as_str().unwrap_or_default().trim();
        if question.is_empty() {
            return;
        }
        let kind = value["tipo"].as_str().unwrap_or("aprofundar");
        {
            let mut st = self.state.lock().unwrap();
            if st.suggested.iter().any(|q| similar(q, question)) {
                return;
            }
            st.suggested.push(question.to_string());
            let excess = st.suggested.len().saturating_sub(RECENT_CARDS);
            st.suggested.drain(..excess);
        }
        let id = self
            .db
            .insert_card(self.session_id, t_ms, kind, question, "alta")
            .unwrap_or(0);
        (self.emit)(AnalysisEvent::Suggestion {
            suggestion: Suggestion {
                id,
                t_ms,
                kind: kind.into(),
                question: question.into(),
                why: value["motivo"].as_str().unwrap_or_default().trim().into(),
                latency_ms,
            },
        });
    }

    fn suggest_prompt(&self, st: &State, index: usize, now_ms: u64) -> String {
        let mut out = format!("Tempo de entrevista: {}.\n", clock(now_ms));
        if let Some(b) = &st.block {
            out.push_str(&format!("Bloco atual do roteiro: {}.\n", b.name));
        }
        let missing: Vec<&str> = st
            .coverage
            .iter()
            .filter(|c| c.level == Level::None)
            .map(|c| c.competency.as_str())
            .collect();
        if !missing.is_empty() {
            out.push_str(&format!("Competências ainda sem evidência: {}.\n", missing.join(", ")));
        }
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        if !st.suggested.is_empty() {
            out.push_str("\nPerguntas já sugeridas (não repita):\n");
            for q in &st.suggested {
                out.push_str(&format!("- {q}\n"));
            }
        }
        out.push_str("\nTrocas recentes (a última resposta pode estar em andamento):\n");
        push_turns(&mut out, &st.turns[index.saturating_sub(2)..=index]);
        out
    }

    fn apply(&self, value: &Value, answer_text: &str, answer_end: u64) {
        let answer_words = fold(answer_text);
        let mut changed = false;
        let (coverage, summary) = {
            let mut st = self.state.lock().unwrap();
            for item in value["cobertura"].as_array().into_iter().flatten() {
                let name = item["competencia"].as_str().unwrap_or_default();
                let evidence = item["trecho"].as_str().unwrap_or_default().trim();
                let level = match item["nivel"].as_str() {
                    Some("forte") => Level::Strong,
                    Some("parcial") => Level::Partial,
                    _ => continue,
                };
                // Avaliação sem trecho que esteja de fato na resposta não entra.
                if !quote_in(evidence, &answer_words) {
                    continue;
                }
                if let Some(c) = st
                    .coverage
                    .iter_mut()
                    .find(|c| fold(&c.competency) == fold(name))
                    && level > c.level
                {
                    c.level = level;
                    c.evidence = evidence.to_string();
                    c.at_ms = answer_end;
                    changed = true;
                }
            }
            if let Some(summary) = value["resumo"].as_str().filter(|s| !s.trim().is_empty()) {
                st.summary = summary.trim().to_string();
            }
            (st.coverage.clone(), st.summary.clone())
        };
        if changed {
            (self.emit)(AnalysisEvent::Coverage {
                items: coverage.clone(),
            });
        }
        if let Ok(json) = serde_json::to_string(&coverage) {
            let _ = self.db.save_coverage(self.session_id, &json, &summary);
        }
        for card in value["cards"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_CARDS_PER_ANSWER)
        {
            let text = card["texto"].as_str().unwrap_or_default().trim();
            if text.is_empty() {
                continue;
            }
            self.card(
                answer_end,
                card["tipo"].as_str().unwrap_or("aprofundar"),
                text,
                card["prioridade"].as_str().unwrap_or("media"),
            );
        }
    }

    fn card(&self, t_ms: u64, kind: &str, text: &str, priority: &str) {
        {
            let mut st = self.state.lock().unwrap();
            // O modelo às vezes repete a mesma sugestão com outras palavras.
            if kind != "proibida" && st.shown_cards.iter().any(|c| similar(c, text)) {
                return;
            }
            st.shown_cards.push(text.to_string());
            let excess = st.shown_cards.len().saturating_sub(RECENT_CARDS);
            st.shown_cards.drain(..excess);
        }
        let id = self
            .db
            .insert_card(self.session_id, t_ms, kind, text, priority)
            .unwrap_or(0);
        (self.emit)(AnalysisEvent::Card {
            card: LiveCard {
                id,
                t_ms,
                kind: kind.into(),
                text: text.into(),
                priority: priority.into(),
            },
        });
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
        if let Some(b) = &st.block {
            let elapsed = clock(now_ms.saturating_sub(b.started_ms));
            match b.minutes {
                Some(m) => out.push_str(&format!(
                    "Bloco atual do roteiro: {} (planejado {m} min, {elapsed} decorridos).\n",
                    b.name
                )),
                None => out.push_str(&format!(
                    "Bloco atual do roteiro: {} ({elapsed}).\n",
                    b.name
                )),
            }
        }
        out.push_str("\nCobertura até agora:\n");
        for c in &st.coverage {
            let level = match c.level {
                Level::None => "nenhuma",
                Level::Partial => "parcial",
                Level::Strong => "forte",
            };
            out.push_str(&format!("- {}: {level}\n", c.competency));
        }
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        if !st.shown_cards.is_empty() {
            out.push_str("\nCards já mostrados ao entrevistador (não repita):\n");
            for c in &st.shown_cards {
                out.push_str(&format!("- {c}\n"));
            }
        }
        out.push_str("\nTrocas recentes (analise a última resposta):\n");
        push_turns(&mut out, &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index]);
        out
    }

    fn schema(&self) -> Value {
        let names: Vec<&str> = self
            .kit
            .competencies
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        let competency = if names.is_empty() {
            json!({ "type": "string" })
        } else {
            json!({ "type": "string", "enum": names })
        };
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["cobertura", "cards", "resumo"],
            "properties": {
                "cobertura": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["competencia", "nivel", "trecho"],
                        "properties": {
                            "competencia": competency,
                            "nivel": { "type": "string", "enum": ["parcial", "forte"] },
                            "trecho": { "type": "string" }
                        }
                    }
                },
                "cards": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["tipo", "texto", "prioridade"],
                        "properties": {
                            "tipo": { "type": "string", "enum": ["indutiva", "positivo"] },
                            "texto": { "type": "string" },
                            "prioridade": { "type": "string", "enum": ["alta", "media", "baixa"] }
                        }
                    }
                },
                "resumo": { "type": "string" }
            }
        })
    }
}

fn suggest_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["pergunta", "tipo", "motivo"],
        "properties": {
            "pergunta": { "type": "string" },
            "tipo": {
                "type": "string",
                "enum": ["aprofundar", "vaga", "contradicao", "nao_tocada", "roteiro"]
            },
            "motivo": { "type": "string" }
        }
    })
}

fn push_turns(out: &mut String, turns: &[Turn]) {
    let mut segments: Vec<&Segment> = turns
        .iter()
        .flat_map(|t| t.question.iter().chain(&t.answer))
        .collect();
    // O "uhum" do entrevistador entra na resposta; na ordem do tempo fica legível.
    segments.sort_by_key(|s| s.start_ms);
    for s in segments {
        let who = if s.source == Source::Mic {
            "Entrevistador"
        } else {
            "Candidato"
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

/// Dois cards dizem quase a mesma coisa: 60% das palavras com mais de 3 letras em comum.
fn similar(a: &str, b: &str) -> bool {
    let words = |t: &str| -> std::collections::HashSet<String> {
        fold(t)
            .split(' ')
            .filter(|w| w.chars().count() > 3)
            .map(String::from)
            .collect()
    };
    let (a, b) = (words(a), words(b));
    let smaller = a.len().min(b.len());
    smaller > 0 && a.intersection(&b).count() as f32 / smaller as f32 >= 0.6
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
