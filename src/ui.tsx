// Peças de layout repetidas, em cima do Radix Themes.

import { useEffect, useState, type ReactNode } from "react";
import { Card, Flex, Heading, Select, Text } from "@radix-ui/themes";

export function Section({ title, description, children, actions }: {
  title?: string;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card size="2">
      <Flex direction="column" gap="3">
        {(title || actions) && (
          <Flex align="center" justify="between" gap="2">
            {title && (
              <Heading as="h2" size="3" weight="medium">
                {title}
              </Heading>
            )}
            {actions}
          </Flex>
        )}
        {description && (
          <Text as="p" size="2" color="gray">
            {description}
          </Text>
        )}
        {children}
      </Flex>
    </Card>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <Flex direction="column" gap="1">
      <Text as="div" size="2" weight="medium">
        {label}
      </Text>
      {children}
      {hint && (
        <Text as="div" size="1" color="gray">
          {hint}
        </Text>
      )}
    </Flex>
  );
}

export type Option = { value: string; label: string; detail?: string };

/** Select do Radix com rótulo e texto de apoio por opção. Radix não aceita valor vazio. */
export function Picker({ value, options, onChange, disabled, placeholder }: {
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  disabled?: boolean;
  placeholder?: string;
}) {
  return (
    <Select.Root value={value} onValueChange={onChange} disabled={disabled}>
      <Select.Trigger placeholder={placeholder} style={{ width: "100%" }} />
      <Select.Content position="popper">
        {options.map((o) => (
          <Select.Item key={o.value} value={o.value}>
            {o.detail ? `${o.label} · ${o.detail}` : o.label}
          </Select.Item>
        ))}
      </Select.Content>
    </Select.Root>
  );
}

/** Tema claro ou escuro, seguindo o Windows. */
export function useSystemDark() {
  const query = "(prefers-color-scheme: dark)";
  const [dark, setDark] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const m = window.matchMedia(query);
    const update = () => setDark(m.matches);
    m.addEventListener("change", update);
    return () => m.removeEventListener("change", update);
  }, []);
  return dark;
}
