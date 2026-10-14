//! Perfil de candidato: o que a própria pessoa escreve sobre si (experiências, histórias,
//! habilidades), em Markdown. No modo candidato ele vai inteiro pro modelo, que tira dali
//! as respostas e os exemplos; o parser só serve pra pré-visualização e pros termos da
//! transcrição.
//!
//! ```text
//! # Nome
//! ## Resumo
//! ## Experiências
//! ### Empresa · Cargo (2021–2024)
//! ## Histórias
//! ### Título da história
//! Situação: ... Ação: ... Resultado: ...
//! ## Habilidades
//! ## Perguntas pra fazer
//! ## Termos
//! ```

use serde::Serialize;
use serde_json::{Value, json};

use crate::kit::fold;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    pub experiences: Vec<String>,
    pub stories: Vec<String>,
    pub terms: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn parse(markdown: &str) -> Profile {
    let mut p = Profile::default();
    let mut section = String::new();
    for line in markdown.lines() {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("### ") {
            match section.as_str() {
                s if s.starts_with("experiencia") => p.experiences.push(title.trim().into()),
                s if s.starts_with("historia") => p.stories.push(title.trim().into()),
                _ => {}
            }
        } else if let Some(title) = line.strip_prefix("## ") {
            section = fold(title);
        } else if let Some(title) = line.strip_prefix("# ") {
            p.name = title.trim().into();
        } else if section.starts_with("termo") && !line.is_empty() {
            p.terms.extend(
                line.split(',')
                    .map(|t| t.trim().trim_start_matches("- ").to_string())
                    .filter(|t| !t.is_empty()),
            );
        }
    }
    if p.name.is_empty() {
        p.warnings.push("Falta o título (# Seu nome).".into());
    }
    if p.experiences.is_empty() {
        p.warnings
            .push("Nenhuma experiência (### dentro de ## Experiências).".into());
    }
    if p.stories.is_empty() {
        p.warnings.push(
            "Nenhuma história (### dentro de ## Histórias). São elas que viram exemplo nas respostas."
                .into(),
        );
    }
    p
}

pub const ANSWER_SYSTEM: &str = "\
Você ajuda, ao vivo, quem está sendo entrevistado pra uma vaga, em português do Brasil. Você \
recebe o perfil da pessoa (experiências, histórias e habilidades, escrito por ela), a vaga \
quando houver, o que ela já disse nesta entrevista e a conversa recente. A última pergunta do \
entrevistador pode estar no meio, chegando aos pedaços: responda pelo que já dá pra entender \
e nunca trate pergunta incompleta como ambígua (os balões são refeitos quando ela terminar).

Devolva três balões curtos, pra pessoa bater o olho enquanto responde:
- responda: sempre preenchido. Como responder a esta pergunta, em 2 ou 3 pontos curtos \
separados por \" · \" (até 30 palavras no total), na primeira pessoa, com os fatos do perfil \
(nomes, números, tecnologias). Comece pelo ponto principal.
- historia: a experiência ou história do perfil que melhor serve de exemplo pra esta \
pergunta: o nome dela e o resultado, com número se houver (até 20 palavras). Se nenhuma \
servir, dica vazia.
- atencao: tipo contradicao só quando a própria pergunta afirma ou supõe sobre a pessoa algo \
que é falso pelo perfil ou pelo que ela disse antes (ex.: \"você liderou o time\" e o perfil \
diz que não). Assunto de pergunta anterior não conta. Nesse caso preencha trecho_antes (copiado exatamente do que ela disse antes \
ou do perfil) e trecho_agora (copiado exatamente da pergunta) e diga como responder sem se \
contradizer. Tipo duas_partes quando a pergunta tem mais de uma parte (diga quais); \
esclarecer quando está ambígua (sugira o que perguntar de volta); requisito quando ligar a \
resposta a um requisito da vaga ajudaria. Fora de contradição, trechos vazios. Se nada \
merecer atenção, dica vazia.

Regras:
- Use só o que está no perfil e no que a pessoa já disse. Nunca invente experiência, número, \
empresa, cargo ou tecnologia. Se o perfil não cobre a pergunta, diga isso em responda e \
sugira contar o mais próximo que ela já fez, com honestidade.
- motivo: até 8 palavras.
- Se um balão na tela ainda serve pra esta pergunta, devolva o mesmo texto.
- A transcrição é automática e pode ter palavras erradas.";

pub fn answer_schema() -> Value {
    let text = json!({ "type": "string" });
    let simple = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["dica", "motivo"],
        "properties": { "dica": text, "motivo": text }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["responda", "historia", "atencao"],
        "properties": {
            "responda": simple,
            "historia": simple,
            "atencao": {
                "type": "object",
                "additionalProperties": false,
                "required": ["tipo", "dica", "motivo", "trecho_antes", "trecho_agora"],
                "properties": {
                    "tipo": {
                        "type": "string",
                        "enum": ["contradicao", "duas_partes", "esclarecer", "requisito"]
                    },
                    "dica": text,
                    "motivo": text,
                    "trecho_antes": text,
                    "trecho_agora": text
                }
            }
        }
    })
}

/// Vai junto do prompt do parecer quando quem usou o app foi o candidato.
pub const NOTES_NOTE: &str = "\n\nQuem vai ler este parecer é a própria pessoa entrevistada, \
que usou o app do lado dela numa entrevista real: escreva as frases das evidências falando com \
ela (\"você\"). Em perguntas pra próxima etapa, liste o que ela deve preparar melhor pras \
próximas entrevistas.";
