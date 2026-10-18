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
//! ## Jeito de falar
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

Antes de tudo, veja que tipo de fala é. Conversa de chamada, cumprimento ou checagem \
(\"tá me ouvindo?\", \"oi, tudo bem?\", \"consegue ver minha tela?\", \"pode esperar um \
minuto?\") não é pergunta de entrevista: responda curto e natural, até 12 palavras (\"Tô \
te ouvindo bem, sim. E você, me ouve?\"), sem se apresentar, sem falar de experiência, \
historia e atencao vazias. Só se apresente quando pedirem (\"fala um pouco de você\"). O \
tamanho da resposta acompanha a pergunta: pergunta simples ou de sim/não, resposta curta.

Devolva três balões. Os dois primeiros são falas prontas: a pessoa lê e fala em voz alta, \
do jeito que está, sem precisar pensar. Nada de instrução (\"fale da experiência X\", \"cite \
Y\", \"comente que...\"): escreva a própria resposta.
- responda: sempre preenchido. A resposta à pergunta, na primeira pessoa, em português falado \
e natural, até 80 palavras numa pergunta de entrevista de verdade (uns 20 a 30 segundos de fala). Comece respondendo direto o \
que foi perguntado na primeira frase; depois, o fato concreto do perfil que sustenta (onde, o \
que eu fiz, o resultado com número). Termine fechando a resposta, sem pergunta.
- historia: a continuação pronta se pedirem mais detalhe ou um exemplo: a história do perfil \
que melhor serve, contada em 2 ou 3 frases faladas (situação, o que eu fiz, resultado), até \
60 palavras. Se nenhuma história servir ou ela já estiver toda em responda, fala vazia.
- atencao: um aviso curto (até 20 palavras), esse sim em forma de dica. Tipo contradicao só \
quando a própria pergunta afirma ou supõe sobre a pessoa algo que é falso pelo perfil ou pelo \
que ela disse antes (ex.: \"você liderou o time\" e o perfil diz que não); assunto de pergunta \
anterior não conta. Nesse caso preencha trecho_antes (copiado exatamente do que ela disse \
antes ou do perfil) e trecho_agora (copiado exatamente da pergunta), e a resposta em responda \
já corrige a suposição com educação. Tipo duas_partes quando a pergunta tem mais de uma parte \
(e responda cobre as duas); esclarecer quando está ambígua (sugira o que perguntar de volta); \
requisito quando a resposta pode ser ligada a um requisito da vaga. Fora de contradição, \
trechos vazios. Se nada merecer atenção, dica vazia.

Jeito de falar:
- Quem pergunta muitas vezes é do RH e não é técnico. Se a pergunta não for técnica, troque \
jargão pelo efeito no negócio (em vez de \"idempotência no Postgres\", \"fiz o sistema \
reconhecer o pagamento repetido e não cobrar de novo\"). Se a pergunta for técnica, pode usar \
os termos.
- Frases curtas, que dá pra falar sem tropeçar. Nada de listas, aspas ou parênteses.
- Se o perfil tem a seção Jeito de falar, escreva como essa pessoa fala: mesmo nível de \
formalidade, mesmo tamanho de frase, as expressões e vícios dela, imitando os exemplos. A fala \
tem que soar como ela, não como texto de IA. Sem a seção, português falado neutro.

Regras:
- Use só o que está no perfil e no que a pessoa já disse. Nunca invente experiência, número, \
empresa, cargo ou tecnologia. Se o perfil não cobre a pergunta, a resposta assume isso com \
honestidade e conta o mais próximo que a pessoa já fez.
- motivo: até 8 palavras.
- Se um balão na tela ainda serve pra esta pergunta, devolva o mesmo texto.
- A transcrição é automática e pode ter palavras erradas.";

pub fn answer_schema() -> Value {
    let text = json!({ "type": "string" });
    let speech = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["fala", "motivo"],
        "properties": { "fala": text, "motivo": text }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["responda", "historia", "atencao"],
        "properties": {
            "responda": speech,
            "historia": speech,
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
