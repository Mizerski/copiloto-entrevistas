import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { average, formatClock, formatMs } from "./format";
import { History } from "./History";
import { BlockClock, CardList, CoveragePanel, visibleCards } from "./Insights";
import { Consent } from "./Consent";
import { Kits } from "./Kits";
import { Notes } from "./Notes";
import { Onboarding } from "./Onboarding";
import { computeMetrics } from "./metrics";
import { Settings } from "./Settings";
import { insertSegment, Transcript } from "./Transcript";
import {
  SOURCES,
  SOURCE_LABEL,
  type AnalysisEvent,
  type AudioApp,
  type Card,
  type CaptureEvent,
  type CoverageItem,
  type Kit,
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

type Tab = "sessao" | "historico" | "kits" | "config";
type Phase = "setup" | "running" | "stopping" | "done";

const TABS: { id: Tab; label: string }[] = [
  { id: "sessao", label: "Sessão" },
  { id: "historico", label: "Histórico" },
  { id: "kits", label: "Kits" },
  { id: "config", label: "Configurações" },
];
const LATENCY_WINDOW = 20;
const REPLAY_OPTION = "__arquivo__";
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

function App() {
  const [tab, setTab] = useState<Tab>("sessao");
  const [phase, setPhase] = useState<Phase>("setup");

  // Preparação
  const [mics, setMics] = useState<Microphone[]>([]);
  const [apps, setApps] = useState<AudioApp[]>([]);
  const [kits, setKits] = useState<KitRow[]>([]);
  const [micId, setMicId] = useState("");
  const [appPid, setAppPid] = useState("");
  const [replayFile, setReplayFile] = useState<string | null>(null);
  const [kitId, setKitId] = useState("");
  const [candidateName, setCandidateName] = useState("");
  const [candidateResume, setCandidateResume] = useState("");
  const [candidateCheckpoints, setCandidateCheckpoints] = useState("");
  const [analyze, setAnalyze] = useState(true);
  const [keyterms, setKeyterms] = useState("");
  const [saveWav, setSaveWav] = useState(false);
  const [deepgramReady, setDeepgramReady] = useState(false);
  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [consent, setConsent] = useState(false);
  const [onboarded, setOnboarded] = useState<boolean | null>(null);

  // Sessão
  const [startedAt, setStartedAt] = useState(0);
  const [sessionId, setSessionId] = useState<number | null>(null);
  const [finished, setFinished] = useState<SessionFull | null>(null);
  const [now, setNow] = useState(Date.now());
  const [kit, setKit] = useState<Kit | null>(null);
  const [levels, setLevels] = useState<Record<Source, number>>({ mic: 0, sistema: 0 });
  const [segments, setSegments] = useState<Segment[]>([]);
  const [partials, setPartials] = useState<Record<Source, string>>(EMPTY_TEXT);
  const [latency, setLatency] = useState<Record<Source, number[]>>({ mic: [], sistema: [] });
  const [echoes, setEchoes] = useState(0);
  const [reconnecting, setReconnecting] = useState<Source | null>(null);
  const [coverage, setCoverage] = useState<CoverageItem[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());
  const [analysisState, setAnalysisState] = useState<string | null>(null);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [blockIndex, setBlockIndex] = useState(0);
  const [blockStartedAt, setBlockStartedAt] = useState(0);
  const [focus, setFocus] = useState(false);
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
      setKits(await invoke<KitRow[]>("list_kits"));
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

    const addMark = (tMs: number, kind: Mark["kind"] = "mark") =>
      setMarks((list) => [...list, { id: -Date.now(), tMs, kind, note: null }]);

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
        if (e.kind === "coverage") setCoverage(e.items);
        else if (e.kind === "card") setCards((list) => [...list, e.card]);
        else {
          setAnalysisState(e.state);
          if (e.state === "erro") addLog(`Análise: ${e.message}`);
        }
      }),
      listen<number>("marcacao", ({ payload }) => {
        addMark(payload);
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

  const metrics = useMemo(() => computeMetrics(segments), [segments]);
  const shownCards = visibleCards(cards, dismissed, focus);

  async function chooseOtherSide(value: string) {
    if (value !== REPLAY_OPTION) {
      setAppPid(value);
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
    return apps.find((a) => String(a.pid) === appPid)?.name ?? null;
  }

  async function chooseBlock(index: number, blocks = kit?.blocks ?? []) {
    const block = blocks[index];
    if (!block) return;
    setBlockIndex(index);
    setBlockStartedAt(Date.now());
    await invoke("set_block", { name: block.name, minutes: block.minutes }).catch(() => {});
  }

  async function start() {
    setSegments([]);
    setPartials(EMPTY_TEXT);
    setLatency({ mic: [], sistema: [] });
    setEchoes(0);
    setReconnecting(null);
    setCoverage([]);
    setCards([]);
    setDismissed(new Set());
    setAnalysisState(null);
    setMarks([]);
    setPaused(false);
    try {
      const started = await invoke<{
        sessionId: number;
        wavDir: string | null;
        kit: Kit | null;
        warnings: string[];
      }>("start_session", {
        request: {
          micId: micId || null,
          appPid: appPid ? Number(appPid) : null,
          replayFile,
          otherSideLabel: otherSideLabel(),
          saveWav,
          transcribe: deepgramReady,
          keyterms: keyterms.split(",").map((t) => t.trim()).filter(Boolean),
          kitId: kitId ? Number(kitId) : null,
          candidateName,
          candidateResume,
          candidateCheckpoints,
          analyze,
        },
      });
      setSessionId(started.sessionId);
      setFinished(null);
      const t0 = Date.now();
      startedAtRef.current = t0;
      setStartedAt(t0);
      setKit(started.kit);
      setPhase("running");
      setConsent(false);
      getCurrentWindow().setTitle("● Gravando · Copiloto").catch(() => {});
      started.warnings.forEach((w) => addLog(w));
      if (started.wavDir) addLog(`WAVs de debug em ${started.wavDir}`);
      if (started.kit?.blocks.length) await chooseBlock(0, started.kit.blocks);
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

  const live = phase !== "setup";

  const providerName = (p: string) => (p === "anthropic" ? "Anthropic" : "o provedor compatível com OpenAI configurado");
  const destinations = [
    deepgramReady
      ? "O áudio das duas pontas vai pra Deepgram, pra ser transcrito. Não é gravado."
      : "Sem chave da Deepgram: nada é transcrito.",
    ...(kitId && analyze && llm
      ? [`O texto da conversa vai pra ${providerName(llm.live.provider)} durante a sessão (${llm.live.model}).`]
      : []),
    "Transcrição, cobertura e notas ficam só neste computador.",
  ];

  const setupView = (
    <>
      {!deepgramReady && (
        <p className="warning">
          Sem chave da Deepgram: a sessão só captura áudio.{" "}
          <button className="link" onClick={() => setTab("config")}>
            Configurar
          </button>
        </p>
      )}
      <section>
        <div className="section-title">Entrevista</div>
        <label>
          Kit
          <select value={kitId} onChange={(e) => setKitId(e.target.value)}>
            <option value="">Sem kit (só transcrição)</option>
            {kits.map((k) => (
              <option key={k.id} value={k.id}>
                {k.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Nome do candidato
          <input type="text" value={candidateName} onChange={(e) => setCandidateName(e.target.value)} />
        </label>
        <label>
          Currículo (colado)
          <textarea rows={4} value={candidateResume} onChange={(e) => setCandidateResume(e.target.value)} />
        </label>
        <label>
          Pontos a checar
          <textarea
            rows={2}
            placeholder="gap de 8 meses em 2024; diz que liderou a migração"
            value={candidateCheckpoints}
            onChange={(e) => setCandidateCheckpoints(e.target.value)}
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={analyze}
            disabled={!kitId}
            onChange={(e) => setAnalyze(e.target.checked)}
          />
          Análise ao vivo com LLM (cobertura e sugestões)
        </label>
      </section>

      <section>
        <div className="section-title">Áudio</div>
        <label>
          Microfone
          <select value={micId} onChange={(e) => setMicId(e.target.value)}>
            {mics.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
                {m.default ? " (padrão)" : ""}
              </option>
            ))}
          </select>
        </label>
        <label>
          Candidato (app da chamada)
          <div className="row">
            <select value={replayFile ? REPLAY_OPTION : appPid} onChange={(e) => chooseOtherSide(e.target.value)}>
              <option value="">Nenhum (só microfone)</option>
              {apps.map((a) => (
                <option key={a.pid} value={a.pid}>
                  {a.name} · {a.pid}
                  {a.active ? " · tocando" : ""}
                </option>
              ))}
              <option value={REPLAY_OPTION}>
                {replayFile ? `Replay: ${fileName(replayFile)}` : "Arquivo WAV (replay)…"}
              </option>
            </select>
            <button onClick={refreshDevices}>Atualizar</button>
          </div>
        </label>
        <label>
          Termos difíceis além dos do kit (vírgula)
          <input
            type="text"
            placeholder="nomes, siglas da empresa"
            value={keyterms}
            onChange={(e) => setKeyterms(e.target.value)}
          />
        </label>
        <label className="check">
          <input type="checkbox" checked={saveWav} onChange={(e) => setSaveWav(e.target.checked)} />
          Salvar WAV de debug
        </label>
      </section>

      <Consent checked={consent} onChange={setConsent} destinations={destinations} />

      <button className="primary" disabled={!consent} onClick={start}>
        Iniciar sessão
      </button>
    </>
  );

  const liveView = (
    <>
      <div className="session-bar">
        {phase === "running" ? (
          <span className={`rec ${paused ? "paused" : ""}`}>
            <span className="dot" /> {formatClock(now - startedAt)}
          </span>
        ) : (
          <span className="muted">{phase === "stopping" ? "Salvando…" : "Sessão salva"}</span>
        )}
        <div className="mini-meters">
          {SOURCES.map((s) => (
            <div key={s} className="mini-meter" title={SOURCE_LABEL[s]}>
              <span>{SOURCE_LABEL[s]}</span>
              <div className="bar">
                <div style={{ width: `${meterPercent(levels[s])}%` }} />
              </div>
            </div>
          ))}
        </div>
        {phase === "running" && (
          <button className="stop" onClick={stop}>
            Parar
          </button>
        )}
        {phase === "done" && (
          <button
            className="primary"
            onClick={() => {
              setPhase("setup");
              refreshSetup();
            }}
          >
            Nova
          </button>
        )}
      </div>

      {phase === "running" && (
        <div className="row actions">
          <button onClick={markMoment} title="Ctrl+Shift+M funciona com outra janela em foco">
            ★ Marcar
          </button>
          <button onClick={togglePause} className={paused ? "primary" : ""}>
            {paused ? "▶ Retomar transcrição" : "⏸ Pausar transcrição"}
          </button>
          <label className="check">
            <input type="checkbox" checked={focus} onChange={(e) => setFocus(e.target.checked)} />
            Foco
          </label>
        </div>
      )}

      {phase === "done" && finished && <Notes session={finished} onChanged={loadFinished} />}

      {paused && <p className="warning">Transcrição pausada: o áudio não está saindo do computador.</p>}
      {reconnecting && <p className="warning">Reconectando a transcrição ({SOURCE_LABEL[reconnecting]})…</p>}

      {kit && kit.blocks.length > 0 && phase === "running" && (
        <BlockClock
          blocks={kit.blocks}
          current={blockIndex}
          blockStartedAt={blockStartedAt}
          now={now}
          onChange={(i) => chooseBlock(i)}
        />
      )}

      <CardList cards={shownCards} onDismiss={(id) => setDismissed((d) => new Set(d).add(id))} />

      {!focus && coverage.length > 0 && (
        <details className="coverage-box" open>
          <summary>
            Cobertura{" "}
            {analysisState === "analisando" && <span className="muted small">· analisando…</span>}
          </summary>
          <CoveragePanel items={coverage} />
        </details>
      )}

      <section className="transcript-box">
        <Transcript segments={segments} partials={partials} marks={marks} follow />
      </section>

      <div className="session-footer">
        <span>
          {metrics.interviewerShare !== null
            ? `Você falou ${Math.round(metrics.interviewerShare * 100)}% · `
            : ""}
          {metrics.questions} perguntas ({metrics.openQuestions} abertas)
          {metrics.answers.length
            ? ` · última resposta ${formatClock(metrics.answers[metrics.answers.length - 1])}`
            : ""}
        </span>
        <span>
          Latência: {SOURCES.map((s) => `${SOURCE_LABEL[s]} ${formatMs(average(latency[s]))}`).join(" · ")}
          {echoes > 0 ? ` · ${echoes} ecos descartados` : ""}
        </span>
        <label className="check">
          <input type="checkbox" checked={onTop} onChange={(e) => toggleOnTop(e.target.checked)} />
          Sempre no topo
        </label>
        <label className="check">
          <input type="checkbox" checked={hidden} onChange={(e) => toggleHidden(e.target.checked)} />
          Esconder do compartilhamento de tela
        </label>
      </div>
    </>
  );

  if (onboarded === false) {
    return (
      <main>
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
    <main>
      <nav className="tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            className={tab === t.id ? "active" : ""}
            onClick={() => {
              setTab(t.id);
              if (t.id === "sessao" && phase === "setup") refreshSetup();
            }}
          >
            {t.label}
            {t.id === "sessao" && phase === "running" ? " ●" : ""}
          </button>
        ))}
      </nav>

      {tab === "sessao" && (live ? liveView : setupView)}
      {tab === "historico" && <History />}
      {tab === "kits" && <Kits onChanged={refreshSetup} />}
      {tab === "config" && <Settings onLog={addLog} />}

      <details className="log">
        <summary>Registro</summary>
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
