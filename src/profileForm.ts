// O perfil em campos separados. Fica gravado como Markdown (é o que vai pro modelo e o
// que importar/exportar usa); a tela edita os campos e monta o Markdown na hora de salvar,
// então a formatação nunca sai errada.

export type Experience = { title: string; details: string };
export type Story = { title: string; situation: string; action: string; result: string; skills: string };

export type ProfileFields = {
  name: string;
  summary: string;
  speaking: string;
  experiences: Experience[];
  stories: Story[];
  skills: string;
  gaps: string;
  questions: string;
  terms: string;
  /** Seções que o formulário não conhece, guardadas como vieram. */
  extra: string;
};

export const EMPTY_EXPERIENCE: Experience = { title: "", details: "" };
export const EMPTY_STORY: Story = { title: "", situation: "", action: "", result: "", skills: "" };

export function emptyProfile(): ProfileFields {
  return {
    name: "",
    summary: "",
    speaking: "",
    experiences: [{ ...EMPTY_EXPERIENCE }],
    stories: [{ ...EMPTY_STORY }],
    skills: "",
    gaps: "",
    questions: "",
    terms: "",
    extra: "",
  };
}

/** Sem acento e em minúsculas, pra reconhecer o título da seção. */
function fold(text: string) {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();
}

const STORY_LABELS: [keyof Story, RegExp][] = [
  ["situation", /^situa/],
  ["action", /^acao/],
  ["result", /^resultado/],
  ["skills", /^competencia/],
];

/** "**Situação:** texto", "Ação (roteiro): texto"… → o campo e o resto da linha. */
function storyLabel(line: string): [keyof Story, string] | null {
  const m = line.match(/^\s*\**\s*([^:*]{3,30}?)\s*:\s*\**\s*(.*)$/);
  if (!m) return null;
  const label = fold(m[1]);
  const field = STORY_LABELS.find(([, re]) => re.test(label));
  return field ? [field[0], m[2]] : null;
}

/** Uma história pode vir com tudo numa linha ("Situação: ... Ação: ... Resultado: ..."). */
function splitInline(line: string) {
  return line.split(/(?=\s(?:\*\*)?(?:Ação|Acao|Resultado|Competências|Competencias)\b[^:]{0,15}:)/).map((p) => p.trim());
}

function parseStory(title: string, body: string[]): Story {
  const story: Story = { ...EMPTY_STORY, title };
  let current: keyof Story = "action";
  const parts: Record<string, string[]> = { situation: [], action: [], result: [], skills: [] };
  let labeled = false;
  for (const raw of body) {
    for (const piece of raw.includes(":") ? splitInline(raw) : [raw]) {
      const hit = storyLabel(piece);
      if (hit) {
        labeled = true;
        current = hit[0];
        if (hit[1]) parts[current].push(hit[1]);
      } else {
        parts[current].push(piece);
      }
    }
  }
  // Sem rótulo nenhum: o texto todo fica como está, em Ação, pra nada se perder.
  if (!labeled) parts.action = body;
  for (const key of ["situation", "action", "result", "skills"] as const) {
    story[key] = parts[key].join("\n").trim();
  }
  return story;
}

function trimBlock(lines: string[]) {
  return lines.join("\n").replace(/^\n+|\s+$/g, "");
}

