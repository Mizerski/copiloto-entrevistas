//! Modo candidato só com texto, sem áudio: um roteiro com as falas do entrevistador
//! (E:) e as suas (C:) passa pela análise como se viesse da transcrição, e mostra os
//! balões que apareceriam (Claude de verdade, com a chave salva pelo app).
//!
//!     cargo run --example candidato_texto -- PERFIL.md ROTEIRO.txt ["descrição da vaga"]
//!
//! O relógio é simulado no ritmo de fala real: ~10 palavras a cada 4 s.

use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use copiloto_entrevistas_lib::analysis::{AnalysisEvent, Analyzer};
use copiloto_entrevistas_lib::audio::Source;
use copiloto_entrevistas_lib::db::{Db, NewSession};
use copiloto_entrevistas_lib::llm::{LlmClient, Provider, Slot};
use copiloto_entrevistas_lib::session::{Session, SessionEvent};
use copiloto_entrevistas_lib::{profile, secrets};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let profile_md = std::fs::read_to_string(&args[0]).expect("ler o perfil");
    let script: Vec<(Source, String)> = std::fs::read_to_string(&args[1])
        .expect("ler o roteiro")
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if let Some(t) = l.strip_prefix("E:") {
                Some((Source::Sistema, t.trim().to_string()))
            } else {
                l.strip_prefix("C:")
                    .map(|t| (Source::Mic, t.trim().to_string()))
            }
        })
        .collect();
    let job = args.get(2).cloned().unwrap_or_default();

    // Com LLM_URL (ex.: o `llm_falso`), usa um servidor compatível com OpenAI no lugar do Claude.
    let llm = if let Ok(url) = std::env::var("LLM_URL") {
        let slot = Slot {
            provider: Provider::Openai,
            model: "falso".into(),
            base_url: url,
            effort: String::new(),
        };
        LlmClient::new(slot, None).expect("configurar a LLM")
    } else {
        let _ = secrets::init();
        let key = secrets::load(Provider::Anthropic.key_name())
            .ok()
            .flatten()
            .expect("salve a chave da Anthropic pelo app");
        LlmClient::new(Slot::default_live(), Some(key)).expect("configurar a LLM")
    };

    let db = Arc::new(Db::open_in_memory().expect("banco em memória"));
    let name = profile::parse(&profile_md).name;
    let (tx, rx) = mpsc::channel::<AnalysisEvent>();
    let slot: Arc<OnceLock<Arc<Analyzer>>> = Arc::default();
    let session = {
        let slot = slot.clone();
        Session::start(
            db.clone(),
            &NewSession {
                candidate_name: Some(name.clone()),
                candidate_resume: Some(profile_md.clone()),
                side: Some("candidato".into()),
                ..Default::default()
            },
            Arc::new(move |e: SessionEvent| {
                if let (SessionEvent::Segment { segment }, Some(a)) = (&e, slot.get()) {
                    a.on_segment(segment);
                }
            }),
        )
        .expect("sessão")
    };
    let analyzer = Analyzer::new(
        session.id,
        db.clone(),
        llm,
        &profile_md,
        &job,
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    );
    let _ = slot.set(analyzer.clone());

    // Os balões chegam por outra thread enquanto o roteiro anda.
    let printer = std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                AnalysisEvent::Answer { answer: a } if a.done => {
                    println!(
                        "    · resposta {} ({} ms): {}",
                        a.turn + 1,
                        a.latency_ms,
                        a.text
                    )
                }
                AnalysisEvent::Status {
                    state,
                    message: Some(m),
                    ..
                } if state == "erro" || state == "descartada" => println!("    ! {state}: {m}"),
                _ => {}
            }
        }
    });

    let mut now: u64 = 0;
    for (source, text) in script {
        let who = if source == Source::Sistema {
            "ENTREVISTADOR"
        } else {
            "VOCÊ"
        };
        println!("\n{who} [{}]: {text}", clock(now));
        let words: Vec<&str> = text.split_whitespace().collect();
        let chunks: Vec<&[&str]> = words.chunks(10).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let end = now + 4_000;
            // Só o último pedaço fecha a fala, como a Deepgram faz na pausa.
            let speech_final = i + 1 == chunks.len();
            session.on_transcript(source, &chunk.join(" "), true, speech_final, now, end, 0);
            now = end;
            std::thread::sleep(Duration::from_millis(4_000));
        }
        // Pausa entre as falas: a pergunta (ou a resposta) fecha.
        for _ in 0..8 {
            now += 500;
            session.tick(now);
            analyzer.tick(now, &session.heard());
            std::thread::sleep(Duration::from_millis(500));
        }
        if source == Source::Sistema {
            // Tempo de ler os balões antes de responder.
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    // Dá tempo dos últimos balões chegarem; a thread que imprime morre com o processo.
    std::thread::sleep(Duration::from_secs(8));
    session.finish(now);
    drop(printer);
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}
