import { useState } from "react";
import { Blockquote, Button, Checkbox, Flex, Text } from "@radix-ui/themes";
import { CheckIcon, CopyIcon } from "@radix-ui/react-icons";
import { Section } from "./ui";

export const NOTICE_TEXT =
  "Antes de começarmos: vou usar um assistente que transcreve esta conversa e me ajuda a tomar notas. " +
  "A transcrição fica no meu computador e é usada só neste processo seletivo; o áudio passa por um serviço " +
  "de transcrição e não é gravado. Se preferir que algum trecho não seja transcrito, é só me avisar que eu pauso. " +
  "Tudo bem pra você?";

export const INVITE_CLAUSE =
  "Nas entrevistas usamos uma ferramenta de transcrição automática para apoiar as anotações de quem entrevista. " +
  "A conversa é transcrita durante a chamada, a transcrição é guardada apenas pelo tempo necessário ao processo " +
  "seletivo e pode ser excluída a pedido. As decisões são tomadas por pessoas; a ferramenta não avalia nem decide. " +
  "Se você não quiser que a conversa seja transcrita, é só nos avisar antes da entrevista.";

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Button
      size="1"
      variant="soft"
      color="gray"
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    >
      {copied ? <CheckIcon /> : <CopyIcon />}
      {copied ? "Copiado" : "Copiar"}
    </Button>
  );
}

function Template({ title, text }: { title: string; text: string }) {
  return (
    <details className="disclosure">
      <summary>
        <Text size="2">{title}</Text>
      </summary>
      <Flex direction="column" gap="2" mt="2" align="start">
        <Blockquote size="2">{text}</Blockquote>
        <CopyButton text={text} />
      </Flex>
    </details>
  );
}

type Props = {
  checked: boolean;
  onChange: (value: boolean) => void;
  /** O que sai do computador nesta sessão. */
  destinations: string[];
};

export function Consent({ checked, onChange, destinations }: Props) {
  return (
    <Section title="Consentimento">
      <Flex direction="column" gap="1">
        {destinations.map((d) => (
          <Text key={d} size="2" color="gray">
            {d}
          </Text>
        ))}
      </Flex>
      <Template title="Texto pra avisar o candidato" text={NOTICE_TEXT} />
      <Template title="Modelo de cláusula pro convite" text={INVITE_CLAUSE} />
      <Text as="label" size="2">
        <Flex gap="2" align="start">
          <Checkbox mt="1" checked={checked} onCheckedChange={(v) => onChange(v === true)} />
          Avisei o candidato de que a conversa será transcrita e analisada
        </Flex>
      </Text>
    </Section>
  );
}
