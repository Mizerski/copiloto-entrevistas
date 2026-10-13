pub mod audio;
pub mod db;
pub mod deepgram;
pub mod live;
pub mod secrets;
pub mod session;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use audio::{AudioApp, Input, Microphone};
use db::{Db, Segment, SessionSummary};
use deepgram::DeepgramConfig;
use live::{Listeners, LiveConfig, LiveSession};

const DEEPGRAM: &str = "deepgram";

struct AppState {
    db: Arc<Db>,
    live: Mutex<Option<LiveSession>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    session_id: i64,
    wav_dir: Option<String>,
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

/// Abre uma sessão: captura e, se `transcribe`, transcreve cada fonte na Deepgram.
/// O outro lado vem de `app_pid` ou, no modo replay, de `replay_file`.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // os argumentos vêm direto da tela
async fn start_session(
    app: AppHandle,
    state: State<'_, AppState>,
    mic_id: Option<String>,
    app_pid: Option<u32>,
    replay_file: Option<String>,
    other_side_label: Option<String>,
    save_wav: bool,
    transcribe: bool,
    keyterms: Vec<String>,
) -> Result<Started, String> {
    let previous = state.live.lock().unwrap().take();
    if let Some(previous) = previous {
        previous.stop().await;
    }
    let wav_dir = if save_wav {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("debug-audio");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Some(dir)
    } else {
        None
    };
    let sistema = match (replay_file, app_pid) {
        (Some(path), _) => Some(Input::File(path.into())),
        (None, Some(pid)) => Some(Input::App(pid)),
        (None, None) => None,
    };
    let deepgram = if transcribe {
        let api_key =
            secrets::load(DEEPGRAM)?.ok_or("salve a chave da Deepgram antes de transcrever")?;
        Some(DeepgramConfig {
            api_key,
            language: "pt-BR".into(),
            keyterms: keyterms
                .into_iter()
                .filter(|t| !t.trim().is_empty())
                .collect(),
        })
    } else {
        None
    };

    let (a, b, c) = (app.clone(), app.clone(), app.clone());
    let live = LiveSession::start(
        LiveConfig {
            mic: Some(Input::Microphone(mic_id)),
            sistema,
            other_side_label,
            wav_dir: wav_dir.clone(),
            deepgram,
        },
        state.db.clone(),
        Listeners {
            on_capture: Arc::new(move |e| {
                let _ = a.emit("captura", e);
            }),
            on_stt: Arc::new(move |e| {
                let _ = b.emit("transcricao", e);
            }),
            on_session: Arc::new(move |e| {
                let _ = c.emit("sessao", e);
            }),
        },
    )?;
    let session_id = live.session.id;
    *state.live.lock().unwrap() = Some(live);
    Ok(Started {
        session_id,
        wav_dir: wav_dir.map(|d| d.display().to_string()),
    })
}

/// Para a sessão. Volta depois que as últimas falas foram gravadas.
#[tauri::command]
async fn stop_session(state: State<'_, AppState>) -> Result<(), String> {
    let live = state.live.lock().unwrap().take();
    if let Some(live) = live {
        live.stop().await;
    }
    Ok(())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    state.db.list_sessions()
}

#[tauri::command]
async fn session_segments(state: State<'_, AppState>, id: i64) -> Result<Vec<Segment>, String> {
    state.db.session_segments(id)
}

#[tauri::command]
async fn delete_session(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let running = state.live.lock().unwrap().as_ref().map(|l| l.session.id);
    if running == Some(id) {
        return Err("pare a sessão antes de apagar".into());
    }
    state.db.delete_session(id)
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Db::open(&dir.join("copiloto.db"))?;
            app.manage(AppState {
                db: Arc::new(db),
                live: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_microphones,
            list_audio_apps,
            start_session,
            stop_session,
            list_sessions,
            session_segments,
            delete_session,
            deepgram_key_status,
            save_deepgram_key,
            test_deepgram_key,
            delete_deepgram_key
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
