//! Testa captura, transcrição, sessão e análise sem a interface, pelo mesmo caminho do app.
//!
//!     cargo run --example sonda -- [opções]
//!
//!     --app PID              outro lado = áudio desse app (loopback por processo)
//!     --arquivo X.wav        outro lado = WAV tocado em tempo real (modo replay)
//!     --mic-arquivo X.wav    você = WAV em vez do microfone
//!     --sem-mic              não captura você
//!     --segundos N           duração (padrão: 5, ou o arquivo mais longo + 4)
//!     --esperado X.txt       texto do outro lado; calcula a taxa de erro de palavras
//!     --esperado-mic X.txt   o mesmo pra você (mostra se o eco vazou)
//!     --termos "a,b"         termos difíceis pra Deepgram
//!     --sem-transcricao      só captura
//!     --perfil X.md          seu perfil (liga os balões de resposta; precisa de LLM)
//!     --vaga "texto"         descrição da vaga
//!     --llm-url URL          LLM compatível com OpenAI pra análise (ex.: o `llm_falso`)
//!     --llm-modelo M         modelo dessa LLM (chave opcional em LLM_API_KEY)
//!     --notas                gera as notas pós-entrevista no fim (precisa de LLM)
//!     --anthropic            LLM = Claude com a configuração padrão do app e a chave salva
//!                            pelo app (--llm-modelo troca o modelo)
//!
//! A chave da Deepgram vem de DEEPGRAM_API_KEY ou, se não definida, do Credential
//! Manager (a mesma que o app salva). A sessão vai pra um banco em memória.
//! Salva mic.wav e sistema.wav em target/sonda.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use copiloto_entrevistas_lib::analysis::AnalysisEvent;
use copiloto_entrevistas_lib::audio::{self, CaptureEvent, Input, Source};
use copiloto_entrevistas_lib::db::{Db, NewSession, Segment};
use copiloto_entrevistas_lib::deepgram::{self, DeepgramConfig, SttEvent};
use copiloto_entrevistas_lib::live::{AnalysisSetup, Listeners, LiveConfig, LiveSession};
use copiloto_entrevistas_lib::llm::{LlmClient, Provider, Slot};
use copiloto_entrevistas_lib::notes;
use copiloto_entrevistas_lib::profile;
use copiloto_entrevistas_lib::secrets;
use copiloto_entrevistas_lib::session::{SessionEvent, words};

#[derive(Default)]
struct Args {
    app: Option<u32>,
    file: Option<PathBuf>,
    mic_file: Option<PathBuf>,
    no_mic: bool,
    seconds: Option<u64>,
    expected: Option<PathBuf>,
    expected_mic: Option<PathBuf>,
    keyterms: Vec<String>,
    no_stt: bool,
    profile: Option<PathBuf>,
    job: Option<String>,
    llm_url: Option<String>,
    llm_model: Option<String>,
    notes: bool,
    anthropic: bool,
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| panic!("{a} precisa de um valor"))
        };
        match a.as_str() {
            "--app" => args.app = Some(value().parse().expect("PID inválido")),
            "--arquivo" => args.file = Some(value().into()),
            "--mic-arquivo" => args.mic_file = Some(value().into()),
            "--sem-mic" => args.no_mic = true,
            "--segundos" => args.seconds = Some(value().parse().expect("segundos inválidos")),
            "--esperado" => args.expected = Some(value().into()),
            "--esperado-mic" => args.expected_mic = Some(value().into()),
            "--termos" => {
                args.keyterms = value().split(',').map(|t| t.trim().to_string()).collect()
            }
            "--sem-transcricao" => args.no_stt = true,
            "--perfil" => args.profile = Some(value().into()),
            "--vaga" => args.job = Some(value()),
            "--llm-url" => args.llm_url = Some(value()),
            "--llm-modelo" => args.llm_model = Some(value()),
            "--notas" => args.notes = true,
            "--anthropic" => args.anthropic = true,
            other => panic!("opção desconhecida: {other}"),
        }
    }
    args
}

fn wav_seconds(path: &PathBuf) -> u64 {
    let r = hound::WavReader::open(path).expect("abrir o WAV");
    r.duration() as u64 / r.spec().sample_rate as u64
}

