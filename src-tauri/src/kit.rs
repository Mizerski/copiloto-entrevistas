//! Kit de entrevista: um Markdown escrito à mão, reaproveitável por vaga.
//!
//! ```text
//! # Título
//! ## Vaga            texto livre
//! ## Competências    ### Nome, com linhas "Forte:" e "Fraco:"
//! ## Roteiro         ### Bloco (N min), com perguntas em "- "
//! ## Perguntas proibidas   itens em "- " (somam à lista padrão do app)
//! ## Termos          separados por vírgula: vão pra transcrição
//! ```

use serde::Serialize;

pub const EXAMPLES: &[&str] = &[
    include_str!("../kits/backend-senior-pagamentos.md"),
    include_str!("../kits/comportamental-geral.md"),
];

/// Perguntas que não podem ser feitas numa entrevista, como sequências de palavras já
/// normalizadas (sem acento, minúsculas). O kit pode acrescentar outras.
const DEFAULT_FORBIDDEN: &[&str] = &[
    "quantos anos voce tem",
    "qual sua idade",
    "qual a sua idade",
    "voce e casado",
    "voce e casada",
    "estado civil",
    "tem filhos",
    "pretende ter filhos",
    "planeja ter filhos",
    "esta gravida",
    "pretende engravidar",
    "qual sua religiao",
    "qual a sua religiao",
    "voce e religioso",
    "voce e religiosa",
    "orientacao sexual",
    "em quem voce votou",
    "qual seu partido",
    "tem alguma doenca",
    "toma algum remedio",
    "tem alguma deficiencia",
];

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kit {
    pub title: String,
    pub job: String,
    pub competencies: Vec<Competency>,
    pub blocks: Vec<Block>,
    /// Só os itens do kit; a lista padrão é somada na checagem.
    pub forbidden: Vec<String>,
    pub terms: Vec<String>,
    /// Problemas que não impedem o uso, mostrados no editor.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Competency {
    pub name: String,
    pub description: String,
    pub strong: String,
    pub weak: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub name: String,
    pub minutes: Option<u32>,
    pub questions: Vec<String>,
}

#[derive(PartialEq)]
enum Section {
    None,
    Job,
    Competencies,
    Script,
    Forbidden,
    Terms,
    Unknown,
}

pub fn parse(markdown: &str) -> Kit {
    let mut kit = Kit::default();
    let mut section = Section::None;
    let mut job = Vec::new();

    for raw in markdown.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("# ") {
            kit.title = title.trim().trim_start_matches("Kit:").trim().to_string();
        } else if let Some(name) = line.strip_prefix("## ") {
            section = match fold(name).as_str() {
                "vaga" => Section::Job,
                "competencias" => Section::Competencies,
                "roteiro" => Section::Script,
                "perguntas proibidas" => Section::Forbidden,
                "termos" => Section::Terms,
                _ => Section::Unknown,
            };
        } else if let Some(name) = line.strip_prefix("### ") {
            match section {
                Section::Competencies => kit.competencies.push(Competency {
                    name: name.trim().to_string(),
                    ..Default::default()
                }),
                Section::Script => {
                    let (name, minutes) = split_minutes(name);
                    kit.blocks.push(Block {
                        name,
                        minutes,
                        questions: Vec::new(),
                    });
                }
                _ => {}
            }
        } else if !line.is_empty() {
            match section {
                Section::Job => job.push(line),
                Section::Competencies => {
                    if let Some(c) = kit.competencies.last_mut() {
                        let folded = fold(line);
                        if folded.starts_with("forte:") {
                            c.strong = after_colon(line);
                        } else if folded.starts_with("fraco:") {
                            c.weak = after_colon(line);
                        } else {
                            push_line(&mut c.description, line);
                        }
                    }
                }
                Section::Script => {
                    if let (Some(b), Some(q)) = (kit.blocks.last_mut(), bullet(line)) {
                        b.questions.push(q);
                    }
                }
                Section::Forbidden => {
                    // Itens entre parênteses são comentários no exemplo.
                    if let Some(item) = bullet(line).filter(|i| !i.starts_with('(')) {
                        kit.forbidden.push(item);
                    }
                }
                Section::Terms => {
                    let items = bullet(line).unwrap_or_else(|| line.to_string());
                    kit.terms.extend(
                        items
                            .split(',')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(String::from),
                    );
                }
                Section::None | Section::Unknown => {}
            }
        }
    }
    kit.job = job.join("\n");

    if kit.title.is_empty() {
        kit.warnings
            .push("Falta o título (linha começando com \"# \").".into());
    }
    if kit.competencies.is_empty() {
        kit.warnings
            .push("Nenhuma competência: o painel de cobertura fica vazio.".into());
    }
    for c in &kit.competencies {
        if c.strong.is_empty() || c.weak.is_empty() {
            kit.warnings
                .push(format!("\"{}\" está sem \"Forte:\" ou \"Fraco:\".", c.name));
        }
    }
    if kit.blocks.is_empty() {
        kit.warnings
            .push("Nenhum bloco no roteiro: o relógio por bloco fica desligado.".into());
    }
    kit
}

impl Kit {
    /// O kit como contexto fixo do prompt de análise.
    pub fn to_prompt(&self) -> String {
        let mut out = format!(
            "# Kit da entrevista: {}\n\n## Vaga\n{}\n\n## Competências\n",
            self.title, self.job
        );
        for c in &self.competencies {
            out.push_str(&format!("### {}\n", c.name));
            if !c.description.is_empty() {
                out.push_str(&format!("{}\n", c.description));
            }
            out.push_str(&format!(
                "Sinal forte: {}\nSinal fraco: {}\n\n",
                c.strong, c.weak
            ));
        }
        out.push_str("## Roteiro\n");
        for b in &self.blocks {
            match b.minutes {
                Some(m) => out.push_str(&format!("### {} ({m} min)\n", b.name)),
                None => out.push_str(&format!("### {}\n", b.name)),
            }
            for q in &b.questions {
                out.push_str(&format!("- {q}\n"));
            }
        }
        out
    }

    /// Se a fala tem uma pergunta proibida, devolve qual.
    pub fn forbidden_in(&self, text: &str) -> Option<String> {
        let spoken: Vec<String> = fold(text).split_whitespace().map(String::from).collect();
        let kit_items = self.forbidden.iter().map(|f| fold(f));
        DEFAULT_FORBIDDEN
            .iter()
            .map(|s| s.to_string())
            .chain(kit_items)
            .find(|phrase| {
                let words: Vec<&str> = phrase.split_whitespace().collect();
                !words.is_empty()
                    && spoken
                        .windows(words.len())
                        .any(|w| w.iter().zip(&words).all(|(a, b)| a == b))
            })
    }
}

/// Minúsculas, sem acento e sem pontuação (pontuação vira espaço).
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_alphanumeric() || c == ':' => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn bullet(line: &str) -> Option<String> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn after_colon(line: &str) -> String {
    line.split_once(':')
        .map(|(_, rest)| rest.trim().to_string())
        .unwrap_or_default()
}

fn push_line(target: &mut String, line: &str) {
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(line);
}

/// "Técnico (25 min)" → ("Técnico", Some(25)).
fn split_minutes(heading: &str) -> (String, Option<u32>) {
    let heading = heading.trim();
    if let Some(open) = heading.rfind('(') {
        let inside = heading[open + 1..].trim_end_matches(')');
        let digits: String = inside.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(minutes) = digits.parse() {
            return (heading[..open].trim().to_string(), Some(minutes));
        }
    }
    (heading.to_string(), None)
}
