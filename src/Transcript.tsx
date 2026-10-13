import { useEffect, useRef } from "react";
import { formatClock } from "./format";
import { SOURCES, SOURCE_LABEL, type Segment, type Source } from "./types";

type Props = {
  segments: Segment[];
  /** Texto parcial de cada fonte, ainda não confirmado. */
  partials?: Record<Source, string>;
  /** Rola sozinho pro fim quando chega fala nova. */
  follow?: boolean;
};

type Block = { key: number; source: Source; startMs: number; texts: string[] };

/** Junta falas seguidas da mesma pessoa num bloco só. */
function toBlocks(segments: Segment[]): Block[] {
  const blocks: Block[] = [];
  for (const s of segments) {
    const last = blocks[blocks.length - 1];
    if (last && last.source === s.source) {
      last.texts.push(s.text);
    } else {
      blocks.push({ key: s.id, source: s.source, startMs: s.startMs, texts: [s.text] });
    }
  }
  return blocks;
}

export function Transcript({ segments, partials, follow }: Props) {
  const end = useRef<HTMLDivElement>(null);
  const blocks = toBlocks(segments);
  // Com o candidato falando, um parcial no microfone quase sempre é eco ou conversa
  // cruzada; esconder evita texto piscando que o filtro de eco vai descartar.
  const visiblePartials = SOURCES.filter(
    (s) => partials?.[s] && !(s === "mic" && partials.sistema),
  );

  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: "end" });
  }, [follow, segments.length, partials?.mic, partials?.sistema]);

  if (blocks.length === 0 && visiblePartials.length === 0) {
    return <p className="empty">A transcrição aparece aqui.</p>;
  }
  return (
    <div className="transcript">
      {blocks.map((b) => (
        <div key={b.key} className={`block ${b.source}`}>
          <div className="who">
            {SOURCE_LABEL[b.source]} <span>{formatClock(b.startMs)}</span>
          </div>
          <p>{b.texts.join(" ")}</p>
        </div>
      ))}
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
