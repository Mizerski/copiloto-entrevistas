// Métricas calculadas da transcrição, sem IA.

import type { Segment } from "./types";

const OPEN_STARTERS = [
  "como",
  "por que",
  "porque",
  "o que",
  "qual",
  "quais",
  "me conta",
  "me fala",
  "conta",
  "fala",
  "descreve",
  "explica",
  "de que forma",
  "em que",
];

export type Metrics = {
  /** Tempo de fala em ms. */
  talkMs: { mic: number; sistema: number };
  /** Parte do tempo de fala que foi do entrevistador, 0..1. */
  interviewerShare: number | null;
  questions: number;
  openQuestions: number;
  /** Duração de cada resposta do candidato, em ms. */
  answers: number[];
};

function fold(text: string) {
  return text
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9?\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** Junta falas seguidas da mesma fonte: cada grupo é uma vez de falar. */
function turnsOf(segments: Segment[]) {
  const turns: { source: Segment["source"]; text: string; startMs: number; endMs: number }[] = [];
  for (const s of segments) {
    const last = turns[turns.length - 1];
    if (last && last.source === s.source) {
      last.text += ` ${s.text}`;
      last.endMs = s.endMs;
    } else {
      turns.push({ source: s.source, text: s.text, startMs: s.startMs, endMs: s.endMs });
    }
  }
  return turns;
}

export function computeMetrics(segments: Segment[]): Metrics {
  const kept = segments.filter((s) => !s.echo);
  const talkMs = { mic: 0, sistema: 0 };
  for (const s of kept) talkMs[s.source] += Math.max(0, s.endMs - s.startMs);
  const total = talkMs.mic + talkMs.sistema;

  let questions = 0;
  let openQuestions = 0;
  const answers: number[] = [];
  for (const t of turnsOf(kept)) {
    if (t.source === "sistema") {
      answers.push(t.endMs - t.startMs);
      continue;
    }
    // Cada trecho terminado em "?" conta como pergunta; o resto da fala é contexto.
    const folded = fold(t.text);
    const parts = folded.split("?").slice(0, -1);
    const asked = parts.length > 0 ? parts : OPEN_STARTERS.some((w) => folded.includes(w)) ? [folded] : [];
    for (const q of asked) {
      questions += 1;
      const lead = q.trim().split(" ").slice(-12).join(" ");
      if (OPEN_STARTERS.some((w) => lead.startsWith(w) || lead.includes(` ${w} `))) openQuestions += 1;
    }
  }
  return {
    talkMs,
    interviewerShare: total > 0 ? talkMs.mic / total : null,
    questions,
    openQuestions,
    answers,
  };
}
