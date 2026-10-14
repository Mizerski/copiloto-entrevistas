# Copiloto de entrevistas

App desktop pra Windows que ouve uma entrevista real (Teams, Meet, Zoom), transcreve as duas pontas e ajuda **quem entrevista**: mostra o que já foi coberto do roteiro, sugere o que aprofundar e, no fim, monta um rascunho de parecer com trechos da conversa. Tudo local, com as chaves de API de cada pessoa. Tauri 2 + React + Rust.

Todas as fases do roadmap estão no código (0 a 5). Ainda falta validar numa entrevista real de uma hora e configurar assinatura e atualização automática ([DISTRIBUICAO.md](DISTRIBUICAO.md)).

## O que faz

- **Captura:** microfone (você) e o áudio de um único app (loopback por processo do WASAPI), as duas fontes em mono 16 kHz no mesmo relógio. Modo replay com arquivo WAV pra testar sem chamada.
- **Transcrição:** uma conexão por fonte com a Deepgram (Nova-3, `pt-BR`), com parciais, termos difíceis do kit e reconexão automática.
- **Eco:** fala do microfone que repete o candidato no mesmo intervalo é descartada (sem fone, a caixa vaza pro microfone).
- **Kit de entrevista:** Markdown com vaga, competências (sinais forte e fraco), roteiro por bloco com tempo, perguntas proibidas e termos. Editor com pré-visualização, importar e exportar.
- **Ao vivo:** três balões com perguntas prontas pra ler, atualizados enquanto o candidato fala (~2 a 5 s com o Haiku): *aprofundar* o que ele acabou de dizer, *checar* (contradição com algo dito antes ou com o currículo, mostrando os dois trechos e os minutos; senão um ponto do currículo) e *avançar* no roteiro. O balão some quando você faz a pergunta (ou clica nele). A memória de fatos guarda o que o candidato afirmou pra comparar depois; relógio por bloco, cobertura das competências (sempre com trecho citado), alerta imediato de pergunta proibida, modo foco, marcação (Ctrl+Shift+M), pausa da transcrição, métricas (tempo de fala, perguntas abertas).
- **Candidato:** do outro lado da mesa. Você escreve seu perfil (aba Perfil: experiências, histórias com situação, ação e resultado, o que ainda não domina) e, quando o entrevistador pergunta, três balões mostram *Responda* (os pontos da resposta, só com fatos do perfil, sem inventar), *Sua história* (o exemplo do perfil que serve) e *Atenção* (a pergunta supõe algo que não bate com o perfil ou com o que você já disse, pergunta em duas partes, ambígua ou ligada a um requisito da vaga). O app pede que você confirme que a empresa permite assistência. `cargo run --example candidato_texto -- perfis/exemplo-backend.md ../testes/candidato-backend.txt` simula uma entrevista só com texto.
- **Treino:** uma IA faz o papel de entrevistador (personas: técnica e direta, acolhedora de RH, exigente), segue o roteiro do kit e fala com a voz do Windows; você responde no microfone. Os balões viram dicas pra você (*faltou dizer*, *cuidado* com contradição ou "a gente", *estrutura*) e o parecer do fim avalia você. `cargo run --example treino_texto -- KIT.md RESPOSTAS.txt [persona] [--notas]` roda um treino só com texto.
- **Parecer:** evidências a favor e contra por competência com trecho e minuto, nota sugerida editável, pontos do currículo, perguntas pra próxima etapa. Exporta em Markdown e PDF. Sem recomendação de contratar.
- **Privacidade:** chaves no Credential Manager; tudo no SQLite local; consentimento obrigatório antes de iniciar, com texto pronto; indicador de gravação na bandeja e no título; retenção automática da transcrição; apagar sessão, transcrição ou tudo.
- **LLM de cada um:** Anthropic (padrão: `claude-haiku-5-5` ao vivo, pela velocidade, e `claude-opus-5-5` com esforço `high` nas notas) ou qualquer API compatível com OpenAI (OpenAI, Groq, OpenRouter, Ollama, LM Studio).

## Rodar

```bash
npm install
npm run tauri dev
```

No primeiro uso o app pede a chave da Deepgram, a chave da LLM (opcional) e testa o microfone.

Instalador: `npm run tauri build` (detalhes em [DISTRIBUICAO.md](DISTRIBUICAO.md)).

### Modo demonstração da interface

Com `npm run dev`, abrir `http://localhost:1420` num navegador comum mostra a interface com o backend simulado (`src/devMock.ts`): sessão ao vivo de exemplo, kits, histórico e notas. Só existe em desenvolvimento.

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
  --kit kits/backend-senior-pagamentos.md --llm-url http://127.0.0.1:11435/v1 --notas
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
- `src-tauri/src/analysis.rs`: turnos, análise por resposta, cobertura, cards, perguntas proibidas
- `src-tauri/src/notes.rs`: parecer pós-entrevista
- `src-tauri/src/llm.rs`: adaptadores Anthropic e compatível com OpenAI
- `src-tauri/src/kit.rs`: parser do kit; `src-tauri/kits/`: kits de exemplo
- `src-tauri/src/live.rs`: liga captura, transcrição, sessão e análise (app e sonda)
- `src-tauri/src/db.rs`: SQLite; `src-tauri/src/secrets.rs`: Credential Manager
- `src-tauri/src/lib.rs`: comandos do Tauri, bandeja, atalho global
- Interface em [Radix Themes](https://www.radix-ui.com/themes) (cinza "sand", destaque laranja, claro e escuro); `src/ui.tsx` tem as peças de layout.
- `src/App.tsx`: preparação e sessão ao vivo; `Insights.tsx`, `Transcript.tsx`, `Notes.tsx`, `History.tsx`, `Kits.tsx`, `Settings.tsx`, `Onboarding.tsx`, `Consent.tsx`
