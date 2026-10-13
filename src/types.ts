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

export type SessionSummary = {
  id: number;
  startedAt: number;
  durationMs: number | null;
  otherSide: string | null;
  interrupted: boolean;
  segments: number;
};

export const SOURCES: Source[] = ["mic", "sistema"];
export const SOURCE_LABEL: Record<Source, string> = { mic: "Você", sistema: "Candidato" };
