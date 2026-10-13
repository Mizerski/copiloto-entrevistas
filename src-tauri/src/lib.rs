pub mod audio;
pub mod deepgram;
pub mod secrets;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager, State};

use audio::{AudioApp, Capture, CaptureConfig, Microphone, Source};
use deepgram::DeepgramConfig;

const DEEPGRAM: &str = "deepgram";

#[derive(Default)]
struct AppState {
    capture: Mutex<Option<Capture>>,
}

// Os comandos são `async` pra rodarem fora da thread principal: a captura usa COM em
// modo MTA e o `stop` espera as threads terminarem.

#[tauri::command]
async fn list_microphones() -> Result<Vec<Microphone>, String> {
    audio::list_microphones()
}

#[tauri::command]
async fn list_audio_apps() -> Result<Vec<AudioApp>, String> {
    audio::list_audio_apps()
}

/// Começa a capturar e, se `transcribe`, a transcrever cada fonte na Deepgram.
/// Retorna a pasta dos WAVs de debug, se `save_wav` estiver ligado.
#[tauri::command]
async fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    mic_id: Option<String>,
    app_pid: Option<u32>,
    save_wav: bool,
    transcribe: bool,
    keyterms: Vec<String>,
) -> Result<Option<String>, String> {
    if let Some(old) = state.capture.lock().unwrap().take() {
        old.stop();
    }
    let wav_dir = if save_wav {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("debug-audio");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Some(dir)
    } else {
        None
    };
    let started_at = Instant::now();

    let mut mic_stt = None;
    let mut sistema_stt = None;
    if transcribe {
        let api_key = secrets::load(DEEPGRAM)?
            .ok_or("salve a chave da Deepgram antes de transcrever")?;
        let config = Arc::new(DeepgramConfig {
            api_key,
            language: "pt-BR".into(),
            keyterms: keyterms.into_iter().filter(|t| !t.trim().is_empty()).collect(),
        });
        let emitter = app.clone();
        let on_stt: Arc<dyn Fn(deepgram::SttEvent) + Send + Sync> = Arc::new(move |event| {
            let _ = emitter.emit("transcricao", event);
        });
        mic_stt = Some(deepgram::spawn(config.clone(), Source::Mic, started_at, on_stt.clone()));
        if app_pid.is_some() {
            sistema_stt = Some(deepgram::spawn(config, Source::Sistema, started_at, on_stt));
        }
    }

    let emitter = app.clone();
    let capture = Capture::start(
        CaptureConfig { mic_id, app_pid, wav_dir: wav_dir.clone(), started_at },
        move |event| {
            let _ = emitter.emit("captura", event);
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
    )?;
    *state.capture.lock().unwrap() = Some(capture);
    Ok(wav_dir.map(|d| d.display().to_string()))
}

#[tauri::command]
async fn stop_capture(state: State<'_, AppState>) -> Result<(), String> {
    let capture = state.capture.lock().unwrap().take();
    if let Some(capture) = capture {
        capture.stop();
    }
    Ok(())
}

/// Chave salva, mascarada, ou `None`.
#[tauri::command]
async fn deepgram_key_status() -> Result<Option<String>, String> {
    Ok(secrets::load(DEEPGRAM)?.map(|k| secrets::mask(&k)))
}

/// Testa a chave na Deepgram e só salva se ela for aceita.
#[tauri::command]
async fn save_deepgram_key(key: String) -> Result<String, String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("a chave está vazia".into());
    }
    deepgram::check_key(&key).await?;
    secrets::save(DEEPGRAM, &key)?;
    Ok(secrets::mask(&key))
}

#[tauri::command]
async fn test_deepgram_key() -> Result<(), String> {
    let key = secrets::load(DEEPGRAM)?.ok_or("nenhuma chave salva")?;
    deepgram::check_key(&key).await
}

#[tauri::command]
async fn delete_deepgram_key() -> Result<(), String> {
    secrets::delete(DEEPGRAM)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(e) = secrets::init() {
        eprintln!("Credential Manager indisponível: {e}");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_microphones,
            list_audio_apps,
            start_capture,
            stop_capture,
            deepgram_key_status,
            save_deepgram_key,
            test_deepgram_key,
            delete_deepgram_key
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
