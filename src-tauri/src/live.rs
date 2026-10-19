//! Liga as peças de uma sessão ao vivo: captura → transcrição por fonte → sessão
//! (filtro de eco e banco) → análise. Usado pelo app e pela sonda.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::analysis::{AnalysisEvent, Analyzer};
use crate::audio::{Capture, CaptureConfig, CaptureEvent, Input, Source};
use crate::db::{Db, NewSession};
use crate::deepgram::{self, DeepgramConfig, SttEvent};
use crate::llm::LlmClient;
use crate::session::{Session, SessionEvent};

const TICK: Duration = Duration::from_millis(250);
/// Depois de parar a captura, quanto esperar a transcrição entregar as últimas falas.
const STT_DRAIN: Duration = Duration::from_secs(8);

pub struct LiveConfig {
    pub mic: Option<Input>,
    pub sistema: Option<Input>,
    pub wav_dir: Option<PathBuf>,
    /// `None` só captura, sem transcrever.
    pub deepgram: Option<DeepgramConfig>,
    /// O que fica gravado na sessão (outro lado, perfil).
    pub record: NewSession,
    /// `None` desliga a análise (só transcreve).
    pub analysis: Option<AnalysisSetup>,
}

pub struct AnalysisSetup {
    pub llm: LlmClient,
    /// O perfil de quem está sendo entrevistado, em Markdown.
    pub profile: String,
    /// A descrição da vaga, se houver.
    pub job: String,
}

pub struct Listeners {
    pub on_capture: Arc<dyn Fn(CaptureEvent) + Send + Sync>,
    pub on_stt: Arc<dyn Fn(SttEvent) + Send + Sync>,
    pub on_session: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    pub on_analysis: Arc<dyn Fn(AnalysisEvent) + Send + Sync>,
}

pub struct LiveSession {
    capture: Capture,
    pub session: Arc<Session>,
    pub analyzer: Option<Arc<Analyzer>>,
    db: Arc<Db>,
    started_at: Instant,
    open_streams: Arc<AtomicUsize>,
    paused: Arc<AtomicBool>,
}

impl LiveSession {
    pub fn start(config: LiveConfig, db: Arc<Db>, listeners: Listeners) -> Result<Self, String> {
        let started_at = Instant::now();

        // A análise precisa do id da sessão, e a sessão precisa avisar a análise: o
        // analisador entra depois, por aqui.
        let analyzer_slot: Arc<OnceLock<Arc<Analyzer>>> = Arc::default();
        let on_session = {
            let slot = analyzer_slot.clone();
            let forward = listeners.on_session.clone();
            Arc::new(move |event: SessionEvent| {
                if let (SessionEvent::Segment { segment }, Some(a)) = (&event, slot.get()) {
                    a.on_segment(segment);
                }
                forward(event);
            })
        };
        let session = Session::start(db.clone(), &config.record, on_session)?;
        let analyzer = config.analysis.map(|setup| {
            let a = Analyzer::new(
                session.id,
                db.clone(),
                setup.llm,
                &setup.profile,
                &setup.job,
                listeners.on_analysis.clone(),
            );
            let _ = analyzer_slot.set(a.clone());
            a
        });

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

        let paused = Arc::new(AtomicBool::new(false));
        let on_capture = listeners.on_capture;
        let capture = Capture::start(
            CaptureConfig {
                mic: config.mic,
                sistema: config.sistema,
                wav_dir: config.wav_dir,
                started_at,
            },
            move |event| on_capture(event),
            {
                let paused = paused.clone();
                move |chunk| {
                    // Pausado, o áudio não sai do computador; a transcrição recebe silêncio.
                    if paused.load(Ordering::Relaxed) {
                        return;
                    }
                    let stt = match chunk.source {
                        Source::Mic => &mic_stt,
                        Source::Sistema => &sistema_stt,
                    };
                    if let Some(tx) = stt {
                        let _ = tx.send(chunk);
                    }
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

        let (ticking, ticking_analyzer) = (session.clone(), analyzer.clone());
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            while !ticking.is_finished() {
                interval.tick().await;
                let now = started_at.elapsed().as_millis() as u64;
                ticking.tick(now);
                if let Some(a) = &ticking_analyzer {
                    a.tick(
                        now,
                        ticking.heard_until(Source::Mic),
                        ticking.heard_until(Source::Sistema),
                    );
                }
            }
        });

        Ok(LiveSession {
            capture,
            session,
            analyzer,
            db,
            started_at,
            open_streams,
            paused,
        })
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Marca o momento atual ("isso foi importante"). Devolve o instante marcado.
    pub fn mark(&self, note: Option<&str>) -> Result<u64, String> {
        let t = self.elapsed_ms();
        self.db.insert_mark(self.session.id, t, "mark", note)?;
        Ok(t)
    }

    /// Pausa ou retoma a transcrição (o candidato pediu pra não ser transcrito).
    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        if self.paused.swap(paused, Ordering::SeqCst) != paused {
            let kind = if paused { "pause" } else { "resume" };
            self.db
                .insert_mark(self.session.id, self.elapsed_ms(), kind, None)?;
        }
        Ok(())
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
        // A última resposta fecha com o fim da sessão.
        if let Some(a) = &self.analyzer {
            a.flush(duration_ms);
        }
        self.session.finish(duration_ms);
    }
}
