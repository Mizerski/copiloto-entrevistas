//! Testa a captura (e, opcionalmente, a transcrição) sem a interface.
//!
//!     cargo run --example sonda -- [pid-do-app] [segundos]
//!
//! Lista microfones e apps com áudio, captura e imprime o nível por segundo.
//! Salva mic.wav e sistema.wav em target/sonda. Com a variável DEEPGRAM_API_KEY
//! definida, também transcreve as duas fontes e imprime o texto final.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use copiloto_entrevistas_lib::audio::{self, Capture, CaptureConfig, CaptureEvent, Source};
use copiloto_entrevistas_lib::deepgram::{self, DeepgramConfig, SttEvent};

fn main() {
    let mut args = std::env::args().skip(1);
    let app_pid: Option<u32> = args.next().and_then(|a| a.parse().ok());
    let seconds: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(5);

    println!("Microfones:");
    for m in audio::list_microphones().expect("listar microfones") {
        println!("  {} {}", if m.default { "*" } else { " " }, m.name);
    }
    println!("Apps com sessão de áudio:");
    for a in audio::list_audio_apps().expect("listar apps") {
        println!("  {:>6}  {}{}", a.pid, a.name, if a.active { "  (ativo)" } else { "" });
    }

    let started_at = Instant::now();
    let (mut mic_stt, mut sistema_stt) = (None, None);
    if let Ok(api_key) = std::env::var("DEEPGRAM_API_KEY") {
        let rt = tauri::async_runtime::handle();
        if let Err(e) = rt.block_on(deepgram::check_key(&api_key)) {
            println!("Chave recusada: {e}");
            return;
        }
        let config = Arc::new(DeepgramConfig { api_key, language: "pt-BR".into(), keyterms: vec![] });
        let on_stt: Arc<dyn Fn(SttEvent) + Send + Sync> = Arc::new(|event| match event {
            SttEvent::Transcript { is_final: false, .. } => {}
            other => println!("{other:?}"),
        });
        mic_stt = Some(deepgram::spawn(config.clone(), Source::Mic, started_at, on_stt.clone()));
        if app_pid.is_some() {
            sistema_stt = Some(deepgram::spawn(config, Source::Sistema, started_at, on_stt));
        }
    }

    let wav_dir = std::path::PathBuf::from("target/sonda");
    std::fs::create_dir_all(&wav_dir).unwrap();

    // Maior RMS de cada fonte em cada segundo.
    let per_second: Arc<Mutex<BTreeMap<(u64, String), f32>>> = Arc::default();
    let sink = per_second.clone();
    let capture = Capture::start(
        CaptureConfig { mic_id: None, app_pid, wav_dir: Some(wav_dir.clone()), started_at },
        move |event| match event {
            CaptureEvent::Level { source, t_ms, rms, .. } => {
                let key = (t_ms / 1000, format!("{source:?}"));
                let mut map = sink.lock().unwrap();
                let v = map.entry(key).or_default();
                *v = v.max(rms);
            }
            other => println!("{other:?}"),
        },
        move |chunk| {
            let stt = match chunk.source {
                Source::Mic => &mic_stt,
                Source::Sistema => &sistema_stt,
            };
            if let Some(tx) = stt {
                let _ = tx.send(chunk);
            }
        },
    )
    .expect("iniciar captura");

    std::thread::sleep(Duration::from_secs(seconds));
    capture.stop();
    // Tempo pros últimos resultados da transcrição chegarem.
    std::thread::sleep(Duration::from_secs(2));

    println!("RMS máximo por segundo:");
    for ((sec, source), rms) in per_second.lock().unwrap().iter() {
        let bar = "#".repeat(((rms * 400.0) as usize).min(60));
        println!("  {sec:>3}s {source:<8} {rms:.4} {bar}");
    }
    println!("WAVs em {}", wav_dir.canonicalize().unwrap().display());
}
