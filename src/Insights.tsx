// Peças da sessão ao vivo: cobertura das competências, cards e relógio por bloco.

import { Badge, Box, Card, Flex, IconButton, Text, Tooltip } from "@radix-ui/themes";
import { ChevronLeftIcon, ChevronRightIcon, Cross2Icon } from "@radix-ui/react-icons";
import { formatClock } from "./format";
import {
  CARD_LABEL,
  LEVEL_LABEL,
  type Block,
  type Card as LiveCard,
  type CoverageItem,
  type BalloonSlot,
  type Suggestion,
} from "./types";

const LEVEL_COLOR = { none: "gray", partial: "orange", strong: "green" } as const;

export function CoveragePanel({ items }: { items: CoverageItem[] }) {
  if (items.length === 0) return null;
  return (
    <Flex direction="column" gap="1">
      {items.map((c) => (
        <Flex key={c.competency} align="center" gap="2" className={`coverage-row ${c.level}`}>
          <span className="coverage-dot" />
          <Tooltip content={c.evidence ? `“${c.evidence}” · ${formatClock(c.atMs)}` : "Ainda sem evidência"}>
            <Text size="2" className="grow truncate">
              {c.competency}
            </Text>
          </Tooltip>
          <Badge size="1" variant="soft" color={LEVEL_COLOR[c.level]}>
            {LEVEL_LABEL[c.level]}
          </Badge>
        </Flex>
      ))}
    </Flex>
  );
}

const CARD_COLOR: Record<string, "red" | "green" | "gray" | "orange"> = {
  proibida: "red",
  contradicao: "red",
  positivo: "green",
};

export function CardList({ cards, onDismiss }: { cards: LiveCard[]; onDismiss?: (id: number) => void }) {
  if (cards.length === 0) return null;
  return (
    <Flex direction="column" gap="2">
      {cards.map((c) => {
        const color = CARD_COLOR[c.kind] ?? (c.priority === "baixa" ? "gray" : "orange");
        return (
          <Card key={`${c.id}-${c.tMs}`} size="1" className={`hint-card ${color}`}>
            <Flex align="start" gap="2">
              <Box className="grow">
                <Flex align="center" gap="2" mb="1">
                  <Badge size="1" variant="soft" color={color}>
                    {CARD_LABEL[c.kind] ?? c.kind}
                  </Badge>
                  <Text size="1" color="gray">
                    {formatClock(c.tMs)}
                  </Text>
                </Flex>
                <Text as="p" size="2">
                  {c.text}
                </Text>
              </Box>
              {onDismiss && (
                <IconButton size="1" variant="ghost" color="gray" aria-label="Dispensar" onClick={() => onDismiss(c.id)}>
                  <Cross2Icon />
                </IconButton>
              )}
            </Flex>
          </Card>
        );
      })}
    </Flex>
  );
}

type SlotConfig = { slot: BalloonSlot; label: string; color: "orange" | "red" | "blue" };

/** Entrevista real: perguntas pra você fazer. */
export const INTERVIEW_SLOTS: SlotConfig[] = [
  { slot: "aprofundar", label: "Aprofundar", color: "orange" },
  { slot: "checar", label: "Checar", color: "red" },
  { slot: "avancar", label: "Avançar", color: "blue" },
];

/** Modo candidato: o que responder ao entrevistador, tirado do seu perfil. */
export const CANDIDATE_SLOTS: SlotConfig[] = [
  { slot: "responda", label: "Responda", color: "orange" },
  { slot: "historia", label: "Se pedirem mais", color: "blue" },
  { slot: "atencao", label: "Atenção", color: "red" },
];

/** Treino: dicas pra você, que está respondendo. */
export const PRACTICE_SLOTS: SlotConfig[] = [
  { slot: "faltou", label: "Faltou dizer", color: "orange" },
  { slot: "cuidado", label: "Cuidado", color: "red" },
  { slot: "estrutura", label: "Estrutura", color: "blue" },
];

type BalloonsProps = {
  suggestions: Partial<Record<BalloonSlot, Suggestion>>;
  waiting: boolean;
  onUse: (s: Suggestion) => void;
  slots?: SlotConfig[];
};

