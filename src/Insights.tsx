// A resposta da sessão ao vivo: uma fala pronta por pergunta do entrevistador, com
// navegação entre as perguntas e a opção de acompanhar sempre a mais nova.

import { useState } from "react";
import { Badge, Button, Card, Flex, IconButton, Switch, Text } from "@radix-ui/themes";
import { ChevronLeftIcon, ChevronRightIcon } from "@radix-ui/react-icons";
import { formatClock } from "./format";
import type { Answer } from "./types";

/** Parágrafos separados por linha em branco e **negrito**; um ** sem par (texto ainda chegando) some. */
function Rich({ text }: { text: string }) {
  return (
    <>
      {text
        .split(/\n\s*\n/)
        .map((p) => p.trim())
        .filter(Boolean)
        .map((p, i) => (
          <p key={i}>
            {p.split(/\*\*(.+?)\*\*/g).map((part, j) =>
              j % 2 ? <strong key={j}>{part}</strong> : part.replace(/\*\*/g, ""),
            )}
          </p>
        ))}
    </>
  );
}

/** As últimas palavras da pergunta, pra lembrar do que se trata. */
function tail(text: string, words = 28) {
  const all = text.split(/\s+/).filter(Boolean);
  return all.length > words ? `…${all.slice(-words).join(" ")}` : all.join(" ");
}

type Props = {
  /** Uma por pergunta, na ordem da entrevista. */
  answers: Answer[];
  /** O entrevistador já falou, mas ainda não chegou resposta. */
  waiting: boolean;
};

export function AnswerFeed({ answers, waiting }: Props) {
  const [follow, setFollow] = useState(true);
  const [index, setIndex] = useState(0);

  const last = answers.length - 1;
  const current = follow ? last : Math.min(index, last);
  const answer = answers[current];
  const behind = !follow && current < last;

  function go(to: number) {
    setIndex(Math.max(0, Math.min(last, to)));
    setFollow(false);
  }

  function toggleFollow(value: boolean) {
    setFollow(value);
    if (!value) setIndex(last);
  }

  return (
    <Card size="2" className="answer-card">
      <Flex direction="column" gap="2">
        <Flex align="center" gap="2">
          <Text size="2" weight="medium" color="orange">
            Responda
          </Text>
          {answer && (
            <Text size="1" color="gray" className="tabular">
              {formatClock(answer.tMs)}
            </Text>
          )}
          <Flex align="center" gap="1" ml="auto">
            <IconButton size="1" variant="soft" color="gray" aria-label="Resposta anterior" disabled={current <= 0} onClick={() => go(current - 1)}>
              <ChevronLeftIcon />
            </IconButton>
            <Text size="1" color="gray" className="tabular answer-count">
              {answers.length ? `${current + 1} de ${answers.length}` : "0 de 0"}
            </Text>
            <IconButton size="1" variant="soft" color="gray" aria-label="Próxima resposta" disabled={current >= last} onClick={() => go(current + 1)}>
              <ChevronRightIcon />
            </IconButton>
          </Flex>
          <Text as="label" size="1" color="gray">
            <Flex gap="1" align="center">
              <Switch size="1" checked={follow} onCheckedChange={toggleFollow} />
              Acompanhar
            </Flex>
          </Text>
        </Flex>

        {answer ? (
          <>
            <Text as="p" size="1" color="gray" className="answer-question">
              {tail(answer.question)}
            </Text>
            <Text as="div" size="3" className="balloon-text" key={`${answer.turn}-${answer.text.length}`}>
              <Rich text={answer.text} />
            </Text>
          </>
        ) : (
          <Text size="2" color="gray">
            {waiting ? "ouvindo…" : "A resposta aparece quando o entrevistador perguntar."}
          </Text>
        )}

        {behind && (
          <Flex align="center" gap="2">
            <Badge color="orange" variant="soft">
              {last - current === 1 ? "1 pergunta nova" : `${last - current} perguntas novas`}
            </Badge>
            <Button size="1" variant="soft" onClick={() => toggleFollow(true)}>
              Ir pra atual
            </Button>
          </Flex>
        )}
      </Flex>
    </Card>
  );
}
