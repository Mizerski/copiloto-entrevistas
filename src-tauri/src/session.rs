//! Uma sessão de entrevista: recebe as falas finais das duas fontes, descarta o eco do
//! microfone, grava no banco e avisa a tela.
//!
//! Eco: sem fone, o microfone capta o candidato saindo da caixa de som, e a fala dele
//! aparece duas vezes. Uma fala do microfone é eco quando quase todas as palavras dela
//! estão no que o candidato disse no mesmo intervalo. Como as duas transcrições chegam
//! com atrasos diferentes, a fala do microfone espera um pouco antes de ser confirmada,
//! mas só quando o candidato estava falando na mesma hora.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::audio::Source;
use crate::db::{Db, NewSession, Segment};

/// Folga, antes e depois da fala do microfone, pra procurar a mesma fala do outro lado.
const ECHO_WINDOW_MS: u64 = 1_500;
/// Quanto a fala do microfone espera pela transcrição do outro lado antes de ser confirmada.
const ECHO_HOLD_MS: u64 = 2_500;
/// Fração das palavras do microfone que precisa aparecer do outro lado pra ser eco.
const ECHO_MIN_OVERLAP: f32 = 0.6;
/// Por quanto tempo guardar o que o outro lado disse, pra comparar.
const RECENT_MS: u64 = 30_000;

#[derive(Debug, Serialize, Clone)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    /// Fala confirmada, ou descartada como eco (`segment.echo`).
    Segment { segment: Segment },
    Finished {
        session_id: i64,
        duration_ms: u64,
        echoes: usize,
    },
}

pub struct Session {
    pub id: i64,
    db: Arc<Db>,
    emit: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Falas do microfone esperando pra saber se são eco.
    pending_mic: Vec<Segment>,
    /// Falas finais recentes do outro lado.
    recent_sistema: VecDeque<Segment>,
    /// Intervalos recentes em que o outro lado estava falando (parciais e finais).
    sistema_activity: VecDeque<(u64, u64)>,
    echoes: usize,
    finished: bool,
}

impl Session {
    pub fn start(
        db: Arc<Db>,
        new: &NewSession,
        emit: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    ) -> Result<Arc<Self>, String> {
        let id = db.create_session(new)?;
        Ok(Arc::new(Session {
            id,
            db,
            emit,
            inner: Mutex::default(),
        }))
    }

    pub fn is_finished(&self) -> bool {
        self.inner.lock().unwrap().finished
    }

    /// Até quando o outro lado foi ouvido falando, contando os resultados parciais.
    pub fn sistema_heard_until(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner
            .sistema_activity
            .iter()
            .map(|&(_, end)| end)
            .max()
            .unwrap_or(0)
    }

    /// Um resultado da transcrição, parcial ou final. Tempos no relógio da sessão.
    pub fn on_transcript(
        &self,
        source: Source,
        text: &str,
        is_final: bool,
        start_ms: u64,
        end_ms: u64,
        latency_ms: u64,
    ) {
        let mut inner = self.inner.lock().unwrap();
        if inner.finished {
            return;
        }
        if source == Source::Sistema {
            inner.sistema_activity.push_back((start_ms, end_ms));
        }
        if !is_final {
            return;
        }
        let segment = Segment {
            id: 0,
            source,
            start_ms,
            end_ms,
            text: text.to_string(),
            latency_ms,
            echo: false,
        };
        match source {
            Source::Sistema => {
                self.commit(segment.clone());
                inner.recent_sistema.push_back(segment);
                // O outro lado pode ter chegado depois do eco que ele causou.
                let pending = std::mem::take(&mut inner.pending_mic);
                for mic in pending {
                    if is_echo(&mic, &inner.recent_sistema) {
                        self.discard(&mut inner, mic);
                    } else {
                        inner.pending_mic.push(mic);
                    }
                }
            }
            Source::Mic => {
                let heard_other_side = inner.sistema_activity.iter().any(|&(s, e)| {
                    overlaps(s, e, segment.start_ms, segment.end_ms, ECHO_WINDOW_MS)
                });
                if !heard_other_side {
                    self.commit(segment);
                } else if is_echo(&segment, &inner.recent_sistema) {
                    self.discard(&mut inner, segment);
                } else {
                    inner.pending_mic.push(segment);
                }
            }
        }
    }

