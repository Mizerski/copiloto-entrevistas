//! Liga as peças de uma sessão ao vivo: captura → transcrição por fonte → sessão
//! (filtro de eco e banco). Usado pelo app e pela sonda.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::audio::{Capture, CaptureConfig, CaptureEvent, Input, Source};
use crate::db::Db;
use crate::deepgram::{self, DeepgramConfig, SttEvent};
use crate::session::{Session, SessionEvent};

const TICK: Duration = Duration::from_millis(250);
/// Depois de parar a captura, quanto esperar a transcrição entregar as últimas falas.
const STT_DRAIN: Duration = Duration::from_secs(8);

pub struct LiveConfig {
    pub mic: Option<Input>,
    pub sistema: Option<Input>,
    /// Como o outro lado aparece no histórico (nome do app ou do arquivo).
    pub other_side_label: Option<String>,
    pub wav_dir: Option<PathBuf>,
    /// `None` só captura, sem transcrever.
    pub deepgram: Option<DeepgramConfig>,
}

pub struct Listeners {
    pub on_capture: Arc<dyn Fn(CaptureEvent) + Send + Sync>,
    pub on_stt: Arc<dyn Fn(SttEvent) + Send + Sync>,
    pub on_session: Arc<dyn Fn(SessionEvent) + Send + Sync>,
}

pub struct LiveSession {
    capture: Capture,
    pub session: Arc<Session>,
    started_at: Instant,
    open_streams: Arc<AtomicUsize>,
}

impl LiveSession {
    pub fn start(config: LiveConfig, db: Arc<Db>, listeners: Listeners) -> Result<Self, String> {
        let started_at = Instant::now();
        let session = Session::start(db, config.other_side_label.as_deref(), listeners.on_session)?;
        let open_streams = Arc::new(AtomicUsize::new(0));

        let (mut mic_stt, mut sistema_stt) = (None, None);
        if let Some(deepgram) = config.deepgram {
            let deepgram = Arc::new(deepgram);
            let on_stt = {
                let session = session.clone();
                let open_streams = open_streams.clone();
                let forward = listeners.on_stt.clone();
                Arc::new(move |event: SttEvent| {
                    match &event {
                        SttEvent::Transcript {
                            source,
                            text,
                            is_final,
                            start_ms,
                            end_ms,
                            latency_ms,
                            ..
                        } => session.on_transcript(
                            *source,
                            text,
                            *is_final,
                            *start_ms,
                            *end_ms,
                            *latency_ms,
                        ),
                        SttEvent::Closed { .. } => {
                            open_streams.fetch_sub(1, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                    forward(event);
                }) as Arc<dyn Fn(SttEvent) + Send + Sync>
            };
            let open = |source| {
                open_streams.fetch_add(1, Ordering::SeqCst);
                Some(deepgram::spawn(
                    deepgram.clone(),
                    source,
                    started_at,
                    on_stt.clone(),
                ))
            };
            if config.mic.is_some() {
                mic_stt = open(Source::Mic);
            }
            if config.sistema.is_some() {
                sistema_stt = open(Source::Sistema);
            }
        }

        let on_capture = listeners.on_capture;
        let capture = Capture::start(
            CaptureConfig {
                mic: config.mic,
                sistema: config.sistema,
                wav_dir: config.wav_dir,
                started_at,
            },
            move |event| on_capture(event),
            move |chunk| {
                let stt = match chunk.source {
                    Source::Mic => &mic_stt,
                    Source::Sistema => &sistema_stt,
                };
                if let Some(tx) = stt {
                    let _ = tx.send(chunk);
                }
            },
        );
        let capture = match capture {
            Ok(c) => c,
            Err(e) => {
                session.finish(0);
                return Err(e);
            }
        };

        let ticking = session.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            while !ticking.is_finished() {
                interval.tick().await;
                ticking.tick(started_at.elapsed().as_millis() as u64);
            }
        });

        Ok(LiveSession {
            capture,
            session,
            started_at,
            open_streams,
        })
    }

    /// Para a captura, espera as últimas falas da transcrição e fecha a sessão.
    pub async fn stop(self) {
        let duration_ms = self.started_at.elapsed().as_millis() as u64;
        let capture = self.capture;
        let _ = tauri::async_runtime::spawn_blocking(move || capture.stop()).await;
        let deadline = Instant::now() + STT_DRAIN;
        while self.open_streams.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.session.finish(duration_ms);
    }
}
