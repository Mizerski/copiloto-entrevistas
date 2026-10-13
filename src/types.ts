// Formas dos dados que vêm do Rust (comandos e eventos).

export type Source = "mic" | "sistema";

export type Microphone = { id: string; name: string; default: boolean };
export type AudioApp = { pid: number; name: string; active: boolean };

export type CaptureEvent =
  | { kind: "started"; source: Source; format: string }
  | { kind: "level"; source: Source; tMs: number; rms: number; peak: number }
  | { kind: "error"; source: Source; message: string }
  | { kind: "stopped"; source: Source; seconds: number };

export type SttEvent =
  | { kind: "connected"; source: Source }
  | {
      kind: "transcript";
      source: Source;
      text: string;
      isFinal: boolean;
      speechFinal: boolean;
      startMs: number;
      endMs: number;
      latencyMs: number;
    }
  | { kind: "utteranceEnd"; source: Source }
  | { kind: "reconnecting"; source: Source; attempt: number; message: string }
  | { kind: "error"; source: Source; message: string }
  | { kind: "closed"; source: Source };

export type Segment = {
  id: number;
  source: Source;
  startMs: number;
  endMs: number;
  text: string;
  latencyMs: number;
  echo: boolean;
};

export type SessionEvent =
  | { kind: "segment"; segment: Segment }
  | { kind: "finished"; sessionId: number; durationMs: number; echoes: number };

export type Level = "none" | "partial" | "strong";

export type CoverageItem = { competency: string; level: Level; evidence: string; atMs: number };

export type Card = { id: number; tMs: number; kind: string; text: string; priority: string };

export type Quote = { text: string; atMs: number | null };

export type BalloonSlot = "aprofundar" | "checar" | "avancar";

export type Suggestion = {
  id: number;
  slot: BalloonSlot;
  tMs: number;
  kind: string;
  question: string;
  why: string;
  /** Só em contradição. `atMs` nulo: o trecho é do currículo. */
  earlier: Quote | null;
  now: Quote | null;
  latencyMs: number;
};

export type AnalysisEvent =
  | { kind: "card"; card: Card }
  | { kind: "suggestion"; suggestion: Suggestion }
  | { kind: "suggestionUsed"; slot: BalloonSlot; id: number }
  | { kind: "coverage"; items: CoverageItem[] }
  | { kind: "status"; state: string; message: string | null; latencyMs: number | null };

export type Mark = { id: number; tMs: number; kind: "mark" | "pause" | "resume"; note: string | null };

export type Competency = { name: string; description: string; strong: string; weak: string };
export type Block = { name: string; minutes: number | null; questions: string[] };
export type Kit = {
  title: string;
  job: string;
  competencies: Competency[];
  blocks: Block[];
  forbidden: string[];
  terms: string[];
  warnings: string[];
};
export type KitRow = { id: number; name: string; markdown: string; updatedAt: number };

export type SessionSummary = {
  id: number;
  startedAt: number;
  durationMs: number | null;
  otherSide: string | null;
  interrupted: boolean;
  segments: number;
  kitName: string | null;
  candidateName: string | null;
  hasNotes: boolean;
};

export type SessionDetail = {
  id: number;
  startedAt: number;
  durationMs: number | null;
  otherSide: string | null;
  interrupted: boolean;
  kitName: string | null;
  kitMarkdown: string | null;
  candidateName: string | null;
  candidateResume: string | null;
  candidateCheckpoints: string | null;
  coverageJson: string | null;
  summary: string | null;
  notesJson: string | null;
};

export type UsageTotals = { calls: number; inputTokens: number; outputTokens: number; cachedTokens: number };

export type SessionFull = {
  detail: SessionDetail;
  segments: Segment[];
  cards: Card[];
  marks: Mark[];
  usage: UsageTotals;
};

export type Provider = "anthropic" | "openai";
export type Slot = { provider: Provider; model: string; baseUrl: string; effort: string };
export type LlmSettings = {
  live: Slot;
  notes: Slot;
  anthropicKey: string | null;
  openaiKey: string | null;
};
export type LlmUsage = {
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  latencyMs: number;
  model: string;
};

export const SOURCES: Source[] = ["mic", "sistema"];
export const SOURCE_LABEL: Record<Source, string> = { mic: "Você", sistema: "Candidato" };

export const LEVEL_LABEL: Record<Level, string> = {
  none: "sem evidência",
  partial: "parcial",
  strong: "forte",
};

export const CARD_LABEL: Record<string, string> = {
  aprofundar: "Aprofundar",
  vaga: "Resposta vaga",
  contradicao: "Contradição",
  nao_tocada: "Não explorada",
  roteiro: "Roteiro",
  curriculo: "Currículo",
  indutiva: "Pergunta indutiva",
  positivo: "Sinal forte",
  proibida: "Pergunta proibida",
};
