pub mod analysis;
pub mod audio;
pub mod db;
pub mod deepgram;
pub mod kit;
pub mod live;
pub mod llm;
pub mod notes;
pub mod secrets;
pub mod session;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use analysis::Candidate;
use audio::{AudioApp, Capture, CaptureConfig, Input, Microphone};
use db::{Card, Db, KitRow, Mark, NewSession, Segment, SessionDetail, SessionSummary, UsageTotals};
use deepgram::DeepgramConfig;
use kit::Kit;
use live::{AnalysisSetup, Listeners, LiveConfig, LiveSession};
use llm::{LlmClient, Provider, Slot, Usage};

const DEEPGRAM: &str = "deepgram";

const TRAY_ID: &str = "principal";

struct AppState {
    db: Arc<Db>,
    live: Mutex<Option<LiveSession>>,
    /// Teste de microfone do primeiro uso: só captura e mostra o nível.
    mic_test: Mutex<Option<Capture>>,
}

fn mark_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyM)
}

/// O ícone da bandeja mostra quando o app está ouvindo.
fn set_recording(app: &AppHandle, recording: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = if recording {
            "Copiloto · gravando a entrevista"
        } else {
            "Copiloto"
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

// Os comandos são `async` pra rodarem fora da thread principal: a captura usa COM em
// modo MTA e o `stop` espera as threads terminarem.

// Dispositivos

#[tauri::command]
async fn list_microphones() -> Result<Vec<Microphone>, String> {
    audio::list_microphones()
}

#[tauri::command]
async fn list_audio_apps() -> Result<Vec<AudioApp>, String> {
    audio::list_audio_apps()
}

// Sessão ao vivo

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    mic_id: Option<String>,
    app_pid: Option<u32>,
    replay_file: Option<String>,
    other_side_label: Option<String>,
    save_wav: bool,
    transcribe: bool,
    keyterms: Vec<String>,
    kit_id: Option<i64>,
    candidate_name: String,
    candidate_resume: String,
    candidate_checkpoints: String,
    /// Chamar a LLM a cada resposta. Sem isso, o kit só serve pro roteiro e pras
    /// perguntas proibidas.
    analyze: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    session_id: i64,
    wav_dir: Option<String>,
    kit: Option<Kit>,
    /// Coisas que não impediram a sessão, mas a pessoa precisa saber.
    warnings: Vec<String>,
}

#[tauri::command]
async fn start_session(
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartRequest,
) -> Result<Started, String> {
    let previous = state.live.lock().unwrap().take();
    if let Some(previous) = previous {
        previous.stop().await;
    }
    let mut warnings = Vec::new();
    let wav_dir = if request.save_wav {
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
    let sistema = match (request.replay_file, request.app_pid) {
        (Some(path), _) => Some(Input::File(path.into())),
        (None, Some(pid)) => Some(Input::App(pid)),
        (None, None) => None,
    };

    let kit_row = request.kit_id.map(|id| state.db.get_kit(id)).transpose()?;
    let kit = kit_row.as_ref().map(|k| kit::parse(&k.markdown));

    let deepgram = if request.transcribe {
        let api_key =
            secrets::load(DEEPGRAM)?.ok_or("salve a chave da Deepgram antes de transcrever")?;
        let mut keyterms: Vec<String> = request
            .keyterms
            .into_iter()
            .filter(|t| !t.trim().is_empty())
            .collect();
        if let Some(k) = &kit {
            keyterms.extend(k.terms.iter().cloned());
        }
        if !request.candidate_name.trim().is_empty() {
            keyterms.push(request.candidate_name.trim().to_string());
        }
        keyterms.sort();
        keyterms.dedup();
        Some(DeepgramConfig {
            api_key,
            language: "pt-BR".into(),
            keyterms,
        })
    } else {
        None
    };

    let analysis = match &kit {
        Some(kit) => {
            let llm = if request.analyze {
                match make_llm(&state.db, "live") {
                    Ok(llm) => Some(llm),
                    Err(e) => {
                        warnings.push(format!("Análise ao vivo desligada: {e}"));
                        None
                    }
                }
            } else {
                None
            };
            Some(AnalysisSetup {
                kit: kit.clone(),
                candidate: Candidate {
                    name: request.candidate_name.trim().to_string(),
                    resume: request.candidate_resume.trim().to_string(),
                    checkpoints: request.candidate_checkpoints.trim().to_string(),
                },
                llm,
            })
        }
        None => None,
    };

    let non_empty = |s: String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let record = NewSession {
        other_side: request.other_side_label,
        kit_name: kit_row.as_ref().map(|k| k.name.clone()),
        kit_markdown: kit_row.as_ref().map(|k| k.markdown.clone()),
        candidate_name: non_empty(request.candidate_name),
        candidate_resume: non_empty(request.candidate_resume),
        candidate_checkpoints: non_empty(request.candidate_checkpoints),
    };

    let emitter = |event: &'static str| {
        let app = app.clone();
        move |payload: serde_json::Value| {
            let _ = app.emit(event, payload);
        }
    };
    let (capture, stt, session, analysis_events) = (
        emitter("captura"),
        emitter("transcricao"),
        emitter("sessao"),
        emitter("analise"),
    );
    let live = LiveSession::start(
        LiveConfig {
            mic: Some(Input::Microphone(request.mic_id)),
            sistema,
            wav_dir: wav_dir.clone(),
            deepgram,
            record,
            analysis,
        },
        state.db.clone(),
        Listeners {
            on_capture: Arc::new(move |e| capture(serde_json::to_value(e).unwrap_or_default())),
            on_stt: Arc::new(move |e| stt(serde_json::to_value(e).unwrap_or_default())),
            on_session: Arc::new(move |e| session(serde_json::to_value(e).unwrap_or_default())),
            on_analysis: Arc::new(move |e| {
                analysis_events(serde_json::to_value(e).unwrap_or_default())
            }),
        },
    )?;
    if let Some(a) = &live.analyzer {
        let _ = app.emit(
            "analise",
            analysis::AnalysisEvent::Coverage {
                items: a.coverage(),
            },
        );
    }
    let session_id = live.session.id;
    *state.live.lock().unwrap() = Some(live);
    set_recording(&app, true);
    Ok(Started {
        session_id,
        wav_dir: wav_dir.map(|d| d.display().to_string()),
        kit,
        warnings,
    })
}

/// Para a sessão. Volta depois que as últimas falas foram gravadas.
#[tauri::command]
async fn stop_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let live = state.live.lock().unwrap().take();
    if let Some(live) = live {
        live.stop().await;
    }
    set_recording(&app, false);
    Ok(())
}

