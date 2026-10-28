// A resposta da sessão ao vivo: uma fala pronta por pergunta do entrevistador, com
// navegação entre as perguntas e a opção de acompanhar sempre a mais nova.

import { memo, useMemo, useState } from "react";
import { Badge, Button, Card, Flex, IconButton, Switch, Text } from "@radix-ui/themes";
import { ChevronLeftIcon, ChevronRightIcon } from "@radix-ui/react-icons";
import { formatClock } from "./format";
import type { Answer, Segment } from "./types";

/** Tamanhos da fonte da resposta, em px; o escolhido fica salvo neste computador. */
const FONT_SIZES = [15, 17, 19, 22, 25];
const FONT_KEY = "copiloto.fonte-resposta";
/** Fração das palavras de um parágrafo que, ouvida no microfone, conta como já falado. */
const SPOKEN_MIN = 0.6;

function loadFontStep() {
  try {
    const saved = localStorage.getItem(FONT_KEY);
    const n = saved === null ? NaN : Number(saved);
    return Number.isInteger(n) && n >= 0 && n < FONT_SIZES.length ? n : 1;
  } catch {
    return 1;
  }
}

function saveFontStep(n: number) {
  try {
    localStorage.setItem(FONT_KEY, String(n));
  } catch {
    // Sem armazenamento, o tamanho só vale até fechar o app.
  }
}

/** Minúsculas, sem acento e sem pontuação, pra comparar o texto com o que foi falado. */
function words(text: string) {
  return text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/\*\*/g, "")
    .split(/[^a-z0-9]+/)
    .filter((w) => w.length > 2);
}

/** Parágrafos separados por linha em branco e **negrito**; um ** sem par (texto ainda
 * chegando) some. Com `spoken`, os parágrafos que você já falou ficam apagados e o
 * próximo a falar fica marcado. */
function Rich({ text, spoken }: { text: string; spoken: Set<string> | null }) {
  const paragraphs = text
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean);
  const said = paragraphs.map((p) => {
    if (!spoken?.size) return false;
    const list = words(p);
    return list.length > 0 && list.filter((w) => spoken.has(w)).length / list.length >= SPOKEN_MIN;
  });
  const next = spoken?.size ? said.findIndex((s, i) => !s && said.slice(i + 1).every((x) => !x)) : -1;
  return (
    <>
      {paragraphs.map((p, i) => (
        <p key={i} className={said[i] ? "spoken" : i === next ? "next" : undefined}>
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
  /** A pergunta que o entrevistador está fazendo agora, antes de você responder. */
  question: { turn: number; question: string } | null;
  /** O entrevistador já falou, mas ainda não chegou resposta. */
  waiting: boolean;
  /** Suas falas, pra marcar o que da resposta você já falou. */
  segments: Segment[];
  micPartial: string;
};

export const AnswerFeed = memo(function AnswerFeed({ answers, question, waiting, segments, micPartial }: Props) {
  const [follow, setFollow] = useState(true);
  const [index, setIndex] = useState(0);
  const [fontStep, setFontStep] = useState(loadFontStep);

  const last = answers.length - 1;
  const current = follow ? last : Math.min(index, last);
  const answer = answers[current];
  const behind = !follow && current < last;
  // Pergunta nova e a resposta dela ainda não chegou: a da pergunta anterior sai da frente.
  const pending = follow && question !== null && (!answers[last] || question.turn > answers[last].turn);
  const questionText = question && answer && question.turn === answer.turn ? question.question : answer?.question;

  // O que você falou desde que esta resposta apareceu, até a próxima pergunta.
  const spoken = useMemo(() => {
    if (!answer || pending) return null;
    const until = answers[current + 1]?.tMs ?? Infinity;
    const mine = segments.filter((s) => s.source === "mic" && s.startMs >= answer.tMs && s.startMs < until);
    const text = mine.map((s) => s.text).join(" ") + (current === last ? ` ${micPartial}` : "");
    return new Set(words(text));
  }, [answer, answers, current, last, pending, segments, micPartial]);

  function go(to: number) {
    setIndex(Math.max(0, Math.min(last, to)));
    setFollow(false);
  }

  function toggleFollow(value: boolean) {
    setFollow(value);
    if (!value) setIndex(last);
  }

  function resize(delta: number) {
    const next = Math.max(0, Math.min(FONT_SIZES.length - 1, fontStep + delta));
    setFontStep(next);
    saveFontStep(next);
  }

  return (
    <Card size="2" className="answer-card">
      <Flex direction="column" gap="2">
        <Flex align="center" gap="2" wrap="wrap">
          <Text size="2" weight="medium" color="orange">
            {pending ? "Nova pergunta" : "Responda"}
          </Text>
          {answer && !pending && (
            <Text size="1" color="gray" className="tabular">
              {formatClock(answer.tMs)}
            </Text>
          )}
          <Flex align="center" gap="1" ml="auto">
            <IconButton size="1" variant="soft" color="gray" aria-label="Diminuir a letra" disabled={fontStep <= 0} onClick={() => resize(-1)}>
              <span className="font-step small">A</span>
            </IconButton>
            <IconButton size="1" variant="soft" color="gray" aria-label="Aumentar a letra" disabled={fontStep >= FONT_SIZES.length - 1} onClick={() => resize(1)}>
              <span className="font-step">A</span>
            </IconButton>
          </Flex>
          <Flex align="center" gap="1">
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

        {pending ? (
          <>
            <Text as="p" size="2" className="answer-question live-question">
              {tail(question.question)}
            </Text>
            <Text size="2" color="gray" className="preparing">
              preparando a resposta…
            </Text>
          </>
        ) : answer ? (
          <>
            <Text as="p" size="1" color="gray" className="answer-question">
              {tail(questionText ?? "")}
            </Text>
            {/* A chave muda só com a pergunta: o texto que vai chegando não pisca. */}
            <div className="balloon-text" key={answer.turn} style={{ fontSize: FONT_SIZES[fontStep] }}>
              <Rich text={answer.text} spoken={current === last ? spoken : null} />
              {!answer.done && <span className="typing" aria-label="chegando" />}
            </div>
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
});
