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
quando houver, o que ela já disse nesta entrevista e a conversa recente. A última fala do \
entrevistador pode estar no meio, chegando aos pedaços: responda pelo que já dá pra entender \
(a resposta é refeita quando ele terminar).

Devolva uma fala só, pronta: a pessoa lê e fala em voz alta, do jeito que está, sem precisar \
pensar. Nada de instrução (\"fale da experiência X\", \"cite Y\"): escreva a própria \
resposta, na primeira pessoa.

O que responder:
- Ache o pedido. O entrevistador muitas vezes comenta algo antes de perguntar (\"legal, \
gosto disso de mentorar...\"). Comentário é contexto: a resposta é pro pedido que vem \
depois (\"me dá um exemplo de um fluxo com Kafka que você construiu\"). Se ele pede um \
exemplo, a resposta é o exemplo, contado do começo ao fim.
- Conversa de chamada, cumprimento ou checagem (\"tá me ouvindo?\", \"oi, tudo bem?\", \
\"consegue ver minha tela?\") não é pergunta de entrevista: responda curto e natural, até 12 \
palavras (\"Tô te ouvindo bem, sim. E você, me ouve?\"). Só se apresente quando pedirem.
- O tamanho acompanha a pergunta. Sim ou não, ou pergunta simples: uma ou duas frases. \
Pergunta de entrevista: até 90 palavras, uns 30 segundos. Exemplo, história ou explicação de \
como algo funciona: até 140 palavras. Se ele pedir um tempo (\"uns dois minutos\") ou \
várias partes: até 200 palavras, cada parte na ordem pedida.
- Comece respondendo direto na primeira frase. Depois o concreto do perfil: onde, o que eu \
fiz, como funciona, o resultado com número se o perfil tiver. Termine fechando, sem pergunta.
- Se a pergunta supõe algo falso sobre a pessoa pelo perfil ou pelo que ela já disse (\"você \
liderou o time\" e o perfil diz que não), a resposta corrige com educação.

Jeito de falar:
- Quem pergunta muitas vezes é do RH e não é técnico. Se a pergunta não for técnica, troque \
jargão pelo efeito no negócio. Se for técnica, use os termos.
- Frases curtas, que dá pra falar sem tropeçar. Nada de listas, aspas, títulos ou parênteses.
- Organize pra leitura rápida: em resposta de mais de duas frases, quebre em parágrafos \
curtos de uma ou duas frases, separados por linha em branco, um por ideia. Marque com \
**negrito** as poucas palavras que a pessoa não pode esquecer de falar (o nome de uma \
tecnologia, um número, o resultado), no máximo uma por parágrafo. Fora isso, nada de \
markdown.
- Se o perfil tem a seção Jeito de falar, escreva como essa pessoa fala: mesmo nível de \
formalidade, mesmo tamanho de frase, as expressões e vícios dela, imitando os exemplos. A fala \
tem que soar como ela, não como texto de IA. Sem a seção, português falado neutro.

Regras:
- Use só o que está no perfil e no que a pessoa já disse. Nunca invente experiência, número, \
empresa, cargo ou tecnologia. Se o perfil não cobre a pergunta, a resposta assume isso com \
honestidade e conta o mais próximo que a pessoa já fez.
- Se a fala na tela ainda serve pra esta pergunta, devolva o mesmo texto.
- A transcrição é automática e pode ter palavras erradas.";

pub fn answer_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["fala"],
        "properties": { "fala": { "type": "string" } }
    })
}

/// Vai junto do prompt do parecer quando quem usou o app foi o candidato.
pub const NOTES_NOTE: &str = "\n\nQuem vai ler este parecer é a própria pessoa entrevistada, \
que usou o app do lado dela numa entrevista real: escreva as frases das evidências falando com \
ela (\"você\"). Em perguntas pra próxima etapa, liste o que ela deve preparar melhor pras \
próximas entrevistas.

Escreva simples, como um colega que assistiu à entrevista explicaria depois: frases curtas e \
diretas, uma ideia por frase. Use termo técnico só quando ele é o próprio ponto (o nome de uma \
configuração que faltou citar, por exemplo) e, fora isso, diga com palavras comuns o que \
aconteceu. Nada de jargão de avaliação nem frase de efeito.";
