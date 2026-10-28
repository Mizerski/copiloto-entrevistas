// Modo demonstração, só em desenvolvimento: quando a página abre num navegador comum
// (sem o Tauri por trás), os comandos e eventos do Rust são simulados. Serve pra ver e
// conferir a interface sem microfone, chamada ou chaves.

import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import exampleProfile from "../src-tauri/perfis/exemplo-backend.md?raw";
import type { KitRow, Segment, SessionFull } from "./types";

const profiles: KitRow[] = [{ id: 1, name: "Ana Souza", markdown: exampleProfile, updatedAt: Date.now() }];

const script: [number, "mic" | "sistema", string][] = [
  [0, "sistema", "Oi Ana, tudo bem? Tá me ouvindo?"],
  [3_000, "mic", "Tô te ouvindo bem, sim. E você, me ouve?"],
  [6_000, "sistema", "Ótimo. Me conta de um problema difícil que você resolveu em pagamentos."],
  [12_000, "mic", "Então, teve um caso no Pix em que o evento de confirmação chegava duas vezes,"],
  [18_000, "mic", "e a gente acabava cobrando duas vezes o cliente."],
];

let timers: number[] = [];
let onboarded = false;
let segmentId = 1;

function seg(startMs: number, source: "mic" | "sistema", text: string): Segment {
  return { id: segmentId++, source, startMs, endMs: startMs + 3_500, text, latencyMs: 900, echo: false };
}

function playDemo() {
  timers.forEach(clearTimeout);
  timers = [];
  const at = (ms: number, fn: () => void) => timers.push(window.setTimeout(fn, ms));
  const levels = window.setInterval(() => {
    emit("captura", { kind: "level", source: "mic", tMs: 0, rms: Math.random() * 0.05, peak: 0 });
    emit("captura", { kind: "level", source: "sistema", tMs: 0, rms: Math.random() * 0.1, peak: 0 });
  }, 100);
  timers.push(levels as unknown as number);
  script.forEach(([t, source, text], i) => {
    at(t / 4 + 400, () => emit("transcricao", { kind: "transcript", source, text: text.split(" ").slice(0, 5).join(" "), isFinal: false, speechFinal: false, startMs: t, endMs: t + 1000, latencyMs: 280 }));
    at(t / 4 + 1200, () => {
      emit("transcricao", { kind: "transcript", source, text, isFinal: true, speechFinal: true, startMs: t, endMs: t + 3500, latencyMs: 950 + i * 40 });
      emit("sessao", { kind: "segment", segment: seg(t, source, text) });
    });
  });
  const question = (turn: number, text: string) => emit("analise", { kind: "question", turn, question: text });
  // A resposta chega aos pedaços, como no streaming de verdade.
  const answer = (startAt: number, turn: number, tMs: number, q: string, text: string) => {
    const words = text.split(" ");
    words.forEach((_, i) =>
      at(startAt + i * 45, () =>
        emit("analise", {
          kind: "answer",
          answer: { turn, tMs, question: q, text: words.slice(0, i + 1).join(" "), latencyMs: 450, done: i === words.length - 1 },
        }),
      ),
    );
  };
  at(500, () => question(0, "Oi Ana, tudo bem? Tá me ouvindo?"));
  answer(1_300, 0, 3_000, "Oi Ana, tudo bem? Tá me ouvindo?", "Tô te ouvindo bem, sim. E você, me ouve?");
  at(1_900, () => question(1, "Ótimo. Me conta de um problema difícil"));
  at(2_900, () => question(1, "Ótimo. Me conta de um problema difícil que você resolveu em pagamentos."));
  answer(
    3_300,
    1,
    9_500,
    "Ótimo. Me conta de um problema difícil que você resolveu em pagamentos.",
    "Teve um caso no **Pix** que me marcou.\n\nO evento de confirmação chegava duas vezes e a gente cobrava o cliente de novo.\n\nEu propus guardar a **chave de idempotência** no Postgres e escrevi o teste que reproduzia.\n\nEm umas duas semanas a duplicidade foi **a zero**.",
  );
}

