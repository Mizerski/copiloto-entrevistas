import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import type { Kit, KitRow } from "./types";

const TEMPLATE = `# Nome da vaga

## Vaga
Descreva a vaga, o nível e o time.

## Competências

### Primeira competência
Forte: o que conta como sinal forte.
Fraco: o que conta como sinal fraco.

## Roteiro

### Abertura (5 min)
- Primeira pergunta

## Perguntas proibidas

## Termos
Termos técnicos e siglas que a transcrição costuma errar, separados por vírgula
`;

export function Kits({ onChanged }: { onChanged?: () => void }) {
  const [kits, setKits] = useState<KitRow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [markdown, setMarkdown] = useState("");
  const [saved, setSaved] = useState("");
  const [parsed, setParsed] = useState<Kit | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  async function load(selectId?: number | null) {
    const list = await invoke<KitRow[]>("list_kits");
    setKits(list);
    const pick = list.find((k) => k.id === selectId) ?? (selectId === undefined ? list[0] : undefined);
    if (pick) {
      setSelected(pick.id);
      setMarkdown(pick.markdown);
      setSaved(pick.markdown);
    }
  }

  useEffect(() => {
    load().catch((e) => setMessage(String(e)));
  }, []);

  // Pré-visualização: o mesmo parser que a sessão usa.
  useEffect(() => {
    const t = setTimeout(() => invoke<Kit>("parse_kit", { markdown }).then(setParsed), 250);
    return () => clearTimeout(t);
  }, [markdown]);

  const dirty = markdown !== saved;

  async function choose(id: number | null) {
    if (dirty && !(await ask("Descartar as alterações não salvas?", { kind: "warning" }))) return;
    if (id === null) {
      setSelected(null);
      setMarkdown(TEMPLATE);
      setSaved("");
      return;
    }
    const kit = kits.find((k) => k.id === id);
    if (kit) {
      setSelected(kit.id);
      setMarkdown(kit.markdown);
      setSaved(kit.markdown);
    }
  }

  async function saveKit() {
    try {
      const id = await invoke<number>("save_kit", { id: selected, markdown });
      setSaved(markdown);
      setMessage("Kit salvo.");
      await load(id);
      onChanged?.();
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function importKit() {
    const path = await open({ filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }] });
    if (typeof path !== "string") return;
    try {
      const id = await invoke<number>("import_kit_file", { path });
      await load(id);
      setMessage("Kit importado.");
      onChanged?.();
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function exportKit() {
    const name = (parsed?.title || "kit").replace(/[\\/:*?"<>|]/g, "-");
    const path = await save({ defaultPath: `${name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!path) return;
    try {
      await invoke("write_text_file", { path, contents: markdown });
      setMessage("Kit exportado.");
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function removeKit() {
    if (selected === null) return;
    if (!(await ask("Apagar este kit? Sessões antigas guardam a própria cópia.", { kind: "warning" }))) return;
    await invoke("delete_kit", { id: selected });
    setSelected(null);
    await load();
    onChanged?.();
  }

  const totalMinutes = parsed?.blocks.reduce((sum, b) => sum + (b.minutes ?? 0), 0) ?? 0;

  return (
    <div className="kits">
      <div className="row">
        <select value={selected ?? ""} onChange={(e) => choose(e.target.value ? Number(e.target.value) : null)}>
          {selected === null && <option value="">Novo kit</option>}
          {kits.map((k) => (
            <option key={k.id} value={k.id}>
              {k.name}
            </option>
          ))}
        </select>
        <button onClick={() => choose(null)}>Novo</button>
        <button onClick={importKit}>Importar</button>
      </div>

      <textarea
        className="kit-editor"
        spellCheck={false}
        value={markdown}
        onChange={(e) => setMarkdown(e.target.value)}
      />

      {parsed && (
        <section className="kit-preview">
          <strong>{parsed.title || "Sem título"}</strong>
          <span className="muted">
            {parsed.competencies.length} competências · {parsed.blocks.length} blocos
            {totalMinutes ? ` (${totalMinutes} min)` : ""} · {parsed.terms.length} termos ·{" "}
            {parsed.forbidden.length} perguntas proibidas próprias
          </span>
          {parsed.warnings.map((w) => (
            <span key={w} className="warning">
              {w}
            </span>
          ))}
        </section>
      )}

      <div className="row">
        <button className="primary" disabled={!dirty} onClick={saveKit}>
          Salvar
        </button>
        <button onClick={exportKit}>Exportar .md</button>
        <span className="grow" />
        {selected !== null && (
          <button className="danger" onClick={removeKit}>
            Apagar
          </button>
        )}
      </div>
      {message && <p className="muted">{message}</p>}
    </div>
  );
}
