//! Captura de áudio: microfone (você) e loopback por processo (o outro lado da chamada).
//!
//! Cada fonte roda na própria thread, entrega mono 16 kHz em f32 e carimba cada trecho
//! com o mesmo relógio da sessão, pra depois dar pra ordenar as falas das duas fontes.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
use wasapi::{
    AudioClient, DeviceEnumerator, Direction, SampleType, SessionState, StreamMode, WaveFormat,
    initialize_mta,
};

pub const SAMPLE_RATE: u32 = 16_000;
/// Janela usada pra calcular o nível que vai pra tela.
const LEVEL_WINDOW_MS: u64 = 50;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Mic,
    Sistema,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Mic => "mic",
            Source::Sistema => "sistema",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "mic" => Some(Source::Mic),
            "sistema" => Some(Source::Sistema),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AudioApp {
    /// PID raiz da árvore de processos do app (é ele que o loopback por processo precisa).
    pub pid: u32,
    pub name: String,
    /// Tem uma sessão de áudio ativa agora.
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct Microphone {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Um trecho de áudio já em mono 16 kHz.
pub struct AudioChunk {
    pub source: Source,
    /// Milissegundos desde o início da sessão, no momento em que o trecho foi lido.
    pub t_ms: u64,
    pub samples: Vec<f32>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CaptureEvent {
    Started {
        source: Source,
        format: String,
    },
    Level {
        source: Source,
        t_ms: u64,
        rms: f32,
        peak: f32,
    },
    Error {
        source: Source,
        message: String,
    },
    Stopped {
        source: Source,
        seconds: f32,
    },
}

/// De onde vem o áudio de uma fonte.
pub enum Input {
    /// Dispositivo de entrada; `None` usa o microfone padrão do Windows.
    Microphone(Option<String>),
    /// Loopback por processo: o áudio que esse app (e os filhos dele) toca.
    App(u32),
    /// Tudo que toca no PC, menos o próprio copiloto. Pra apps que o loopback por
    /// processo não pega (áudio tocado por outro processo, drivers virtuais etc.).
    Desktop,
    /// Modo replay: um WAV tocado em tempo real, como se fosse a chamada.
    File(PathBuf),
}

pub struct CaptureConfig {
    /// Você. `None` desliga.
    pub mic: Option<Input>,
    /// O outro lado. `None` desliga.
    pub sistema: Option<Input>,
    /// Se definido, salva `mic.wav` e `sistema.wav` aqui (só pra depurar o spike).
    pub wav_dir: Option<PathBuf>,
    /// Relógio da sessão. Os carimbos `t_ms` dos trechos contam a partir daqui.
    pub started_at: Instant,
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Capture {
    /// Abre as fontes pedidas e começa a capturar. Retorna erro se nenhuma fonte abrir.
    /// `on_audio` recebe cada trecho já convertido, na thread de processamento.
    pub fn start(
        config: CaptureConfig,
        on_event: impl Fn(CaptureEvent) + Send + 'static,
        on_audio: impl FnMut(AudioChunk) + Send + 'static,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let started_at = config.started_at;
        let (chunk_tx, chunk_rx) = mpsc::channel::<AudioChunk>();
        let (event_tx, event_rx) = mpsc::channel::<CaptureEvent>();

        let mut threads = Vec::new();
        let targets = [(Source::Mic, config.mic), (Source::Sistema, config.sistema)]
            .into_iter()
            .filter_map(|(source, input)| Some((source, input?)));

        // Arquivos de replay carregam primeiro e só começam a tocar juntos, depois que
        // todas as fontes abriram; senão um começa segundos antes do outro.
        let go = StartSignal::default();
        let mut opened = 0;
        for (source, target) in targets {
            let (ready_tx, ready_rx) = mpsc::channel();
            let stop = stop.clone();
            let go = go.clone();
            let chunk_tx = chunk_tx.clone();
            let event_tx = event_tx.clone();
            let handle = thread::Builder::new()
                .name(format!("captura-{}", source.label()))
                .spawn(move || match target {
                    Input::File(path) => {
                        file_thread(source, path, started_at, stop, go, chunk_tx, ready_tx)
                    }
                    target => capture_thread(
                        source, target, started_at, stop, chunk_tx, event_tx, ready_tx,
                    ),
                })
                .map_err(|e| e.to_string())?;
            threads.push(handle);
            match ready_rx.recv() {
                Ok(Ok(format)) => {
                    opened += 1;
                    on_event(CaptureEvent::Started { source, format });
                }
                Ok(Err(message)) => on_event(CaptureEvent::Error { source, message }),
                Err(_) => on_event(CaptureEvent::Error {
                    source,
                    message: "a thread de captura terminou antes de abrir".into(),
                }),
            }
        }
        drop(chunk_tx);
        drop(event_tx);

        if opened == 0 {
            stop.store(true, Ordering::Relaxed);
            go.fire();
            for t in threads {
                let _ = t.join();
            }
            return Err("nenhuma fonte de áudio abriu".into());
        }

        let wav_dir = config.wav_dir;
        threads.push(
            thread::Builder::new()
                .name("captura-processa".into())
                .spawn(move || process_thread(chunk_rx, event_rx, wav_dir, on_event, on_audio))
                .map_err(|e| e.to_string())?,
        );
        go.fire();

        Ok(Capture { stop, threads })
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads {
            let _ = t.join();
        }
    }
}

fn capture_thread(
    source: Source,
    target: Input,
    started_at: Instant,
    stop: Arc<AtomicBool>,
    chunk_tx: mpsc::Sender<AudioChunk>,
    event_tx: mpsc::Sender<CaptureEvent>,
    ready_tx: mpsc::Sender<Result<String, String>>,
) {
    let _ = initialize_mta().ok();

    let (client, format) = match open_client(&target) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let channels = format.get_nchannels() as usize;
    let rate = format.get_samplespersec();
    let decimate = (rate / SAMPLE_RATE) as usize;
    let block_align = format.get_blockalign() as usize;

    let setup = (|| {
        let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
        let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
        client.start_stream().map_err(|e| e.to_string())?;
        Ok::<_, String>((event, capture))
    })();
    let (event, capture) = match setup {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let _ = ready_tx.send(Ok(format!("{rate} Hz, {channels} canal(is)")));

    let mut bytes: VecDeque<u8> = VecDeque::new();
    let mut pending: Vec<f32> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // O loopback por processo só gera evento quando o app toca algo; o timeout
        // curto é só pra conferir o `stop` de tempos em tempos.
        if event.wait_for_event(200).is_err() {
            continue;
        }
        let read = (|| {
            while capture.get_next_packet_size()?.unwrap_or(0) > 0 {
                capture.read_from_device_to_deque(&mut bytes)?;
            }
            Ok::<_, wasapi::WasapiError>(())
        })();
        if let Err(e) = read {
            let _ = event_tx.send(CaptureEvent::Error {
                source,
                message: e.to_string(),
            });
            break;
        }

        let t_ms = started_at.elapsed().as_millis() as u64;
        let frames = bytes.len() / block_align;
        for _ in 0..frames {
            let mut sum = 0.0f32;
            for _ in 0..channels {
                let b = [
                    bytes.pop_front().unwrap(),
                    bytes.pop_front().unwrap(),
                    bytes.pop_front().unwrap(),
                    bytes.pop_front().unwrap(),
                ];
                sum += f32::from_le_bytes(b);
            }
            pending.push(sum / channels as f32);
        }
        let samples = if decimate > 1 {
            // Fallback quando o Windows não entrega 16 kHz direto: média de blocos.
            // É um filtro fraco, suficiente pro spike; trocar por `rubato` se for usar.
            let usable = pending.len() / decimate * decimate;
            let out = pending[..usable]
                .chunks(decimate)
                .map(|c| c.iter().sum::<f32>() / decimate as f32)
                .collect();
            pending.drain(..usable);
            out
        } else {
            std::mem::take(&mut pending)
        };
        if !samples.is_empty()
            && chunk_tx
                .send(AudioChunk {
                    source,
                    t_ms,
                    samples,
                })
                .is_err()
        {
            break;
        }
    }
    let _ = client.stop_stream();
}

/// Cliente de loopback do dispositivo de saída padrão: captura o que o Windows toca nele.
fn default_output_loopback() -> Result<AudioClient, wasapi::WasapiError> {
    let enumerator = DeviceEnumerator::new()?;
    enumerator
        .get_default_device(&Direction::Render)?
        .get_iaudioclient()
}

/// Tenta abrir já em mono 16 kHz; se o Windows recusar, cai pra 48 kHz estéreo.
fn open_client(target: &Input) -> Result<(AudioClient, WaveFormat), String> {
    let wanted = [
        WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None),
        WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None),
    ];
    let mut last_error = String::new();
    for format in wanted {
        let mut client = match target {
            Input::Microphone(id) => {
                let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
                let device = match id {
                    Some(id) => enumerator.get_device(id),
                    None => enumerator.get_default_device(&Direction::Capture),
                }
                .map_err(|e| format!("microfone não encontrado: {e}"))?;
                device.get_iaudioclient().map_err(|e| e.to_string())?
            }
            Input::App(pid) => AudioClient::new_application_loopback_client(*pid, true)
                .map_err(|e| format!("loopback do processo {pid} falhou: {e}"))?,
            // Primeiro tenta o loopback por processo em modo "exclusão" (tudo menos a
            // nossa árvore, então o app nunca escuta a si mesmo). Se o Windows ou o
            // driver recusar, cai pro loopback clássico do dispositivo de saída padrão.
            Input::Desktop => AudioClient::new_application_loopback_client(
                std::process::id(),
                false,
            )
            .or_else(|_| default_output_loopback())
            .map_err(|e| format!("loopback do PC falhou: {e}"))?,
            Input::File(_) => return Err("arquivo não é dispositivo".into()),
        };
        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 200_000,
        };
        match client.initialize_client(&format, &Direction::Capture, &mode) {
            Ok(()) => return Ok((client, format)),
            Err(e) => last_error = e.to_string(),
        }
        if matches!(target, Input::Desktop) {
            // O cliente por processo pode abrir e só falhar ao inicializar.
            if let Ok(mut device) = default_output_loopback() {
                if device
                    .initialize_client(&format, &Direction::Capture, &mode)
                    .is_ok()
                {
                    return Ok((device, format));
                }
            }
        }
    }
    Err(format!("nenhum formato aceito: {last_error}"))
}

/// Sinal único de largada, esperado pelas fontes de arquivo.
#[derive(Clone, Default)]
struct StartSignal(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

impl StartSignal {
    fn fire(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }

    fn wait(&self) {
        let mut fired = self.0.0.lock().unwrap();
        while !*fired {
            fired = self.0.1.wait(fired).unwrap();
        }
    }
}

/// Toca um WAV em tempo real, em blocos de 20 ms, carimbando cada bloco no relógio da
/// sessão como se tivesse acabado de ser capturado.
fn file_thread(
    source: Source,
    path: PathBuf,
    started_at: Instant,
    stop: Arc<AtomicBool>,
    go: StartSignal,
    chunk_tx: mpsc::Sender<AudioChunk>,
    ready_tx: mpsc::Sender<Result<String, String>>,
) {
    let samples = match read_wav(&path) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
    let _ = ready_tx.send(Ok(format!("arquivo {name}, {seconds:.1} s")));

    go.wait();
    const BLOCK: usize = SAMPLE_RATE as usize / 50;
    let begin = Instant::now();
    for (i, block) in samples.chunks(BLOCK).enumerate() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Agenda pelo início do arquivo pra não acumular atraso de `sleep`.
        let due = begin + std::time::Duration::from_millis(20 * (i as u64 + 1));
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        let t_ms = started_at.elapsed().as_millis() as u64;
        if chunk_tx
            .send(AudioChunk {
                source,
                t_ms,
                samples: block.to_vec(),
            })
            .is_err()
        {
            break;
        }
    }
}

/// Lê um WAV PCM ou float como mono 16 kHz. Aceita taxas múltiplas de 16 kHz.
fn read_wav(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("não abriu {}: {e}", path.display()))?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
        }
    }
    .map_err(|e| format!("WAV corrompido: {e}"))?;

    let channels = spec.channels as usize;
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32);
    if spec.sample_rate % SAMPLE_RATE != 0 {
        return Err(format!(
            "o WAV está em {} Hz; use 16, 32 ou 48 kHz",
            spec.sample_rate
        ));
    }
    let decimate = (spec.sample_rate / SAMPLE_RATE) as usize;
    let mono: Vec<f32> = mono.collect();
    Ok(mono
        .chunks(decimate)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect())
}

