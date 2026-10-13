# Copiloto de entrevistas

App desktop pra Windows que ouve uma entrevista real (Teams, Meet, Zoom), transcreve as duas pontas e ajuda quem entrevista. Tauri 2 + React + Rust.

Estado atual: spike da fase 0, que valida captura de áudio e transcrição em streaming.

## O que o spike faz

- Captura o microfone e o áudio de um único app (loopback por processo do WASAPI), as duas fontes em mono 16 kHz e no mesmo relógio.
- Transcreve cada fonte numa conexão própria com a Deepgram (Nova-3, `pt-BR`), com texto parcial e final.
- Mede a latência entre o fim do áudio capturado e a chegada do texto.
- Guarda a chave da Deepgram no Credential Manager do Windows.
- Testa "sempre no topo" e "esconder do compartilhamento de tela".

## Rodar

```bash
npm install
npm run tauri dev
```

Sonda de linha de comando, sem interface (lista dispositivos, captura e salva WAVs em `src-tauri/target/sonda`):

```bash
cd src-tauri
cargo run --example sonda -- <pid-do-app> <segundos>
```

Com `DEEPGRAM_API_KEY` definida, a sonda também transcreve.

## Código

- `src-tauri/src/audio.rs`: captura, níveis e WAV de debug
- `src-tauri/src/deepgram.rs`: streaming de transcrição e cálculo de latência
- `src-tauri/src/secrets.rs`: chaves no Credential Manager
- `src-tauri/src/lib.rs`: comandos do Tauri
- `src/App.tsx`: tela do spike
