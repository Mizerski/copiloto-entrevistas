//! Transcrição em streaming pela Deepgram: uma conexão WebSocket por fonte de áudio.
//!
//! A Deepgram devolve tempos relativos ao áudio que recebeu (`start`, `duration`).
//! A `Timeline` converte esses tempos pro relógio da sessão, o mesmo da captura, e
//! com isso dá pra medir a latência: quanto tempo depois de o áudio ser capturado o
//! texto dele chegou.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use url::Url;

use crate::audio::{AudioChunk, Source, SAMPLE_RATE};

const LISTEN_URL: &str = "wss://api.deepgram.com/v1/listen";
/// Sem áudio da fonte por mais que isso, o adaptador começa a mandar silêncio.
const SILENCE_AFTER: Duration = Duration::from_millis(300);
const TICK: Duration = Duration::from_millis(100);
/// Depois do `CloseStream`, quanto esperar os últimos resultados.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

pub struct DeepgramConfig {
    pub api_key: String,
    pub language: String,
    /// Termos que a transcrição costuma errar (tecnologias, siglas, nomes).
    pub keyterms: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SttEvent {
    Connected {
        source: Source,
    },
    Transcript {
        source: Source,
        text: String,
        is_final: bool,
        speech_final: bool,
        /// Início e fim da fala no relógio da sessão.
        start_ms: u64,
        end_ms: u64,
        /// Do fim do áudio capturado até o texto chegar.
        latency_ms: u64,
    },
    UtteranceEnd {
        source: Source,
    },
    Error {
        source: Source,
        message: String,
    },
    Closed {
        source: Source,
    },
}

/// Abre a conexão de uma fonte. Mande os trechos de áudio pelo canal retornado;
/// fechar o canal encerra a transcrição depois que os últimos resultados chegarem.
pub fn spawn(
    config: Arc<DeepgramConfig>,
    source: Source,
    started_at: Instant,
    on_event: Arc<dyn Fn(SttEvent) + Send + Sync>,
) -> mpsc::UnboundedSender<AudioChunk> {
    let (tx, rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        if let Err(message) = run(&config, source, started_at, rx, on_event.as_ref()).await {
            on_event(SttEvent::Error { source, message });
        }
        on_event(SttEvent::Closed { source });
    });
    tx
}

/// Abre e fecha uma conexão só pra saber se a chave é aceita. Não manda áudio.
pub async fn check_key(api_key: &str) -> Result<(), String> {
    let config = DeepgramConfig {
        api_key: api_key.to_string(),
        language: "pt-BR".into(),
        keyterms: Vec::new(),
    };
    let mut ws = connect(&config).await?;
    let _ = ws.send(close_stream()).await;
    let _ = ws.close(None).await;
    Ok(())
}