fn process_thread(
    chunk_rx: mpsc::Receiver<AudioChunk>,
    event_rx: mpsc::Receiver<CaptureEvent>,
    wav_dir: Option<PathBuf>,
    on_event: impl Fn(CaptureEvent),
    mut on_audio: impl FnMut(AudioChunk),
) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writers: HashMap<Source, hound::WavWriter<_>> = HashMap::new();
    let mut levels: HashMap<Source, LevelWindow> = HashMap::new();
    let mut totals: HashMap<Source, usize> = HashMap::new();

    // O canal fecha quando as duas threads de captura terminam.
    for chunk in chunk_rx {
        while let Ok(e) = event_rx.try_recv() {
            on_event(e);
        }
        *totals.entry(chunk.source).or_default() += chunk.samples.len();

        if let Some(dir) = &wav_dir {
            if let Entry::Vacant(slot) = writers.entry(chunk.source) {
                let path = dir.join(format!("{}.wav", chunk.source.label()));
                match hound::WavWriter::create(&path, spec) {
                    Ok(w) => {
                        slot.insert(w);
                    }
                    Err(e) => on_event(CaptureEvent::Error {
                        source: chunk.source,
                        message: format!("não deu pra criar {}: {e}", path.display()),
                    }),
                }
            }
            if let Some(w) = writers.get_mut(&chunk.source) {
                for s in &chunk.samples {
                    let _ = w.write_sample(*s);
                }
            }
        }

        let window = levels.entry(chunk.source).or_default();
        for s in &chunk.samples {
            window.push(*s);
            if window.count >= (SAMPLE_RATE as u64 * LEVEL_WINDOW_MS / 1000) as usize {
                on_event(window.take(chunk.source, chunk.t_ms));
            }
        }
        on_audio(chunk);
    }
    while let Ok(e) = event_rx.try_recv() {
        on_event(e);
    }
    for (_, w) in writers {
        let _ = w.finalize();
    }
    for (source, n) in totals {
        on_event(CaptureEvent::Stopped {
            source,
            seconds: n as f32 / SAMPLE_RATE as f32,
        });
    }
}

