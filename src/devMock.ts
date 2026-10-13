// Modo demonstração, só em desenvolvimento: quando a página abre num navegador comum
// (sem o Tauri por trás), os comandos e eventos do Rust são simulados. Serve pra ver e
// conferir a interface sem microfone, chamada ou chaves.

import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import backendKit from "../src-tauri/kits/backend-senior-pagamentos.md?raw";
import behaviorKit from "../src-tauri/kits/comportamental-geral.md?raw";
import type { Kit, KitRow, Segment, SessionFull } from "./types";

const kits: KitRow[] = [
  { id: 1, name: "Backend sênior · Pagamentos", markdown: backendKit, updatedAt: Date.now() },
  { id: 2, name: "Entrevista comportamental · Geral", markdown: behaviorKit, updatedAt: Date.now() },
];

/** Versão simplificada do parser do Rust, suficiente pra demonstração. */
function parseKit(markdown: string): Kit {
  const kit: Kit = { title: "", job: "", competencies: [], blocks: [], forbidden: [], terms: [], warnings: [] };
  let section = "";
  for (const raw of markdown.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("# ")) kit.title = line.slice(2);
    else if (line.startsWith("## ")) section = line.slice(3).toLowerCase();
    else if (line.startsWith("### ")) {
      const name = line.slice(4);
      if (section.startsWith("compet")) kit.competencies.push({ name, description: "", strong: "", weak: "" });
      if (section === "roteiro") {
        const m = name.match(/^(.*)\((\d+) min\)$/);
        kit.blocks.push({ name: m ? m[1].trim() : name, minutes: m ? Number(m[2]) : null, questions: [] });
      }
    } else if (line.startsWith("- ") && section === "roteiro") kit.blocks[kit.blocks.length - 1]?.questions.push(line.slice(2));
    else if (line.startsWith("Forte:")) kit.competencies[kit.competencies.length - 1].strong = line.slice(6).trim();
    else if (line.startsWith("Fraco:")) kit.competencies[kit.competencies.length - 1].weak = line.slice(6).trim();
    else if (section === "termos" && line) kit.terms.push(...line.split(",").map((t) => t.trim()));
  }
  if (!kit.competencies.length) kit.warnings.push("Nenhuma competência: o painel de cobertura fica vazio.");
  return kit;
}

const script: [number, "mic" | "sistema", string][] = [
  [0, "mic", "Me conta sobre um projeto difícil em que você trabalhou recentemente."],
  [5_900, "sistema", "No meu último projeto eu fui responsável pela migração do processamento de pagamentos,"],
  [10_400, "sistema", "para uma arquitetura orientada a eventos. A gente usava Kafka para publicar os eventos de transação,"],
  [18_000, "sistema", "e cada consumidor precisava ser idempotente, porque o mesmo evento podia chegar duas vezes."],
  [26_000, "mic", "E como vocês garantiam isso na prática?"],
  [29_000, "sistema", "Eu propus guardar a chave de idempotência no Postgres com uma restrição de unicidade."],
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
  const kit = parseKit(backendKit);
  const at = (ms: number, fn: () => void) => timers.push(window.setTimeout(fn, ms));
  emit("analise", {
    kind: "coverage",
    items: kit.competencies.map((c) => ({ competency: c.name, level: "none", evidence: "", atMs: 0 })),
  });
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
  at(5_000, () => {
    emit("analise", { kind: "status", state: "analisando", message: null, latencyMs: null });
  });
  at(6_500, () => {
    emit("analise", {
      kind: "coverage",
      items: kit.competencies.map((c, i) => ({
        competency: c.name,
        level: i === 0 ? "strong" : i === 1 ? "partial" : "none",
        evidence: i < 2 ? "cada consumidor precisava ser idempotente" : "",
        atMs: 18_000,
      })),
    });
    const balloon = (id: number, slot: string, kind: string, question: string, why: string, extra = {}) =>
      emit("analise", { kind: "suggestion", suggestion: { id, slot, tMs: 18_000, kind, question, why, earlier: null, now: null, latencyMs: 1900, ...extra } });
    balloon(11, "aprofundar", "aprofundar", "Você falou que a gente usava Kafka. Qual foi exatamente a sua parte nessa migração?", "separar o papel dela do time");
    balloon(12, "checar", "contradicao", "Me ajuda a entender: no currículo você liderou a migração, e agora contou que entrou na metade. Como foi?", "esclarecer liderança", {
      earlier: { text: "Liderou a migração de pagamentos para eventos", atMs: null },
      now: { text: "eu entrei no projeto quando ele já estava na metade", atMs: 16_000 },
    });
    balloon(13, "avancar", "nao_tocada", "Como você explicava o andamento da migração pro pessoal de negócio?", "comunicação ainda sem evidência");
    emit("analise", { kind: "status", state: "ok", message: null, latencyMs: 1800 });
  });
  at(8_000, () => emit("analise", { kind: "card", card: { id: 3, tMs: 26_000, kind: "proibida", text: "Pergunta proibida (\"tem filhos\"). Mude de assunto.", priority: "alta" } }));
}