async fn run(
    config: &DeepgramConfig,
    source: Source,
    started_at: Instant,
    mut audio: mpsc::UnboundedReceiver<AudioChunk>,
    on_event: &(dyn Fn(SttEvent) + Send + Sync),
) -> Result<(), String> {
    let ws = connect(config).await?;
    on_event(SttEvent::Connected { source });
    let (mut write, mut read) = ws.split();

    let mut timeline = Timeline::default();
    let mut ticker = tokio::time::interval(TICK);
    let mut last_audio = Instant::now();
    let mut closing_since: Option<Instant> = None;
    let send_err = |e: WsError| format!("falha ao enviar áudio: {e}");

    loop {
        tokio::select! {
            chunk = audio.recv(), if closing_since.is_none() => match chunk {
                Some(chunk) => {
                    last_audio = Instant::now();
                    timeline.push(chunk.samples.len(), chunk.t_ms);
                    write.send(Message::Binary(to_pcm16(&chunk.samples).into())).await.map_err(send_err)?;
                }
                None => {
                    write.send(close_stream()).await.map_err(send_err)?;
                    closing_since = Some(Instant::now());
                }
            },
            _ = ticker.tick() => {
                if let Some(since) = closing_since {
                    if since.elapsed() > CLOSE_GRACE {
                        break;
                    }
                } else if last_audio.elapsed() >= SILENCE_AFTER {
                    // O loopback fica mudo quando o app não toca nada. Sem áudio a Deepgram
                    // não fecha a frase e acaba derrubando a conexão por inatividade.
                    let samples = SAMPLE_RATE as usize * TICK.as_millis() as usize / 1000;
                    timeline.push(samples, started_at.elapsed().as_millis() as u64);
                    write.send(Message::Binary(vec![0u8; samples * 2].into())).await.map_err(send_err)?;
                }
            },
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Some(event) = parse_message(&text, source, &timeline, started_at) {
                        on_event(event);
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    if let Some(frame) = frame.filter(|f| f.code != CloseCode::Normal) {
                        return Err(format!("a Deepgram fechou a conexão ({}): {}", frame.code, frame.reason));
                    }
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(format!("conexão caiu: {e}")),
                None => break,
            },
        }
    }
    Ok(())
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(config: &DeepgramConfig) -> Result<Socket, String> {
    let mut url = Url::parse(LISTEN_URL).expect("URL fixa");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("model", "nova-3")
            .append_pair("language", &config.language)
            .append_pair("encoding", "linear16")
            .append_pair("sample_rate", &SAMPLE_RATE.to_string())
            .append_pair("channels", "1")
            .append_pair("interim_results", "true")
            .append_pair("smart_format", "true")
            .append_pair("punctuate", "true")
            .append_pair("endpointing", "300")
            .append_pair("utterance_end_ms", "1000");
        // Um parâmetro por termo; vírgula dentro de um termo vira parte do termo.
        for term in &config.keyterms {
            q.append_pair("keyterm", term);
        }
    }
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| e.to_string())?;
    let auth = HeaderValue::from_str(&format!("Token {}", config.api_key.trim()))
        .map_err(|_| "a chave tem caracteres inválidos".to_string())?;
    request.headers_mut().insert("Authorization", auth);

    match connect_async(request).await {
        Ok((ws, _)) => Ok(ws),
        Err(WsError::Http(response)) => {
            let status = response.status().as_u16();
            let body = response
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            Err(match status {
                401 | 403 => "a Deepgram recusou a chave (inválida ou sem permissão)".into(),
                402 => "a conta da Deepgram está sem crédito".into(),
                400 => format!("a Deepgram recusou a configuração: {body}"),
                _ => format!("a Deepgram respondeu {status}: {body}"),
            })
        }
        Err(e) => Err(format!("não conectou na Deepgram: {e}")),
    }
}

fn close_stream() -> Message {
    Message::Text(r#"{"type":"CloseStream"}"#.into())
}

fn to_pcm16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect()
}

fn parse_message(text: &str, source: Source, timeline: &Timeline, started_at: Instant) -> Option<SttEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    match v["type"].as_str()? {
        "Results" => {
            let text = v["channel"]["alternatives"][0]["transcript"].as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let start = v["start"].as_f64().unwrap_or(0.0);
            let end = start + v["duration"].as_f64().unwrap_or(0.0);
            let end_ms = timeline.capture_ms(end);
            Some(SttEvent::Transcript {
                source,
                text: text.to_string(),
                is_final: v["is_final"].as_bool().unwrap_or(false),
                speech_final: v["speech_final"].as_bool().unwrap_or(false),
                start_ms: timeline.capture_ms(start),
                end_ms,
                latency_ms: (started_at.elapsed().as_millis() as u64).saturating_sub(end_ms),
            })
        }
        "UtteranceEnd" => Some(SttEvent::UtteranceEnd { source }),
        "Error" => Some(SttEvent::Error {
            source,
            message: v["description"].as_str().unwrap_or(text).to_string(),
        }),
        _ => None,
    }
}

/// Liga o tempo do áudio enviado (segundos desde o início da conexão) ao relógio da sessão.
#[derive(Default)]
struct Timeline {
    sent_samples: usize,
    /// (amostras enviadas até o fim do trecho, momento da captura do fim do trecho em ms)
    marks: Vec<(usize, u64)>,
}

impl Timeline {
    fn push(&mut self, samples: usize, captured_at_ms: u64) {
        self.sent_samples += samples;
        self.marks.push((self.sent_samples, captured_at_ms));
    }

    fn capture_ms(&self, audio_seconds: f64) -> u64 {
        let sample = (audio_seconds * SAMPLE_RATE as f64) as usize;
        let i = self.marks.partition_point(|(end, _)| *end < sample);
        let Some(&(end, captured_at)) = self.marks.get(i).or(self.marks.last()) else {
            return 0;
        };
        let behind_ms = end.saturating_sub(sample) as u64 * 1000 / SAMPLE_RATE as u64;
        captured_at.saturating_sub(behind_ms)
    }
}