#[derive(Default)]
struct LevelWindow {
    sum_sq: f32,
    peak: f32,
    count: usize,
}

impl LevelWindow {
    fn push(&mut self, s: f32) {
        self.sum_sq += s * s;
        self.peak = self.peak.max(s.abs());
        self.count += 1;
    }

    fn take(&mut self, source: Source, t_ms: u64) -> CaptureEvent {
        let event = CaptureEvent::Level {
            source,
            t_ms,
            rms: (self.sum_sq / self.count as f32).sqrt(),
            peak: self.peak,
        };
        *self = Self::default();
        event
    }
}

/// Roda `f` numa thread própria com COM em modo MTA. Os comandos síncronos do Tauri
/// rodam na thread principal, que já é STA por causa do WebView.
fn in_mta<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    thread::spawn(move || {
        let _ = initialize_mta().ok();
        f()
    })
    .join()
    .map_err(|_| "a thread de áudio entrou em pânico".to_string())?
}

pub fn list_microphones() -> Result<Vec<Microphone>, String> {
    in_mta(|| {
        let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let default_id = enumerator
            .get_default_device(&Direction::Capture)
            .and_then(|d| d.get_id())
            .ok();
        let mut mics = Vec::new();
        let collection = enumerator
            .get_device_collection(&Direction::Capture)
            .map_err(|e| e.to_string())?;
        for device in &collection {
            let device = device.map_err(|e| e.to_string())?;
            let id = device.get_id().map_err(|e| e.to_string())?;
            mics.push(Microphone {
                name: device.get_friendlyname().unwrap_or_else(|_| id.clone()),
                default: default_id.as_deref() == Some(id.as_str()),
                id,
            });
        }
        Ok(mics)
    })
}

