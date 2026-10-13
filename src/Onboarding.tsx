import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { KeyField } from "./Settings";
import type { CaptureEvent, LlmSettings, LlmUsage, Microphone } from "./types";

const STEPS = ["Boas-vindas", "Transcrição", "Análise", "Microfone", "Pronto"];

/** Primeiro uso: chaves, teste de conexão e de microfone. Dá pra pular a qualquer momento. */
export function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [deepgramMask, setDeepgramMask] = useState<string | null>(null);
  const [deepgramError, setDeepgramError] = useState<string | null>(null);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [llmResult, setLlmResult] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [mics, setMics] = useState<Microphone[]>([]);
  const [micId, setMicId] = useState("");
  const [level, setLevel] = useState(0);
  const peak = useRef(0);

  useEffect(() => {
    invoke<string | null>("deepgram_key_status").then(setDeepgramMask);
    invoke<LlmSettings>("llm_settings").then(setLlm);
    invoke<Microphone[]>("list_microphones").then((m) => {
      setMics(m);
      setMicId(m.find((x) => x.default)?.id ?? m[0]?.id ?? "");
    });
    const off = listen<CaptureEvent>("captura", ({ payload: e }) => {
      if (e.kind === "level" && e.source === "mic") {
        peak.current = Math.max(peak.current, e.rms);
        setLevel(e.rms);
      }
    });
    return () => {
      off.then((f) => f());
      invoke("mic_test", { micId: null, on: false });
    };
  }, []);

  // O teste de microfone roda só enquanto o passo está aberto.
  useEffect(() => {
    if (step !== 3 || !micId) return;
    peak.current = 0;
    invoke("mic_test", { micId, on: true }).catch(() => {});
    return () => {
      invoke("mic_test", { micId: null, on: false });
    };
  }, [step, micId]);

  async function finish() {
    await invoke("set_flag", { key: "onboarded", value: true });
    onDone();
  }

  async function testLlm() {
    setTesting(true);
    try {
      const u = await invoke<LlmUsage>("test_llm", { name: "live" });
      setLlmResult(`Funcionou: ${u.model}, ${u.latencyMs} ms.`);
    } catch (e) {
      setLlmResult(`Falhou: ${e}`);
    } finally {
      setTesting(false);
    }
  }

  const percent = level > 0 ? Math.max(0, Math.min(100, ((20 * Math.log10(level) + 60) / 60) * 100)) : 0;
  const heard = peak.current > 0.02;

  return (
    <div className="onboarding">
      <ol className="steps">
        {STEPS.map((s, i) => (
          <li key={s} className={i === step ? "active" : i < step ? "done" : ""}>
            {s}
          </li>
        ))}
      </ol>

      {step === 0 && (
        <section>
          <div className="section-title">O Copiloto ajuda quem entrevista</div>
          <p>
            Durante uma entrevista no Teams, Meet ou Zoom, ele transcreve você e o candidato, mostra o que já foi coberto
            do roteiro e sugere o que aprofundar. No fim, monta um rascunho de parecer com trechos da conversa.
          </p>
          <ul className="plain">
            <li>O candidato precisa ser avisado de que a conversa será transcrita. O app traz um texto pronto.</li>
            <li>Tudo fica neste computador. Cada serviço usado (transcrição, LLM) é pago com a sua própria chave.</li>
            <li>O app não decide nada: as notas são sugestões, a avaliação é sua.</li>
          </ul>
        </section>
      )}

      {step === 1 && (
        <section>
          <div className="section-title">Transcrição: Deepgram</div>
          <p className="hint">
            Crie uma conta e uma chave em{" "}
            <button className="link" onClick={() => openUrl("https://console.deepgram.com")}>
              console.deepgram.com
            </button>
            . Contas novas recebem crédito grátis.
          </p>
          <KeyField
            label="Chave da Deepgram"
            mask={deepgramMask}
            onSave={async (key) => {
              try {
                setDeepgramMask(await invoke<string>("save_deepgram_key", { key }));
                setDeepgramError(null);
              } catch (e) {
                setDeepgramError(String(e));
              }
            }}
            onDelete={async () => {
              await invoke("delete_deepgram_key");
              setDeepgramMask(null);
            }}
          />
          {deepgramError && <p className="error">{deepgramError}</p>}
          {deepgramMask && <p className="ok-text">Chave aceita pela Deepgram.</p>}
        </section>
      )}

      {step === 2 && llm && (
        <section>
          <div className="section-title">Análise: LLM (opcional)</div>
          <p className="hint">
            Sem LLM o app transcreve, mostra o roteiro e avisa de perguntas proibidas. Com LLM, mostra cobertura das
            competências, sugestões e gera o parecer. O padrão é Claude (Anthropic); outros provedores e modelos locais
            ficam em Configurações.
          </p>
          <KeyField
            label="Chave da Anthropic"
            mask={llm.anthropicKey}
            onSave={async (key) => {
              const mask = await invoke<string>("save_llm_key", { provider: "anthropic", key });
              setLlm({ ...llm, anthropicKey: mask });
            }}
            onDelete={async () => {
              await invoke("delete_llm_key", { provider: "anthropic" });
              setLlm({ ...llm, anthropicKey: null });
            }}
          />
          <div className="row">
            <button disabled={testing} onClick={testLlm}>
              {testing ? "Testando…" : "Testar conexão"}
            </button>
            <span className="muted small">{llmResult}</span>
          </div>
        </section>
      )}

      {step === 3 && (
        <section>
          <div className="section-title">Microfone</div>
          <p className="hint">Fale algo. A barra deve se mexer. Nada é gravado nem enviado neste teste.</p>
          <select value={micId} onChange={(e) => setMicId(e.target.value)}>
            {mics.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
          <div className="bar big">
            <div style={{ width: `${percent}%` }} />
          </div>
          {heard ? <p className="ok-text">Ouvindo bem.</p> : <p className="muted small">Esperando som…</p>}
          <p className="hint">
            Use fone de ouvido nas entrevistas: sem ele, o microfone capta o candidato saindo da caixa de som. O app
            descarta esse eco, mas com fone a transcrição fica mais limpa.
          </p>
        </section>
      )}

      {step === 4 && (
        <section>
          <div className="section-title">Pronto</div>
          <ul className="plain">
            <li>Monte ou ajuste um kit na aba Kits (já vêm dois exemplos).</li>
            <li>Antes de iniciar, avise o candidato. O texto pronto fica na tela da sessão.</li>
            <li>Ctrl+Shift+M marca um momento importante, mesmo com a chamada em primeiro plano.</li>
            <li>Se o candidato pedir, pause a transcrição; o áudio para de sair do computador.</li>
          </ul>
        </section>
      )}

      <div className="row">
        <button className="link" onClick={finish}>
          Pular
        </button>
        <span className="grow" />
        {step > 0 && <button onClick={() => setStep(step - 1)}>Voltar</button>}
        {step < STEPS.length - 1 ? (
          <button className="primary" onClick={() => setStep(step + 1)}>
            Continuar
          </button>
        ) : (
          <button className="primary" onClick={finish}>
            Começar
          </button>
        )}
      </div>
    </div>
  );
}
