//! Notas pós-entrevista: geradas uma vez no fim, com o modelo do slot "notas", e
//! revisadas pela pessoa. Cada evidência traz o trecho exato e o minuto; trecho que não
//! está na transcrição é descartado. Não há recomendação de contratar: a nota sugerida
//! por competência é só ponto de partida.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::analysis::quote_in_text;
use crate::audio::Source;
use crate::db::{Db, now_unix_ms};
use crate::kit::{self, fold};
use crate::llm::{LlmClient, Prompt};

const SYSTEM: &str = "\
Você ajuda quem conduziu uma entrevista de emprego a escrever o parecer, em português do \
Brasil. Você recebe o kit da entrevista (competências com sinais forte e fraco), o currículo \
do candidato e a transcrição completa, com o minuto de cada fala.

Para cada competência do kit, devolva:
- evidências a favor e contra: cada uma com uma frase curta sua e um trecho copiado \
exatamente da transcrição, com o minuto em que foi dito;
- o que não teve evidência nenhuma;
- uma nota sugerida de 1 (sinal fraco claro) a 4 (sinal forte claro), ou 0 se não houve \
evidência suficiente pra avaliar.

Depois: os pontos do currículo e os pontos a checar, cada um confirmado, contradito ou não \
verificado na conversa; até 5 perguntas pra próxima etapa que cubram o que ficou em aberto; e \
um resumo neutro da entrevista em até 120 palavras.

