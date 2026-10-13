import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { Badge, Button, Callout, Flex, Text, TextArea } from "@radix-ui/themes";
import { DownloadIcon, ExclamationTriangleIcon, PlusIcon, TrashIcon, UploadIcon } from "@radix-ui/react-icons";
import { Picker, Section } from "./ui";
import type { Kit, KitRow } from "./types";

const NEW = "__novo__";

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

  async function choose(value: string) {
    if (dirty && !(await ask("Descartar as alterações não salvas?", { kind: "warning" }))) return;
    setMessage(null);
    if (value === NEW) {
      setSelected(null);
      setMarkdown(TEMPLATE);
      setSaved("");
      return;
    }
    const kit = kits.find((k) => k.id === Number(value));
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
  const options = [
    ...(selected === null ? [{ value: NEW, label: "Novo kit" }] : []),
    ...kits.map((k) => ({ value: String(k.id), label: k.name })),
  ];

  return (
    <Flex direction="column" gap="3">
      <Flex gap="2">
        <div className="grow">
          <Picker value={selected === null ? NEW : String(selected)} options={options} onChange={choose} />
        </div>
        <Button variant="soft" color="gray" onClick={() => choose(NEW)}>
          <PlusIcon /> Novo
        </Button>
        <Button variant="soft" color="gray" onClick={importKit}>
          <UploadIcon /> Importar
        </Button>
      </Flex>

      <TextArea
        className="kit-editor"
        resize="vertical"
        spellCheck={false}
        value={markdown}
        onChange={(e) => setMarkdown(e.target.value)}
      />

      {parsed && (
        <Section>
          <Text size="2" weight="medium">
            {parsed.title || "Sem título"}
          </Text>
          <Flex gap="2" wrap="wrap">
            <Badge variant="soft" color="gray">
              {parsed.competencies.length} competências
            </Badge>
            <Badge variant="soft" color="gray">
              {parsed.blocks.length} blocos{totalMinutes ? ` · ${totalMinutes} min` : ""}
            </Badge>
            <Badge variant="soft" color="gray">
              {parsed.terms.length} termos
            </Badge>
            <Badge variant="soft" color="gray">
              {parsed.forbidden.length} perguntas proibidas próprias
            </Badge>
          </Flex>
          {parsed.warnings.map((w) => (
            <Callout.Root key={w} size="1" color="amber">
              <Callout.Icon>
                <ExclamationTriangleIcon />
              </Callout.Icon>
              <Callout.Text>{w}</Callout.Text>
            </Callout.Root>
          ))}
        </Section>
      )}

      <Flex gap="2" align="center">
        <Button disabled={!dirty} onClick={saveKit}>
          Salvar
        </Button>
        <Button variant="soft" color="gray" onClick={exportKit}>
          <DownloadIcon /> Exportar .md
        </Button>
        {message && (
          <Text size="1" color="gray">
            {message}
          </Text>
        )}
        <span className="grow" />
        {selected !== null && (
          <Button variant="ghost" color="red" onClick={removeKit}>
            <TrashIcon /> Apagar
          </Button>
        )}
      </Flex>
    </Flex>
  );
}