/// Liga ou desliga o teste de microfone (só nível, nada sai do computador).
#[tauri::command]
async fn mic_test(
    app: AppHandle,
    state: State<'_, AppState>,
    mic_id: Option<String>,
    on: bool,
) -> Result<(), String> {
    let previous = state.mic_test.lock().unwrap().take();
    if let Some(previous) = previous {
        previous.stop();
    }
    if on {
        let capture = Capture::start(
            CaptureConfig {
                mic: Some(Input::Microphone(mic_id)),
                sistema: None,
                wav_dir: None,
                started_at: Instant::now(),
            },
            move |event| {
                let _ = app.emit("captura", event);
            },
            |_| {},
        )?;
        *state.mic_test.lock().unwrap() = Some(capture);
    }
    Ok(())
}

/// Preferências simples guardadas no banco (ex.: se o primeiro uso já foi feito).
const FLAGS: &[&str] = &["onboarded"];

#[tauri::command]
async fn get_flag(state: State<'_, AppState>, key: String) -> Result<bool, String> {
    if !FLAGS.contains(&key.as_str()) {
        return Err(format!("preferência desconhecida: {key}"));
    }
    Ok(state.db.get_setting(&key)?.as_deref() == Some("1"))
}

#[tauri::command]
async fn set_flag(state: State<'_, AppState>, key: String, value: bool) -> Result<(), String> {
    if !FLAGS.contains(&key.as_str()) {
        return Err(format!("preferência desconhecida: {key}"));
    }
    state.db.set_setting(&key, if value { "1" } else { "0" })
}

/// Marca o momento atual da sessão. Devolve o instante marcado.
#[tauri::command]
async fn mark_moment(state: State<'_, AppState>, note: Option<String>) -> Result<u64, String> {
    let live = state.live.lock().unwrap();
    let live = live.as_ref().ok_or("nenhuma sessão em andamento")?;
    live.mark(note.as_deref())
}