const fullSession: SessionFull = {
  detail: {
    id: 7,
    startedAt: Date.now() - 86_400_000,
    durationMs: 2_712_000,
    otherSide: "ms-teams.exe",
    interrupted: false,
    kitName: "Backend sênior · Pagamentos",
    kitMarkdown: backendKit,
    candidateName: "Ana Souza",
    candidateResume: "Backend há 7 anos, Java e Kotlin. Liderou a migração de pagamentos para eventos.",
    candidateCheckpoints: "Diz que liderou a migração",
    coverageJson: JSON.stringify(
      parseKit(backendKit).competencies.map((c, i) => ({
        competency: c.name,
        level: i < 2 ? "strong" : i === 2 ? "partial" : "none",
        evidence: i < 3 ? "cada consumidor precisava ser idempotente" : "",
        atMs: 18_000,
      })),
    ),
    summary: "Candidata descreveu a migração de pagamentos para eventos com Kafka.",
    notesJson: null,
  },
  segments: script.map(([t, s, text]) => seg(t, s, text)),
  cards: [{ id: 1, tMs: 18_000, kind: "aprofundar", text: "Pergunte qual foi a parte dela na migração.", priority: "media" }],
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
        case "list_kits":
          return kits;
        case "parse_kit":
          return parseKit(String(a.markdown ?? ""));
        case "save_kit":
          return a.id ?? 99;
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
        case "start_session":
          playDemo();
          return { sessionId: 1, wavDir: null, kit: parseKit(backendKit), warnings: [] };
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
              kitName: "Backend sênior · Pagamentos",
              candidateName: "Ana Souza",
              hasNotes: false,
            },
          ];
        case "session_full":
          return fullSession;
        case "generate_notes": {
          const notes = {
            generatedAt: Date.now(),
            model: "demonstração",
            competencies: parseKit(backendKit).competencies.map((c, i) => ({
              name: c.name,
              suggestedScore: i === 0 ? 4 : i === 1 ? 3 : 0,
              score: i === 0 ? 4 : i === 1 ? 3 : 0,
              forEvidence:
                i < 2
                  ? [{ text: "Explicou idempotência com um caso real.", quote: "cada consumidor precisava ser idempotente", minute: "0:18" }]
                  : [],
              against: i === 1 ? [{ text: "Não disse o que fazia com mensagens que falhavam.", quote: "A gente usava Kafka", minute: "0:10" }] : [],
              missing: i >= 2 ? "Não apareceu na conversa." : "",
              comment: "",
            })),
            resumePoints: [{ point: "Liderou a migração de pagamentos", status: "nao_verificado", comment: "Falou sempre no plural." }],
            nextQuestions: ["Qual foi exatamente a sua parte na migração?", "Como vocês tratavam mensagens que falhavam?"],
            summary: "Entrevista técnica sobre a migração de pagamentos para eventos.",
            interviewerNotes: "",
            reviewed: false,
            droppedQuotes: 1,
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
