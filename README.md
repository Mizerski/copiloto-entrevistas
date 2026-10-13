# Copiloto de entrevistas

App desktop pra Windows que ouve uma entrevista real (Teams, Meet, Zoom), transcreve as duas pontas e ajuda quem entrevista. Tauri 2 + React + Rust.

Estado atual: fase 1, transcrição ao vivo com sessões salvas. A fase 0 (spike de captura e transcrição) está concluída no código; falta validar numa chamada real.

## O que já faz

- Captura o microfone e o áudio de um único app (loopback por processo do WASAPI), as duas fontes em mono 16 kHz e no mesmo relógio.
- Transcreve cada fonte numa conexão própria com a Deepgram (Nova-3, `pt-BR`), com texto parcial e final.
- Mede a latência entre o fim do áudio capturado e a chegada do texto.
- Guarda a chave da Deepgram no Credential Manager do Windows.
- Testa "sempre no topo" e "esconder do compartilhamento de tela".
- Reconecta sozinho se a conexão com a Deepgram cair; o áudio do intervalo é enviado depois.
- Descarta o eco: fala do microfone que repete o que o candidato disse na mesma hora (sem fone, a caixa de som vaza pro microfone).
- Salva cada sessão no SQLite (`%APPDATA%\com.wesley.copiloto\copiloto.db`), com histórico pra rever e apagar.

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

A sonda transcreve com a chave salva pelo app (ou `DEEPGRAM_API_KEY`). `--sem-transcricao` desliga.

## Modo replay

Um WAV pode entrar como "outro lado" no lugar do app da chamada, tocado em tempo real pelo mesmo caminho. Serve pra testar sem marcar uma entrevista. Na tela é a opção "Arquivo WAV (replay)…"; na sonda:

```bash
cd src-tauri
cargo run --example sonda -- --arquivo ../testes/resposta-pagamentos.wav --sem-mic --esperado ../testes/resposta-pagamentos.txt --termos "Kafka,idempotente,idempotência,Postgres,observabilidade"
```

O WAV de teste é gerado com a voz pt-BR do Windows a partir do texto (o `.wav` não vai pro git):

```powershell
.\scripts\gerar-audio-teste.ps1 testes\resposta-pagamentos.txt
```

### Teste de eco

Simula um notebook sem fone: o microfone recebe duas perguntas e, no meio, a resposta do candidato vazando 0,3× mais baixa e 60 ms atrasada.

```powershell
.\scripts\gerar-audio-teste.ps1 testes\pergunta-1.txt
.\scripts\gerar-audio-teste.ps1 testes\pergunta-2.txt
```

```bash
cd src-tauri
cargo run --example montar_eco -- ../testes
cargo run --example sonda -- --arquivo ../testes/eco-sistema.wav --mic-arquivo ../testes/eco-mic.wav --esperado ../testes/resposta-pagamentos.txt --esperado-mic ../testes/eco-mic-esperado.txt --termos "Kafka,idempotente,idempotência,Postgres,observabilidade"
```

Resultado em 13/10/2026: as 15 falas de eco foram descartadas, nenhuma vazou, e as duas perguntas ficaram.

### Medições (13/10/2026, resposta de 57 s com voz sintética)

| Configuração | Erro de palavras | Latência final (média / p90) | Parcial (média) |
| --- | --- | --- | --- |
| Nova-3, `smart_format` | 9,2% | 1,09 s / 1,76 s | 341 ms |
| + termos difíceis | 4,1% | 1,03 s / 1,75 s | 284 ms |
| + sem `smart_format` | 1,0% | 1,09 s / 1,96 s | 255 ms |

Sem os termos, "Kafka" saiu como "café" e "idempotente" como "idem potente". O `smart_format` trocava "uma" por "1". Voz sintética é mais limpa que fala real, então o erro numa entrevista de verdade vai ser maior.

## Código

- `src-tauri/src/audio.rs`: captura, níveis e WAV de debug
- `src-tauri/src/deepgram.rs`: streaming de transcrição, reconexão e cálculo de latência
- `src-tauri/src/session.rs`: sessão ao vivo, filtro de eco
- `src-tauri/src/live.rs`: liga captura, transcrição e sessão (usado pelo app e pela sonda)
- `src-tauri/src/db.rs`: SQLite
- `src-tauri/src/secrets.rs`: chaves no Credential Manager
- `src-tauri/src/lib.rs`: comandos do Tauri
- `src/App.tsx`: preparação e janela da sessão; `src/History.tsx`: histórico