fn main() {
    let args = parse_args();

    println!("Microfones:");
    for m in audio::list_microphones().expect("listar microfones") {
        println!("  {} {}", if m.default { "*" } else { " " }, m.name);
    }
    println!("Apps com sessão de áudio:");
    for a in audio::list_audio_apps().expect("listar apps") {
        println!(
            "  {:>6}  {}{}",
            a.pid,
            a.name,
            if a.active { "  (ativo)" } else { "" }
        );
    }

    let longest_file = [&args.file, &args.mic_file]
        .into_iter()
        .flatten()
        .map(wav_seconds)
        .max();
    let seconds = args.seconds.unwrap_or(longest_file.map_or(5, |s| s + 4));
    let sistema = match (&args.file, args.app) {
        (Some(f), _) => Some(Input::File(f.clone())),
        (None, Some(pid)) => Some(Input::App(pid)),
        (None, None) => None,
    };
    let mic = match (&args.mic_file, args.no_mic) {
        (Some(f), _) => Some(Input::File(f.clone())),
        (None, true) => None,
        (None, false) => Some(Input::Microphone(None)),
    };

    let profile_md = args
        .profile
        .as_ref()
        .map(|p| std::fs::read_to_string(p).expect("ler o perfil"));
    let parsed = profile_md.as_deref().map(profile::parse);
    if let Some(p) = &parsed {
        println!("Perfil: {} ({} histórias)", p.name, p.stories.len());
        for w in &p.warnings {
            println!("  aviso: {w}");
        }
    }

    let deepgram_config = if args.no_stt {
        None
    } else {
        let api_key = std::env::var("DEEPGRAM_API_KEY").ok().or_else(|| {
            secrets::init().ok()?;
            secrets::load("deepgram").ok().flatten()
        });
        let Some(api_key) = api_key else {
            println!("Sem chave: defina DEEPGRAM_API_KEY ou salve a chave pelo app.");
            return;
        };
        if let Err(e) = tauri::async_runtime::block_on(deepgram::check_key(&api_key)) {
            println!("Chave recusada: {e}");
            return;
        }
        let mut keyterms = args.keyterms.clone();
        if let Some(p) = &parsed {
            keyterms.extend(p.terms.iter().cloned());
        }
        Some(DeepgramConfig {
            api_key,
            language: "pt-BR".into(),
            keyterms,
        })
    };

    // `for_notes`: com --anthropic, o slot de notas usa esforço maior, como no app.
    let make_llm = |for_notes: bool| {
        if args.anthropic {
            let _ = secrets::init();
            let key = secrets::load(Provider::Anthropic.key_name())
                .ok()
                .flatten()
                .expect("salve a chave da Anthropic pelo app");
            let mut slot = if for_notes {
                Slot::default_notes()
            } else {
                Slot::default_live()
            };
            if let Some(m) = &args.llm_model {
                slot.model = m.clone();
            }
            return Some(LlmClient::new(slot, Some(key)).expect("configurar a LLM"));
        }
        args.llm_url.as_ref().map(|url| {
            let slot = Slot {
                provider: Provider::Openai,
                model: args.llm_model.clone().unwrap_or_else(|| "falso".into()),
                base_url: url.clone(),
                effort: String::new(),
            };
            LlmClient::new(slot, std::env::var("LLM_API_KEY").ok()).expect("configurar a LLM")
        })
    };
    let llm = make_llm(false);

    let wav_dir = PathBuf::from("target/sonda");
    std::fs::create_dir_all(&wav_dir).unwrap();

    let db = Arc::new(Db::open_in_memory().expect("banco em memória"));
    let partial_latency: Arc<Mutex<BTreeMap<String, Vec<u64>>>> = Arc::default();
    let per_second: Arc<Mutex<BTreeMap<(u64, String), f32>>> = Arc::default();
    let listeners = {
        let (partial_latency, per_second) = (partial_latency.clone(), per_second.clone());
        Listeners {
            on_capture: Arc::new(move |event| match event {
                CaptureEvent::Level {
                    source, t_ms, rms, ..
                } => {
                    let mut map = per_second.lock().unwrap();
                    let v = map.entry((t_ms / 1000, format!("{source:?}"))).or_default();
                    *v = v.max(rms);
                }
                other => println!("{other:?}"),
            }),
            on_stt: Arc::new(move |event| match event {
                SttEvent::Transcript {
                    source,
                    is_final: false,
                    latency_ms,
                    ..
                } => {
                    let mut map = partial_latency.lock().unwrap();
                    map.entry(format!("{source:?}"))
                        .or_default()
                        .push(latency_ms);
                }
                SttEvent::Transcript { .. } => {}
                other => println!("{other:?}"),
            }),
            on_session: Arc::new(|event| match event {
                SessionEvent::Segment { segment: s } => println!(
                    "  [{:?} {:>5.1}s +{} ms]{} {}",
                    s.source,
                    s.start_ms as f32 / 1000.0,
                    s.latency_ms,
                    if s.echo { " ECO DESCARTADO:" } else { "" },
                    s.text
                ),
                other => println!("{other:?}"),
            }),
            on_analysis: Arc::new(|event| match event {
                AnalysisEvent::Suggestion { suggestion: s } => {
                    println!(
                        "    BALÃO {} [{}] {} ({}; {} ms, fala até {})",
                        s.slot, s.kind, s.question, s.why, s.latency_ms, s.t_ms
                    );
                    if let (Some(a), Some(b)) = (&s.earlier, &s.now) {
                        println!(
                            "      antes {:?}: {} | agora {:?}: {}",
                            a.at_ms, a.text, b.at_ms, b.text
                        );
                    }
                }
                AnalysisEvent::SuggestionUsed { slot, id } => {
                    println!("    BALÃO {slot} usado (card {id})")
                }
                AnalysisEvent::Status {
                    state,
                    message,
                    latency_ms,
                } => println!(
                    "    análise: {state}{}{}",
                    latency_ms
                        .map(|l| format!(" em {l} ms"))
                        .unwrap_or_default(),
                    message.map(|m| format!(" ({m})")).unwrap_or_default()
                ),
            }),
        }
    };

    let analysis = match (&profile_md, llm) {
        (Some(profile), Some(llm)) => Some(AnalysisSetup {
            llm,
            profile: profile.clone(),
            job: args.job.clone().unwrap_or_default(),
        }),
        _ => None,
    };
    let analyzing = analysis.is_some();
    let live = LiveSession::start(
        LiveConfig {
            mic,
            sistema,
            wav_dir: Some(wav_dir.clone()),
            deepgram: deepgram_config,
            record: NewSession {
                other_side: Some("sonda".into()),
                candidate_name: parsed.as_ref().map(|p| p.name.clone()),
                candidate_resume: profile_md.clone(),
                side: Some("candidato".into()),
                ..Default::default()
            },
            analysis,
        },
        db.clone(),
        listeners,
    )
    .expect("iniciar sessão");
    let session_id = live.session.id;

    println!("Capturando por {seconds} s…");
    std::thread::sleep(Duration::from_secs(seconds));
    tauri::async_runtime::block_on(live.stop());
    // Tempo pra última análise voltar.
    if analyzing {
        std::thread::sleep(Duration::from_secs(3));
    }

    if args.no_stt {
        println!("RMS máximo por segundo:");
        for ((sec, source), rms) in per_second.lock().unwrap().iter() {
            let bar = "#".repeat(((rms * 400.0) as usize).min(60));
            println!("  {sec:>3}s {source:<8} {rms:.4} {bar}");
        }
    } else {
        let segments = db.session_segments(session_id).expect("ler a sessão");
        report(&segments, &partial_latency.lock().unwrap());
        compare(&segments, Source::Sistema, args.expected.as_ref());
        compare(&segments, Source::Mic, args.expected_mic.as_ref());
        let cards = db.session_cards(session_id).expect("ler os cards");
        let usage = db.session_usage(session_id).expect("ler o uso");
        println!(
            "\nCards gravados: {} | chamadas de LLM: {} ({} tokens de entrada, {} de saída)",
            cards.len(),
            usage.calls,
            usage.input_tokens,
            usage.output_tokens
        );
    }
    if let (true, Some(llm)) = (args.notes, make_llm(true)) {
        println!("\nGerando notas…");
        let started = std::time::Instant::now();
        match tauri::async_runtime::block_on(notes::generate(&db, &llm, session_id)) {
            Ok(n) => {
                println!(
                    "  Modelo {} em {:.1} s",
                    n.model,
                    started.elapsed().as_secs_f32()
                );
                println!("  Resumo: {}", n.summary);
                for c in &n.competencies {
                    println!("  {}: nota sugerida {}", c.name, c.suggested_score);
                    for e in &c.for_evidence {
                        println!("    + [{}] {} — \"{}\"", e.minute, e.text, e.quote);
                    }
                    for e in &c.against {
                        println!("    - [{}] {} — \"{}\"", e.minute, e.text, e.quote);
                    }
                    if !c.missing.is_empty() {
                        println!("    sem evidência: {}", c.missing);
                    }
                }
                for p in &n.resume_points {
                    println!("  Currículo [{}] {} — {}", p.status, p.point, p.comment);
                }
                println!(
                    "  Evidências descartadas por trecho inexistente: {}",
                    n.dropped_quotes
                );
                for q in &n.next_questions {
                    println!("  Próxima pergunta: {q}");
                }
                let usage = db.session_usage(session_id).expect("ler o uso");
                println!(
                    "  Uso total da sessão: {} chamadas, {} tokens de entrada ({} do cache), {} de saída",
                    usage.calls, usage.input_tokens, usage.cached_tokens, usage.output_tokens
                );
            }
            Err(e) => println!("  Falhou: {e}"),
        }
    }
    println!("WAVs em {}", wav_dir.canonicalize().unwrap().display());
}

