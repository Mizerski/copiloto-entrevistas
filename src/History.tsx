import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { Badge, Button, Card, Flex, Heading, Text } from "@radix-ui/themes";
import { ArrowLeftIcon, TrashIcon } from "@radix-ui/react-icons";
import { formatClock, formatDate } from "./format";
import { Notes } from "./Notes";
import { Transcript } from "./Transcript";
import { Section } from "./ui";
import type { SessionFull, SessionSummary } from "./types";

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
    const { detail, segments, marks, usage } = open;
    const kept = segments.filter((s) => !s.echo);
    const echoes = segments.length - kept.length;
    const facts = [
      formatClock(detail.durationMs ?? 0),
      detail.otherSide ?? "só microfone",
      `${kept.length} falas`,
      echoes > 0 ? `${echoes} ecos descartados` : null,
      usage.calls > 0 ? `${usage.calls} chamadas de LLM` : null,
    ].filter(Boolean);
    return (
      <Flex direction="column" gap="3">
        <Flex align="center" gap="2" className="no-print">
          <Button
            variant="ghost"
            color="gray"
            onClick={() => {
              setOpen(null);
              load();
            }}
          >
            <ArrowLeftIcon /> Voltar
          </Button>
          <span className="grow" />
          <Button variant="ghost" color="red" onClick={() => remove(detail.id, detail.startedAt)}>
            <TrashIcon /> Apagar sessão
          </Button>
        </Flex>
        <Flex direction="column" gap="1">
          <Heading size="4" weight="medium">
            {detail.candidateName ?? "Sem perfil"}
          </Heading>
          <Text size="2" color="gray">
            {formatDate(detail.startedAt)}
            {detail.interrupted ? " · interrompida" : ""}
          </Text>
          <Text size="1" color="gray">
            {facts.join(" · ")}
          </Text>
        </Flex>

        <Notes session={open} onChanged={() => show(detail.id)} />

        <div className="no-print">
          {segments.length > 0 ? (
            <Section title="Transcrição">
              <Transcript segments={kept} marks={marks} />
            </Section>
          ) : (
            <Text size="2" color="gray">
              A transcrição desta sessão foi apagada; as notas ficaram.
            </Text>
          )}
        </div>
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="2">
      {error && (
        <Text size="2" color="red">
          {error}
        </Text>
      )}
      {sessions.length === 0 ? (
        <Text size="2" color="gray">
          Nenhuma sessão gravada ainda.
        </Text>
      ) : (
        sessions.map((s) => (
          <Card key={s.id} asChild size="2">
            <button className="session-item" onClick={() => show(s.id)}>
              <Flex justify="between" align="center" gap="2">
                <Flex direction="column" gap="1" style={{ minWidth: 0 }}>
                  <Text size="2" weight="medium" className="truncate">
                    {s.candidateName ?? "Sem perfil"}
                  </Text>
                  <Text size="1" color="gray" className="truncate">
                    {formatDate(s.startedAt)} ·{" "}
                    {s.durationMs === null ? "em andamento" : formatClock(s.durationMs)} ·{" "}
                    {s.segments} falas
                  </Text>
                </Flex>
                <Flex gap="1" flexShrink="0">
                  {s.hasNotes && (
                    <Badge variant="soft" color="green">
                      parecer
                    </Badge>
                  )}
                  {s.interrupted && (
                    <Badge variant="soft" color="amber">
                      interrompida
                    </Badge>
                  )}
                </Flex>
              </Flex>
            </button>
          </Card>
        ))
      )}
    </Flex>
  );
}
