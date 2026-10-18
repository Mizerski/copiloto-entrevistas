# Copiloto de entrevistas

App desktop pra Windows que ouve uma entrevista real (Teams, Meet, Zoom), transcreve as duas pontas e ajuda **quem está sendo entrevistado**: a cada pergunta, mostra uma resposta pronta tirada do seu perfil, do seu jeito de falar, e no fim monta um parecer do que melhorar. Tudo local, com as chaves de API de cada pessoa. Tauri 2 + React + Rust.

Todas as fases do roadmap estão no código (0 a 5). Ainda falta validar numa entrevista real de uma hora e configurar assinatura e atualização automática ([DISTRIBUICAO.md](DISTRIBUICAO.md)).

## O que faz

- **Captura:** microfone (você) e o áudio de um único app (loopback por processo do WASAPI), as duas fontes em mono 16 kHz no mesmo relógio. Modo replay com arquivo WAV pra testar sem chamada.
- **Transcrição:** uma conexão por fonte com a Deepgram (Nova-3, `pt-BR`), com parciais, termos difíceis do perfil e reconexão automática.
- **Eco:** fala do microfone que repete o entrevistador no mesmo intervalo é descartada (sem fone, a caixa vaza pro microfone).
- **Perfil:** formulário com caixas separadas (resumo, jeito de falar, experiências, histórias com situação, ação e resultado, habilidades, o que ainda não domina, termos). Fica gravado como Markdown; importar e exportar `.md` continuam valendo.
- **Ao vivo:** quando o entrevistador pergunta, três balões mostram *Responda* (a fala pronta, só com fatos do perfil, no tamanho da pergunta), *Se pedirem mais* (a história do perfil que serve) e *Atenção* (a pergunta supõe algo que não bate com o perfil ou com o que você já disse, mostrando os dois trechos; pergunta em duas partes, ambígua ou ligada a um requisito da vaga). Marcação (Ctrl+Shift+M) e pausa da transcrição. O app pede que você confirme que a empresa permite assistência. `cargo run --example candidato_texto -- perfis/exemplo-backend.md ../testes/candidato-backend.txt` simula uma entrevista só com texto.
- **Parecer:** evidências a favor e contra por competência com trecho e minuto, pontos do perfil confirmados ou não, o que preparar pras próximas entrevistas. Exporta em Markdown e PDF.
- **Privacidade:** chaves no Credential Manager; tudo no SQLite local; confirmação obrigatória antes de iniciar; indicador de gravação na bandeja e no título; retenção automática da transcrição; apagar sessão, transcrição ou tudo.
- **LLM de cada um:** Anthropic (padrão: `claude-haiku-5-5` ao vivo, pela velocidade, e `claude-opus-5-5` com esforço `high` nas notas) ou qualquer API compatível com OpenAI (OpenAI, Groq, OpenRouter, Ollama, LM Studio).

## Rodar

```bash
npm install
npm run tauri dev
```

No primeiro uso o app pede a chave da Deepgram, a chave da LLM e testa o microfone.

Instalador: `npm run tauri build` (detalhes em [DISTRIBUICAO.md](DISTRIBUICAO.md)).

### Modo demonstração da interface

Com `npm run dev`, abrir `http://localhost:1420` num navegador comum mostra a interface com o backend simulado (`src/devMock.ts`): sessão ao vivo de exemplo, perfil, histórico e parecer. Só existe em desenvolvimento.

## Testes de ponta a ponta (sonda)

A sonda roda o mesmo caminho do app (captura → transcrição → sessão → análise → notas) sem interface, com banco em memória. As opções estão no comentário do topo de `src-tauri/examples/sonda.rs`.

Áudio de teste com a voz pt-BR do Windows (o `.wav` não vai pro git):

```powershell
.\scripts\gerar-audio-teste.ps1 testes\resposta-pagamentos.txt
.\scripts\gerar-audio-teste.ps1 testes\pergunta-1.txt
.\scripts\gerar-audio-teste.ps1 testes\pergunta-2.txt
```

```bash
cd src-tauri
cargo run --example montar_eco -- ../testes          # monta eco-mic.wav e eco-sistema.wav
cargo run --example llm_falso                         # LLM falso em 127.0.0.1:11435 (outro terminal)
cargo run --example sonda -- --arquivo ../testes/eco-sistema.wav --mic-arquivo ../testes/eco-mic.wav \
  --esperado ../testes/resposta-pagamentos.txt --esperado-mic ../testes/eco-mic-esperado.txt \
  --perfil perfis/exemplo-backend.md --llm-url http://127.0.0.1:11435/v1 --notas
```

A transcrição usa a chave da Deepgram salva pelo app. Pra testar a qualidade da análise e das notas, troque o LLM falso por um real (`--llm-url`, `--llm-modelo`, chave em `LLM_API_KEY`).

### Medições (13/10/2026, voz sintética)

| Teste | Resultado |
| --- | --- |
| Transcrição, resposta de 57 s, Nova-3 com `smart_format` | 9,2% de erro de palavras |
| + termos difíceis | 4,1% |
| + sem `smart_format` (configuração atual) | 1,0%; latência final 1,1 s em média, 2,2 s no pior caso; parciais ~300 ms |
| Eco: resposta vazando 0,3× no microfone, 60 ms de atraso | 15 de 15 falas de eco descartadas, nenhuma vazou, as perguntas ficaram |
| Análise por resposta (LLM falso) | 1 chamada por resposta; pergunta proibida ("tem filhos") alertada na hora |

Sem os termos, "Kafka" saiu como "café". Voz sintética é mais limpa que fala real; o erro numa entrevista de verdade vai ser maior. A qualidade das sugestões e das notas ainda não foi medida com um modelo de verdade.

## Código

- `src-tauri/src/audio.rs`: captura (microfone, loopback por processo, arquivo), níveis, WAV de debug
- `src-tauri/src/deepgram.rs`: streaming de transcrição, reconexão, latência
- `src-tauri/src/session.rs`: sessão ao vivo, filtro de eco
- `src-tauri/src/analysis.rs`: turnos, respostas sugeridas, memória de fatos e contradições
- `src-tauri/src/notes.rs`: parecer pós-entrevista
- `src-tauri/src/llm.rs`: adaptadores Anthropic e compatível com OpenAI
- `src-tauri/src/profile.rs`: prompt das respostas; `src-tauri/perfis/`: perfil de exemplo
- `src-tauri/src/live.rs`: liga captura, transcrição, sessão e análise (app e sonda)
- `src-tauri/src/db.rs`: SQLite; `src-tauri/src/secrets.rs`: Credential Manager
- `src-tauri/src/lib.rs`: comandos do Tauri, bandeja, atalho global
- Interface em [Radix Themes](https://www.radix-ui.com/themes) (cinza "sand", destaque laranja, claro e escuro); `src/ui.tsx` tem as peças de layout.
- `src/App.tsx`: preparação e sessão ao vivo; `Insights.tsx` (balões), `Transcript.tsx`, `Notes.tsx`, `History.tsx`, `Profiles.tsx` e `profileForm.ts` (formulário do perfil ↔ Markdown), `Settings.tsx`, `Onboarding.tsx`
