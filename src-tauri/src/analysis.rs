//! Análise ao vivo. Agrupa as falas em turnos (pergunta do entrevistador + resposta do
//! candidato), chama a LLM a cada resposta fechada e mantém a cobertura das
//! competências do kit. Perguntas proibidas são checadas localmente, sem LLM, na hora.
//!
//! Uma resposta fecha quando o entrevistador volta a falar ou quando o candidato fica
//! em silêncio por `CLOSE_AFTER_SILENCE_MS`. Se uma resposta nova fechar antes de a
//! análise anterior voltar, o resultado antigo é descartado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::audio::Source;
use crate::db::{Db, Segment};
use crate::kit::{Kit, fold};
use crate::llm::{LlmClient, Prompt};

const CLOSE_AFTER_SILENCE_MS: u64 = 5_000;
/// Turnos anteriores mandados junto com o atual, pra dar contexto.
const CONTEXT_TURNS: usize = 3;
const MAX_CARDS_PER_ANSWER: usize = 2;

const SYSTEM: &str = "\
Você é o copiloto de quem está conduzindo uma entrevista de emprego, em português do Brasil. \
Você recebe o kit da entrevista, o currículo do candidato e as trocas recentes. Analise só a \
última resposta do candidato e devolva:

1. cobertura: as competências do kit que essa resposta tocou, com o nível de evidência \
(parcial ou forte, comparando com os sinais do kit) e um trecho curto copiado exatamente da \
resposta que justifica. Sem trecho, não inclua a competência.
2. cards: no máximo 2 sugestões curtas (até 15 palavras), no imperativo, pro entrevistador \
usar agora. Tipos: aprofundar (algo vago ou dito no plural sem dizer a parte da pessoa), \
vaga (resposta sem resultado ou exemplo concreto), contradicao (com o currículo ou com algo \
dito antes), nao_tocada (competência importante ainda sem evidência, considerando o tempo e o \
roteiro), indutiva (a pergunta sugeria a resposta ou era fechada demais), positivo (evidência \
forte que vale registrar; use pouco). Se não houver nada útil, devolva cards vazio.
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
    /// `analisando`, `ok`, `descartada` ou `erro`.
    Status {
        state: String,
        message: Option<String>,
        latency_ms: Option<u64>,
    },
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
}

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
        out.push_str("\nTrocas recentes (analise a última resposta):\n");
        for turn in &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index] {
            for s in turn.question.iter().chain(&turn.answer) {
                let who = if s.source == Source::Mic {
                    "Entrevistador"
                } else {
                    "Candidato"
                };
                out.push_str(&format!("[{}] {who}: {}\n", clock(s.start_ms), s.text));
            }
        }
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
                            "tipo": {
                                "type": "string",
                                "enum": ["aprofundar", "vaga", "contradicao", "nao_tocada", "indutiva", "positivo"]
                            },
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