    /// Chamado a cada fração de segundo: confirma falas do microfone que já esperaram o
    /// bastante e esquece o que é antigo demais pra comparar.
    pub fn tick(&self, now_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.finished {
            return;
        }
        self.release_pending(&mut inner, Some(now_ms));
        let horizon = now_ms.saturating_sub(RECENT_MS);
        while inner
            .recent_sistema
            .front()
            .is_some_and(|s| s.end_ms < horizon)
        {
            inner.recent_sistema.pop_front();
        }
        while inner
            .sistema_activity
            .front()
            .is_some_and(|&(_, e)| e < horizon)
        {
            inner.sistema_activity.pop_front();
        }
    }

    /// Fecha a sessão: confirma o que estava esperando e grava a duração. Idempotente.
    pub fn finish(&self, duration_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.finished {
            return;
        }
        self.release_pending(&mut inner, None);
        inner.finished = true;
        if let Err(e) = self.db.finish_session(self.id, duration_ms) {
            eprintln!("sessão {}: {e}", self.id);
        }
        (self.emit)(SessionEvent::Finished {
            session_id: self.id,
            duration_ms,
            echoes: inner.echoes,
        });
    }

    /// Confirma ou descarta as falas pendentes; com `now_ms`, só as que já esperaram.
    fn release_pending(&self, inner: &mut Inner, now_ms: Option<u64>) {
        let pending = std::mem::take(&mut inner.pending_mic);
        for mic in pending {
            if is_echo(&mic, &inner.recent_sistema) {
                self.discard(inner, mic);
            } else if now_ms.is_none_or(|now| now >= mic.end_ms + ECHO_HOLD_MS) {
                self.commit(mic);
            } else {
                inner.pending_mic.push(mic);
            }
        }
    }

    fn commit(&self, mut segment: Segment) {
        self.store(&mut segment);
        (self.emit)(SessionEvent::Segment { segment });
    }

    fn discard(&self, inner: &mut Inner, mut segment: Segment) {
        segment.echo = true;
        inner.echoes += 1;
        self.store(&mut segment);
        (self.emit)(SessionEvent::Segment { segment });
    }

    fn store(&self, segment: &mut Segment) {
        match self.db.insert_segment(self.id, segment) {
            Ok(id) => segment.id = id,
            Err(e) => eprintln!("sessão {}: {e}", self.id),
        }
    }
}

fn overlaps(a_start: u64, a_end: u64, b_start: u64, b_end: u64, slack: u64) -> bool {
    a_end + slack >= b_start && a_start <= b_end + slack
}

fn is_echo(mic: &Segment, recent_sistema: &VecDeque<Segment>) -> bool {
    let mic_words = words(&mic.text);
    if mic_words.is_empty() {
        return false;
    }
    let heard: HashSet<String> = recent_sistema
        .iter()
        .filter(|s| {
            overlaps(
                s.start_ms,
                s.end_ms,
                mic.start_ms,
                mic.end_ms,
                ECHO_WINDOW_MS,
            )
        })
        .flat_map(|s| words(&s.text))
        .collect();
    let found = mic_words.iter().filter(|w| heard.contains(*w)).count();
    // Falas curtas ("sim", "tá bom") só contam como eco se forem idênticas.
    if mic_words.len() <= 2 {
        return found == mic_words.len();
    }
    found as f32 / mic_words.len() as f32 >= ECHO_MIN_OVERLAP
}

/// Minúsculas, sem pontuação.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}