#[tauri::command]
async fn set_paused(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    let live = state.live.lock().unwrap();
    live.as_ref()
        .ok_or("nenhuma sessão em andamento")?
        .set_paused(paused)
}

#[tauri::command]
async fn set_block(
    state: State<'_, AppState>,
    name: String,
    minutes: Option<u32>,
) -> Result<(), String> {
    let live = state.live.lock().unwrap();
    live.as_ref()
        .ok_or("nenhuma sessão em andamento")?
        .set_block(name, minutes);
    Ok(())
}

// Histórico

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionFull {
    detail: SessionDetail,
    segments: Vec<Segment>,
    cards: Vec<Card>,
    marks: Vec<Mark>,
    usage: UsageTotals,
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    state.db.list_sessions()
}

#[tauri::command]
async fn session_full(state: State<'_, AppState>, id: i64) -> Result<SessionFull, String> {
    Ok(SessionFull {
        detail: state.db.session_detail(id)?,
        segments: state.db.session_segments(id)?,
        cards: state.db.session_cards(id)?,
        marks: state.db.session_marks(id)?,
        usage: state.db.session_usage(id)?,
    })
}

fn ensure_not_running(state: &AppState, id: i64) -> Result<(), String> {
    let running = state.live.lock().unwrap().as_ref().map(|l| l.session.id);
    if running == Some(id) {
        return Err("pare a sessão antes".into());
    }
    Ok(())
}

#[tauri::command]
async fn delete_session(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    ensure_not_running(&state, id)?;
    state.db.delete_session(id)
}

// Notas e dados

#[tauri::command]
async fn generate_notes(state: State<'_, AppState>, id: i64) -> Result<notes::Notes, String> {
    ensure_not_running(&state, id)?;
    let llm = make_llm(&state.db, "notes")?;
    notes::generate(&state.db, &llm, id).await
}

#[tauri::command]
async fn save_notes(
    state: State<'_, AppState>,
    id: i64,
    notes: notes::Notes,
) -> Result<(), String> {
    let json = serde_json::to_string(&notes).map_err(|e| e.to_string())?;
    state.db.save_notes(id, &json)
}

/// Apaga a transcrição de uma sessão e mantém as notas.
#[tauri::command]
async fn delete_transcript(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    ensure_not_running(&state, id)?;
    state.db.delete_transcript(id)
}

const RETENTION_KEY: &str = "retention.days";

/// Dias até apagar a transcrição de sessões com notas; 0 = nunca.
#[tauri::command]
async fn retention_days(state: State<'_, AppState>) -> Result<i64, String> {
    Ok(state
        .db
        .get_setting(RETENTION_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

#[tauri::command]
async fn set_retention_days(state: State<'_, AppState>, days: i64) -> Result<usize, String> {
    state.db.set_setting(RETENTION_KEY, &days.to_string())?;
    if days > 0 {
        state.db.purge_old_transcripts(days)
    } else {
        Ok(0)
    }
}

#[tauri::command]
async fn delete_everything(state: State<'_, AppState>) -> Result<(), String> {
    if state.live.lock().unwrap().is_some() {
        return Err("pare a sessão antes".into());
    }
    state.db.delete_everything()
}

// Kits

#[tauri::command]
async fn list_kits(state: State<'_, AppState>) -> Result<Vec<KitRow>, String> {
    state.db.list_kits()
}

#[tauri::command]
async fn save_kit(
    state: State<'_, AppState>,
    id: Option<i64>,
    markdown: String,
) -> Result<i64, String> {
    state.db.save_kit(id, &markdown)
}

#[tauri::command]
async fn delete_kit(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_kit(id)
}

#[tauri::command]
async fn parse_kit(markdown: String) -> Kit {
    kit::parse(&markdown)
}

#[tauri::command]
async fn import_kit_file(state: State<'_, AppState>, path: String) -> Result<i64, String> {
    let markdown = std::fs::read_to_string(&path).map_err(|e| format!("não leu {path}: {e}"))?;
    state.db.save_kit(None, &markdown)
}

/// Grava texto num arquivo escolhido pela pessoa no diálogo de salvar.
#[tauri::command]
async fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("não gravou {path}: {e}"))
}

// LLM

fn slot_key(name: &str) -> Result<&'static str, String> {
    match name {
        "live" => Ok("llm.live"),
        "notes" => Ok("llm.notes"),
        _ => Err(format!("slot desconhecido: {name}")),
    }
}

