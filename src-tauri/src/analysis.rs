//! Análise ao vivo. Agrupa as falas em turnos (pergunta do entrevistador + resposta do
//! candidato) e faz duas chamadas à LLM:
//!
//! - Sugestões: três balões fixos (aprofundar, checar, avançar), cada um com uma pergunta
//!   pronta pra ler em voz alta. Rodam enquanto o candidato ainda fala (a cada
//!   `SUGGEST_EVERY_WORDS` palavras novas), com resposta curta, pra já estarem na tela
//!   quando ele parar. Um balão só troca quando a pergunta muda de verdade, e some quando
//!   o entrevistador faz a pergunta (ou clica nele).
//! - Análise: cobertura das competências, resumo e os fatos que o candidato afirmou, a
//!   cada resposta fechada. Não tem pressa.
//!
//! Os fatos formam a memória da entrevista: o balão "checar" compara a resposta atual com
//! eles e com o currículo. Uma contradição só aparece com os dois trechos conferidos na
//! transcrição (ou no currículo), pra não acusar ninguém por erro do modelo.
//!
//! Perguntas proibidas são checadas localmente, sem LLM, na hora.
//!
//! No modo candidato (`Side::Candidate`), quem usa o app é quem está sendo entrevistado:
//! o microfone é o candidato e a chamada é o entrevistador. Quando o entrevistador
//! pergunta, os balões mostram como responder com o que está no perfil da pessoa (nunca
//! inventando experiência), a história do perfil que serve de exemplo e o que exige
//! atenção (contradição com o que ela já disse, pergunta em duas partes).
//!
//! No modo treino (`practice`), os papéis se invertem: quem usa o app é o candidato (o
//! microfone) e o entrevistador é uma IA, cuja fala entra como a outra ponta. Os balões
//! viram dicas pra quem treina, e a cada resposta fechada a IA prepara a próxima fala.
//!
//! Uma resposta fecha quando o entrevistador volta a falar (mais que um "uhum") ou quando
//! o candidato fica em silêncio por `CLOSE_AFTER_SILENCE_MS`. Resultado que chega depois
//! de um mais novo é descartado.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};

use crate::audio::Source;
use crate::db::{Db, Segment};
use crate::kit::{Kit, fold};
use crate::llm::{LlmClient, Prompt};
use crate::practice::{self, Persona};

const CLOSE_AFTER_SILENCE_MS: u64 = 3_000;
/// Turnos anteriores mandados junto com o atual, pra dar contexto.
const CONTEXT_TURNS: usize = 3;
const MAX_CARDS_PER_ANSWER: usize = 1;
/// Palavras novas da resposta que disparam uma sugestão enquanto o candidato fala.
const SUGGEST_EVERY_WORDS: usize = 12;
/// Fala do entrevistador com até tantas palavras no meio da resposta é só "uhum",
/// "entendi": não fecha a resposta.
const BACKCHANNEL_WORDS: usize = 3;
/// Os balões, na ordem da tela.
const SLOTS: [&str; 3] = ["aprofundar", "checar", "avancar"];
/// Os balões do treino: dicas pra quem está respondendo.
const PRACTICE_SLOTS: [&str; 3] = ["faltou", "cuidado", "estrutura"];
/// Os balões do modo candidato: o que responder à pergunta do entrevistador.
const CANDIDATE_SLOTS: [&str; 3] = ["responda", "historia", "atencao"];
/// Modo candidato: palavras novas da pergunta que disparam outra sugestão.
const ANSWER_EVERY_WORDS: usize = 6;
/// Modo candidato: silêncio do entrevistador que fecha a pergunta.
const QUESTION_DONE_MS: u64 = 1_200;
/// Conversa mandada pro entrevistador simulado (trocas mais recentes).
const INTERVIEWER_TURNS: usize = 6;
/// Fatos guardados na memória da entrevista (os mais recentes).
const MAX_FACTS: usize = 60;

const SUGGEST_SYSTEM: &str = "\
Você sopra perguntas pra quem está conduzindo uma entrevista, em português do Brasil, ao vivo. \
Você recebe o kit da entrevista (vaga, competências, roteiro), o currículo, os fatos que o \
candidato já afirmou e as trocas recentes. A última resposta do candidato pode estar no meio.

