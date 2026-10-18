// Perfil de candidato: o que você escreve sobre si pra o app usar no modo candidato.
// Mesmo jeito dos kits: Markdown com pré-visualização, importar e exportar.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { Badge, Button, Callout, Flex, Text, TextArea } from "@radix-ui/themes";
import { DownloadIcon, ExclamationTriangleIcon, PlusIcon, TrashIcon, UploadIcon } from "@radix-ui/react-icons";
import { Picker, Section } from "./ui";
import type { KitRow, ProfileInfo } from "./types";

const NEW = "__novo__";

const TEMPLATE = `# Seu nome

## Resumo
Duas ou três frases: o que você faz, há quanto tempo, no que é forte.

## Experiências

### Empresa · Cargo (2022–hoje)
- Time, tecnologias, o que era seu.
- Um resultado com número.

## Histórias
Casos que você conta bem. São eles que viram exemplo nas respostas.

### Nome curto da história
Situação: o problema e o contexto. Ação: o que VOCÊ fez. Resultado: o que mudou, com número. Competências: liderança, conflito, performance...

## Habilidades
Linguagens, ferramentas, idiomas.

## Pontos a melhorar
O que você ainda não domina (o app usa isso pra não inventar que você domina).

## Jeito de falar
Como você fala de verdade: formal ou solto, frases curtas ou longas, expressões e vícios que
você usa ("então", "basicamente", "a gente"). Cole 2 ou 3 frases suas, do jeito que sairiam
na fala. As respostas sugeridas imitam esse jeito.

## Perguntas pra fazer
- O que você quer saber da empresa.

## Termos
Nomes e siglas que a transcrição costuma errar, separados por vírgula
`;

export function Profiles({ onChanged }: { onChanged?: () => void }) {
  const [profiles, setProfiles] = useState<KitRow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [markdown, setMarkdown] = useState("");
  const [saved, setSaved] = useState("");
  const [parsed, setParsed] = useState<ProfileInfo | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  async function load(selectId?: number | null) {
    const list = await invoke<KitRow[]>("list_profiles");
    setProfiles(list);
    const pick = list.find((p) => p.id === selectId) ?? (selectId === undefined ? list[0] : undefined);
    if (pick) {
      setSelected(pick.id);
      setMarkdown(pick.markdown);
      setSaved(pick.markdown);
    } else if (list.length === 0) {
      setSelected(null);
      setMarkdown(TEMPLATE);
      setSaved("");
    }
  }

  useEffect(() => {
    load().catch((e) => setMessage(String(e)));
  }, []);

  useEffect(() => {
    const t = setTimeout(() => invoke<ProfileInfo>("parse_profile", { markdown }).then(setParsed), 250);
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
    const profile = profiles.find((p) => p.id === Number(value));
    if (profile) {
      setSelected(profile.id);
      setMarkdown(profile.markdown);
      setSaved(profile.markdown);
    }
  }

  async function saveProfile() {
    try {
      const id = await invoke<number>("save_profile", { id: selected, markdown });
      setSaved(markdown);
      setMessage("Perfil salvo.");
      await load(id);
      onChanged?.();
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function importProfile() {
    const path = await open({ filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }] });
    if (typeof path !== "string") return;
    try {
      const id = await invoke<number>("import_profile_file", { path });
      await load(id);
      setMessage("Perfil importado.");
      onChanged?.();
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function exportProfile() {
    const name = (parsed?.name || "perfil").replace(/[\\/:*?"<>|]/g, "-");
    const path = await save({ defaultPath: `${name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!path) return;
    try {
      await invoke("write_text_file", { path, contents: markdown });
      setMessage("Perfil exportado.");
    } catch (e) {
      setMessage(String(e));
    }
  }

  async function removeProfile() {
    if (selected === null) return;
    if (!(await ask("Apagar este perfil?", { kind: "warning" }))) return;
    await invoke("delete_profile", { id: selected });
    setSelected(null);
    await load();
    onChanged?.();
  }

  const options = [
    ...(selected === null ? [{ value: NEW, label: "Novo perfil" }] : []),
    ...profiles.map((p) => ({ value: String(p.id), label: p.name })),
  ];

  return (
    <Flex direction="column" gap="3">
      <Text size="2" color="gray">
        Seu perfil de candidato. No modo Candidato, as respostas sugeridas saem só daqui: escreva experiências,
        histórias com números e o que você ainda não domina.
      </Text>

      <Flex gap="2">
        <div className="grow">
          <Picker value={selected === null ? NEW : String(selected)} options={options} onChange={choose} />
        </div>
        <Button variant="soft" color="gray" onClick={() => choose(NEW)}>
          <PlusIcon /> Novo
        </Button>
        <Button variant="soft" color="gray" onClick={importProfile}>
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
            {parsed.name || "Sem nome"}
          </Text>
          <Flex gap="2" wrap="wrap">
            <Badge variant="soft" color="gray">
              {parsed.experiences.length} experiências
            </Badge>
            <Badge variant="soft" color="gray">
              {parsed.stories.length} histórias
            </Badge>
            <Badge variant="soft" color="gray">
              {parsed.terms.length} termos
            </Badge>
          </Flex>
          {parsed.stories.length > 0 && (
            <Text size="1" color="gray">
              Histórias: {parsed.stories.join(" · ")}
            </Text>
          )}
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
        <Button disabled={!dirty} onClick={saveProfile}>
          Salvar
        </Button>
        <Button variant="soft" color="gray" onClick={exportProfile}>
          <DownloadIcon /> Exportar .md
        </Button>
        {message && (
          <Text size="1" color="gray">
            {message}
          </Text>
        )}
        <span className="grow" />
        {selected !== null && (
          <Button variant="ghost" color="red" onClick={removeProfile}>
            <TrashIcon /> Apagar
          </Button>
        )}
      </Flex>
    </Flex>
  );
}
