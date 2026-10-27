import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Badge,
  Box,
  Button,
  Callout,
  Card,
  Checkbox,
  Flex,
  IconButton,
  Progress,
  ScrollArea,
  Switch,
  Tabs,
  Text,
  TextArea,
  TextField,
  Tooltip,
} from "@radix-ui/themes";
import {
  ExclamationTriangleIcon,
  PauseIcon,
  PlayIcon,
  ReloadIcon,
  StarIcon,
  StopIcon,
} from "@radix-ui/react-icons";
import { average, formatClock, formatMs } from "./format";
import { History } from "./History";
import { AnswerFeed } from "./Insights";
import { Notes } from "./Notes";
import { Onboarding } from "./Onboarding";
import { Profiles } from "./Profiles";
import { Settings } from "./Settings";
import { insertSegment, Transcript } from "./Transcript";
import { Field, Picker, Section } from "./ui";
import {
  SOURCES,
  SOURCE_LABEL,
  type AnalysisEvent,
  type AudioApp,
  type Answer,
  type CaptureEvent,
  type KitRow,
  type LlmSettings,
  type Mark,
  type Microphone,
  type Segment,
  type SessionEvent,
  type SessionFull,
  type Source,
  type SttEvent,
} from "./types";
import "./App.css";

type Tab = "sessao" | "historico" | "perfil" | "config";
type Phase = "setup" | "running" | "stopping" | "done";

const LATENCY_WINDOW = 20;
const NONE = "__nenhum__";
const REPLAY = "__arquivo__";
/** PID 0: o backend entende como "todo o áudio do PC". */
const DESKTOP = "0";
const EMPTY_TEXT: Record<Source, string> = { mic: "", sistema: "" };

/** RMS linear → 0..100, numa escala de -60 dB a 0 dB. */
function meterPercent(rms: number) {
  if (rms <= 0) return 0;
  const db = 20 * Math.log10(rms);
  return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
}

function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

function SwitchRow({ label, checked, onChange, disabled }: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <Text as="label" size="2">
      <Flex gap="2" align="center">
        <Switch size="1" checked={checked} disabled={disabled} onCheckedChange={onChange} />
        {label}
      </Flex>
    </Text>
  );
}

