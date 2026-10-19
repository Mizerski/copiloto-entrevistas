import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, save } from "@tauri-apps/plugin-dialog";
import { Badge, Button, Callout, Flex, Heading, Separator, Text, TextArea } from "@radix-ui/themes";
import { DownloadIcon, FileTextIcon, InfoCircledIcon, ReloadIcon, TrashIcon } from "@radix-ui/react-icons";
import { formatDate } from "./format";
import { Field, Picker, Section } from "./ui";
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
const SCORE_OPTIONS = SCORE_LABEL.map((label, i) => ({ value: String(i), label }));
const STATUS: Record<string, { label: string; color: "green" | "red" | "gray" }> = {
  confirmado: { label: "Confirmado", color: "green" },
  contradito: { label: "Contradito", color: "red" },
  nao_verificado: { label: "Não verificado", color: "gray" },
};

function EvidenceList({ title, items, color }: { title: string; items: Evidence[]; color: "green" | "red" }) {
  if (items.length === 0) return null;
  return (
    <Flex direction="column" gap="2">
      <Text size="2" weight="bold" color={color}>
        {title}
      </Text>
      <ul className="evidence">
        {items.map((e, i) => (
          <li key={i}>
            <Text as="p" size="3">
              {e.text}
            </Text>
            <Text as="p" size="2" color="gray" className="quote">
              “{e.quote}”{" "}
              <Text size="1" className="tabular minute">
                {e.minute}
              </Text>
            </Text>
          </li>
        ))}
      </ul>
    </Flex>
  );
}

