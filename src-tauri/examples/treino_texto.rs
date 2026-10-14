//! Treino só com texto, sem áudio: o entrevistador simulado (Claude, com a chave salva
//! pelo app) conduz, e as respostas do candidato vêm de um arquivo, uma por linha. Serve
//! pra ver a persona, as dicas dos balões e o parecer do fim sem falar no microfone.
//!
//!     cargo run --example treino_texto -- KIT.md RESPOSTAS.txt [persona] [--notas]
//!
//! O relógio é simulado: cada resposta "dura" 4 s a cada 10 palavras.

use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use copiloto_entrevistas_lib::analysis::{AnalysisEvent, Analyzer, Candidate, Side};
use copiloto_entrevistas_lib::audio::Source;
use copiloto_entrevistas_lib::db::{Db, NewSession};
use copiloto_entrevistas_lib::llm::{LlmClient, Provider, Slot};
use copiloto_entrevistas_lib::session::{Session, SessionEvent};
use copiloto_entrevistas_lib::{kit, notes, practice, secrets};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let with_notes = args.iter().any(|a| a == "--notas");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--notas").collect();
    let kit_md = std::fs::read_to_string(args[0]).expect("ler o kit");
    let answers: Vec<String> = std::fs::read_to_string(args[1])
        .expect("ler as respostas")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    let persona = practice::persona(args.get(2).map_or("exigente", |s| s.as_str()))
        .expect("persona desconhecida");

    let _ = secrets::init();
    let key = secrets::load(Provider::Anthropic.key_name())
        .ok()
        .flatten()
        .expect("salve a chave da Anthropic pelo app");
    let llm = |slot: Slot| LlmClient::new(slot, Some(key.clone())).expect("configurar a LLM");

    let db = Arc::new(Db::open_in_memory().expect("banco em memória"));
    let kit = kit::parse(&kit_md);
    let (tx, rx) = mpsc::channel::<AnalysisEvent>();
    let slot: Arc<OnceLock<Arc<Analyzer>>> = Arc::default();
    let session = {
        let slot = slot.clone();
        Session::start(
            db.clone(),
            &NewSession {
                kit_name: Some(kit.title.clone()),
                kit_markdown: Some(kit_md.clone()),
                practice_persona: Some(persona.id.into()),
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
        Some(llm(Slot::default_live())),
        kit,
        &Candidate::default(),
        Side::Practice(persona),
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    );
    let _ = slot.set(analyzer.clone());

    let mut now: u64 = 0;
    let mut answers = answers.into_iter();
    analyzer.start_practice();
    println!("Persona: {}\n", persona.name);
    loop {
        let event = match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(e) => e,
            Err(_) => {
                println!("(nada em 60 s)");
                break;
            }
        };
        match event {
            AnalysisEvent::InterviewerSays { text, end } => {
                // A fala "toca" e entra na transcrição como a outra ponta.
                let spoken = text.split_whitespace().count() as u64 * 400;
                println!("ENTREVISTADOR [{}]: {text}\n", clock(now));
                session.on_transcript(Source::Sistema, &text, true, now, now + spoken, 0);
                now += spoken + 1_000;
                if end {
                    break;
                }
                let Some(answer) = answers.next() else {
                    println!("(acabaram as respostas)");
                    break;
                };
                println!("VOCÊ [{}]: {answer}\n", clock(now));
                // A resposta chega em pedaços de ~10 palavras, como da transcrição.
                let words: Vec<&str> = answer.split_whitespace().collect();
                for chunk in words.chunks(10) {
                    let end = now + 4_000;
                    session.on_transcript(Source::Mic, &chunk.join(" "), true, now, end, 0);
                    now = end;
                    // Ritmo de fala real: ~10 palavras a cada 4 s.
                    std::thread::sleep(Duration::from_millis(4_000));
                }
                // Silêncio: a resposta fecha.
                now += 3_500;
                session.tick(now);
                analyzer.tick(now, session.heard_until(Source::Mic), now);
            }
            AnalysisEvent::Suggestion { suggestion: s } => {
                print!("  · {} [{}] {}", s.slot, s.kind, s.question);
                if let (Some(a), Some(b)) = (&s.earlier, &s.now) {
                    print!("  (antes: \"{}\" | agora: \"{}\")", a.text, b.text);
                }
                println!();
            }
            AnalysisEvent::Status {
                state,
                message: Some(m),
                ..
            } if state == "erro" || state == "descartada" => println!("  ! {state}: {m}"),
            _ => {}
        }
    }
    // Dá tempo da última análise voltar.
    std::thread::sleep(Duration::from_secs(6));
    session.finish(now);

    if with_notes {
        println!("\nGerando o parecer…");
        let llm = llm(Slot::default_notes());
        match tauri::async_runtime::block_on(notes::generate(&db, &llm, session.id)) {
            Ok(n) => {
                for c in &n.competencies {
                    println!("\n{} — nota {}", c.name, c.suggested_score);
                    for e in &c.for_evidence {
                        println!("  + [{}] {} (\"{}\")", e.minute, e.text, e.quote);
                    }
                    for e in &c.against {
                        println!("  - [{}] {} (\"{}\")", e.minute, e.text, e.quote);
                    }
                }
                println!("\nResumo: {}", n.summary);
            }
            Err(e) => println!("parecer falhou: {e}"),
        }
    }
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}