function App() {
  const [tab, setTab] = useState<Tab>("sessao");
  const [phase, setPhase] = useState<Phase>("setup");

  // Preparação
  const [mics, setMics] = useState<Microphone[]>([]);
  const [apps, setApps] = useState<AudioApp[]>([]);
  const [micId, setMicId] = useState("");
  const [appPid, setAppPid] = useState("");
  const [replayFile, setReplayFile] = useState<string | null>(null);
  const [keyterms, setKeyterms] = useState("");
  const [saveWav, setSaveWav] = useState(false);
  const [deepgramReady, setDeepgramReady] = useState(false);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  const [profiles, setProfiles] = useState<KitRow[]>([]);
  const [profileId, setProfileId] = useState("");
  const [jobDescription, setJobDescription] = useState("");
  const [candidateAware, setCandidateAware] = useState(false);

  // Sessão
  const [startedAt, setStartedAt] = useState(0);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [finished, setFinished] = useState<SessionFull | null>(null);
  const [now, setNow] = useState(Date.now());
  const [levels, setLevels] = useState<Record<Source, number>>({ mic: 0, sistema: 0 });
  const [segments, setSegments] = useState<Segment[]>([]);
  const [partials, setPartials] = useState<Record<Source, string>>(EMPTY_TEXT);
  const [latency, setLatency] = useState<Record<Source, number[]>>({ mic: [], sistema: [] });
  const [echoes, setEchoes] = useState(0);
  const [reconnecting, setReconnecting] = useState<Source | null>(null);
  const [answers, setAnswers] = useState<Answer[]>([]);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [paused, setPaused] = useState(false);
  const [onTop, setOnTop] = useState(false);
  const [hidden, setHidden] = useState(false);

  const [log, setLog] = useState<string[]>([]);
  const lastLevel = useRef<Record<Source, number>>({ mic: 0, sistema: 0 });
  const startedAtRef = useRef(0);

  const addLog = (line: string) =>
    setLog((l) => [`${new Date().toLocaleTimeString()}  ${line}`, ...l].slice(0, 80));

  async function refreshDevices() {
    try {
      const [m, a] = await Promise.all([
        invoke<Microphone[]>("list_microphones"),
        invoke<AudioApp[]>("list_audio_apps"),
      ]);
      setMics(m);
      setApps(a);
      setMicId((cur) => cur || m.find((x) => x.default)?.id || "");
    } catch (e) {
      addLog(`Erro ao listar dispositivos: ${e}`);
    }
  }

  async function refreshSetup() {
    try {
      const list = await invoke<KitRow[]>("list_profiles");
      setProfiles(list);
      setProfileId((cur) => (list.some((p) => String(p.id) === cur) ? cur : list[0] ? String(list[0].id) : ""));
      setDeepgramReady((await invoke<string | null>("deepgram_key_status")) !== null);
      setLlm(await invoke<LlmSettings>("llm_settings"));
    } catch (e) {
      addLog(`Erro ao carregar: ${e}`);
    }
  }

  useEffect(() => {
    refreshDevices();
    refreshSetup();
    invoke<boolean>("get_flag", { key: "onboarded" })
      .then(setOnboarded)
      .catch(() => setOnboarded(true));

    const subscriptions = [
      listen<CaptureEvent>("captura", ({ payload: e }) => {
        if (e.kind === "level") lastLevel.current[e.source] = e.rms;
        else if (e.kind === "started") addLog(`${SOURCE_LABEL[e.source]}: capturando (${e.format})`);
        else if (e.kind === "error") addLog(`${SOURCE_LABEL[e.source]}: ${e.message}`);
        else if (e.kind === "stopped") addLog(`${SOURCE_LABEL[e.source]}: ${e.seconds.toFixed(1)} s capturados`);
      }),
      listen<SttEvent>("transcricao", ({ payload: e }) => {
        if (e.kind === "transcript") {
          setPartials((p) => ({ ...p, [e.source]: e.isFinal ? "" : e.text }));
          if (e.isFinal) {
            setLatency((prev) => ({
              ...prev,
              [e.source]: [...prev[e.source], e.latencyMs].slice(-LATENCY_WINDOW),
            }));
          }
        } else if (e.kind === "connected") {
          setReconnecting(null);
          addLog(`${SOURCE_LABEL[e.source]}: transcrição conectada`);
        } else if (e.kind === "reconnecting") {
          setReconnecting(e.source);
          addLog(`${SOURCE_LABEL[e.source]}: reconectando (tentativa ${e.attempt}): ${e.message}`);
        } else if (e.kind === "error") {
          addLog(`${SOURCE_LABEL[e.source]}: transcrição: ${e.message}`);
        } else if (e.kind === "closed") {
          setPartials((p) => ({ ...p, [e.source]: "" }));
        }
      }),
      listen<SessionEvent>("sessao", ({ payload: e }) => {
        if (e.kind === "segment") {
          if (e.segment.echo) setEchoes((n) => n + 1);
          else setSegments((list) => insertSegment(list, e.segment));
        } else if (e.kind === "finished") {
          addLog(`Sessão salva (${formatClock(e.durationMs)}, ${e.echoes} ecos descartados)`);
        }
      }),
      listen<AnalysisEvent>("analise", ({ payload: e }) => {
        if (e.kind === "answer") setAnswers((list) => upsertAnswer(list, e.answer));
        else if (e.kind === "status" && e.state === "erro") addLog(`Análise: ${e.message}`);
      }),
      listen<number>("marcacao", ({ payload }) => {
        setMarks((list) => [...list, { id: -Date.now(), tMs: payload, kind: "mark", note: null }]);
        addLog(`Momento marcado em ${formatClock(payload)} (Ctrl+Shift+M)`);
      }),
    ];

    // Níveis chegam a 20 Hz por fonte; a tela redesenha a cada quadro.
    let frame = 0;
    const tick = () => {
      setLevels({ ...lastLevel.current });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    const clock = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      subscriptions.forEach((s) => s.then((off) => off()));
      cancelAnimationFrame(frame);
      clearInterval(clock);
    };
  }, []);

  async function chooseOtherSide(value: string) {
    if (value !== REPLAY) {
      setAppPid(value === NONE ? "" : value);
      setReplayFile(null);
      return;
    }
    const file = await open({ filters: [{ name: "Áudio WAV", extensions: ["wav"] }] });
    if (typeof file === "string") {
      setReplayFile(file);
      setAppPid("");
    }
  }

  function otherSideLabel() {
    if (replayFile) return `replay: ${fileName(replayFile)}`;
    if (appPid === DESKTOP) return "áudio do PC";
    return apps.find((a) => String(a.pid) === appPid)?.name ?? null;
  }

  async function start() {
    setSegments([]);
    setPartials(EMPTY_TEXT);
    setLatency({ mic: [], sistema: [] });
    setEchoes(0);
    setReconnecting(null);
    setAnswers([]);
    setMarks([]);
    setPaused(false);
    try {
      const started = await invoke<{ sessionId: number; wavDir: string | null }>("start_session", {
        request: {
          micId: micId || null,
          appPid: appPid ? Number(appPid) : null,
          replayFile,
          otherSideLabel: otherSideLabel(),
          saveWav,
          keyterms: keyterms.split(",").map((t) => t.trim()).filter(Boolean),
          profileId: Number(profileId),
          jobDescription,
        },
      });
      setSessionId(started.sessionId);
      setFinished(null);
      const t0 = Date.now();
      startedAtRef.current = t0;
      setStartedAt(t0);
      setPhase("running");
      setCandidateAware(false);
      getCurrentWindow().setTitle("● Gravando · Copiloto").catch(() => {});
      if (started.wavDir) addLog(`WAVs de debug em ${started.wavDir}`);
    } catch (e) {
      addLog(`Não iniciou: ${e}`);
    }
  }

  async function stop() {
    setPhase("stopping");
    await invoke("stop_session");
    getCurrentWindow().setTitle("Copiloto").catch(() => {});
    lastLevel.current = { mic: 0, sistema: 0 };
    setPartials(EMPTY_TEXT);
    setReconnecting(null);
    setPhase("done");
    await loadFinished();
  }

  async function loadFinished() {
    if (sessionId === null) return;
    try {
      setFinished(await invoke<SessionFull>("session_full", { id: sessionId }));
    } catch (e) {
      addLog(`Não carregou a sessão: ${e}`);
    }
  }

  async function markMoment() {
    try {
      const t = await invoke<number>("mark_moment", { note: null });
      setMarks((list) => [...list, { id: -Date.now(), tMs: t, kind: "mark", note: null }]);
    } catch (e) {
      addLog(`Marcação: ${e}`);
    }
  }

  async function togglePause() {
    const next = !paused;
    try {
      await invoke("set_paused", { paused: next });
      setPaused(next);
      const t = Date.now() - startedAtRef.current;
      setMarks((list) => [...list, { id: -Date.now(), tMs: t, kind: next ? "pause" : "resume", note: null }]);
    } catch (e) {
      addLog(`Pausa: ${e}`);
    }
  }

  async function toggleOnTop(value: boolean) {
    await getCurrentWindow().setAlwaysOnTop(value);
    setOnTop(value);
  }

  async function toggleHidden(value: boolean) {
    await getCurrentWindow().setContentProtected(value);
    setHidden(value);
  }

  const providerName = (p: string) => (p === "anthropic" ? "Anthropic" : "o provedor compatível com OpenAI");
  const otherSideOptions = [
    { value: NONE, label: "Nenhum (só microfone)" },
    { value: DESKTOP, label: "Todo o áudio do PC", detail: "pra apps que não aparecem ou não pegam" },
    ...apps.map((a) => ({ value: String(a.pid), label: a.name, detail: a.active ? "tocando agora" : undefined })),
    { value: REPLAY, label: replayFile ? `Replay: ${fileName(replayFile)}` : "Arquivo WAV (replay)…" },
  ];

  const setupView = (
    <Flex direction="column" gap="3">
      {!deepgramReady && (
        <Callout.Root color="amber" size="1">
          <Callout.Icon>
            <ExclamationTriangleIcon />
          </Callout.Icon>
          <Callout.Text>
            Sem chave da Deepgram, não dá pra começar.{" "}
            <Button size="1" variant="ghost" onClick={() => setTab("config")}>
              Configurar
            </Button>
          </Callout.Text>
        </Callout.Root>
      )}

      <Section title="Você">
          <Field label="Perfil" hint="As respostas sugeridas saem só do que está nele.">
            <Flex gap="2">
              <Box className="grow" minWidth="0">
                <Picker
                  value={profileId || NONE}
                  options={
                    profiles.length
                      ? profiles.map((p) => ({ value: String(p.id), label: p.name }))
                      : [{ value: NONE, label: "Nenhum perfil ainda" }]
                  }
                  onChange={(v) => setProfileId(v === NONE ? "" : v)}
                />
              </Box>
              <Button variant="soft" color="gray" onClick={() => setTab("perfil")}>
                Editar
              </Button>
            </Flex>
          </Field>
          <Field label="Vaga" hint="Cole a descrição. Ajuda a ligar suas respostas aos requisitos.">
            <TextArea rows={3} value={jobDescription} onChange={(e) => setJobDescription(e.target.value)} />
          </Field>
        </Section>

      <Section title="Áudio">
        <Field label="Microfone (você)">
          <Picker
            value={micId}
            options={mics.map((m) => ({ value: m.id, label: m.name, detail: m.default ? "padrão" : undefined }))}
            onChange={setMicId}
          />
        </Field>
        <Field label="Entrevistador (app da chamada)">
          <Flex gap="2">
            <Box className="grow" minWidth="0">
              <Picker
                value={replayFile ? REPLAY : appPid || NONE}
                options={otherSideOptions}
                onChange={chooseOtherSide}
              />
            </Box>
            <Tooltip content="Atualizar a lista de apps">
              <IconButton variant="soft" color="gray" onClick={refreshDevices} aria-label="Atualizar">
                <ReloadIcon />
              </IconButton>
            </Tooltip>
          </Flex>
        </Field>
        <Field
          label="Termos difíceis além dos do perfil"
          hint="Separados por vírgula: nomes, siglas da empresa."
        >
          <TextField.Root value={keyterms} onChange={(e) => setKeyterms(e.target.value)} />
        </Field>
        <SwitchRow label="Salvar WAV de debug" checked={saveWav} onChange={setSaveWav} />
      </Section>

      <Section title="Antes de começar">
          <Text size="2" color="gray">
            O áudio das duas pontas vai pra Deepgram e o texto da conversa vai pra{" "}
            {llm ? `${providerName(llm.live.provider)} (${llm.live.model})` : "LLM configurada"}. Tudo fica salvo só
            neste computador.
          </Text>
          <Text as="label" size="2">
            <Flex gap="2" align="start">
              <Checkbox checked={candidateAware} onCheckedChange={(v) => setCandidateAware(v === true)} />
              <span>
                Sei que a voz do entrevistador vai ser transcrita e que muitas empresas não permitem assistência
                durante a entrevista; confirmei que posso usar.
              </span>
            </Flex>
          </Text>
          <Button size="3" disabled={!candidateAware || !profileId || !deepgramReady} onClick={start}>
            Iniciar
          </Button>
          {!profileId && (
            <Text size="1" color="gray">
              Crie seu perfil na aba Perfil primeiro.
            </Text>
          )}
      </Section>
    </Flex>
  );

  const liveView = (
    <Flex direction="column" gap="3" className="live">
      <Card size="1">
        <Flex align="center" gap="3">
          {phase === "running" ? (
            <Badge size="2" color={paused ? "gray" : "red"} variant="soft" className="tabular">
              <span className={`rec-dot ${paused ? "paused" : ""}`} /> {formatClock(now - startedAt)}
            </Badge>
          ) : (
            <Text size="2" color="gray">
              {phase === "stopping" ? "Salvando…" : "Sessão salva"}
            </Text>
          )}
          <Flex direction="column" gap="1" className="grow" minWidth="0">
            {SOURCES.map((s) => (
              <Flex key={s} align="center" gap="2">
                <Text size="1" color="gray" className="meter-label">
                  {SOURCE_LABEL[s]}
                </Text>
                <Progress size="1" value={meterPercent(levels[s])} color="green" className="meter" />
              </Flex>
            ))}
          </Flex>
          {phase === "running" && (
            <Button color="red" onClick={stop}>
              <StopIcon /> Parar
            </Button>
          )}
          {phase === "done" && (
            <Button
              onClick={() => {
                setPhase("setup");
                refreshSetup();
              }}
            >
              Nova sessão
            </Button>
          )}
        </Flex>
      </Card>

      {phase === "running" && (
        <Flex align="center" gap="2" wrap="wrap">
          <Tooltip content="Ctrl+Shift+M funciona com a chamada em primeiro plano">
            <Button variant="soft" color="gray" onClick={markMoment}>
              <StarIcon /> Marcar
            </Button>
          </Tooltip>
          <Button variant="soft" color={paused ? "orange" : "gray"} onClick={togglePause}>
            {paused ? <PlayIcon /> : <PauseIcon />}
            {paused ? "Retomar" : "Pausar transcrição"}
          </Button>
        </Flex>
      )}

      {phase === "done" && finished && (
        <div className="print-area">
          <Notes session={finished} onChanged={loadFinished} />
        </div>
      )}

      {paused && (
        <Callout.Root color="gray" size="1">
          <Callout.Text>Transcrição pausada: o áudio não está saindo do computador.</Callout.Text>
        </Callout.Root>
      )}
      {reconnecting && (
        <Callout.Root color="amber" size="1">
          <Callout.Text>Reconectando a transcrição ({SOURCE_LABEL[reconnecting]})…</Callout.Text>
        </Callout.Root>
      )}

      {phase === "running" && (
        <AnswerFeed answers={answers} waiting={segments.some((s) => s.source === "sistema")} />
      )}

      <Card size="1" className="transcript-card">
        <ScrollArea type="auto" scrollbars="vertical" className="transcript-scroll">
          <Box p="2" pr="4">
            <Transcript segments={segments} partials={partials} marks={marks} follow />
          </Box>
        </ScrollArea>
      </Card>

      <Flex direction="column" gap="2">
        <Text size="1" color="gray" className="tabular">
          latência {SOURCES.map((s) => `${SOURCE_LABEL[s].toLowerCase()} ${formatMs(average(latency[s]))}`).join(", ")}
          {echoes > 0 ? ` · ${echoes} ecos descartados` : ""}
        </Text>
        <Flex gap="4" wrap="wrap">
          <SwitchRow label="Sempre no topo" checked={onTop} onChange={toggleOnTop} />
          <SwitchRow label="Esconder do compartilhamento de tela" checked={hidden} onChange={toggleHidden} />
        </Flex>
      </Flex>
    </Flex>
  );

  if (onboarded === false) {
    return (
      <main className="app">
        <Onboarding
          onDone={() => {
            setOnboarded(true);
            refreshSetup();
            refreshDevices();
          }}
        />
      </main>
    );
  }

  return (
    <main className="app">
      <Tabs.Root
        value={tab}
        onValueChange={(v) => {
          setTab(v as Tab);
          if (v === "sessao" && phase === "setup") refreshSetup();
        }}
      >
        <Tabs.List size="2" className="no-print">
          <Tabs.Trigger value="sessao">
            Sessão
            {phase === "running" && <span className="rec-dot tab-dot" />}
          </Tabs.Trigger>
          <Tabs.Trigger value="historico">Histórico</Tabs.Trigger>
          <Tabs.Trigger value="perfil">Perfil</Tabs.Trigger>
          <Tabs.Trigger value="config">Configurações</Tabs.Trigger>
        </Tabs.List>
      </Tabs.Root>

      <Box className="content">
        {tab === "sessao" && (phase === "setup" ? setupView : liveView)}
        {tab === "historico" && <History />}
        {tab === "perfil" && <Profiles onChanged={refreshSetup} />}
        {tab === "config" && <Settings onLog={addLog} />}
      </Box>

      <details className="disclosure log no-print">
        <summary>
          <Text size="1" color="gray">
            Registro
          </Text>
        </summary>
        <ol>
          {log.map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ol>
      </details>
    </main>
  );
}

export default App;

/** Troca a resposta da mesma pergunta ou acrescenta, mantendo a ordem das perguntas. */
function upsertAnswer(list: Answer[], answer: Answer) {
  const next = list.filter((a) => a.turn !== answer.turn);
  next.push(answer);
  return next.sort((a, b) => a.turn - b.turn);
}
