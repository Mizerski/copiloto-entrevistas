import { useState } from "react";

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
    <button
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    >
      {copied ? "Copiado" : "Copiar"}
    </button>
  );
}

type Props = {
  checked: boolean;
  onChange: (value: boolean) => void;
  /** "O áudio vai pra…": o que sai do computador nesta sessão. */
  destinations: string[];
};

export function Consent({ checked, onChange, destinations }: Props) {
  return (
    <section className="consent">
      <div className="section-title">Consentimento</div>
      <ul className="plain small">
        {destinations.map((d) => (
          <li key={d}>{d}</li>
        ))}
      </ul>
      <details>
        <summary>Texto pra avisar o candidato</summary>
        <p className="quote-box">{NOTICE_TEXT}</p>
        <CopyButton text={NOTICE_TEXT} />
      </details>
      <details>
        <summary>Modelo de cláusula pro convite da entrevista</summary>
        <p className="quote-box">{INVITE_CLAUSE}</p>
        <CopyButton text={INVITE_CLAUSE} />
      </details>
      <label className="check">
        <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        Avisei o candidato de que a conversa será transcrita e analisada
      </label>
    </section>
  );
}
