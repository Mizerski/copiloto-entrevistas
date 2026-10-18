// Os balões da sessão ao vivo: o que responder ao entrevistador, tirado do seu perfil.

import { Card, Flex, Text } from "@radix-ui/themes";
import { formatClock } from "./format";
import { CARD_LABEL, type BalloonSlot, type Suggestion } from "./types";

type SlotConfig = { slot: BalloonSlot; label: string; color: "orange" | "red" | "blue" };

const SLOTS: SlotConfig[] = [
  { slot: "responda", label: "Responda", color: "orange" },
  { slot: "historia", label: "Se pedirem mais", color: "blue" },
  { slot: "atencao", label: "Atenção", color: "red" },
];

type BalloonsProps = {
  suggestions: Partial<Record<BalloonSlot, Suggestion>>;
  waiting: boolean;
  onUse: (s: Suggestion) => void;
};

/** Três balões fixos, cada um com uma função. Clicar dispensa. */
export function Balloons({ suggestions, waiting, onUse }: BalloonsProps) {
  return (
    <Flex direction="column" gap="2">
      {SLOTS.map(({ slot, label, color }) => {
        const s = suggestions[slot];
        // O balão vermelho só fica vermelho quando é contradição; o resto é âmbar.
        const tone = color === "red" && s?.kind !== "contradicao" ? "amber" : color;
        if (!s) {
          return (
            <Card key={slot} size="1" className="hint-card gray balloon empty">
              <Text size="1" weight="medium" color="gray">
                {label}
              </Text>
              <Text size="1" color="gray" ml="2">
                {waiting ? "ouvindo…" : "aparece quando o entrevistador perguntar"}
              </Text>
            </Card>
          );
        }
        return (
          <Card
            key={slot}
            size="1"
            className={`hint-card ${tone} balloon`}
            role="button"
            tabIndex={0}
            title="Dispensar"
            onClick={() => onUse(s)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onUse(s)}
          >
            <Flex align="center" gap="2" mb="1">
              <Text size="1" weight="medium" color={tone}>
                {label}
              </Text>
              {s.kind !== slot && (
                <Text size="1" color="gray">
                  {CARD_LABEL[s.kind] ?? s.kind}
                </Text>
              )}
              <Text size="1" color="gray" className="tabular" ml="auto">
                {formatClock(s.tMs)}
              </Text>
            </Flex>
            {/* A continuação é texto pra ler em voz alta, sem peso. */}
            <Text
              as="p"
              size="3"
              weight={s.slot === "historia" ? "regular" : "medium"}
              key={s.id}
              className="balloon-text"
            >
              {s.question}
            </Text>
            {s.earlier && s.now && (
              <Flex direction="column" gap="1" mt="2" className="contradiction">
                <Text size="1" color="gray">
                  {s.earlier.atMs === null ? "No perfil" : `Aos ${formatClock(s.earlier.atMs)}`}:{" "}
                  <span className="quote">“{s.earlier.text}”</span>
                </Text>
                <Text size="1" color="gray">
                  Agora{s.now.atMs !== null ? ` (${formatClock(s.now.atMs)})` : ""}:{" "}
                  <span className="quote">“{s.now.text}”</span>
                </Text>
              </Flex>
            )}
            {s.why && !s.earlier && (
              <Text as="p" size="1" color="gray" mt="1">
                {s.why}
              </Text>
            )}
          </Card>
        );
      })}
    </Flex>
  );
}
