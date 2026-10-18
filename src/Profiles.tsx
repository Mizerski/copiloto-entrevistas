// Perfil de candidato: o que você escreve sobre si, em campos separados. O app monta o
// Markdown na hora de salvar; importar e exportar .md continuam funcionando.

import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { Button, Callout, Card, Flex, IconButton, Text, TextArea, TextField } from "@radix-ui/themes";
import { DownloadIcon, ExclamationTriangleIcon, PlusIcon, TrashIcon, UploadIcon } from "@radix-ui/react-icons";
import { Field, Picker, Section } from "./ui";
import {
  EMPTY_EXPERIENCE,
  EMPTY_STORY,
  emptyProfile,
  parseProfile,
  profileWarnings,
  toMarkdown,
  type ProfileFields,
} from "./profileForm";
import type { KitRow } from "./types";

const NEW = "__novo__";

/** Caixa de texto que cresce com o conteúdo, até um limite. */
function Box({ value, onChange, placeholder, rows = 3 }: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  rows?: number;
}) {
  const lines = Math.min(16, Math.max(rows, value.split("\n").length + 1));
  return <TextArea rows={lines} resize="vertical" placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />;
}

function ItemCard({ title, onRemove, children }: { title: string; onRemove?: () => void; children: ReactNode }) {
  return (
    <Card size="1" variant="surface">
      <Flex direction="column" gap="2">
        <Flex align="center" gap="2">
          <Text size="1" color="gray" className="grow">
            {title}
          </Text>
          {onRemove && (
            <IconButton size="1" variant="ghost" color="gray" aria-label="Remover" onClick={onRemove}>
              <TrashIcon />
            </IconButton>
          )}
        </Flex>
        {children}
      </Flex>
    </Card>
  );
}