fn load_slot(db: &Db, name: &str) -> Result<Slot, String> {
    let stored = db.get_setting(slot_key(name)?)?;
    Ok(stored
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_else(|| {
            if name == "live" {
                Slot::default_live()
            } else {
                Slot::default_notes()
            }
        }))
}

fn make_llm(db: &Db, name: &str) -> Result<LlmClient, String> {
    let slot = load_slot(db, name)?;
    let key = secrets::load(slot.provider.key_name())?;
    LlmClient::new(slot, key)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmSettings {
    live: Slot,
    notes: Slot,
    anthropic_key: Option<String>,
    openai_key: Option<String>,
}

#[tauri::command]
async fn llm_settings(state: State<'_, AppState>) -> Result<LlmSettings, String> {
    let mask = |p: Provider| secrets::load(p.key_name()).map(|k| k.map(|k| secrets::mask(&k)));
    Ok(LlmSettings {
        live: load_slot(&state.db, "live")?,
        notes: load_slot(&state.db, "notes")?,
        anthropic_key: mask(Provider::Anthropic)?,
        openai_key: mask(Provider::Openai)?,
    })
}

#[tauri::command]
async fn save_llm_slot(state: State<'_, AppState>, name: String, slot: Slot) -> Result<(), String> {
    let json = serde_json::to_string(&slot).map_err(|e| e.to_string())?;
    state.db.set_setting(slot_key(&name)?, &json)
}

#[tauri::command]
async fn save_llm_key(provider: Provider, key: String) -> Result<String, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("a chave está vazia".into());
    }
    secrets::save(provider.key_name(), key)?;
    Ok(secrets::mask(key))
}

#[tauri::command]
async fn delete_llm_key(provider: Provider) -> Result<(), String> {
    secrets::delete(provider.key_name())
}

#[tauri::command]
async fn test_llm(state: State<'_, AppState>, name: String) -> Result<Usage, String> {
    let llm = make_llm(&state.db, &name)?;
    llm.test().await
}

// Deepgram

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
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed || shortcut != &mark_shortcut() {
                        return;
                    }
                    let state = app.state::<AppState>();
                    let live = state.live.lock().unwrap();
                    if let Some(t) = live.as_ref().and_then(|l| l.mark(None).ok()) {
                        let _ = app.emit("marcacao", t);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Db::open(&dir.join("copiloto.db"))?;
            // Retenção: transcrições de sessões já com notas somem depois de N dias.
            if let Some(days) = db
                .get_setting(RETENTION_KEY)?
                .and_then(|v| v.parse::<i64>().ok())
                && days > 0
            {
                db.purge_old_transcripts(days)?;
            }
            app.manage(AppState {
                db: Arc::new(db),
                live: Mutex::new(None),
                mic_test: Mutex::new(None),
            });
            // Outro app pode já ter o atalho; o botão na tela continua funcionando.
            if let Err(e) = app.global_shortcut().register(mark_shortcut()) {
                eprintln!("atalho Ctrl+Shift+M indisponível: {e}");
            }

            let show = MenuItem::with_id(app, "mostrar", "Mostrar o Copiloto", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "sair", "Sair", true, None::<&str>)?;
            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("Copiloto")
                .menu(&Menu::with_items(app, &[&show, &quit])?)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "mostrar" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "sair" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_microphones,
            list_audio_apps,
            start_session,
            stop_session,
            mic_test,
            get_flag,
            set_flag,
            mark_moment,
            set_paused,
            set_block,
            list_sessions,
            session_full,
            delete_session,
            generate_notes,
            save_notes,
            delete_transcript,
            retention_days,
            set_retention_days,
            delete_everything,
            list_kits,
            save_kit,
            delete_kit,
            parse_kit,
            import_kit_file,
            write_text_file,
            llm_settings,
            save_llm_slot,
            save_llm_key,
            delete_llm_key,
            test_llm,
            deepgram_key_status,
            save_deepgram_key,
            test_deepgram_key,
            delete_deepgram_key
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