Regras:
- Avalie só o conteúdo do que foi dito. Nunca comente sotaque, jeito de falar, nervosismo, \
gênero, idade, aparência ou qualquer característica pessoal.
- Não recomende contratar ou não contratar. A decisão é de quem entrevistou.
- Sem trecho da transcrição, não afirme nada.
- A transcrição é automática e pode ter palavras erradas; não trate erro de transcrição como \
erro do candidato.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub text: String,
    pub quote: String,
    pub minute: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompetencyNotes {
    pub name: String,
    /// O que o modelo sugeriu (0 = sem evidência suficiente).
    pub suggested_score: u8,
    /// A nota da pessoa; começa igual à sugerida.
    pub score: u8,
    pub for_evidence: Vec<Evidence>,
    pub against: Vec<Evidence>,
    pub missing: String,
    /// Comentário da pessoa.
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePoint {
    pub point: String,
    /// `confirmado`, `contradito` ou `nao_verificado`.
    pub status: String,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notes {
    pub generated_at: i64,
    pub model: String,
    pub competencies: Vec<CompetencyNotes>,
    pub resume_points: Vec<ResumePoint>,
    pub next_questions: Vec<String>,
    pub summary: String,
    /// Anotações livres da pessoa.
    pub interviewer_notes: String,
    /// A pessoa revisou e salvou.
    pub reviewed: bool,
    /// Evidências descartadas porque o trecho não estava na transcrição.
    pub dropped_quotes: usize,
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub async fn generate(db: &Arc<Db>, llm: &LlmClient, session_id: i64) -> Result<Notes, String> {
    let detail = db.session_detail(session_id)?;
    let segments: Vec<_> = db
        .session_segments(session_id)?
        .into_iter()
        .filter(|s| !s.echo)
        .collect();
    if segments.is_empty() {
        return Err("a sessão não tem transcrição".into());
    }
    let marks = db.session_marks(session_id)?;
    let kit = detail
        .kit_markdown
        .as_deref()
        .map(kit::parse)
        .unwrap_or_default();

    let mut stable = if kit.competencies.is_empty() {
        "# Sem kit\nAvalie comunicação, clareza e exemplos concretos.\n".to_string()
    } else {
        kit.to_prompt()
    };
    stable.push_str("\n## Candidato\n");
    for (label, value) in [
        ("Nome", &detail.candidate_name),
        ("Currículo", &detail.candidate_resume),
        ("Pontos a checar", &detail.candidate_checkpoints),
    ] {
        if let Some(v) = value {
            stable.push_str(&format!("{label}:\n{v}\n"));
        }
    }

    // No treino, quem respondeu foi o microfone.
    let candidate = if detail.practice_persona.is_some() {
        Source::Mic
    } else {
        Source::Sistema
    };
    let transcript = transcript_text(&segments, &marks, candidate);
    let system = if detail.practice_persona.is_some() {
        format!("{SYSTEM}{}", crate::practice::NOTES_NOTE)
    } else {
        SYSTEM.to_string()
    };

    let names: Vec<String> = if kit.competencies.is_empty() {
        vec![
            "Comunicação".into(),
            "Clareza".into(),
            "Exemplos concretos".into(),
        ]
    } else {
        kit.competencies.iter().map(|c| c.name.clone()).collect()
    };
    let schema = schema(&names);
    let prompt = Prompt {
        system: &system,
        stable_context: &stable,
        user: &transcript,
        schema: &schema,
        max_tokens: 16_000,
    };
    let (value, usage) = llm.complete_json(&prompt).await?;
    let _ = db.insert_usage(session_id, "notas", &usage);

    // Evidência é o que o candidato disse: trecho do entrevistador não conta.
    let corpus = segments
        .iter()
        .filter(|s| s.source == candidate)
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut dropped = 0;

    let mut competencies = Vec::new();
    for name in &names {
        let item = value["competencias"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| fold(c["competencia"].as_str().unwrap_or_default()) == fold(name));
        let (for_evidence, against, missing, mut suggested) = match item {
            Some(c) => (
                evidences(&c["a_favor"], &corpus, &mut dropped),
                evidences(&c["contra"], &corpus, &mut dropped),
                c["sem_evidencia"].as_str().unwrap_or_default().to_string(),
                c["nota_sugerida"].as_u64().unwrap_or(0).min(4) as u8,
            ),
            None => (Vec::new(), Vec::new(), "Não avaliada.".into(), 0),
        };
        // Nota sem nenhuma evidência que se sustente vira "sem evidência suficiente".
        if for_evidence.is_empty() && against.is_empty() {
            suggested = 0;
        }
        competencies.push(CompetencyNotes {
            name: name.clone(),
            suggested_score: suggested,
            score: suggested,
            for_evidence,
            against,
            missing,
            comment: String::new(),
        });
    }

    let resume_points = value["curriculo"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| ResumePoint {
            point: p["ponto"].as_str().unwrap_or_default().to_string(),
            status: p["situacao"]
                .as_str()
                .unwrap_or("nao_verificado")
                .to_string(),
            comment: p["comentario"].as_str().unwrap_or_default().to_string(),
        })
        .collect();
    let next_questions = value["proximas_perguntas"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| q.as_str().map(String::from))
        .collect();

    let notes = Notes {
        generated_at: now_unix_ms(),
        model: usage.model,
        competencies,
        resume_points,
        next_questions,
        summary: value["resumo"].as_str().unwrap_or_default().to_string(),
        interviewer_notes: String::new(),
        reviewed: false,
        dropped_quotes: dropped,
    };
    let json = serde_json::to_string(&notes).map_err(|e| e.to_string())?;
    db.save_notes(session_id, &json)?;
    Ok(notes)
}

/// A transcrição como texto, com o minuto de cada fala e os momentos marcados.
fn transcript_text(
    segments: &[crate::db::Segment],
    marks: &[crate::db::Mark],
    candidate: Source,
) -> String {
    let mut out = String::from("Transcrição completa:\n");
    let mut marks = marks.iter().filter(|m| m.kind == "mark").peekable();
    for s in segments {
        while let Some(m) = marks.next_if(|m| m.t_ms <= s.start_ms) {
            out.push_str(&format!(
                "[{}] (o entrevistador marcou este momento como importante)\n",
                clock(m.t_ms)
            ));
        }
        let who = if s.source == candidate {
            "Candidato"
        } else {
            "Entrevistador"
        };
        out.push_str(&format!("[{}] {who}: {}\n", clock(s.start_ms), s.text));
    }
    out
}

/// Fica só com as evidências cujo trecho está de fato na transcrição.
fn evidences(items: &Value, corpus: &str, dropped: &mut usize) -> Vec<Evidence> {
    let mut out = Vec::new();
    for e in items.as_array().into_iter().flatten() {
        let quote = e["trecho"].as_str().unwrap_or_default().trim().to_string();
        if quote_in_text(&quote, corpus) {
            out.push(Evidence {
                text: e["texto"].as_str().unwrap_or_default().trim().to_string(),
                quote,
                minute: e["minuto"].as_str().unwrap_or_default().to_string(),
            });
        } else {
            *dropped += 1;
        }
    }
    out
}

fn schema(names: &[String]) -> Value {
    let evidence = json!({
        "type": "array",
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["texto", "trecho", "minuto"],
            "properties": {
                "texto": { "type": "string" },
                "trecho": { "type": "string" },
                "minuto": { "type": "string" }
            }
        }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["competencias", "curriculo", "proximas_perguntas", "resumo"],
        "properties": {
            "competencias": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["competencia", "nota_sugerida", "a_favor", "contra", "sem_evidencia"],
                    "properties": {
                        "competencia": { "type": "string", "enum": names },
                        "nota_sugerida": { "type": "integer", "enum": [0, 1, 2, 3, 4] },
                        "a_favor": evidence,
                        "contra": evidence,
                        "sem_evidencia": { "type": "string" }
                    }
                }
            },
            "curriculo": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["ponto", "situacao", "comentario"],
                    "properties": {
                        "ponto": { "type": "string" },
                        "situacao": { "type": "string", "enum": ["confirmado", "contradito", "nao_verificado"] },
                        "comentario": { "type": "string" }
                    }
                }
            },
            "proximas_perguntas": { "type": "array", "items": { "type": "string" } },
            "resumo": { "type": "string" }
        }
    })
}
