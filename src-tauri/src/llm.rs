//! LLMs: um contrato só ("mande este prompt, devolva JSON neste formato") e um adaptador
//! por formato de API. Cada pessoa usa a própria chave.
//!
//! - Anthropic: Messages API, com o schema garantido por `output_config.format`.
//! - Compatível com OpenAI: OpenAI, Groq, OpenRouter, Ollama, LM Studio e outros que
//!   imitam `/chat/completions`. O schema vai no prompt e a resposta é validada aqui.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Se o modelo recusar por política de segurança, a própria API tenta outro modelo.
const ANTHROPIC_FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const TIMEOUT: Duration = Duration::from_secs(90);

/// Modelos que aceitam o fallback automático em caso de recusa. Os outros (Haiku,
/// modelos antigos) recusam o parâmetro, então ele só vai pra estes.
fn supports_fallback(model: &str) -> bool {
    [
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-sonnet-5-5",
    ]
    .contains(&model)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    /// Qualquer API no formato `/chat/completions`.
    Openai,
}

impl Provider {
    /// Nome da entrada da chave no Credential Manager.
    pub fn key_name(self) -> &'static str {
        match self {
            Provider::Anthropic => "llm-anthropic",
            Provider::Openai => "llm-openai",
        }
    }
}

/// Configuração de um "slot" de modelo (análise ao vivo ou notas).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub provider: Provider,
    pub model: String,
    /// Só pra compatível com OpenAI (ex.: `http://localhost:11434/v1` pro Ollama).
    #[serde(default)]
    pub base_url: String,
    /// Só pra Anthropic: `low`, `medium`, `high`...
    #[serde(default)]
    pub effort: String,
}

impl Slot {
    pub fn default_live() -> Self {
        Slot {
            provider: Provider::Anthropic,
            model: "claude-opus-5-5".into(),
            base_url: "https://api.openai.com/v1".into(),
            effort: "low".into(),
        }
    }

    pub fn default_notes() -> Self {
        Slot {
            effort: "high".into(),
            ..Self::default_live()
        }
    }
}

/// Um pedido: a parte fixa do prompt (vai pro cache quando o provedor tem), a parte
/// que muda a cada chamada e o formato da resposta.
pub struct Prompt<'a> {
    pub system: &'a str,
    pub stable_context: &'a str,
    pub user: &'a str,
    /// JSON Schema da resposta (objetos com `additionalProperties: false`).
    pub schema: &'a Value,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub latency_ms: u64,
    pub model: String,
}

pub struct LlmClient {
    http: reqwest::Client,
    slot: Slot,
    api_key: Option<String>,
}

impl LlmClient {
    pub fn new(slot: Slot, api_key: Option<String>) -> Result<Self, String> {
        if slot.provider == Provider::Anthropic && api_key.is_none() {
            return Err("salve a chave da Anthropic nas configurações".into());
        }
        if slot.model.trim().is_empty() {
            return Err("escolha um modelo nas configurações".into());
        }
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(LlmClient {
            http,
            slot,
            api_key,
        })
    }

    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub async fn complete_json(&self, prompt: &Prompt<'_>) -> Result<(Value, Usage), String> {
        let started = Instant::now();
        let (text, mut usage) = match self.slot.provider {
            Provider::Anthropic => self.anthropic(prompt).await?,
            Provider::Openai => self.openai(prompt).await?,
        };
        usage.latency_ms = started.elapsed().as_millis() as u64;
        let value = parse_json(&text)
            .ok_or_else(|| format!("o modelo não devolveu JSON válido: {}", preview(&text)))?;
        Ok((value, usage))
    }

