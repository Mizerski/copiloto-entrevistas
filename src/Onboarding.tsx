import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, Flex, Heading, Link, Progress, Text } from "@radix-ui/themes";
import { CheckCircledIcon } from "@radix-ui/react-icons";
import { KeyField } from "./Settings";
import { Field, Picker, Section } from "./ui";
import type { CaptureEvent, LlmSettings, LlmUsage, Microphone } from "./types";

const STEPS = ["Boas-vindas", "Transcrição", "Análise", "Microfone", "Pronto"];

function Ok({ children }: { children: string }) {
  return (
    <Flex align="center" gap="1">
      <CheckCircledIcon color="var(--green-11)" />
      <Text size="2" color="green">
        {children}
      </Text>
    </Flex>
  );
}

/** Primeiro uso: chaves, teste de conexão e de microfone. Dá pra pular a qualquer momento. */
export function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [deepgramMask, setDeepgramMask] = useState<string | null>(null);
  const [deepgramError, setDeepgramError] = useState<string | null>(null);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [llmResult, setLlmResult] = useState<{ ok: boolean; text: string } | null>(null);
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
      setLlmResult({ ok: true, text: `${u.model} respondeu em ${(u.latencyMs / 1000).toFixed(1)} s` });
    } catch (e) {
      setLlmResult({ ok: false, text: String(e) });
    } finally {
      setTesting(false);
    }
  }

  const percent = level > 0 ? Math.max(0, Math.min(100, ((20 * Math.log10(level) + 60) / 60) * 100)) : 0;
  const heard = peak.current > 0.02;

  return (
    <Flex direction="column" gap="4" className="onboarding">
      <Flex direction="column" gap="2">
        <Text size="1" color="gray">
          Passo {step + 1} de {STEPS.length} · {STEPS[step]}
        </Text>
        <Progress value={((step + 1) / STEPS.length) * 100} size="1" />
      </Flex>

      {step === 0 && (
        <Flex direction="column" gap="3">
          <Heading size="6" weight="medium">
            O Copiloto ajuda quem entrevista
          </Heading>
          <Text as="p" size="3" color="gray">
            Durante uma entrevista no Teams, Meet ou Zoom, ele transcreve você e o candidato, mostra o que já foi
            coberto do roteiro e sugere o que aprofundar. No fim, monta um rascunho de parecer com trechos da
            conversa.
          </Text>
          <Section>
            <Text size="2">O candidato precisa ser avisado de que a conversa será transcrita. O app traz um texto pronto.</Text>
            <Text size="2">Tudo fica neste computador. Cada serviço usado é pago com a sua própria chave.</Text>
            <Text size="2">O app não decide nada: as notas são sugestões, a avaliação é sua.</Text>
          </Section>
        </Flex>
      )}

      {step === 1 && (
        <Section
          title="Transcrição"
          description={
            <>
              Crie uma conta e uma chave em{" "}
              <Link href="#" onClick={() => openUrl("https://console.deepgram.com")}>
                console.deepgram.com
              </Link>
              . Contas novas recebem crédito grátis.
            </>
          }
        >
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
          {deepgramError && (
            <Text size="2" color="red">
              {deepgramError}
            </Text>
          )}
          {deepgramMask && <Ok>Chave aceita pela Deepgram.</Ok>}
        </Section>
      )}

      {step === 2 && llm && (
        <Section
          title="Análise (opcional)"
          description="Sem LLM o app transcreve, mostra o roteiro e avisa de perguntas proibidas. Com LLM, mostra cobertura, sugestões e gera o parecer. Outros provedores e modelos locais ficam em Configurações."
        >
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
          <Flex align="center" gap="3">
            <Button variant="soft" color="gray" loading={testing} onClick={testLlm}>
              Testar conexão
            </Button>
            {llmResult &&
              (llmResult.ok ? (
                <Ok>{llmResult.text}</Ok>
              ) : (
                <Text size="1" color="red">
                  {llmResult.text}
                </Text>
              ))}
          </Flex>
        </Section>
      )}

      {step === 3 && (
        <Section title="Microfone" description="Fale algo; a barra deve se mexer. Nada é gravado nem enviado neste teste.">
          <Field label="Dispositivo">
            <Picker
              value={micId}
              options={mics.map((m) => ({ value: m.id, label: m.name }))}
              onChange={setMicId}
            />
          </Field>
          <Progress value={percent} size="3" color={heard ? "green" : "gray"} />
          {heard ? (
            <Ok>Ouvindo bem.</Ok>
          ) : (
            <Text size="1" color="gray">
              Esperando som…
            </Text>
          )}
          <Text size="1" color="gray">
            Use fone nas entrevistas: sem ele, o microfone capta o candidato saindo da caixa de som. O app descarta
            esse eco, mas com fone a transcrição fica mais limpa.
          </Text>
        </Section>
      )}

      {step === 4 && (
        <Section title="Pronto">
          <Text size="2">Monte ou ajuste um kit na aba Kits; já vêm dois exemplos.</Text>
          <Text size="2">Antes de iniciar, avise o candidato. O texto pronto fica na tela da sessão.</Text>
          <Text size="2">Ctrl+Shift+M marca um momento importante, mesmo com a chamada em primeiro plano.</Text>
          <Text size="2">Se o candidato pedir, pause a transcrição; o áudio para de sair do computador.</Text>
        </Section>
      )}

      <Flex align="center" gap="2">
        <Button variant="ghost" color="gray" onClick={finish}>
          Pular
        </Button>
        <span className="grow" />
        {step > 0 && (
          <Button variant="soft" color="gray" onClick={() => setStep(step - 1)}>
            Voltar
          </Button>
        )}
        {step < STEPS.length - 1 ? (
          <Button onClick={() => setStep(step + 1)}>Continuar</Button>
        ) : (
          <Button onClick={finish}>Começar</Button>
        )}
      </Flex>
    </Flex>
  );
}
