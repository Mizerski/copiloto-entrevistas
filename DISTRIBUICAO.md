# Distribuição

O que já está pronto e o que depende de decisões ou contas que não dá pra criar daqui.

## Instalador (pronto)

```bash
npm run tauri build
```

Gera `src-tauri/target/release/bundle/nsis/Copiloto_<versão>_x64-setup.exe`:

- instala por usuário, sem pedir administrador;
- em português do Brasil;
- usa o WebView2 que já vem no Windows 10 e 11 (o instalador baixa se faltar).

O app instalado roda com uma política de conteúdo (CSP) que só permite os arquivos do próprio app e a ponte com o Rust. As chamadas à Deepgram e às LLMs saem do Rust, nunca da página.

## Assinatura de código (falta o certificado)

Sem assinatura, o Windows mostra o aviso "O Windows protegeu o computador" na primeira execução. Pra tirar o aviso:

1. Compre um certificado de assinatura de código (OV ou EV) de uma autoridade certificadora, ou use o Azure Trusted Signing (assinatura na nuvem, cobrada por mês).
2. Configure em `src-tauri/tauri.conf.json`, em `bundle.windows`:
   - certificado local: `certificateThumbprint`, `digestAlgorithm: "sha256"` e `timestampUrl` da autoridade;
   - Azure Trusted Signing: `signCommand` chamando a ferramenta de assinatura da Microsoft.
3. Rode `npm run tauri build` de novo; o instalador e o executável saem assinados.

Certificado EV elimina o aviso na hora. Com OV, o aviso some aos poucos, conforme o instalador ganha reputação no SmartScreen.

## Atualização automática (falta onde hospedar)

O Tauri tem um plugin de atualização que confere uma URL, baixa o instalador novo e confere a assinatura antes de instalar. Pra ligar:

1. Gere o par de chaves de atualização (a privada fica com você, fora do git):

   ```bash
   npm run tauri signer generate -- -w ~/.tauri/copiloto.key
   ```

2. Escolha onde publicar as versões. O caminho mais simples é GitHub Releases: o plugin lê um `latest.json` anexado ao release.
3. Instale o plugin (`npm run tauri add updater`), coloque a chave pública e a URL do `latest.json` em `plugins.updater` no `tauri.conf.json` e ligue `bundle.createUpdaterArtifacts`.
4. No build, defina `TAURI_SIGNING_PRIVATE_KEY` (e a senha, se tiver) pra gerar os arquivos `.sig`.
5. No app, chame `check()` do plugin ao abrir e ofereça a atualização.

Isso não foi ligado agora porque exige decidir o repositório público (ou servidor) e guardar a chave privada.

## Antes do primeiro cliente

- Revisão jurídica do texto de consentimento (`src/Consent.tsx`) e da cláusula de convite.
- Verificar se o uso de provedores fora do Brasil (Deepgram, Anthropic) cabe na política da empresa que vai usar.
- Teste numa chamada real de uma hora (critério da fase 1 do plano).
