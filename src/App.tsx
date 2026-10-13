import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

type Source = "mic" | "sistema";

type Microphone = { id: string; name: string; default: boolean };
type AudioApp = { pid: number; name: string; active: boolean };

type CaptureEvent =
  | { kind: "started"; source: Source; format: string }
  | { kind: "level"; source: Source; tMs: number; rms: number; peak: number }
  | { kind: "error"; source: Source; message: string }
  | { kind: "stopped"; source: Source; seconds: number };

type SttEvent =
  | { kind: "connected"; source: Source }
  | {
      kind: "transcript";
      source: Source;
      text: string;
      isFinal: boolean;
      speechFinal: boolean;
      startMs: number;
      endMs: number;
      latencyMs: number;
    }
  | { kind: "utteranceEnd"; source: Source }
  | { kind: "error"; source: Source; message: string }
  | { kind: "closed"; source: Source };

type Line = { id: number; source: Source; text: string; startMs: number; latencyMs: number };

type Latency = { partial: number[]; final: number[] };

const SOURCES: Source[] = ["mic", "sistema"];
const SOURCE_LABEL: Record<Source, string> = { mic: "Você", sistema: "Outro lado" };
const LATENCY_WINDOW = 20;

/** RMS linear → 0..100, numa escala de -60 dB a 0 dB. */
function meterPercent(rms: number) {
  if (rms <= 0) return 0;
  const db = 20 * Math.log10(rms);
  return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
}

function average(values: number[]) {
  return values.length ? values.reduce((a, b) => a + b, 0) / values.length : null;
}

