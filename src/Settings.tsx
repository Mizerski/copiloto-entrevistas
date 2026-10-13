import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { LlmSettings, LlmUsage, Provider, Slot } from "./types";

const SLOT_LABEL = {
  live: { title: "Análise ao vivo", hint: "Roda a cada resposta; precisa ser rápido." },
  notes: { title: "Notas pós-entrevista", hint: "Roda uma vez no fim; pode ser o melhor modelo." },
} as const;

const PROVIDER_LABEL: Record<Provider, string> = {
  anthropic: "Anthropic (Claude)",
  openai: "Compatível com OpenAI",
};

type SlotName = keyof typeof SLOT_LABEL;

function KeyField(props: {
  label: string;
  mask: string | null;
  onSave: (key: string) => Promise<void>;
  onDelete: () => Promise<void>;
  hint?: string;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  if (props.mask) {
    return (
      <div className="row key-row">
        <span>
          {props.label}: <code>{props.mask}</code>
        </span>
        <button onClick={props.onDelete}>Remover</button>
      </div>
    );
  }
  return (
    <label>
      {props.label}
      <div className="row">
        <input
          type="password"
          autoComplete="off"
          placeholder={props.hint ?? "cole a chave aqui"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <button
          disabled={busy || !value.trim()}
          onClick={async () => {
            setBusy(true);
            try {
              await props.onSave(value);
              setValue("");
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy ? "Testando…" : "Salvar"}
        </button>
      </div>
    </label>
  );
}

export function Settings({ onLog }: { onLog: (line: string) => void }) {
  const [deepgramMask, setDeepgramMask] = useState<string | null>(null);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [testing, setTesting] = useState<SlotName | null>(null);
  const [results, setResults] = useState<Partial<Record<SlotName, string>>>({});
  const [retention, setRetention] = useState(0);

  async function load() {
    setDeepgramMask(await invoke<string | null>("deepgram_key_status"));
    setLlm(await invoke<LlmSettings>("llm_settings"));
    setRetention(await invoke<number>("retention_days"));
  }

  async function changeRetention(days: number) {
    const purged = await invoke<number>("set_retention_days", { days });
    setRetention(days);
    if (purged > 0) onLog(`Transcrição apagada em ${purged} sessão(ões) antigas com notas`);
  }

  async function deleteEverything() {
    const ok = await ask("Apagar todas as sessões, transcrições e notas? Kits e chaves ficam. Não dá pra desfazer.", {
      kind: "warning",
      okLabel: "Apagar tudo",
    });
    if (!ok) return;
    try {
      await invoke("delete_everything");
      onLog("Todas as sessões foram apagadas");
    } catch (e) {
      onLog(`Não apagou: ${e}`);
    }
  }

  useEffect(() => {
    load().catch((e) => onLog(`Configurações: ${e}`));
  }, []);

  async function saveSlot(name: SlotName, slot: Slot) {
    setLlm((prev) => (prev ? { ...prev, [name]: slot } : prev));
    await invoke("save_llm_slot", { name, slot });
  }

  async function test(name: SlotName) {
    setTesting(name);
    try {
      const u = await invoke<LlmUsage>("test_llm", { name });
      setResults((r) => ({ ...r, [name]: `Funcionou: ${u.model}, ${u.latencyMs} ms.` }));
    } catch (e) {
      setResults((r) => ({ ...r, [name]: `Falhou: ${e}` }));
    } finally {
      setTesting(null);
    }
  }

  return (
    <div className="settings">
      <section>
        <div className="section-title">Transcrição · Deepgram</div>
        <KeyField
          label="Chave da Deepgram"
          mask={deepgramMask}
          onSave={async (key) => {
            try {
              setDeepgramMask(await invoke<string>("save_deepgram_key", { key }));
              onLog("Chave da Deepgram aceita e salva");
            } catch (e) {
              onLog(`Chave da Deepgram não salva: ${e}`);
            }
          }}
          onDelete={async () => {
            await invoke("delete_deepgram_key");
            setDeepgramMask(null);
          }}
        />
        <p className="hint">O áudio das duas pontas vai pra Deepgram pra ser transcrito.</p>
      </section>

      <section>
        <div className="section-title">Chaves de LLM</div>
        {llm && (
          <>
            <KeyField
              label="Anthropic"
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
            <KeyField
              label="Compatível com OpenAI"
              hint="opcional pra Ollama e LM Studio"
              mask={llm.openaiKey}
              onSave={async (key) => {
                const mask = await invoke<string>("save_llm_key", { provider: "openai", key });
                setLlm({ ...llm, openaiKey: mask });
              }}
              onDelete={async () => {
                await invoke("delete_llm_key", { provider: "openai" });
                setLlm({ ...llm, openaiKey: null });
              }}
            />
            <p className="hint">
              O texto da conversa (não o áudio) vai pro provedor escolhido. As chaves ficam no Credential
              Manager do Windows.
            </p>
          </>
        )}
      </section>

      {llm &&
        (Object.keys(SLOT_LABEL) as SlotName[]).map((name) => {
          const slot = llm[name];
          const update = (patch: Partial<Slot>) => saveSlot(name, { ...slot, ...patch });
          return (
            <section key={name}>
              <div className="section-title">{SLOT_LABEL[name].title}</div>
              <p className="hint">{SLOT_LABEL[name].hint}</p>
              <label>
                Provedor
                <select value={slot.provider} onChange={(e) => update({ provider: e.target.value as Provider })}>
                  {(Object.keys(PROVIDER_LABEL) as Provider[]).map((p) => (
                    <option key={p} value={p}>
                      {PROVIDER_LABEL[p]}
                    </option>
                  ))}
                </select>
              </label>
              {slot.provider === "openai" && (
                <label>
                  Endereço da API
                  <input
                    type="text"
                    value={slot.baseUrl}
                    placeholder="https://api.openai.com/v1 · http://localhost:11434/v1"
                    onChange={(e) => update({ baseUrl: e.target.value })}
                  />
                </label>
              )}
              <label>
                Modelo
                <input type="text" value={slot.model} onChange={(e) => update({ model: e.target.value })} />
              </label>
              {slot.provider === "anthropic" && (
                <label>
                  Esforço
                  <select value={slot.effort} onChange={(e) => update({ effort: e.target.value })}>
                    {["low", "medium", "high", "xhigh", "max"].map((e) => (
                      <option key={e} value={e}>
                        {e}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <div className="row">
                <button disabled={testing !== null} onClick={() => test(name)}>
                  {testing === name ? "Testando…" : "Testar conexão"}
                </button>
                <span className="muted small">{results[name]}</span>
              </div>
            </section>
          );
        })}
      <section>
        <div className="section-title">Dados</div>
        <p className="hint">
          Tudo fica neste computador. A transcrição é dado pessoal de quem foi entrevistado: guarde só o tempo
          necessário.
        </p>
        <label>
          Apagar a transcrição de sessões com notas depois de
          <select value={retention} onChange={(e) => changeRetention(Number(e.target.value))}>
            <option value={0}>nunca</option>
            <option value={7}>7 dias</option>
            <option value={30}>30 dias</option>
            <option value={90}>90 dias</option>
          </select>
        </label>
        <div className="row">
          <button className="danger" onClick={deleteEverything}>
            Apagar todas as sessões
          </button>
        </div>
      </section>
    </div>
  );
}