/** Três balões fixos, cada um com uma função. O que você perguntar (ou clicar) some. */
export function Balloons({ suggestions, waiting, onUse, slots = INTERVIEW_SLOTS }: BalloonsProps) {
  const practice = slots === PRACTICE_SLOTS;
  const candidate = slots === CANDIDATE_SLOTS;
  const emptyText = candidate
    ? "aparece quando o entrevistador perguntar"
    : practice
      ? "aparece enquanto você responde"
      : "aparece quando o candidato responder";
  return (
    <Flex direction="column" gap="2">
      {slots.map(({ slot, label, color }) => {
        const s = suggestions[slot];
        // O balão vermelho só fica vermelho quando é contradição; o resto é âmbar.
        const tone = color === "red" && s?.kind !== "contradicao" ? "amber" : color;
        if (!s) {
          return (
            <Card key={slot} size="1" className="hint-card gray balloon empty">
              <Text size="1" weight="medium" color="gray">
                {label}
              </Text>
              <Text size="1" color="gray" ml="2">
                {waiting ? "ouvindo…" : emptyText}
              </Text>
            </Card>
          );
        }
        return (
          <Card
            key={slot}
            size="1"
            className={`hint-card ${tone} balloon`}
            role="button"
            tabIndex={0}
            title={practice || candidate ? "Dispensar" : "Marcar como perguntada"}
            onClick={() => onUse(s)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onUse(s)}
          >
            <Flex align="center" gap="2" mb="1">
              <Text size="1" weight="medium" color={tone}>
                {label}
              </Text>
              {s.kind !== slot && (
                <Text size="1" color="gray">
                  {CARD_LABEL[s.kind] ?? s.kind}
                </Text>
              )}
              <Text size="1" color="gray" className="tabular" ml="auto">
                {formatClock(s.tMs)}
              </Text>
            </Flex>
            {/* No modo candidato, a continuação é texto pra ler em voz alta, sem peso. */}
            <Text
              as="p"
              size="3"
              weight={s.slot === "historia" ? "regular" : "medium"}
              key={s.id}
              className="balloon-text"
            >
              {s.question}
            </Text>
            {s.earlier && s.now && (
              <Flex direction="column" gap="1" mt="2" className="contradiction">
                <Text size="1" color="gray">
                  {s.earlier.atMs === null ? "No currículo" : `Aos ${formatClock(s.earlier.atMs)}`}:{" "}
                  <span className="quote">“{s.earlier.text}”</span>
                </Text>
                <Text size="1" color="gray">
                  Agora{s.now.atMs !== null ? ` (${formatClock(s.now.atMs)})` : ""}:{" "}
                  <span className="quote">“{s.now.text}”</span>
                </Text>
              </Flex>
            )}
            {s.why && !s.earlier && (
              <Text as="p" size="1" color="gray" mt="1">
                {s.why}
              </Text>
            )}
          </Card>
        );
      })}
    </Flex>
  );
}

type ClockProps = {
  blocks: Block[];
  current: number;
  blockStartedAt: number;
  now: number;
  onChange: (index: number) => void;
};

export function BlockClock({ blocks, current, blockStartedAt, now, onChange }: ClockProps) {
  if (blocks.length === 0) return null;
  const block = blocks[current];
  const elapsed = now - blockStartedAt;
  const planned = block.minutes ? block.minutes * 60_000 : null;
  const over = planned !== null && elapsed > planned;
  return (
    <Card size="1">
      <Flex align="center" gap="2">
        <IconButton
          size="1"
          variant="soft"
          color="gray"
          aria-label="Bloco anterior"
          disabled={current === 0}
          onClick={() => onChange(current - 1)}
        >
          <ChevronLeftIcon />
        </IconButton>
        <Flex direction="column" className="grow" style={{ minWidth: 0 }}>
          <Flex align="baseline" gap="2">
            <Text size="2" weight="medium" className="truncate">
              {block.name}
            </Text>
            <Text size="1" color={over ? "red" : "gray"} weight={over ? "medium" : "regular"} className="tabular">
              {formatClock(elapsed)}
              {planned !== null ? ` / ${block.minutes} min` : ""}
            </Text>
            <Text size="1" color="gray" className="tabular" ml="auto">
              {current + 1}/{blocks.length}
            </Text>
          </Flex>
          {block.questions[0] && (
            <Text size="1" color="gray" className="truncate">
              {block.questions[0]}
            </Text>
          )}
        </Flex>
        <IconButton
          size="1"
          variant="soft"
          color="gray"
          aria-label="Próximo bloco"
          disabled={current === blocks.length - 1}
          onClick={() => onChange(current + 1)}
        >
          <ChevronRightIcon />
        </IconButton>
      </Flex>
    </Card>
  );
}

/** Os cards que cabem na tela: perguntas proibidas primeiro, depois os mais novos. */
export function visibleCards(cards: LiveCard[], dismissed: Set<number>, focus: boolean, max = 3) {
  const active = cards.filter((c) => !dismissed.has(c.id) && (!focus || c.kind === "proibida"));
  const forbidden = active.filter((c) => c.kind === "proibida");
  const others = active.filter((c) => c.kind !== "proibida");
  return [...forbidden.slice(-max), ...others.slice(-(max - Math.min(forbidden.length, max)))]
    .slice(0, max)
    .sort((a, b) => b.tMs - a.tMs);
}