fn report(segments: &[Segment], partials: &BTreeMap<String, Vec<u64>>) {
    println!("\nLatência (do fim do áudio até o texto chegar):");
    for source in [Source::Mic, Source::Sistema] {
        let mut lat: Vec<u64> = segments
            .iter()
            .filter(|s| s.source == source && !s.echo)
            .map(|s| s.latency_ms)
            .collect();
        let partial = partials
            .get(&format!("{source:?}"))
            .cloned()
            .unwrap_or_default();
        if lat.is_empty() && partial.is_empty() {
            continue;
        }
        lat.sort_unstable();
        println!(
            "  {source:?}: final n={} média {} p50 {} p90 {} máx {} | parcial n={} média {}",
            lat.len(),
            ms(mean(&lat)),
            ms(percentile(&lat, 0.5)),
            ms(percentile(&lat, 0.9)),
            ms(lat.last().copied()),
            partial.len(),
            ms(mean(&partial)),
        );
    }
    let echoes = segments.iter().filter(|s| s.echo).count();
    println!("Falas do microfone descartadas como eco: {echoes}");
}

/// Compara o que ficou (sem os ecos) de uma fonte com o texto esperado.
fn compare(segments: &[Segment], source: Source, expected: Option<&PathBuf>) {
    let Some(expected) = expected else { return };
    let reference = std::fs::read_to_string(expected).expect("ler o texto esperado");
    let hypothesis: String = segments
        .iter()
        .filter(|s| s.source == source && !s.echo)
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let (errors, total) = word_errors(&words(&reference), &words(&hypothesis));
    println!(
        "\nErro de palavras (WER) de {source:?}: {:.1}% ({errors} de {total} palavras)",
        100.0 * errors as f32 / total.max(1) as f32
    );
    println!("Saiu: {hypothesis}");
}

fn mean(v: &[u64]) -> Option<u64> {
    (!v.is_empty()).then(|| v.iter().sum::<u64>() / v.len() as u64)
}

fn percentile(sorted: &[u64], p: f32) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let i = ((sorted.len() - 1) as f32 * p).round() as usize;
    Some(sorted[i])
}

fn ms(v: Option<u64>) -> String {
    v.map(|v| format!("{v} ms")).unwrap_or_else(|| "—".into())
}

/// Distância de edição em palavras (substituições + inserções + remoções).
fn word_errors(reference: &[String], hypothesis: &[String]) -> (usize, usize) {
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    for (i, r) in reference.iter().enumerate() {
        let mut row = vec![i + 1; hypothesis.len() + 1];
        for (j, h) in hypothesis.iter().enumerate() {
            let cost = usize::from(r != h);
            row[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1);
        }
        prev = row;
    }
    (prev[hypothesis.len()], reference.len())
}
