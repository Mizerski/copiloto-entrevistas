import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { formatDate } from "./format";
import type { SessionFull } from "./types";

type Evidence = { text: string; quote: string; minute: string };
type CompetencyNotes = {
  name: string;
  suggestedScore: number;
  score: number;
  forEvidence: Evidence[];
  against: Evidence[];
  missing: string;
  comment: string;
};
type ResumePoint = { point: string; status: string; comment: string };
export type NotesData = {
  generatedAt: number;
  model: string;
  competencies: CompetencyNotes[];
  resumePoints: ResumePoint[];
  nextQuestions: string[];
  summary: string;
  interviewerNotes: string;
  reviewed: boolean;
  droppedQuotes: number;
};

const SCORE_LABEL = ["Sem evidência suficiente", "1 · Sinal fraco", "2 · Abaixo do esperado", "3 · Adequado", "4 · Sinal forte"];
const STATUS_LABEL: Record<string, string> = {
  confirmado: "Confirmado",
  contradito: "Contradito",
  nao_verificado: "Não verificado",
};

function EvidenceList({ title, items }: { title: string; items: Evidence[] }) {
  if (items.length === 0) return null;
  return (
    <div className="evidence">
      <span className="evidence-title">{title}</span>
      <ul>
        {items.map((e, i) => (
          <li key={i}>
            {e.text} <q>{e.quote}</q> <span className="muted">{e.minute}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function notesToMarkdown(session: SessionFull, n: NotesData) {
  const d = session.detail;
  const lines = [
    `# Parecer · ${d.candidateName ?? "Candidato"}`,
    "",
    `Entrevista de ${formatDate(d.startedAt)}${d.kitName ? ` · ${d.kitName}` : ""}`,
    "",
    "## Resumo",
    "",
    n.summary,
    "",
    "## Competências",
    "",
  ];
  for (const c of n.competencies) {
    lines.push(`### ${c.name}: ${SCORE_LABEL[c.score]}`, "");
    if (c.score !== c.suggestedScore) lines.push(`Sugestão do modelo: ${SCORE_LABEL[c.suggestedScore]}`, "");
    for (const [label, list] of [
      ["A favor", c.forEvidence],
      ["Contra", c.against],
    ] as const) {
      if (list.length) {
        lines.push(`${label}:`, "");
        list.forEach((e) => lines.push(`- ${e.text} "${e.quote}" (${e.minute})`));
        lines.push("");
      }
    }
    if (c.missing) lines.push(`Sem evidência: ${c.missing}`, "");
    if (c.comment) lines.push(`Comentário: ${c.comment}`, "");
  }
  if (n.resumePoints.length) {
    lines.push("## Currículo e pontos a checar", "");
    n.resumePoints.forEach((p) => lines.push(`- ${STATUS_LABEL[p.status] ?? p.status}: ${p.point}. ${p.comment}`));
    lines.push("");
  }
  if (n.nextQuestions.length) {
    lines.push("## Perguntas pra próxima etapa", "");
    n.nextQuestions.forEach((q) => lines.push(`- ${q}`));
    lines.push("");
  }
  if (n.interviewerNotes) lines.push("## Anotações de quem entrevistou", "", n.interviewerNotes, "");
  lines.push(
    "---",
    "",
    "Notas geradas com ajuda de IA a partir da transcrição automática e revisadas por quem entrevistou. As notas por competência são sugestões; não há recomendação de contratação.",
  );
  return lines.join("\n");
}

export function Notes({ session, onChanged }: { session: SessionFull; onChanged: () => void }) {
  const stored: NotesData | null = session.detail.notesJson ? JSON.parse(session.detail.notesJson) : null;
  const [notes, setNotes] = useState<NotesData | null>(stored);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    setNotes(session.detail.notesJson ? JSON.parse(session.detail.notesJson) : null);
    setDirty(false);
  }, [session.detail.id, session.detail.notesJson]);

  const id = session.detail.id;
  const hasTranscript = session.segments.some((s) => !s.echo);

  async function generate() {
    if (notes && !(await ask("Gerar de novo? As notas e a revisão atuais serão substituídas.", { kind: "warning" })))
      return;
    setBusy("Gerando notas… numa entrevista longa pode levar um ou dois minutos.");
    setMessage(null);
    try {
      setNotes(await invoke<NotesData>("generate_notes", { id }));
      setDirty(false);
      onChanged();
    } catch (e) {
      setMessage(`Não gerou: ${e}`);
    } finally {
      setBusy(null);
    }
  }

  function edit(patch: (n: NotesData) => NotesData) {
    setNotes((n) => (n ? patch(n) : n));
    setDirty(true);
  }

  async function saveReview() {
    if (!notes) return;
    const reviewed = { ...notes, reviewed: true };
    await invoke("save_notes", { id, notes: reviewed });
    setNotes(reviewed);
    setDirty(false);
    setMessage("Revisão salva.");
    onChanged();
  }

  async function exportMarkdown() {
    if (!notes) return;
    const name = `parecer-${(session.detail.candidateName ?? "candidato").replace(/[\\/:*?"<>|\s]+/g, "-")}.md`;
    const path = await save({ defaultPath: name, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!path) return;
    await invoke("write_text_file", { path, contents: notesToMarkdown(session, notes) });
    setMessage("Parecer exportado.");
  }

  async function deleteTranscript() {
    const ok = await ask(
      "Apagar a transcrição desta sessão? As notas ficam; os trechos citados nelas continuam.",
      { kind: "warning", okLabel: "Apagar transcrição" },
    );
    if (!ok) return;
    await invoke("delete_transcript", { id });
    onChanged();
  }

  if (!notes) {
    return (
      <section className="notes">
        <div className="section-title">Notas pós-entrevista</div>
        <p className="hint">
          Evidências por competência, com trecho e minuto, pontos do currículo e perguntas pra próxima etapa. Usa o
          modelo de notas das Configurações; o texto da transcrição vai pra esse provedor.
        </p>
        <button className="primary" disabled={!hasTranscript || busy !== null} onClick={generate}>
          Gerar notas
        </button>
        {busy && <p className="muted">{busy}</p>}
        {message && <p className="error">{message}</p>}
      </section>
    );
  }

  return (
    <section className="notes">
      <div className="notes-head">
        <div className="section-title">Parecer</div>
        <span className={notes.reviewed ? "badge ok" : "badge"}>{notes.reviewed ? "revisado" : "rascunho"}</span>
      </div>
      <p className="hint">
        Gerado por {notes.model} em {formatDate(notes.generatedAt)}. As notas são sugestões; a avaliação é sua.
        {notes.droppedQuotes > 0 &&
          ` ${notes.droppedQuotes} evidência(s) descartada(s) porque o trecho não estava na transcrição.`}
      </p>
      {notes.summary && <p>{notes.summary}</p>}

      {notes.competencies.map((c, i) => (
        <div key={c.name} className="competency">
          <div className="competency-head">
            <strong>{c.name}</strong>
            <select
              className="no-print"
              value={c.score}
              onChange={(e) =>
                edit((n) => ({
                  ...n,
                  competencies: n.competencies.map((x, j) => (j === i ? { ...x, score: Number(e.target.value) } : x)),
                }))
              }
            >
              {SCORE_LABEL.map((label, score) => (
                <option key={score} value={score}>
                  {label}
                </option>
              ))}
            </select>
            <span className="print-only">{SCORE_LABEL[c.score]}</span>
          </div>
          {c.score !== c.suggestedScore && (
            <span className="muted small">Sugestão do modelo: {SCORE_LABEL[c.suggestedScore]}</span>
          )}
          <EvidenceList title="A favor" items={c.forEvidence} />
          <EvidenceList title="Contra" items={c.against} />
          {c.missing && <p className="muted small">Sem evidência: {c.missing}</p>}
          <textarea
            className="no-print"
            rows={2}
            placeholder="Seu comentário"
            value={c.comment}
            onChange={(e) =>
              edit((n) => ({
                ...n,
                competencies: n.competencies.map((x, j) => (j === i ? { ...x, comment: e.target.value } : x)),
              }))
            }
          />
          {c.comment && <p className="print-only">Comentário: {c.comment}</p>}
        </div>
      ))}

      {notes.resumePoints.length > 0 && (
        <div>
          <div className="section-title">Currículo e pontos a checar</div>
          <ul className="plain">
            {notes.resumePoints.map((p, i) => (
              <li key={i}>
                <span className={`badge ${p.status}`}>{STATUS_LABEL[p.status] ?? p.status}</span> {p.point}.{" "}
                <span className="muted">{p.comment}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {notes.nextQuestions.length > 0 && (
        <div>
          <div className="section-title">Perguntas pra próxima etapa</div>
          <ul className="plain">
            {notes.nextQuestions.map((q, i) => (
              <li key={i}>{q}</li>
            ))}
          </ul>
        </div>
      )}

      <label>
        Suas anotações
        <textarea
          className="no-print"
          rows={3}
          value={notes.interviewerNotes}
          onChange={(e) => edit((n) => ({ ...n, interviewerNotes: e.target.value }))}
        />
      </label>
      {notes.interviewerNotes && <p className="print-only">{notes.interviewerNotes}</p>}

      <div className="row wrap no-print">
        <button className="primary" disabled={!dirty && notes.reviewed} onClick={saveReview}>
          Salvar revisão
        </button>
        <button onClick={exportMarkdown}>Exportar .md</button>
        <button onClick={() => window.print()}>Imprimir / PDF</button>
        <span className="grow" />
        <button disabled={!hasTranscript || busy !== null} onClick={generate}>
          Gerar de novo
        </button>
        {hasTranscript && (
          <button className="danger" onClick={deleteTranscript}>
            Apagar transcrição
          </button>
        )}
      </div>
      {busy && <p className="muted no-print">{busy}</p>}
      {message && <p className="muted no-print">{message}</p>}
    </section>
  );
}
