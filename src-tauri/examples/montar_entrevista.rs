//! Monta as duas trilhas de uma entrevista a partir das falas geradas por
//! `scripts/gerar-entrevista.ps1` (001-E.wav, 002-C.wav... na pasta do roteiro).
//!
//!     cargo run --example montar_entrevista -- ../testes/entrevista-1 [--eco] [--repetir N]
//!
//! Grava `mic.wav` (entrevistador) e `sistema.wav` (candidato) na mesma pasta, as duas
//! do mesmo tamanho e no mesmo relógio. Com `--eco`, o candidato também vaza no microfone,
//! mais baixo e um pouco atrasado, como num notebook sem fone. Com `--repetir N`, a
//! entrevista inteira se repete N vezes (pra testes longos).

use std::path::{Path, PathBuf};

const RATE: usize = 16_000;
const TURN_GAP_MS: usize = 1_200;
const LOOP_GAP_MS: usize = 3_000;
const ECHO_GAIN: f32 = 0.3;
const ECHO_DELAY_MS: usize = 60;

fn ms(ms: usize) -> usize {
    ms * RATE / 1000
}

fn read(path: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let spec = r.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels),
        (RATE as u32, 1),
        "{}: precisa ser mono 16 kHz",
        path.display()
    );
    r.samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect()
}

fn write(path: &Path, samples: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in samples {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    println!(
        "Gerado: {} ({:.1} min)",
        path.display(),
        samples.len() as f32 / RATE as f32 / 60.0
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("pasta da entrevista"));
    let (mut echo, mut repeat) = (false, 1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--eco" => echo = true,
            "--repetir" => {
                repeat = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("--repetir N")
            }
            other => panic!("opção desconhecida: {other}"),
        }
    }

    // read_dir não garante ordem; os nomes começam com o número da fala.
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    let mut lines: Vec<(bool, Vec<f32>)> = Vec::new();
    for p in names {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name.len() == 9 && name.ends_with(".wav") && name.as_bytes()[3] == b'-' {
            lines.push((name.contains("-E"), read(&p)));
        }
    }
    assert!(!lines.is_empty(), "nenhuma fala em {}", dir.display());

    let one_pass: usize = lines.iter().map(|(_, s)| s.len() + ms(TURN_GAP_MS)).sum();
    let total = (one_pass + ms(LOOP_GAP_MS)) * repeat + ms(ECHO_DELAY_MS);
    let (mut mic, mut sistema) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut at = 0;
    for _ in 0..repeat {
        for (interviewer, samples) in &lines {
            let track = if *interviewer { &mut mic } else { &mut sistema };
            track[at..at + samples.len()].copy_from_slice(samples);
            if echo && !interviewer {
                let start = at + ms(ECHO_DELAY_MS);
                for (i, s) in samples.iter().enumerate() {
                    mic[start + i] += s * ECHO_GAIN;
                }
            }
            at += samples.len() + ms(TURN_GAP_MS);
        }
        at += ms(LOOP_GAP_MS);
    }
    write(&dir.join("mic.wav"), &mic);
    write(&dir.join("sistema.wav"), &sistema);
}