function Heading2({ children }: { children: React.ReactNode }) {
  return (
    <Heading as="h3" size="4" weight="bold" mt="2">
      {children}
    </Heading>
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
    n.resumePoints.forEach((p) => lines.push(`- ${STATUS[p.status]?.label ?? p.status}: ${p.point}. ${p.comment}`));
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
  const [notes, setNotes] = useState<NotesData | null>(null);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
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
    setBusy(true);
    setMessage(null);
    try {
      setNotes(await invoke<NotesData>("generate_notes", { id }));
      setDirty(false);
      onChanged();
    } catch (e) {
      setMessage(`Não gerou: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  function editCompetency(i: number, patch: Partial<CompetencyNotes>) {
    setNotes((n) =>
      n ? { ...n, competencies: n.competencies.map((c, j) => (j === i ? { ...c, ...patch } : c)) } : n,
    );
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
    const ok = await ask("Apagar a transcrição desta sessão? As notas ficam; os trechos citados nelas continuam.", {
      kind: "warning",
      okLabel: "Apagar transcrição",
    });
    if (!ok) return;
    await invoke("delete_transcript", { id });
    onChanged();
  }

  if (!notes) {
    return (
      <Section
        title="Parecer"
        description="Evidências por competência com trecho e minuto, pontos do currículo e perguntas pra próxima etapa. Usa o modelo de notas das Configurações."
      >
        <Flex align="center" gap="3">
          <Button disabled={!hasTranscript} loading={busy} onClick={generate}>
            <FileTextIcon /> Gerar parecer
          </Button>
          {busy && (
            <Text size="1" color="gray">
              Numa entrevista longa leva um ou dois minutos.
            </Text>
          )}
        </Flex>
        {message && (
          <Text size="2" color="red">
            {message}
          </Text>
        )}
      </Section>
    );
  }

  return (
    <Section
      title="Parecer"
      actions={
        <Badge variant="soft" color={notes.reviewed ? "green" : "gray"}>
          {notes.reviewed ? "revisado" : "rascunho"}
        </Badge>
      }
    >
      <Text size="1" color="gray">
        Gerado por {notes.model} em {formatDate(notes.generatedAt)}. As notas são sugestões; a avaliação é sua.
      </Text>
      {notes.droppedQuotes > 0 && (
        <Callout.Root size="1" color="gray">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text>
            {notes.droppedQuotes} evidência(s) descartada(s) porque o trecho citado não estava na transcrição.
          </Callout.Text>
        </Callout.Root>
      )}
      {notes.summary && (
        <>
          <Heading2>Resumo</Heading2>
          <Text as="p" size="3" className="reading">
            {notes.summary}
          </Text>
        </>
      )}

      {notes.competencies.map((c, i) => (
        <Flex key={c.name} direction="column" gap="2">
          <Separator size="4" />
          <Flex align="center" justify="between" gap="2">
            <Heading2>{c.name}</Heading2>
            <div className="no-print score-picker">
              <Picker
                value={String(c.score)}
                options={SCORE_OPTIONS}
                onChange={(v) => editCompetency(i, { score: Number(v) })}
              />
            </div>
            <Text size="2" className="print-only">
              {SCORE_LABEL[c.score]}
            </Text>
          </Flex>
          {c.score !== c.suggestedScore && (
            <Text size="1" color="gray">
              Sugestão do modelo: {SCORE_LABEL[c.suggestedScore]}
            </Text>
          )}
          <EvidenceList title="A favor" items={c.forEvidence} color="green" />
          <EvidenceList title="Contra" items={c.against} color="red" />
          {c.missing && (
            <Text as="p" size="2" color="gray">
              <Text weight="bold">Sem evidência:</Text> {c.missing}
            </Text>
          )}
          <TextArea
            className="no-print"
            size="1"
            placeholder="Seu comentário"
            value={c.comment}
            onChange={(e) => editCompetency(i, { comment: e.target.value })}
          />
          {c.comment && (
            <Text size="2" className="print-only">
              Comentário: {c.comment}
            </Text>
          )}
        </Flex>
      ))}

      {notes.resumePoints.length > 0 && (
        <Flex direction="column" gap="2">
          <Separator size="4" />
          <Heading2>Currículo e pontos a checar</Heading2>
          {notes.resumePoints.map((p, i) => (
            <Flex key={i} gap="2" align="start">
              <Badge variant="soft" color={STATUS[p.status]?.color ?? "gray"} style={{ flexShrink: 0 }}>
                {STATUS[p.status]?.label ?? p.status}
              </Badge>
              <Text size="3">
                {p.point}.{" "}
                <Text color="gray" size="3">
                  {p.comment}
                </Text>
              </Text>
            </Flex>
          ))}
        </Flex>
      )}

      {notes.nextQuestions.length > 0 && (
        <Flex direction="column" gap="2">
          <Separator size="4" />
          <Heading2>Perguntas pra próxima etapa</Heading2>
          <ul className="plain reading">
            {notes.nextQuestions.map((q, i) => (
              <li key={i}>
                <Text size="3">{q}</Text>
              </li>
            ))}
          </ul>
        </Flex>
      )}

      <Separator size="4" />
      <Field label="Suas anotações">
        <TextArea
          className="no-print"
          value={notes.interviewerNotes}
          onChange={(e) => {
            setNotes({ ...notes, interviewerNotes: e.target.value });
            setDirty(true);
          }}
        />
        {notes.interviewerNotes && (
          <Text size="2" className="print-only">
            {notes.interviewerNotes}
          </Text>
        )}
      </Field>

      <Flex gap="2" wrap="wrap" align="center" className="no-print">
        <Button disabled={!dirty && notes.reviewed} onClick={saveReview}>
          Salvar revisão
        </Button>
        <Button variant="soft" color="gray" onClick={exportMarkdown}>
          <DownloadIcon /> .md
        </Button>
        <Button variant="soft" color="gray" onClick={() => window.print()}>
          Imprimir / PDF
        </Button>
        <span className="grow" />
        <Button variant="ghost" color="gray" disabled={!hasTranscript} loading={busy} onClick={generate}>
          <ReloadIcon /> Gerar de novo
        </Button>
        {hasTranscript && (
          <Button variant="ghost" color="red" onClick={deleteTranscript}>
            <TrashIcon /> Apagar transcrição
          </Button>
        )}
      </Flex>
      {message && (
        <Text size="1" color="gray" className="no-print">
          {message}
        </Text>
      )}
    </Section>
  );
}