/// Apps com sessão de áudio em algum dispositivo de saída, agrupados pelo processo raiz.
/// Chrome e o Teams novo tocam áudio num processo filho; o loopback precisa do pai.
pub fn list_audio_apps() -> Result<Vec<AudioApp>, String> {
    in_mta(|| {
        let system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
        );
        let root_of = |pid: u32| -> (u32, String) {
            let Some(mut proc) = system.process(Pid::from_u32(pid)) else {
                return (pid, format!("processo {pid}"));
            };
            let name = proc.name().to_string_lossy().into_owned();
            while let Some(parent) = proc.parent().and_then(|p| system.process(p)) {
                if parent.name() != proc.name() {
                    break;
                }
                proc = parent;
            }
            (proc.pid().as_u32(), name)
        };

        let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
        let collection = enumerator
            .get_device_collection(&Direction::Render)
            .map_err(|e| e.to_string())?;
        let mut apps: Vec<AudioApp> = Vec::new();
        for device in &collection {
            let Ok(device) = device else { continue };
            let Ok(manager) = device.get_iaudiosessionmanager() else {
                continue;
            };
            let Ok(sessions) = manager.get_audiosessionenumerator() else {
                continue;
            };
            for i in 0..sessions.get_count().unwrap_or(0) {
                let Ok(session) = sessions.get_session(i) else {
                    continue;
                };
                let Ok(pid) = session.get_process_id() else {
                    continue;
                };
                if pid == 0 {
                    continue; // sons do sistema
                }
                let active = session.get_state().ok() == Some(SessionState::Active);
                let (root, name) = root_of(pid);
                match apps.iter_mut().find(|a| a.pid == root) {
                    Some(app) => app.active |= active,
                    None => apps.push(AudioApp {
                        pid: root,
                        name,
                        active,
                    }),
                }
            }
        }
        apps.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.cmp(&b.name)));
        Ok(apps)
    })
}