Devolva três balões, cada um com uma função:
- aprofundar: retoma algo que o candidato acabou de dizer, de preferência com as palavras dele. \
Pede um exemplo concreto, a parte dele quando falou no plural, números, como decidiu.
- checar: tipo contradicao sempre que a resposta atual não bate com um fato dito antes ou com o \
currículo (ex.: disse que sempre jogou sozinho e agora conta algo feito em grupo). Compare a \
resposta com cada fato da lista antes de escolher outro tipo. \
Preencha trecho_antes (copiado exatamente do trecho do fato ou do currículo) e trecho_agora \
(copiado exatamente da resposta atual), e faça a pergunta convidando a esclarecer, sem acusar \
(\"Me ajuda a entender: antes você falou..., agora...\"). Detalhe que mudou pouco, \
complemento ou erro de transcrição não é contradição. Sem contradição, use tipo curriculo (um \
ponto do currículo ou dos pontos a checar ainda não verificado, de preferência ligado à \
conversa) ou vaga (a resposta ficou sem exemplo, resultado ou a parte dele), com os trechos \
vazios.
- avancar: leva a conversa adiante: a próxima pergunta do roteiro que ainda não foi feita \
(tipo roteiro) ou uma competência importante ainda sem evidência (tipo nao_tocada), \
considerando o bloco atual e o tempo.

Cada pergunta:
- Escrita exatamente como o entrevistador falaria em voz alta, na segunda pessoa, em tom de \
conversa, com até 25 palavras. Nada de instrução (\"pergunte sobre...\"): a pergunta em si.
- Aberta, sem sugerir a resposta.
- motivo: até 8 palavras dizendo o que a pergunta busca.

Se um balão que já está na tela ainda serve, devolva o mesmo texto. Não repita perguntas que o \
entrevistador já fez. Se não houver nada bom pra um balão, devolva pergunta vazia nele.

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
4. fatos: até 5 afirmações concretas que o candidato fez nesta resposta e que daria pra \
contradizer depois (cargo, empresa, tempo, números, datas, tecnologias, o que ele fez \
pessoalmente e o que foi do time, preferências, hábitos). Cada uma resumida em até 12 palavras, \
com o trecho copiado exatamente da resposta.

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
    /// Pergunta nova num balão. Substitui a que estava naquela posição.
    Suggestion {
        suggestion: Suggestion,
    },
    /// O entrevistador fez a pergunta do balão (ou clicou nele): o balão esvazia.
    SuggestionUsed {
        slot: String,
        id: i64,
    },
    /// Treino: o entrevistador simulado está preparando a próxima fala.
    InterviewerThinking,
    /// Treino: a próxima fala do entrevistador, pra ler em voz alta. `end`: é a
    /// despedida.
    InterviewerSays {
        text: String,
        end: bool,
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
    /// Posição na tela: `aprofundar`, `checar` ou `avancar`.
    pub slot: String,
    pub t_ms: u64,
    pub kind: String,
    pub question: String,
    pub why: String,
    /// Só em contradição: o que foi dito antes e o que foi dito agora.
    pub earlier: Option<Quote>,
    pub now: Option<Quote>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub text: String,
    /// Quando foi dito; `None` quando o trecho é do currículo.
    pub at_ms: Option<u64>,
}

/// Uma afirmação do candidato, guardada pra comparar com o que ele disser depois.
struct Fact {
    at_ms: u64,
    claim: String,
    quote: String,
}

/// De que lado da entrevista o app está.
#[derive(Debug, Clone)]
pub enum Side {
    /// Quem usa o app entrevista (o microfone pergunta).
    Interviewer,
    /// Treino: uma IA entrevista, com esta persona.
    Practice(&'static Persona),
    /// Quem usa o app está sendo entrevistado. O perfil vai em `Candidate::resume`.
    Candidate { job: String },
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
    /// Balões na tela, no máximo um por posição.
    slots: Vec<Suggestion>,
    /// Perguntas sugeridas que o entrevistador já fez, pra não repetir.
    asked: Vec<String>,
    facts: Vec<Fact>,
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
    /// Currículo e pontos a checar, pra conferir trechos citados em contradições.
    resume: String,
    state: Mutex<State>,
    generation: AtomicU64,
    emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
    /// Quem pergunta: o microfone numa entrevista real, a outra ponta no treino.
    interviewer: Source,
    practice: Option<&'static Persona>,
    /// Modo candidato: o app ajuda quem responde.
    candidate_side: bool,
    /// Treino: a IA está preparando ou falando a próxima fala.
    interviewer_busy: AtomicBool,
    /// Treino: a pessoa respondeu desde a última fala da IA.
    answered: AtomicBool,
    /// Treino: a IA já se despediu.
    ended: AtomicBool,
}

impl Analyzer {
    /// Sem `llm`, só faz a checagem local de perguntas proibidas.
    pub fn new(
        session_id: i64,
        db: Arc<Db>,
        llm: Option<LlmClient>,
        kit: Kit,
        candidate: &Candidate,
        side: Side,
        emit: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
    ) -> Arc<Self> {
        let practice = match side {
            Side::Practice(p) => Some(p),
            _ => None,
        };
        let stable_context = match &side {
            Side::Candidate { job } => {
                let mut out = String::new();
                if !job.trim().is_empty() {
                    out.push_str(&format!("# Vaga\n{}\n\n", job.trim()));
                }
                out.push_str(&format!(
                    "# Perfil de quem está sendo entrevistado (escrito pela própria pessoa)\n{}\n",
                    candidate.resume
                ));
                out
            }
            _ => {
                let mut out = kit.to_prompt();
                out.push_str("\n## Candidato\n");
                if !candidate.name.is_empty() {
                    out.push_str(&format!("Nome: {}\n", candidate.name));
                }
                if !candidate.resume.is_empty() {
                    out.push_str(&format!("Currículo:\n{}\n", candidate.resume));
                }
                if !candidate.checkpoints.is_empty() {
                    out.push_str(&format!("Pontos a checar:\n{}\n", candidate.checkpoints));
                }
                out
            }
        };
        let candidate_side = matches!(side, Side::Candidate { .. });
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
            resume: format!("{}\n{}", candidate.resume, candidate.checkpoints),
            state: Mutex::new(State {
                turns: Vec::new(),
                coverage,
                summary: String::new(),
                block: None,
                shown_cards: Vec::new(),
                slots: Vec::new(),
                asked: Vec::new(),
                facts: Vec::new(),
                suggest: SuggestState::default(),
            }),
            generation: AtomicU64::new(0),
            emit,
            interviewer: if practice.is_some() || candidate_side {
                Source::Sistema
            } else {
                Source::Mic
            },
            practice,
            candidate_side,
            interviewer_busy: AtomicBool::new(false),
            answered: AtomicBool::new(true),
            ended: AtomicBool::new(false),
        })
    }

    /// De onde vem a fala do candidato.
    pub fn candidate_source(&self) -> Source {
        match self.interviewer {
            Source::Mic => Source::Sistema,
            Source::Sistema => Source::Mic,
        }
    }

    fn slot_names(&self) -> [&'static str; 3] {
        if self.practice.is_some() {
            PRACTICE_SLOTS
        } else if self.candidate_side {
            CANDIDATE_SLOTS
        } else {
            SLOTS
        }
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
        let from_interviewer = segment.source == self.interviewer;
        if from_interviewer
            && self.interviewer == Source::Mic
            && let Some(phrase) = self.kit.forbidden_in(&segment.text)
        {
            self.card(
                segment.end_ms,
                "proibida",
                &format!("Pergunta proibida (\"{phrase}\"). Mude de assunto."),
                "alta",
            );
        }
        let (close, question, new_turn) = {
            let mut st = self.state.lock().unwrap();
            match from_interviewer {
                true => {
                    let previous_open = st.turns.last().is_some_and(|t| !t.answer.is_empty());
                    // No treino, toda fala da IA é uma vez dela, mesmo curta.
                    if previous_open
                        && self.practice.is_none()
                        && word_count(&segment.text) <= BACKCHANNEL_WORDS
                    {
                        st.turns.last_mut().unwrap().answer.push(segment.clone());
                        return;
                    }
                    let new_turn = previous_open || st.turns.is_empty();
                    if new_turn {
                        st.turns.push(Turn {
                            question: vec![segment.clone()],
                            ..Default::default()
                        });
                    } else {
                        st.turns.last_mut().unwrap().question.push(segment.clone());
                    }
                    // O entrevistador voltou a falar: a resposta anterior fechou.
                    let close = previous_open && needs_analysis(&st.turns[st.turns.len() - 2]);
                    (
                        close,
                        Some(join(&st.turns.last().unwrap().question)),
                        new_turn,
                    )
                }
                false => {
                    if st.turns.is_empty() {
                        st.turns.push(Turn::default());
                    }
                    st.turns.last_mut().unwrap().answer.push(segment.clone());
                    (false, None, false)
                }
            }
        };
        if self.candidate_side {
            if from_interviewer {
                // Pergunta nova: os balões eram da anterior.
                if new_turn {
                    self.take_slots(|_| true, false);
                }
                self.suggest_answer(segment.end_ms, false);
            }
            if close {
                self.analyze(segment.end_ms);
            }
            return;
        }
        match (question, self.practice) {
            // Treino: a IA falou de novo; as dicas eram da resposta anterior.
            (Some(_), Some(_)) => {
                self.interviewer_busy.store(false, Ordering::SeqCst);
                self.take_slots(|_| true, false);
            }
            (Some(question), None) => {
                self.take_slots(|s| spoken_covers(&question, &s.question), true);
            }
            (None, Some(_)) => {
                if !self.interviewer_busy.load(Ordering::SeqCst) {
                    self.answered.store(true, Ordering::SeqCst);
                }
            }
            (None, None) => {}
        }
        if close {
            self.analyze(segment.end_ms);
        } else if !from_interviewer {
            self.suggest(segment.end_ms, false);
        }
    }

    /// Treino: a IA abre a entrevista.
    pub fn start_practice(self: &Arc<Self>) {
        self.interviewer_next(0, true);
    }

    /// Treino: a pessoa pediu a próxima pergunta sem esperar (ou a anterior falhou).
    pub fn practice_next(self: &Arc<Self>, now_ms: u64) {
        self.interviewer_busy.store(false, Ordering::SeqCst);
        self.interviewer_next(now_ms, true);
    }

    /// Treino: pede a próxima fala do entrevistador simulado. Só quando a pessoa já
    /// respondeu à fala anterior (ou `force`), e uma de cada vez.
    fn interviewer_next(self: &Arc<Self>, now_ms: u64, force: bool) {
        let (Some(persona), Some(llm)) = (self.practice, self.llm.clone()) else {
            return;
        };
        if self.ended.load(Ordering::SeqCst) {
            return;
        }
        if !force && !self.answered.load(Ordering::SeqCst) {
            return;
        }
        if self.interviewer_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.answered.store(false, Ordering::SeqCst);
        let user = self.interviewer_prompt(persona, now_ms);
        (self.emit)(AnalysisEvent::InterviewerThinking);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let schema = practice::interviewer_schema();
            let prompt = Prompt {
                system: practice::INTERVIEWER_SYSTEM,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                max_tokens: 2500,
            };
            match llm.complete_json(&prompt).await {
                Ok((value, usage)) => {
                    let _ = this
                        .db
                        .insert_usage(this.session_id, "entrevistador", &usage);
                    let text = clean_speech(value["fala"].as_str().unwrap_or_default());
                    let end = value["encerrar"].as_bool().unwrap_or(false);
                    if text.is_empty() {
                        this.interviewer_busy.store(false, Ordering::SeqCst);
                        return;
                    }
                    if end {
                        this.ended.store(true, Ordering::SeqCst);
                    }
                    // Continua ocupada até a fala ser lida e voltar como segmento.
                    (this.emit)(AnalysisEvent::InterviewerSays { text, end });
                }
                Err(e) => {
                    this.interviewer_busy.store(false, Ordering::SeqCst);
                    this.status("erro", Some(format!("entrevistador: {e}")), None);
                }
            }
        });
    }

    fn interviewer_prompt(&self, persona: &Persona, now_ms: u64) -> String {
        let st = self.state.lock().unwrap();
        let planned: u32 = self.kit.blocks.iter().filter_map(|b| b.minutes).sum();
        let mut out = format!(
            "Seu estilo ({}): {}\n\nTempo de entrevista: {}{}.\n",
            persona.name,
            persona.style,
            clock(now_ms),
            if planned > 0 {
                format!(" de {planned} min planejados")
            } else {
                String::new()
            }
        );
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        let asked: Vec<String> = st
            .turns
            .iter()
            .filter(|t| !t.question.is_empty())
            .map(|t| join(&t.question))
            .collect();
        if asked.is_empty() {
            out.push_str(
                "\nA entrevista ainda não começou: faça a abertura e a primeira pergunta.\n",
            );
        } else {
            out.push_str("\nTudo o que você já falou, em ordem:\n");
            for q in &asked {
                out.push_str(&format!("- {q}\n"));
            }
            out.push_str("\nConversa recente:\n");
            let from = st.turns.len().saturating_sub(INTERVIEWER_TURNS);
            push_turns(&mut out, &st.turns[from..], self.interviewer);
        }
        out
    }

    /// `heard_until_ms`: até quando o candidato foi ouvido, contando os parciais. As
    /// falas finais chegam a cada poucos segundos e com atraso; olhar só pra elas
    /// confunde pausa de transcrição com fim de resposta.
    /// `interviewer_heard_ms`: o mesmo pra quem pergunta (fecha a pergunta no modo
    /// candidato).
    pub fn tick(self: &Arc<Self>, now_ms: u64, heard_until_ms: u64, interviewer_heard_ms: u64) {
        if self.candidate_side {
            let question_done = {
                let st = self.state.lock().unwrap();
                st.turns.last().is_some_and(|t| {
                    let last = t
                        .question
                        .last()
                        .map_or(0, |s| s.end_ms)
                        .max(interviewer_heard_ms);
                    !t.question.is_empty()
                        && t.answer.is_empty()
                        && now_ms >= last + QUESTION_DONE_MS
                })
            };
            if question_done {
                self.suggest_answer(now_ms, true);
            }
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

    /// Clicou no balão: conta como pergunta feita (ou dica vista, no treino).
    pub fn use_suggestion(&self, id: i64) {
        self.take_slots(|s| s.id == id, true);
    }

    /// Tira da tela os balões usados. `used_mark`: grava no banco como usado.
    fn take_slots(&self, used: impl Fn(&Suggestion) -> bool, used_mark: bool) {
        let taken: Vec<Suggestion> = {
            let mut st = self.state.lock().unwrap();
            let (taken, kept) = std::mem::take(&mut st.slots).into_iter().partition(&used);
            st.slots = kept;
            for s in &taken {
                st.asked.push(s.question.clone());
            }
            let excess = st.asked.len().saturating_sub(RECENT_CARDS);
            st.asked.drain(..excess);
            taken
        };
        for s in taken {
            if used_mark {
                let _ = self.db.mark_card_used(s.id);
            }
            (self.emit)(AnalysisEvent::SuggestionUsed {
                slot: s.slot,
                id: s.id,
            });
        }
    }

    fn analyze(self: &Arc<Self>, now_ms: u64) {
        let Some(llm) = self.llm.clone() else { return };
        self.suggest(now_ms, true);
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
        // Treino: a resposta fechou, a vez é do entrevistador.
        self.interviewer_next(now_ms, false);
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
            // Chegou depois da análise de uma resposta mais nova: cobertura e fatos desta
            // resposta continuam valendo, só o resumo é que ficou velho.
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

    /// Pede a próxima pergunta. Durante a resposta, só a cada `SUGGEST_EVERY_WORDS`
    /// palavras novas; no fim (`closing`), sempre que houver algo novo. Uma de cada vez:
    /// se chegar fala nova enquanto uma está no ar, outra sai quando ela voltar.
    fn suggest(self: &Arc<Self>, now_ms: u64, closing: bool) {
        if self.candidate_side {
            return;
        }
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
            let (system, schema) = if this.practice.is_some() {
                (practice::COACH_SYSTEM, practice::coach_schema())
            } else {
                (SUGGEST_SYSTEM, suggest_schema())
            };
            let prompt = Prompt {
                system,
                stable_context: &this.stable_context,
                user: &user,
                schema: &schema,
                // O esforço do modelo também gasta tokens: folga pra não cortar a resposta.
                max_tokens: 2500,
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
                        this.show_suggestions(&value, turn_index, at_ms, usage.latency_ms);
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

    /// Modo candidato: o que responder à pergunta do entrevistador. Começa enquanto ele
    /// ainda pergunta (a cada `ANSWER_EVERY_WORDS` palavras novas) e de novo quando ele
    /// para de falar, pra estar na tela quando for a sua vez.
    fn suggest_answer(self: &Arc<Self>, now_ms: u64, closing: bool) {
        let Some(llm) = self.llm.clone() else { return };
        let (user, turn_index) = {
            let mut st = self.state.lock().unwrap();
            let Some(index) = st.turns.len().checked_sub(1) else {
                return;
            };
            if st.turns[index].question.is_empty() {
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
                        this.show_suggestions(&value, turn_index, at_ms, usage.latency_ms);
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
        if !st.slots.is_empty() {
            out.push_str("\nBalões na tela agora:\n");
            for s in &st.slots {
                out.push_str(&format!("- {}: {}\n", s.slot, s.question));
            }
        }
        out.push_str("\nConversa recente (a última pergunta pode estar em andamento):\n");
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index],
            self.interviewer,
        );
        out
    }

    /// Atualiza os balões. Um balão só troca quando a pergunta nova é diferente da que
    /// está nele; pergunta vazia mantém o que está na tela.
    fn show_suggestions(&self, value: &Value, turn_index: usize, t_ms: u64, latency_ms: u64) {
        let mut shown = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            for slot in self.slot_names() {
                let v = &value[slot];
                let question = v["pergunta"]
                    .as_str()
                    .or(v["dica"].as_str())
                    .unwrap_or_default()
                    .trim();
                if question.is_empty() {
                    continue;
                }
                let kind = v["tipo"].as_str().unwrap_or(slot);
                let (earlier, now) = if kind == "contradicao" {
                    match self.check_contradiction(&st, turn_index, v) {
                        Some((earlier, now)) => (Some(earlier), Some(now)),
                        // Trecho que não está na transcrição: o modelo inventou.
                        None => {
                            self.status(
                                "descartada",
                                Some(format!("contradição sem trecho conferido: {question}")),
                                None,
                            );
                            continue;
                        }
                    }
                } else {
                    (None, None)
                };
                let repeated = st.asked.iter().any(|q| similar(q, question))
                    || st.slots.iter().any(|s| similar(&s.question, question));
                if repeated {
                    continue;
                }
                let priority = if kind == "contradicao" {
                    "alta"
                } else {
                    "media"
                };
                let id = self
                    .db
                    .insert_card(self.session_id, t_ms, kind, question, priority)
                    .unwrap_or(0);
                let suggestion = Suggestion {
                    id,
                    slot: slot.into(),
                    t_ms,
                    kind: kind.into(),
                    question: question.into(),
                    why: v["motivo"].as_str().unwrap_or_default().trim().into(),
                    earlier,
                    now,
                    latency_ms,
                };
                st.slots.retain(|s| s.slot != slot);
                st.slots.push(suggestion.clone());
                shown.push(suggestion);
            }
        }
        for suggestion in shown {
            (self.emit)(AnalysisEvent::Suggestion { suggestion });
        }
    }

    /// Confere os dois trechos de uma contradição: o de agora tem que estar na resposta
    /// atual, e o de antes numa resposta anterior ou no currículo.
    fn check_contradiction(
        &self,
        st: &State,
        turn_index: usize,
        v: &Value,
    ) -> Option<(Quote, Quote)> {
        let before = v["trecho_antes"].as_str()?.trim();
        let now = v["trecho_agora"].as_str()?.trim();
        if before.is_empty() || now.is_empty() {
            return None;
        }
        let turn = st.turns.get(turn_index)?;
        // No modo candidato, o "agora" é a pergunta do entrevistador (ou o começo da sua
        // resposta); nos outros, a resposta do candidato.
        let current: Vec<&Segment> = if self.candidate_side {
            turn.question.iter().chain(&turn.answer).collect()
        } else {
            turn.answer.iter().collect()
        };
        let now_at = find_quote(now, &current)?;
        let earlier: Vec<&Segment> = st.turns[..turn_index]
            .iter()
            .flat_map(|t| &t.answer)
            .filter(|s| s.source != self.interviewer)
            .collect();
        let before_at = match find_quote(before, &earlier) {
            Some(at) => Some(at),
            None if quote_in_text(before, &self.resume) => None,
            None => return None,
        };
        Some((
            Quote {
                text: before.into(),
                at_ms: before_at,
            },
            Quote {
                text: now.into(),
                at_ms: Some(now_at),
            },
        ))
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
            out.push_str(&format!(
                "Competências ainda sem evidência: {}.\n",
                missing.join(", ")
            ));
        }
        if !st.summary.is_empty() {
            out.push_str(&format!("\nResumo da conversa até aqui:\n{}\n", st.summary));
        }
        if !st.facts.is_empty() {
            out.push_str("\nFatos que o candidato afirmou antes:\n");
            for f in &st.facts {
                out.push_str(&format!(
                    "- [{}] {} (\"{}\")\n",
                    clock(f.at_ms),
                    f.claim,
                    f.quote
                ));
            }
        }
        if !st.slots.is_empty() {
            out.push_str("\nBalões na tela agora:\n");
            for s in &st.slots {
                out.push_str(&format!("- {}: {}\n", s.slot, s.question));
            }
        }
        if !st.asked.is_empty() {
            out.push_str("\nPerguntas sugeridas que o entrevistador já fez (não repita):\n");
            for q in &st.asked {
                out.push_str(&format!("- {q}\n"));
            }
        }
        out.push_str("\nTrocas recentes (a última resposta pode estar em andamento):\n");
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(2)..=index],
            self.interviewer,
        );
        out
    }

    fn apply(&self, value: &Value, answer: &[Segment], current: bool) {
        let answer_words = fold(&join(answer));
        let answer_end = answer.last().map_or(0, |s| s.end_ms);
        let mut changed = false;
        let (coverage, summary) = {
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
            if let Some(summary) = value["resumo"]
                .as_str()
                .filter(|s| current && !s.trim().is_empty())
            {
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
        // As observações sobre a condução são pra quem entrevista.
        let cards = if self.candidate_side {
            None
        } else {
            value["cards"].as_array()
        };
        for card in cards.into_iter().flatten().take(MAX_CARDS_PER_ANSWER) {
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
        push_turns(
            &mut out,
            &st.turns[index.saturating_sub(CONTEXT_TURNS)..=index],
            self.interviewer,
        );
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
            "required": ["cobertura", "cards", "resumo", "fatos"],
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
}

fn suggest_schema() -> Value {
    let text = json!({ "type": "string" });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["aprofundar", "checar", "avancar"],
        "properties": {
            "aprofundar": {
                "type": "object",
                "additionalProperties": false,
                "required": ["pergunta", "motivo"],
                "properties": { "pergunta": text, "motivo": text }
            },
            "checar": {
                "type": "object",
                "additionalProperties": false,
                "required": ["tipo", "pergunta", "motivo", "trecho_antes", "trecho_agora"],
                "properties": {
                    "tipo": { "type": "string", "enum": ["contradicao", "curriculo", "vaga"] },
                    "pergunta": text,
                    "motivo": text,
                    "trecho_antes": text,
                    "trecho_agora": text
                }
            },
            "avancar": {
                "type": "object",
                "additionalProperties": false,
                "required": ["tipo", "pergunta", "motivo"],
                "properties": {
                    "tipo": { "type": "string", "enum": ["roteiro", "nao_tocada"] },
                    "pergunta": text,
                    "motivo": text
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

/// O entrevistador fez a pergunta do balão? Metade das palavras com mais de 3 letras
/// da pergunta (e pelo menos 3) aparecem no que ele falou.
fn spoken_covers(spoken: &str, question: &str) -> bool {
    let said = content_words(spoken);
    let asked = content_words(question);
    let found = asked.iter().filter(|w| said.contains(*w)).count();
    found >= 3 && found as f32 / asked.len() as f32 >= 0.5
}

fn content_words(text: &str) -> HashSet<String> {
    fold(text)
        .split(' ')
        .filter(|w| w.chars().count() > 3)
        .map(String::from)
        .collect()
}

fn push_turns(out: &mut String, turns: &[Turn], interviewer: Source) {
    let mut segments: Vec<&Segment> = turns
        .iter()
        .flat_map(|t| t.question.iter().chain(&t.answer))
        .collect();
    // O "uhum" do entrevistador entra na resposta; na ordem do tempo fica legível.
    segments.sort_by_key(|s| s.start_ms);
    for s in segments {
        let who = if s.source == interviewer {
            "Entrevistador"
        } else {
            "Candidato"
        };
        out.push_str(&format!("[{}] {who}: {}\n", clock(s.start_ms), s.text));
    }
}

/// O modelo às vezes deixa lixo depois do ponto final ("…beacon?although",
/// "…tiver.](ignore)"). Na voz sintética isso soa como erro: corta.
fn clean_speech(text: &str) -> String {
    let text = text.trim();
    if let Some(i) = text.rfind(['.', '?', '!']) {
        let tail = text[i + 1..].trim();
        let junk = tail.chars().any(|c| c.is_ascii_alphabetic())
            && tail.chars().all(|c| c.is_ascii() && !c.is_whitespace());
        if junk {
            return text[..=i].to_string();
        }
    }
    text.to_string()
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
    let (a, b) = (content_words(a), content_words(b));
    let smaller = a.len().min(b.len());
    smaller > 0 && a.intersection(&b).count() as f32 / smaller as f32 >= 0.6
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
