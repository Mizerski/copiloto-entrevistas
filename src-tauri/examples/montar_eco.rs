//! Monta os WAVs do teste de eco a partir das falas geradas por `gerar-audio-teste.ps1`.
//!
//!     cargo run --example montar_eco -- ../testes
//!
//! Lê `pergunta-1.wav`, `resposta-pagamentos.wav` e `pergunta-2.wav` (mono, 16 kHz) e
//! grava:
//! - `eco-sistema.wav`: o candidato. Silêncio enquanto você pergunta, depois a resposta.
//! - `eco-mic.wav`: você. As duas perguntas e, no meio, a resposta do candidato vazando
//!   da caixa de som, mais baixa e um pouco atrasada, como num notebook sem fone.

use std::path::Path;

const RATE: usize = 16_000;
const ECHO_GAIN: f32 = 0.3;
const ECHO_DELAY_MS: usize = 60;
const GAP_MS: usize = 1_000;

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
        "Gerado: {} ({:.1} s)",
        path.display(),
        samples.len() as f32 / RATE as f32
    );
}

fn ms(ms: usize) -> usize {
    ms * RATE / 1000
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../testes".into());
    let dir = Path::new(&dir);
    let q1 = read(&dir.join("pergunta-1.wav"));
    let answer = read(&dir.join("resposta-pagamentos.wav"));
    let q2 = read(&dir.join("pergunta-2.wav"));

    let answer_at = q1.len() + ms(GAP_MS);
    let q2_at = answer_at + answer.len() + ms(GAP_MS);
    let total = q2_at + q2.len() + ms(GAP_MS);

    let mut sistema = vec![0.0; total];
    sistema[answer_at..answer_at + answer.len()].copy_from_slice(&answer);

    let mut mic = vec![0.0; total];
    mic[..q1.len()].copy_from_slice(&q1);
    let echo_at = answer_at + ms(ECHO_DELAY_MS);
    for (i, s) in answer.iter().enumerate() {
        mic[echo_at + i] += s * ECHO_GAIN;
    }
    mic[q2_at..q2_at + q2.len()].copy_from_slice(&q2);

    write(&dir.join("eco-sistema.wav"), &sistema);
    write(&dir.join("eco-mic.wav"), &mic);
}