export function Profiles({ onChanged }: { onChanged?: () => void }) {
  const [profiles, setProfiles] = useState<KitRow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [fields, setFields] = useState<ProfileFields>(emptyProfile);
  const [saved, setSaved] = useState("");
  const [message, setMessage] = useState<string | null>(null);

  function show(markdown: string | null) {
    const parsed = markdown === null ? emptyProfile() : parseProfile(markdown);
    setFields(parsed);
    // Compara com o Markdown que os campos montam, senão abrir já contaria como alteração.
    setSaved(markdown === null ? "" : toMarkdown(parsed));
  }

  async function load(selectId?: number | null) {
    const list = await invoke<KitRow[]>("list_profiles");
    setProfiles(list);
    const pick = list.find((p) => p.id === selectId) ?? (selectId === undefined ? list[0] : undefined);
    if (pick) {
      setSelected(pick.id);
      show(pick.markdown);
    } else if (list.length === 0) {
      setSelected(null);
      show(null);
    }
  }

  useEffect(() => {
    load().catch((e) => setMessage(String(e)));
  }, []);

  const markdown = toMarkdown(fields);
  const dirty = markdown !== saved;
  const warnings = profileWarnings(fields);
  const set = <K extends keyof ProfileFields>(key: K, value: ProfileFields[K]) => setFields((f) => ({ ...f, [key]: value }));

  async function choose(value: string) {
    if (dirty && !(await ask("Descartar as alterações não salvas?", { kind: "warning" }))) return;
    setMessage(null);
    if (value === NEW) {
      setSelected(null);
      show(null);
      return;
    }
    const profile = profiles.find((p) => p.id === Number(value));
    if (profile) {
      setSelected(profile.id);
      show(profile.markdown);
    }
  }

  async function saveProfile() {
    try {
      const id = await invoke<number>("save_profile", { id: selected, markdown });
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
    const name = (fields.name || "perfil").replace(/[\\/:*?"<>|]/g, "-");
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

  const updateList = <K extends "experiences" | "stories">(key: K, index: number, patch: Partial<ProfileFields[K][number]>) =>
    setFields((f) => ({ ...f, [key]: f[key].map((item, i) => (i === index ? { ...item, ...patch } : item)) }));
  const removeFrom = (key: "experiences" | "stories", index: number) =>
    setFields((f) => ({ ...f, [key]: f[key].filter((_, i) => i !== index) }));

  return (
    <Flex direction="column" gap="3">
      <Text size="2" color="gray">
        As respostas sugeridas saem só daqui. Preencha cada caixa; o app cuida da formatação.
      </Text>

      <Flex gap="2">
        <div className="grow">
          <Picker value={selected === null ? NEW : String(selected)} options={options} onChange={choose} />
        </div>
        <Button variant="soft" color="gray" onClick={() => choose(NEW)}>
          <PlusIcon /> Novo
        </Button>
        <Button variant="soft" color="gray" onClick={importProfile}>
          <UploadIcon /> Importar .md
        </Button>
      </Flex>

      <Section title="Você">
        <Field label="Nome">
          <TextField.Root value={fields.name} placeholder="Seu nome" onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field label="Resumo" hint="Duas ou três frases: o que você faz, há quanto tempo, no que é forte.">
          <Box value={fields.summary} onChange={(v) => set("summary", v)} />
        </Field>
      </Section>

      <Section
        title="Jeito de falar"
        description="Como você fala de verdade numa conversa, não como quer que alguém te explique. As respostas imitam isso."
      >
        <Field
          label="Seu jeito e exemplos"
          hint="Diga o tom (formal ou solto, frases curtas ou longas, expressões que usa) e cole 2 ou 3 frases suas do jeito que sairiam na fala."
        >
          <Box
            rows={5}
            value={fields.speaking}
            onChange={(v) => set("speaking", v)}
            placeholder={'Falo informal, frases curtas, uso "então" e "a gente".\n- "Então, hoje eu tô na DBServer, no time de previdência."\n- "Basicamente, a gente publica num tópico e um consumer pega dali."'}
          />
        </Field>
      </Section>

      <Section title="Experiências">
        {fields.experiences.map((e, i) => (
          <ItemCard
            key={i}
            title={`Experiência ${i + 1}`}
            onRemove={fields.experiences.length > 1 ? () => removeFrom("experiences", i) : undefined}
          >
            <TextField.Root
              value={e.title}
              placeholder="Empresa · Cargo (2022–hoje)"
              onChange={(ev) => updateList("experiences", i, { title: ev.target.value })}
            />
            <Box
              value={e.details}
              onChange={(v) => updateList("experiences", i, { details: v })}
              placeholder={"- Time, tecnologias, o que era seu.\n- Um resultado com número."}
            />
          </ItemCard>
        ))}
        <Button variant="soft" color="gray" onClick={() => set("experiences", [...fields.experiences, { ...EMPTY_EXPERIENCE }])}>
          <PlusIcon /> Experiência
        </Button>
      </Section>

      <Section title="Histórias" description="Casos que você conta bem. São eles que viram exemplo nas respostas.">
        {fields.stories.map((s, i) => (
          <ItemCard key={i} title={`História ${i + 1}`} onRemove={fields.stories.length > 1 ? () => removeFrom("stories", i) : undefined}>
            <TextField.Root
              value={s.title}
              placeholder="Nome curto da história"
              onChange={(ev) => updateList("stories", i, { title: ev.target.value })}
            />
            <Field label="Situação">
              <Box rows={2} value={s.situation} onChange={(v) => updateList("stories", i, { situation: v })} placeholder="O problema e o contexto." />
            </Field>
            <Field label="Ação">
              <Box value={s.action} onChange={(v) => updateList("stories", i, { action: v })} placeholder="O que VOCÊ fez, passo a passo." />
            </Field>
            <Field label="Resultado">
              <Box rows={2} value={s.result} onChange={(v) => updateList("stories", i, { result: v })} placeholder="O que mudou, com número se tiver." />
            </Field>
            <Field label="Competências">
              <TextField.Root
                value={s.skills}
                placeholder="liderança, conflito, performance…"
                onChange={(ev) => updateList("stories", i, { skills: ev.target.value })}
              />
            </Field>
          </ItemCard>
        ))}
        <Button variant="soft" color="gray" onClick={() => set("stories", [...fields.stories, { ...EMPTY_STORY }])}>
          <PlusIcon /> História
        </Button>
      </Section>

      <Section title="Mais sobre você">
        <Field label="Habilidades" hint="Linguagens, ferramentas, idiomas.">
          <Box value={fields.skills} onChange={(v) => set("skills", v)} />
        </Field>
        <Field label="Pontos a melhorar" hint="O que você ainda não domina. O app usa isso pra não inventar que você domina.">
          <Box value={fields.gaps} onChange={(v) => set("gaps", v)} />
        </Field>
        <Field label="Perguntas pra fazer" hint="O que você quer saber da empresa.">
          <Box rows={2} value={fields.questions} onChange={(v) => set("questions", v)} />
        </Field>
        <Field label="Termos" hint="Nomes e siglas que a transcrição costuma errar, separados por vírgula.">
          <TextField.Root value={fields.terms} onChange={(e) => set("terms", e.target.value)} />
        </Field>
        {fields.extra.trim() !== "" && (
          <Field label="Outras anotações" hint="Seções do arquivo importado que não têm caixa própria. Vão pro modelo como estão.">
            <Box value={fields.extra} onChange={(v) => set("extra", v)} />
          </Field>
        )}
      </Section>

      {warnings.map((w) => (
        <Callout.Root key={w} size="1" color="amber">
          <Callout.Icon>
            <ExclamationTriangleIcon />
          </Callout.Icon>
          <Callout.Text>{w}</Callout.Text>
        </Callout.Root>
      ))}

      <Flex gap="2" align="center" className="profile-actions">
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