const fullSession: SessionFull = {
  detail: {
    id: 7,
    startedAt: Date.now() - 86_400_000,
    durationMs: 2_712_000,
    otherSide: "ms-teams.exe",
    interrupted: false,
    kitName: null,
    kitMarkdown: null,
    candidateName: "Ana Souza",
    candidateResume: exampleProfile,
    candidateCheckpoints: null,
    coverageJson: null,
    summary: "Ana contou o caso do pagamento duplicado no Pix.",
    notesJson: null,
    practicePersona: null,
    side: "candidato",
  },
  segments: script.map(([t, s, text]) => seg(t, s, text)),
  cards: [],
  marks: [{ id: 1, tMs: 20_000, kind: "mark", note: null }],
  usage: { calls: 14, inputTokens: 42_000, outputTokens: 3_100, cachedTokens: 30_000 },
};

export function installDevMock() {
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "list_microphones":
          return [{ id: "mic-1", name: "Microfone do Headset (demo)", default: true }];
        case "list_audio_apps":
          return [
            { pid: 4242, name: "ms-teams.exe", active: true },
            { pid: 5151, name: "chrome.exe", active: false },
          ];
        case "deepgram_key_status":
          return "••••d3m0";
        case "llm_settings":
          return {
            live: { provider: "anthropic", model: "claude-opus-5-5", baseUrl: "https://api.openai.com/v1", effort: "low" },
            notes: { provider: "anthropic", model: "claude-opus-5-5", baseUrl: "https://api.openai.com/v1", effort: "high" },
            anthropicKey: null,
            openaiKey: null,
          };
        case "test_llm":
          throw "salve a chave da Anthropic nas configurações";
        case "list_profiles":
          return profiles;
        case "save_profile":
          return a.id ?? 99;
        case "parse_profile":
          return { name: "Ana Souza", experiences: [], stories: [], terms: [], warnings: [] };
        case "start_session":
          playDemo();
          return { sessionId: 1, wavDir: null };
        case "stop_session":
          timers.forEach(clearTimeout);
          return null;
        case "mark_moment":
          return 21_000;
        case "list_sessions":
          return [
            {
              id: 7,
              startedAt: fullSession.detail.startedAt,
              durationMs: 2_712_000,
              otherSide: "ms-teams.exe",
              interrupted: false,
              segments: 312,
              kitName: null,
              candidateName: "Ana Souza",
              hasNotes: false,
              practicePersona: null,
              side: "candidato",
            },
          ];
        case "session_full":
          return fullSession;
        case "generate_notes": {
          const notes = {
            generatedAt: Date.now(),
            model: "demonstração",
            competencies: ["Comunicação", "Clareza", "Exemplos concretos"].map((name, i) => ({
              name,
              suggestedScore: i === 2 ? 4 : 3,
              score: i === 2 ? 4 : 3,
              forEvidence: [{ text: "Contou um caso real com resultado.", quote: "a gente acabava cobrando duas vezes o cliente", minute: "0:18" }],
              against: [],
              missing: "",
              comment: "",
            })),
            resumePoints: [],
            nextQuestions: ["Prepare um número pro caso do checkout."],
            summary: "Entrevista técnica; você contou o caso do Pix.",
            interviewerNotes: "",
            reviewed: false,
            droppedQuotes: 0,
          };
          fullSession.detail.notesJson = JSON.stringify(notes);
          return new Promise((resolve) => setTimeout(() => resolve(notes), 800));
        }
        case "save_notes":
          fullSession.detail.notesJson = JSON.stringify(a.notes);
          return null;
        case "get_flag":
          return onboarded;
        case "set_flag":
          onboarded = Boolean(a.value);
          return null;
        case "retention_days":
          return 30;
        case "set_retention_days":
          return 0;
        default:
          // Janela, diálogos e o que mais não precisa de resposta na demonstração.
          return cmd.startsWith("plugin:dialog|ask") ? true : null;
      }
    },
    { shouldMockEvents: true },
  );
}
