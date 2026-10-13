import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { Button, Code, Flex, Text, TextField } from "@radix-ui/themes";
import { CheckCircledIcon, CrossCircledIcon, LockClosedIcon } from "@radix-ui/react-icons";
import { Field, Picker, Section, type Option } from "./ui";
import type { LlmSettings, LlmUsage, Provider, Slot } from "./types";

const SLOTS = {
  live: { title: "Análise ao vivo", hint: "Roda a cada resposta do candidato. Velocidade importa mais." },
  notes: { title: "Notas pós-entrevista", hint: "Roda uma vez no fim. Qualidade importa mais." },
} as const;
type SlotName = keyof typeof SLOTS;

const PROVIDERS: Option[] = [
  { value: "anthropic", label: "Anthropic (Claude)" },
  { value: "openai", label: "Compatível com OpenAI" },
];

const CUSTOM = "__outro__";

const ANTHROPIC_MODELS: Option[] = [
  { value: "claude-haiku-5-5", label: "Claude Haiku 5.5", detail: "mais rápido e barato" },
  { value: "claude-sonnet-5-5", label: "Claude Sonnet 5.5", detail: "equilíbrio" },
  { value: "claude-opus-5-5", label: "Claude Opus 5.5", detail: "melhor qualidade" },
  { value: CUSTOM, label: "Outro modelo…" },
];

const EFFORTS: Option[] = [
  { value: "low", label: "Baixo", detail: "mais rápido" },
  { value: "medium", label: "Médio" },
  { value: "high", label: "Alto" },
  { value: "xhigh", label: "Muito alto" },
  { value: "max", label: "Máximo", detail: "mais lento" },
];

/** Endereços conhecidos de APIs compatíveis com OpenAI. */
const OPENAI_HOSTS: (Option & { url: string; model: string })[] = [
  { value: "openai", label: "OpenAI", url: "https://api.openai.com/v1", model: "" },
  { value: "groq", label: "Groq", url: "https://api.groq.com/openai/v1", model: "" },
  { value: "openrouter", label: "OpenRouter", url: "https://openrouter.ai/api/v1", model: "" },
  { value: "ollama", label: "Ollama", detail: "local", url: "http://localhost:11434/v1", model: "" },
  { value: "lmstudio", label: "LM Studio", detail: "local", url: "http://localhost:1234/v1", model: "" },
  { value: CUSTOM, label: "Outro endereço…", url: "", model: "" },
];

export function KeyField(props: {
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
      <Field label={props.label}>
        <Flex align="center" gap="2">
          <LockClosedIcon />
          <Code variant="ghost" className="grow">
            {props.mask}
          </Code>
          <Button size="1" variant="soft" color="gray" onClick={props.onDelete}>
            Remover
          </Button>
        </Flex>
      </Field>
    );
  }
  return (
    <Field label={props.label}>
      <Flex gap="2">
        <TextField.Root
          className="grow"
          type="password"
          autoComplete="off"
          placeholder={props.hint ?? "cole a chave aqui"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <Button
          disabled={busy || !value.trim()}
          loading={busy}
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
          Salvar
        </Button>
      </Flex>
    </Field>
  );
}

