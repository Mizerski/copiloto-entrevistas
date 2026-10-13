import { useEffect, useRef } from "react";
import { formatClock } from "./format";
import { SOURCES, SOURCE_LABEL, type Mark, type Segment, type Source } from "./types";

type Props = {
  segments: Segment[];
  /** Texto parcial de cada fonte, ainda não confirmado. */
  partials?: Record<Source, string>;
  marks?: Mark[];
  /** Rola sozinho pro fim quando chega fala nova. */
  follow?: boolean;
};

type Item =
  | { type: "block"; key: string; source: Source; startMs: number; texts: string[] }
  | { type: "mark"; key: string; mark: Mark };

const MARK_LABEL: Record<Mark["kind"], string> = {
  mark: "★ Momento marcado",
  pause: "⏸ Transcrição pausada",
  resume: "▶ Transcrição retomada",
};

/** Junta falas seguidas da mesma pessoa num bloco e intercala as marcações pelo tempo. */
function toItems(segments: Segment[], marks: Mark[]): Item[] {
  const items: Item[] = [];
  const pending = [...marks].sort((a, b) => a.tMs - b.tMs);
  for (const s of segments) {
    while (pending.length && pending[0].tMs <= s.startMs) {
      const mark = pending.shift()!;
      items.push({ type: "mark", key: `m${mark.id}-${mark.tMs}`, mark });
    }
    const last = items[items.length - 1];
    if (last?.type === "block" && last.source === s.source) {
      last.texts.push(s.text);
    } else {
      items.push({ type: "block", key: `s${s.id}`, source: s.source, startMs: s.startMs, texts: [s.text] });
    }
  }
  for (const mark of pending) items.push({ type: "mark", key: `m${mark.id}-${mark.tMs}`, mark });
  return items;
}

export function Transcript({ segments, partials, marks = [], follow }: Props) {
  const end = useRef<HTMLDivElement>(null);
  const items = toItems(segments, marks);
  // Com o candidato falando, um parcial no microfone quase sempre é eco ou conversa
  // cruzada; esconder evita texto piscando que o filtro de eco vai descartar.
  const visiblePartials = SOURCES.filter((s) => partials?.[s] && !(s === "mic" && partials.sistema));

  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: "end" });
  }, [follow, segments.length, marks.length, partials?.mic, partials?.sistema]);

  if (items.length === 0 && visiblePartials.length === 0) {
    return <p className="empty">A transcrição aparece aqui.</p>;
  }
  return (
    <div className="transcript">
      {items.map((item) =>
        item.type === "mark" ? (
          <div key={item.key} className={`mark-line ${item.mark.kind}`}>
            {MARK_LABEL[item.mark.kind]} · {formatClock(item.mark.tMs)}
            {item.mark.note ? ` · ${item.mark.note}` : ""}
          </div>
        ) : (
          <div key={item.key} className={`block ${item.source}`}>
            <div className="who">
              {SOURCE_LABEL[item.source]} <span>{formatClock(item.startMs)}</span>
            </div>
            <p>{item.texts.join(" ")}</p>
          </div>
        ),
      )}
      {visiblePartials.map((s) => (
        <div key={`partial-${s}`} className={`block partial ${s}`}>
          <div className="who">{SOURCE_LABEL[s]}</div>
          <p>{partials![s]}</p>
        </div>
      ))}
      <div ref={end} />
    </div>
  );
}

/** Insere mantendo a ordem do momento da fala (as fontes chegam fora de ordem). */
export function insertSegment(list: Segment[], segment: Segment): Segment[] {
  const next = [...list, segment];
  next.sort((a, b) => a.startMs - b.startMs || a.id - b.id);
  return next;
}
