import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { average, formatClock, formatMs } from "./format";
import { History } from "./History";
import { insertSegment, Transcript } from "./Transcript";
import {
  SOURCES,
  SOURCE_LABEL,
  type AudioApp,
  type CaptureEvent,
  type Microphone,
  type Segment,
  type SessionEvent,
  type Source,
  type SttEvent,
} from "./types";
import "./App.css";

type Phase = "setup" | "running" | "stopping" | "done";
type Latency = Record<Source, number[]>;

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
  const [tab, setTab] = useState<"sessao" | "historico">("sessao");
  const [phase, setPhase] = useState<Phase>("setup");

  // Preparação
  const [mics, setMics] = useState<Microphone[]>([]);
  const [apps, setApps] = useState<AudioApp[]>([]);
  const [micId, setMicId] = useState("");
  const [appPid, setAppPid] = useState("");
  const [replayFile, setReplayFile] = useState<string | null>(null);
  const [saveWav, setSaveWav] = useState(false);
  const [keyMask, setKeyMask] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [transcribe, setTranscribe] = useState(true);
  const [keyterms, setKeyterms] = useState("");

  // Sessão
  const [startedAt, setStartedAt] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [levels, setLevels] = useState<Record<Source, number>>({ mic: 0, sistema: 0 });
  const [segments, setSegments] = useState<Segment[]>([]);
  const [partials, setPartials] = useState<Record<Source, string>>(EMPTY_TEXT);
  const [latency, setLatency] = useState<Latency>({ mic: [], sistema: [] });
  const [echoes, setEchoes] = useState(0);
  const [reconnecting, setReconnecting] = useState<Source | null>(null);
  const [onTop, setOnTop] = useState(false);
  const [hidden, setHidden] = useState(false);

  const [log, setLog] = useState<string[]>([]);
  const lastLevel = useRef<Record<Source, number>>({ mic: 0, sistema: 0 });

  const addLog = (line: string) =>
    setLog((l) => [`${new Date().toLocaleTimeString()}  ${line}`, ...l].slice(0, 60));

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

  useEffect(() => {
    refreshDevices();
    invoke<string | null>("deepgram_key_status")
      .then(setKeyMask)
      .catch((e) => addLog(`Credential Manager: ${e}`));

    const subscriptions = [
      listen<CaptureEvent>("captura", ({ payload: e }) => {
        if (e.kind === "level") lastLevel.current[e.source] = e.rms;
        else if (e.kind === "started") addLog(`${SOURCE_LABEL[e.source]}: capturando (${e.format})`);
        else if (e.kind === "error") addLog(`${SOURCE_LABEL[e.source]}: ${e.message}`);
        else if (e.kind === "stopped")
          addLog(`${SOURCE_LABEL[e.source]}: ${e.seconds.toFixed(1)} s capturados`);
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

  async function start() {
    setSegments([]);
    setPartials(EMPTY_TEXT);
    setLatency({ mic: [], sistema: [] });
    setEchoes(0);
    setReconnecting(null);
    try {
      const started = await invoke<{ sessionId: number; wavDir: string | null }>("start_session", {
        micId: micId || null,
        appPid: appPid ? Number(appPid) : null,
        replayFile,
        otherSideLabel: otherSideLabel(),
        saveWav,
        transcribe: transcribe && keyMask !== null,
        keyterms: keyterms.split(",").map((t) => t.trim()).filter(Boolean),
      });
      setStartedAt(Date.now());
      setPhase("running");
      if (started.wavDir) addLog(`WAVs de debug em ${started.wavDir}`);
    } catch (e) {
      addLog(`Não iniciou: ${e}`);
    }
  }

  async function stop() {
    setPhase("stopping");
    await invoke("stop_session");
    lastLevel.current = { mic: 0, sistema: 0 };
    setPartials(EMPTY_TEXT);
    setReconnecting(null);
    setPhase("done");
  }

  async function saveKey() {
    setKeyBusy(true);
    try {
      setKeyMask(await invoke<string>("save_deepgram_key", { key: keyInput }));
      setKeyInput("");
      addLog("Chave da Deepgram aceita e salva no Credential Manager");
    } catch (e) {
      addLog(`Chave não salva: ${e}`);
    } finally {
      setKeyBusy(false);
    }
  }

  async function testKey() {
    setKeyBusy(true);
    try {
      await invoke("test_deepgram_key");
      addLog("Chave da Deepgram ok");
    } catch (e) {
      addLog(`Teste da chave falhou: ${e}`);
    } finally {
      setKeyBusy(false);
    }
  }

  async function deleteKey() {
    await invoke("delete_deepgram_key");
    setKeyMask(null);
    addLog("Chave da Deepgram removida");
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

  const setupView = (
    <>
      <section>
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
            <select
              value={replayFile ? REPLAY_OPTION : appPid}
              onChange={(e) => chooseOtherSide(e.target.value)}
            >
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
        <label className="check">
          <input type="checkbox" checked={saveWav} onChange={(e) => setSaveWav(e.target.checked)} />
          Salvar WAV de debug
        </label>
      </section>

      <section>
        <div className="section-title">Transcrição · Deepgram</div>
        {keyMask === null ? (
          <label>
            Chave de API
            <div className="row">
              <input
                type="password"
                autoComplete="off"
                placeholder="cole a chave aqui"
                value={keyInput}
                onChange={(e) => setKeyInput(e.target.value)}
              />
              <button disabled={keyBusy || !keyInput.trim()} onClick={saveKey}>
                {keyBusy ? "Testando…" : "Salvar"}
              </button>
            </div>
          </label>
        ) : (
          <div className="row key-row">
            <span>
              Chave salva <code>{keyMask}</code>
            </span>
            <button disabled={keyBusy} onClick={testKey}>
              Testar
            </button>
            <button disabled={keyBusy} onClick={deleteKey}>
              Remover
            </button>
          </div>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={transcribe}
            disabled={keyMask === null}
            onChange={(e) => setTranscribe(e.target.checked)}
          />
          Transcrever durante a sessão
        </label>
        <label>
          Termos difíceis (separados por vírgula)
          <input
            type="text"
            placeholder="Kafka, Kubernetes, Pix, nome do candidato"
            value={keyterms}
            onChange={(e) => setKeyterms(e.target.value)}
          />
        </label>
      </section>

      <button className="primary" onClick={start}>
        Iniciar sessão
      </button>
    </>
  );

  const liveView = (
    <>
      <div className="session-bar">
        {phase === "running" ? (
          <span className="rec">
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
          <button className="primary" onClick={() => setPhase("setup")}>
            Nova
          </button>
        )}
      </div>

      {reconnecting && (
        <p className="warning">Reconectando a transcrição ({SOURCE_LABEL[reconnecting]})…</p>
      )}

      <section className="transcript-box">
        <Transcript segments={segments} partials={partials} follow />
      </section>

      <div className="session-footer">
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

  return (
    <main>
      <nav className="tabs">
        <button className={tab === "sessao" ? "active" : ""} onClick={() => setTab("sessao")}>
          Sessão
        </button>
        <button
          className={tab === "historico" ? "active" : ""}
          onClick={() => setTab("historico")}
        >
          Histórico
        </button>
      </nav>

      {tab === "historico" ? <History /> : live ? liveView : setupView}

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
