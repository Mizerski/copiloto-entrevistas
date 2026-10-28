//! Servidor de LLM falso, compatível com `/v1/chat/completions`, pra testar o
//! encanamento da análise sem chave nem custo. Não avalia nada de verdade: devolve
//! respostas no formato pedido, citando um trecho real da última resposta.
//!
//!     cargo run --example llm_falso            (escuta em 127.0.0.1:11435)
//!
//! Na sonda: `--llm-url http://127.0.0.1:11435/v1 --llm-modelo falso`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use serde_json::{Value, json};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:11435").expect("porta 11435 ocupada");
    println!("LLM falso em http://127.0.0.1:11435/v1");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            if let Err(e) = handle(stream) {
                eprintln!("erro: {e}");
            }
        });
    }
}

fn handle(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let system = request["messages"][0]["content"]
        .as_str()
        .unwrap_or_default();
    let user = request["messages"][1]["content"]
        .as_str()
        .unwrap_or_default();

    if request["stream"] == true {
        return stream_answer(stream, user);
    }
    let content = if system.contains("\"cobertura\"") {
        analysis(system, user)
    } else if system.contains("\"competencias\"") {
        notes(user)
    } else {
        json!({ "ok": true })
    };
    println!(
        "pedido de {} bytes → {}",
        body.len(),
        preview(&content.to_string())
    );
    let response = json!({
        "model": "falso",
        "choices": [{ "message": { "role": "assistant", "content": content.to_string() }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": body.len() / 4, "completion_tokens": 120 }
    })
    .to_string();
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )
}

/// Resposta sugerida em streaming (Server-Sent Events), uma palavra por vez, repetindo o
/// fim da pergunta pra dar pra ver qual versão chegou.
fn stream_answer(mut stream: TcpStream, user: &str) -> std::io::Result<()> {
    let question = user
        .lines()
        .rev()
        .find_map(|l| l.split_once("Entrevistador").map(|(_, t)| t))
        .unwrap_or_default();
    let tail: Vec<&str> = question.split_whitespace().rev().take(6).collect();
    let text = format!(
        "Resposta pra \"{}\".\n\nSegundo parágrafo com **um destaque**.",
        tail.into_iter().rev().collect::<Vec<_>>().join(" ")
    );
    println!("stream → {}", preview(&text));
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
    )?;
    for word in text.split_inclusive(' ') {
        let chunk = json!({ "model": "falso", "choices": [{ "delta": { "content": word } }] });
        write!(stream, "data: {chunk}\n\n")?;
        stream.flush()?;
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let usage = json!({ "model": "falso", "choices": [], "usage": { "prompt_tokens": user.len() / 4, "completion_tokens": 30 } });
    write!(stream, "data: {usage}\n\ndata: [DONE]\n\n")
}

/// Análise de uma resposta: a primeira competência do kit, forte, citando as primeiras
/// palavras da última fala do candidato.
fn analysis(system: &str, user: &str) -> Value {
    let competency = system
        .lines()
        .skip_while(|l| !l.starts_with("## Competências"))
        .find_map(|l| l.strip_prefix("### "))
        .unwrap_or("Comunicação")
        .to_string();
    let answer = user
        .lines()
        .rev()
        .find_map(|l| l.split_once("] Candidato: ").map(|(_, t)| t))
        .unwrap_or_default();
    let quote: String = answer
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "cobertura": [{ "competencia": competency, "nivel": "forte", "trecho": quote }],
        "cards": [{ "tipo": "aprofundar", "texto": "Pergunte qual foi a parte dele nessa decisão.", "prioridade": "media" }],
        "resumo": format!("Candidato falou sobre: {quote}…")
    })
}

/// Notas finais: uma competência com uma evidência citada da transcrição.
fn notes(user: &str) -> Value {
    let line = user.lines().find_map(|l| {
        l.split_once("] Candidato: ")
            .map(|(t, text)| (t.trim_start_matches('['), text))
    });
    let (minute, quote) = line
        .map(|(t, text)| {
            (
                t.to_string(),
                text.split_whitespace()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .unwrap_or_default();
    json!({
        "competencias": [{
            "competencia": "Modelagem de dados e consistência",
            "nota_sugerida": 3,
            "a_favor": [{ "texto": "Explicou a solução com um caso real.", "trecho": quote, "minuto": minute }],
            "contra": [],
            "sem_evidencia": "Não falou de testes."
        }],
        "curriculo": [{ "ponto": "Liderou a migração", "situacao": "nao_verificado", "comentario": "Usou sempre o plural." }],
        "proximas_perguntas": ["Qual foi exatamente a sua parte na migração?"],
        "resumo": "Entrevista técnica curta sobre pagamentos."
    })
}

fn preview(s: &str) -> String {
    s.chars().take(100).collect()
}
