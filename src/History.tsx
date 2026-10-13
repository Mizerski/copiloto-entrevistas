import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { formatClock, formatDate } from "./format";
import { Transcript } from "./Transcript";
import type { Segment, SessionSummary } from "./types";

export function History() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [open, setOpen] = useState<SessionSummary | null>(null);
  const [segments, setSegments] = useState<Segment[]>([]);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      setSessions(await invoke<SessionSummary[]>("list_sessions"));
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    load();
  }, []);

  async function show(s: SessionSummary) {
    try {
      setSegments(await invoke<Segment[]>("session_segments", { id: s.id }));
      setOpen(s);
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(s: SessionSummary) {
    const ok = await ask(`Apagar a sessão de ${formatDate(s.startedAt)}? Não dá pra desfazer.`, {
      title: "Apagar sessão",
      kind: "warning",
      okLabel: "Apagar",
      cancelLabel: "Cancelar",
    });
    if (!ok) return;
    try {
      await invoke("delete_session", { id: s.id });
      setOpen(null);
      load();
    } catch (e) {
      setError(String(e));
    }
  }

  if (open) {
    const kept = segments.filter((s) => !s.echo);
    const echoes = segments.length - kept.length;
    return (
      <div className="history">
        <div className="row">
          <button onClick={() => setOpen(null)}>← Voltar</button>
          <span className="grow" />
          <button className="danger" onClick={() => remove(open)}>
            Apagar
          </button>
        </div>
        <div className="session-meta">
          <strong>{formatDate(open.startedAt)}</strong>
          <span>
            {formatClock(open.durationMs ?? 0)} · {open.otherSide ?? "só microfone"} ·{" "}
            {kept.length} falas
            {echoes > 0 ? ` · ${echoes} ecos descartados` : ""}
            {open.interrupted ? " · interrompida" : ""}
          </span>
        </div>
        <section>
          <Transcript segments={kept} />
        </section>
      </div>
    );
  }

  return (
    <div className="history">
      {error && <p className="error">{error}</p>}
      {sessions.length === 0 ? (
        <p className="empty">Nenhuma sessão gravada ainda.</p>
      ) : (
        <ul className="session-list">
          {sessions.map((s) => (
            <li key={s.id}>
              <button onClick={() => show(s)}>
                <strong>{formatDate(s.startedAt)}</strong>
                <span>
                  {s.durationMs === null ? "em andamento" : formatClock(s.durationMs)} ·{" "}
                  {s.otherSide ?? "só microfone"} · {s.segments} falas
                  {s.interrupted ? " · interrompida" : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
