// Peças da sessão ao vivo: cobertura das competências, cards e relógio por bloco.

import { formatClock } from "./format";
import { CARD_LABEL, LEVEL_LABEL, type Block, type Card, type CoverageItem } from "./types";

export function CoveragePanel({ items }: { items: CoverageItem[] }) {
  if (items.length === 0) return null;
  return (
    <ul className="coverage">
      {items.map((c) => (
        <li key={c.competency} className={c.level} title={c.evidence ? `“${c.evidence}” (${formatClock(c.atMs)})` : ""}>
          <span className="dot" />
          <span className="name">{c.competency}</span>
          <span className="level">{LEVEL_LABEL[c.level]}</span>
        </li>
      ))}
    </ul>
  );
}

export function CardList({ cards, onDismiss }: { cards: Card[]; onDismiss?: (id: number) => void }) {
  if (cards.length === 0) return null;
  return (
    <ul className="cards">
      {cards.map((c) => (
        <li key={`${c.id}-${c.tMs}`} className={`card ${c.kind} ${c.priority}`}>
          <div className="card-head">
            <span>{CARD_LABEL[c.kind] ?? c.kind}</span>
            <span className="muted">{formatClock(c.tMs)}</span>
            {onDismiss && (
              <button className="link" title="Dispensar" onClick={() => onDismiss(c.id)}>
                ×
              </button>
            )}
          </div>
          <p>{c.text}</p>
        </li>
      ))}
    </ul>
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
    <div className="block-clock">
      <button disabled={current === 0} onClick={() => onChange(current - 1)} title="Bloco anterior">
        ‹
      </button>
      <div className="block-info">
        <strong>{block.name}</strong>
        <span className={over ? "over" : "muted"}>
          {formatClock(elapsed)}
          {planned !== null ? ` de ${block.minutes} min` : ""}
          {` · ${current + 1}/${blocks.length}`}
        </span>
        {block.questions[0] && <span className="muted next-q">{block.questions[0]}</span>}
      </div>
      <button disabled={current === blocks.length - 1} onClick={() => onChange(current + 1)} title="Próximo bloco">
        ›
      </button>
    </div>
  );
}

/** Os cards que cabem na tela: perguntas proibidas primeiro, depois os mais novos. */
export function visibleCards(cards: Card[], dismissed: Set<number>, focus: boolean, max = 3) {
  const active = cards.filter((c) => !dismissed.has(c.id) && (!focus || c.kind === "proibida"));
  const forbidden = active.filter((c) => c.kind === "proibida");
  const others = active.filter((c) => c.kind !== "proibida");
  return [...forbidden.slice(-max), ...others.slice(-(max - Math.min(forbidden.length, max)))]
    .slice(0, max)
    .sort((a, b) => b.tMs - a.tMs);
}