export function parseProfile(markdown: string): ProfileFields {
  const p: ProfileFields = { ...emptyProfile(), experiences: [], stories: [] };
  const extra: string[] = [];
  let section = "";
  let sectionTitle = "";
  let buffer: string[] = [];
  let item: { title: string; body: string[] } | null = null;

  const flushItem = () => {
    if (!item) return;
    if (section.startsWith("experiencia")) p.experiences.push({ title: item.title, details: trimBlock(item.body) });
    else if (section.startsWith("historia")) p.stories.push(parseStory(item.title, trimBlock(item.body).split("\n")));
    item = null;
  };
  const flushSection = () => {
    flushItem();
    const text = trimBlock(buffer);
    buffer = [];
    if (section.startsWith("resumo")) p.summary = text;
    else if (section.startsWith("jeito")) p.speaking = text;
    else if (section.startsWith("habilidade")) p.skills = text;
    else if (section.startsWith("pontos a melhorar") || section.startsWith("ponto")) p.gaps = text;
    else if (section.startsWith("pergunta")) p.questions = text;
    else if (section.startsWith("termo")) p.terms = text.replace(/\n+/g, ", ");
    // Texto solto antes dos ### de experiências e histórias não tem campo: vai pra extra.
    else if ((section.startsWith("experiencia") || section.startsWith("historia")) && text) {
      extra.push(`## ${sectionTitle}\n${text}`);
    } else if (section && !section.startsWith("experiencia") && !section.startsWith("historia")) {
      extra.push(`## ${sectionTitle}\n${text}`);
    } else if (!section && text) extra.push(text);
  };

  // Editores de Markdown escapam colchetes e sublinhados ("\[preencher\]", "ERRO\_X"):
  // na caixa de texto isso só atrapalha.
  const clean = markdown.replace(/\r\n/g, "\n").replace(/\\([[\]_*])/g, "$1");
  for (const line of clean.split("\n")) {
    if (line.startsWith("### ")) {
      flushItem();
      item = { title: line.slice(4).trim(), body: [] };
    } else if (line.startsWith("## ")) {
      flushSection();
      sectionTitle = line.slice(3).trim();
      section = fold(sectionTitle);
    } else if (line.startsWith("# ") && !p.name) {
      p.name = line.slice(2).trim();
    } else if (item) {
      item.body.push(line);
    } else {
      buffer.push(line);
    }
  }
  flushSection();
  p.extra = extra.join("\n\n");
  if (p.experiences.length === 0) p.experiences.push({ ...EMPTY_EXPERIENCE });
  if (p.stories.length === 0) p.stories.push({ ...EMPTY_STORY });
  return p;
}

export function toMarkdown(p: ProfileFields): string {
  const out: string[] = [`# ${p.name.trim() || "Sem nome"}`];
  const section = (title: string, body: string) => {
    if (body.trim()) out.push(`## ${title}\n${body.trim()}`);
  };
  section("Resumo", p.summary);
  section("Jeito de falar", p.speaking);
  const experiences = p.experiences.filter((e) => e.title.trim() || e.details.trim());
  if (experiences.length) {
    out.push(
      "## Experiências\n\n" +
        experiences.map((e) => `### ${e.title.trim() || "Sem título"}\n${e.details.trim()}`.trim()).join("\n\n"),
    );
  }
  const stories = p.stories.filter((s) => s.title.trim() || s.situation.trim() || s.action.trim() || s.result.trim());
  if (stories.length) {
    const story = (s: Story) =>
      [
        `### ${s.title.trim() || "Sem título"}`,
        s.situation.trim() && `Situação: ${s.situation.trim()}`,
        s.action.trim() && `Ação: ${s.action.trim()}`,
        s.result.trim() && `Resultado: ${s.result.trim()}`,
        s.skills.trim() && `Competências: ${s.skills.trim()}`,
      ]
        .filter(Boolean)
        .join("\n\n");
    out.push("## Histórias\n\n" + stories.map(story).join("\n\n"));
  }
  section("Habilidades", p.skills);
  section("Pontos a melhorar", p.gaps);
  section("Perguntas pra fazer", p.questions);
  section("Termos", p.terms);
  if (p.extra.trim()) out.push(p.extra.trim());
  return out.join("\n\n") + "\n";
}

/** O que ainda falta ou ficou com marcação de preencher. */
export function profileWarnings(p: ProfileFields): string[] {
  const w: string[] = [];
  if (!p.name.trim()) w.push("Falta o seu nome.");
  if (!p.experiences.some((e) => e.title.trim())) w.push("Nenhuma experiência.");
  if (!p.stories.some((s) => s.title.trim())) w.push("Nenhuma história. São elas que viram exemplo nas respostas.");
  if (!p.speaking.trim()) w.push("Sem jeito de falar: as respostas saem num tom neutro.");
  const pending = p.stories.filter((s) => /\[preencher|\[o que|\[como/i.test(`${s.situation} ${s.action} ${s.result}`));
  if (pending.length) w.push(`Histórias com campo a preencher: ${pending.map((s) => s.title).join(", ")}.`);
  return w;
}