function formatMs(ms: number | null) {
  if (ms === null) return "—";
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function formatClock(ms: number) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function emptyLatency(): Record<Source, Latency> {
  return { mic: { partial: [], final: [] }, sistema: { partial: [], final: [] } };
}

function App() {
  const [mics, setMics] = useState<Microphone[]>([]);
  const [apps, setApps] = useState<AudioApp[]>([]);
  const [micId, setMicId] = useState("");
  const [appPid, setAppPid] = useState("");
  const [saveWav, setSaveWav] = useState(false);
  const [running, setRunning] = useState(false);
  const [levels, setLevels] = useState<Record<Source, number>>({ mic: 0, sistema: 0 });
  const [log, setLog] = useState<string[]>([]);
  const [onTop, setOnTop] = useState(false);
  const [hidden, setHidden] = useState(false);

  const [keyMask, setKeyMask] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [transcribe, setTranscribe] = useState(true);
  const [keyterms, setKeyterms] = useState("");

  const [lines, setLines] = useState<Line[]>([]);
  const [partials, setPartials] = useState<Record<Source, string>>({ mic: "", sistema: "" });
  const [latency, setLatency] = useState<Record<Source, Latency>>(emptyLatency);

  const lastLevel = useRef<Record<Source, number>>({ mic: 0, sistema: 0 });
  const nextLineId = useRef(0);
  const transcriptEnd = useRef<HTMLDivElement>(null);

  const addLog = (line: string) =>
    setLog((l) => [`${new Date().toLocaleTimeString()}  ${line}`, ...l].slice(0, 40));

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

  function recordLatency(source: Source, kind: keyof Latency, ms: number) {
    setLatency((prev) => ({
      ...prev,
      [source]: { ...prev[source], [kind]: [...prev[source][kind], ms].slice(-LATENCY_WINDOW) },
    }));
  }

  useEffect(() => {
    refreshDevices();
    invoke<string | null>("deepgram_key_status")
      .then(setKeyMask)
      .catch((e) => addLog(`Credential Manager: ${e}`));

    const unlistenCapture = listen<CaptureEvent>("captura", ({ payload: e }) => {
      if (e.kind === "level") {
        lastLevel.current[e.source] = e.rms;
      } else if (e.kind === "started") {
        addLog(`${SOURCE_LABEL[e.source]}: capturando (${e.format})`);
      } else if (e.kind === "error") {
        addLog(`${SOURCE_LABEL[e.source]}: ${e.message}`);
      } else if (e.kind === "stopped") {
        addLog(`${SOURCE_LABEL[e.source]}: ${e.seconds.toFixed(1)} s capturados`);
      }
    });

    const unlistenStt = listen<SttEvent>("transcricao", ({ payload: e }) => {
      if (e.kind === "transcript") {
        if (e.isFinal) {
          const line: Line = {
            id: nextLineId.current++,
            source: e.source,
            text: e.text,
            startMs: e.startMs,
            latencyMs: e.latencyMs,
          };
          // As duas fontes chegam fora de ordem; a ordem certa é a do momento da fala.
          setLines((prev) => [...prev, line].sort((a, b) => a.startMs - b.startMs).slice(-300));
          setPartials((p) => ({ ...p, [e.source]: "" }));
          recordLatency(e.source, "final", e.latencyMs);
        } else {
          setPartials((p) => ({ ...p, [e.source]: e.text }));
          recordLatency(e.source, "partial", e.latencyMs);
        }
      } else if (e.kind === "connected") {
        addLog(`${SOURCE_LABEL[e.source]}: transcrição conectada`);
      } else if (e.kind === "error") {
        addLog(`${SOURCE_LABEL[e.source]}: transcrição: ${e.message}`);
      } else if (e.kind === "closed") {
        addLog(`${SOURCE_LABEL[e.source]}: transcrição encerrada`);
        setPartials((p) => ({ ...p, [e.source]: "" }));
      }
    });

    // Os níveis chegam a 20 Hz por fonte; a tela só redesenha a cada quadro.
    let frame = 0;
    const tick = () => {
      setLevels({ ...lastLevel.current });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => {
      unlistenCapture.then((f) => f());
      unlistenStt.then((f) => f());
      cancelAnimationFrame(frame);
    };
  }, []);

  useEffect(() => {
    transcriptEnd.current?.scrollIntoView({ block: "end" });
  }, [lines, partials]);

  async function start() {
    try {
      setLines([]);
      setPartials({ mic: "", sistema: "" });
      setLatency(emptyLatency());
      const dir = await invoke<string | null>("start_capture", {
        micId: micId || null,
        appPid: appPid ? Number(appPid) : null,
        saveWav,
        transcribe: transcribe && keyMask !== null,
        keyterms: keyterms.split(",").map((t) => t.trim()).filter(Boolean),
      });
      setRunning(true);
      if (dir) addLog(`WAVs de debug em ${dir}`);
    } catch (e) {
      addLog(`Não iniciou: ${e}`);
    }
  }

  async function stop() {
    await invoke("stop_capture");
    lastLevel.current = { mic: 0, sistema: 0 };
    setRunning(false);
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

  return (
    <main>
      <header>
        <h1>Copiloto</h1>
        <span className="tag">spike · fase 0</span>
      </header>

      <section>
        <label>
          Microfone
          <select value={micId} disabled={running} onChange={(e) => setMicId(e.target.value)}>
            {mics.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
                {m.default ? " (padrão)" : ""}
              </option>
            ))}
          </select>
        </label>

        <label>
          App da chamada
          <div className="row">
            <select value={appPid} disabled={running} onChange={(e) => setAppPid(e.target.value)}>
              <option value="">Nenhum (só microfone)</option>
              {apps.map((a) => (
                <option key={a.pid} value={a.pid}>
                  {a.name} · {a.pid}
                  {a.active ? " · tocando" : ""}
                </option>
              ))}
            </select>
            <button disabled={running} onClick={refreshDevices}>
              Atualizar
            </button>
          </div>
        </label>

        <label className="check">
          <input
            type="checkbox"
            checked={saveWav}
            disabled={running}
            onChange={(e) => setSaveWav(e.target.checked)}
          />
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
            <button disabled={keyBusy || running} onClick={testKey}>
              Testar
            </button>
            <button disabled={keyBusy || running} onClick={deleteKey}>
              Remover
            </button>
          </div>
        )}
        <label className="check">
          <input
            type="checkbox"
            checked={transcribe}
            disabled={running || keyMask === null}
            onChange={(e) => setTranscribe(e.target.checked)}
          />
          Transcrever durante a captura
        </label>
        <label>
          Termos difíceis (separados por vírgula)
          <input
            type="text"
            placeholder="Kafka, Kubernetes, PIX, nome do candidato"
            value={keyterms}
            disabled={running}
            onChange={(e) => setKeyterms(e.target.value)}
          />
        </label>
      </section>

      <button className={running ? "stop" : "primary"} onClick={running ? stop : start}>
        {running ? "Parar" : "Iniciar"}
      </button>

      <section>
        {SOURCES.map((s) => (
          <div className="meter" key={s}>
            <span>{SOURCE_LABEL[s]}</span>
            <div className="bar">
              <div style={{ width: `${meterPercent(levels[s])}%` }} />
            </div>
          </div>
        ))}
        <table className="latency">
          <thead>
            <tr>
              <th>Latência</th>
              <th>Parcial</th>
              <th>Final</th>
            </tr>
          </thead>
          <tbody>
            {SOURCES.map((s) => (
              <tr key={s}>
                <td>{SOURCE_LABEL[s]}</td>
                <td>{formatMs(average(latency[s].partial))}</td>
                <td>{formatMs(average(latency[s].final))}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="transcript">
        {lines.length === 0 && !partials.mic && !partials.sistema && (
          <p className="empty">A transcrição aparece aqui.</p>
        )}
        {lines.map((l) => (
          <p key={l.id} className={`line ${l.source}`}>
            <span className="who">
              {SOURCE_LABEL[l.source]} · {formatClock(l.startMs)}
            </span>
            {l.text}
            <span className="lag">+{formatMs(l.latencyMs)}</span>
          </p>
        ))}
        {SOURCES.filter((s) => partials[s]).map((s) => (
          <p key={`partial-${s}`} className={`line partial ${s}`}>
            <span className="who">{SOURCE_LABEL[s]}</span>
            {partials[s]}
          </p>
        ))}
        <div ref={transcriptEnd} />
      </section>

      <section>
        <label className="check">
          <input type="checkbox" checked={onTop} onChange={(e) => toggleOnTop(e.target.checked)} />
          Sempre no topo
        </label>
        <label className="check">
          <input type="checkbox" checked={hidden} onChange={(e) => toggleHidden(e.target.checked)} />
          Esconder do compartilhamento de tela
        </label>
      </section>

      <ol className="log">
        {log.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ol>
    </main>
  );
}

export default App;
