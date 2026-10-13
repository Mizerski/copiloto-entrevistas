import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { formatClock, formatDate } from "./format";
import { CardList, CoveragePanel } from "./Insights";
import { computeMetrics } from "./metrics";
import { Notes } from "./Notes";
import { Transcript } from "./Transcript";
import type { CoverageItem, SessionFull, SessionSummary } from "./types";

export function History() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [open, setOpen] = useState<SessionFull | null>(null);
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

  async function show(id: number) {
    try {
      setOpen(await invoke<SessionFull>("session_full", { id }));
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(id: number, startedAt: number) {
    const ok = await ask(`Apagar a sessão de ${formatDate(startedAt)}? Não dá pra desfazer.`, {
      title: "Apagar sessão",
      kind: "warning",
      okLabel: "Apagar",
      cancelLabel: "Cancelar",
    });
    if (!ok) return;
    try {
      await invoke("delete_session", { id });
      setOpen(null);
      load();
    } catch (e) {
      setError(String(e));
    }
  }

  if (open) {
    const { detail, segments, cards, marks, usage } = open;
    const kept = segments.filter((s) => !s.echo);
    const echoes = segments.length - kept.length;
    const coverage: CoverageItem[] = detail.coverageJson ? JSON.parse(detail.coverageJson) : [];
    const m = computeMetrics(segments);
    return (
      <div className="history">
        <div className="row no-print">
          <button
            onClick={() => {
              setOpen(null);
              load();
            }}
          >
            ← Voltar
          </button>
          <span className="grow" />
          <button className="danger" onClick={() => remove(detail.id, detail.startedAt)}>
            Apagar sessão
          </button>
        </div>
        <div className="session-meta">
          <strong>
            {detail.candidateName ?? "Candidato sem nome"} · {formatDate(detail.startedAt)}
          </strong>
          <span>
            {formatClock(detail.durationMs ?? 0)} · {detail.kitName ?? "sem kit"} ·{" "}
            {detail.otherSide ?? "só microfone"}
            {detail.interrupted ? " · interrompida" : ""}
          </span>
          <span>
            {kept.length} falas
            {echoes > 0 ? ` · ${echoes} ecos descartados` : ""}
            {m.interviewerShare !== null ? ` · você falou ${Math.round(m.interviewerShare * 100)}%` : ""}
            {` · ${m.questions} perguntas (${m.openQuestions} abertas)`}
            {usage.calls > 0
              ? ` · ${usage.calls} chamadas de LLM, ${usage.inputTokens + usage.outputTokens} tokens`
              : ""}
          </span>
        </div>

        <Notes session={open} onChanged={() => show(detail.id)} />

        {coverage.length > 0 && (
          <section className="no-print">
            <div className="section-title">Cobertura ao vivo</div>
            <CoveragePanel items={coverage} />
          </section>
        )}
        {cards.length > 0 && (
          <details className="no-print">
            <summary>Cards mostrados durante a sessão ({cards.length})</summary>
            <CardList cards={cards} />
          </details>
        )}
        {segments.length > 0 ? (
          <section className="no-print">
            <div className="section-title">Transcrição</div>
            <Transcript segments={kept} marks={marks} />
          </section>
        ) : (
          <p className="muted no-print">A transcrição desta sessão foi apagada; as notas ficaram.</p>
        )}
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
              <button onClick={() => show(s.id)}>
                <strong>
                  {s.candidateName ?? "Candidato sem nome"} · {formatDate(s.startedAt)}
                </strong>
                <span>
                  {s.durationMs === null ? "em andamento" : formatClock(s.durationMs)} ·{" "}
                  {s.kitName ?? "sem kit"} · {s.segments} falas
                  {s.hasNotes ? " · com notas" : ""}
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