function SlotEditor({ name, slot, onChange }: { name: SlotName; slot: Slot; onChange: (s: Slot) => void }) {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const known = ANTHROPIC_MODELS.some((m) => m.value === slot.model);
  const [customModel, setCustomModel] = useState(!known && slot.provider === "anthropic");
  const host = OPENAI_HOSTS.find((h) => h.url === slot.baseUrl)?.value ?? CUSTOM;
  const update = (patch: Partial<Slot>) => {
    setResult(null);
    onChange({ ...slot, ...patch });
  };

  async function test() {
    setTesting(true);
    try {
      const u = await invoke<LlmUsage>("test_llm", { name });
      setResult({ ok: true, text: `${u.model} respondeu em ${(u.latencyMs / 1000).toFixed(1)} s` });
    } catch (e) {
      setResult({ ok: false, text: String(e) });
    } finally {
      setTesting(false);
    }
  }

  return (
    <Section title={SLOTS[name].title} description={SLOTS[name].hint}>
      <Field label="Provedor">
        <Picker
          value={slot.provider}
          options={PROVIDERS}
          onChange={(v) =>
            update(
              v === "anthropic"
                ? name === "live"
                  ? { provider: "anthropic", model: "claude-haiku-5-5", effort: "low" }
                  : { provider: "anthropic", model: "claude-opus-5-5", effort: "high" }
                : { provider: v as Provider, model: "", baseUrl: OPENAI_HOSTS[0].url },
            )
          }
        />
      </Field>

      {slot.provider === "anthropic" ? (
        <>
          <Field label="Modelo">
            <Picker
              value={customModel ? CUSTOM : slot.model}
              options={ANTHROPIC_MODELS}
              onChange={(v) => {
                setCustomModel(v === CUSTOM);
                if (v !== CUSTOM) update({ model: v });
              }}
            />
            {customModel && (
              <TextField.Root
                placeholder="id do modelo, ex.: claude-opus-5"
                value={slot.model}
                onChange={(e) => update({ model: e.target.value })}
              />
            )}
          </Field>
          <Field label="Esforço" hint="Quanto o modelo pensa antes de responder. Menos esforço, resposta mais rápida.">
            <Picker value={slot.effort || "medium"} options={EFFORTS} onChange={(v) => update({ effort: v })} />
          </Field>
        </>
      ) : (
        <>
          <Field label="Serviço">
            <Picker
              value={host}
              options={OPENAI_HOSTS}
              onChange={(v) => {
                const h = OPENAI_HOSTS.find((x) => x.value === v)!;
                update({ baseUrl: h.url, model: h.model || slot.model });
              }}
            />
            {host === CUSTOM && (
              <TextField.Root
                placeholder="https://…/v1"
                value={slot.baseUrl}
                onChange={(e) => update({ baseUrl: e.target.value })}
              />
            )}
          </Field>
          <Field label="Modelo" hint="O nome exato que o serviço usa, ex.: llama-3.3-70b-versatile no Groq.">
            <TextField.Root value={slot.model} onChange={(e) => update({ model: e.target.value })} />
          </Field>
        </>
      )}

      <Flex align="center" gap="3">
        <Button variant="soft" color="gray" loading={testing} onClick={test}>
          Testar conexão
        </Button>
        {result && (
          <Flex align="center" gap="1" minWidth="0">
            {result.ok ? <CheckCircledIcon color="var(--green-11)" /> : <CrossCircledIcon color="var(--red-11)" />}
            <Text size="1" color={result.ok ? "green" : "red"} className="truncate" title={result.text}>
              {result.text}
            </Text>
          </Flex>
        )}
      </Flex>
    </Section>
  );
}

const RETENTION: Option[] = [
  { value: "0", label: "Nunca" },
  { value: "7", label: "Depois de 7 dias" },
  { value: "30", label: "Depois de 30 dias" },
  { value: "90", label: "Depois de 90 dias" },
];

export function Settings({ onLog }: { onLog: (line: string) => void }) {
  const [deepgramMask, setDeepgramMask] = useState<string | null>(null);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [retention, setRetention] = useState(0);

  async function load() {
    setDeepgramMask(await invoke<string | null>("deepgram_key_status"));
    setLlm(await invoke<LlmSettings>("llm_settings"));
    setRetention(await invoke<number>("retention_days"));
  }

  useEffect(() => {
    load().catch((e) => onLog(`Configurações: ${e}`));
  }, []);

  async function saveSlot(name: SlotName, slot: Slot) {
    setLlm((prev) => (prev ? { ...prev, [name]: slot } : prev));
    await invoke("save_llm_slot", { name, slot });
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

  return (
    <Flex direction="column" gap="3">
      <Section title="Chaves de API" description="Ficam no Credential Manager do Windows; a tela só vê o final.">
        <KeyField
          label="Deepgram (transcrição)"
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
          </>
        )}
        <Text size="1" color="gray">
          O áudio vai pra Deepgram; o texto da conversa (não o áudio) vai pro provedor de LLM escolhido.
        </Text>
      </Section>

      {llm &&
        (Object.keys(SLOTS) as SlotName[]).map((name) => (
          <SlotEditor key={name} name={name} slot={llm[name]} onChange={(s) => saveSlot(name, s)} />
        ))}

      <Section
        title="Dados"
        description="Tudo fica neste computador. A transcrição é dado pessoal de quem foi entrevistado: guarde só o tempo necessário."
      >
        <Field label="Apagar a transcrição de sessões com notas">
          <Picker
            value={String(retention)}
            options={RETENTION}
            onChange={(v) => changeRetention(Number(v))}
          />
        </Field>
        <Flex>
          <Button variant="soft" color="red" onClick={deleteEverything}>
            Apagar todas as sessões
          </Button>
        </Flex>
      </Section>
    </Flex>
  );
}