    /// Pedido mínimo pra conferir chave, modelo e endereço.
    pub async fn test(&self) -> Result<Usage, String> {
        let schema = json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"],
            "additionalProperties": false
        });
        let prompt = Prompt {
            system: "Responda só com o JSON pedido.",
            stable_context: "",
            user: "Devolva {\"ok\": true}.",
            schema: &schema,
            max_tokens: 2000,
        };
        let (_, usage) = self.complete_json(&prompt).await?;
        Ok(usage)
    }

    async fn anthropic(&self, p: &Prompt<'_>) -> Result<(String, Usage), String> {
        let mut system = vec![json!({ "type": "text", "text": p.system })];
        if !p.stable_context.is_empty() {
            // Kit e currículo se repetem em toda chamada da sessão: ficam no cache.
            system.push(json!({
                "type": "text",
                "text": p.stable_context,
                "cache_control": { "type": "ephemeral" }
            }));
        }
        let mut output_config = json!({ "format": { "type": "json_schema", "schema": p.schema } });
        if !self.slot.effort.is_empty() {
            output_config["effort"] = json!(self.slot.effort);
        }
        let mut body = json!({
            "model": self.slot.model,
            "max_tokens": p.max_tokens,
            "system": system,
            "messages": [{ "role": "user", "content": p.user }],
            "output_config": output_config,
        });
        let mut request = self
            .http
            .post(ANTHROPIC_URL)
            .header("x-api-key", self.api_key.as_deref().unwrap_or_default())
            .header("anthropic-version", ANTHROPIC_VERSION);
        if supports_fallback(&self.slot.model) {
            body["fallbacks"] = json!("default");
            request = request.header("anthropic-beta", ANTHROPIC_FALLBACK_BETA);
        }
        let response = request.json(&body).send().await.map_err(network_error)?;
        let v = read_body(response).await?;
        if v["stop_reason"] == "refusal" {
            return Err("o modelo recusou o pedido".into());
        }
        if v["stop_reason"] == "max_tokens" {
            return Err("a resposta passou do limite de tokens".into());
        }
        let text = v["content"]
            .as_array()
            .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
            .and_then(|b| b["text"].as_str())
            .ok_or("resposta sem texto")?
            .to_string();
        let u = &v["usage"];
        let usage = Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0)
                + u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
            cached_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
            latency_ms: 0,
            model: v["model"].as_str().unwrap_or(&self.slot.model).to_string(),
        };
        Ok((text, usage))
    }

    async fn openai(&self, p: &Prompt<'_>) -> Result<(String, Usage), String> {
        let system = format!(
            "{}\n\n{}\n\nResponda só com um objeto JSON, sem texto em volta, seguindo este JSON Schema:\n{}",
            p.system, p.stable_context, p.schema
        );
        let url = format!(
            "{}/chat/completions",
            self.slot.base_url.trim_end_matches('/')
        );
        let mut body = json!({
            "model": self.slot.model,
            "max_tokens": p.max_tokens,
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": p.user }
            ],
            "response_format": { "type": "json_object" },
        });
        let mut response = self.post_openai(&url, &body).await?;
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            // Nem todo servidor compatível aceita `response_format`; o prompt já pede JSON.
            body.as_object_mut().unwrap().remove("response_format");
            response = self.post_openai(&url, &body).await?;
        }
        let v = read_body(response).await?;
        let choice = &v["choices"][0];
        if choice["finish_reason"] == "length" {
            return Err("a resposta passou do limite de tokens".into());
        }
        let text = choice["message"]["content"]
            .as_str()
            .ok_or("resposta sem texto")?
            .to_string();
        let u = &v["usage"];
        let usage = Usage {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
            cached_tokens: u["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            latency_ms: 0,
            model: v["model"].as_str().unwrap_or(&self.slot.model).to_string(),
        };
        Ok((text, usage))
    }

    async fn post_openai(&self, url: &str, body: &Value) -> Result<reqwest::Response, String> {
        let mut request = self.http.post(url).json(body);
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            request = request.bearer_auth(key);
        }
        request.send().await.map_err(network_error)
    }
}

fn network_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "o provedor demorou demais pra responder".into()
    } else if e.is_connect() {
        format!("não conectou no provedor: {e}")
    } else {
        format!("falha na chamada: {e}")
    }
}

/// Lê o corpo e transforma erros HTTP em mensagens que a pessoa entende.
async fn read_body(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if status.is_success() {
        return serde_json::from_str(&text)
            .map_err(|_| format!("resposta inválida: {}", preview(&text)));
    }
    let detail = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| preview(&text));
    Err(match status.as_u16() {
        401 | 403 => format!("chave recusada pelo provedor ({detail})"),
        402 => format!("conta sem saldo ({detail})"),
        404 => format!("modelo ou endereço não encontrado ({detail})"),
        429 => format!("limite de uso atingido, tente de novo em instantes ({detail})"),
        code => format!("o provedor respondeu {code}: {detail}"),
    })
}

/// Aceita JSON puro ou JSON dentro de texto/cerca de código.
fn parse_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(text.trim()) {
        return Some(v);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

fn preview(text: &str) -> String {
    let t: String = text.chars().take(200).collect();
    if text.chars().count() > 200 {
        format!("{t}…")
    } else {
        t
    }
}
