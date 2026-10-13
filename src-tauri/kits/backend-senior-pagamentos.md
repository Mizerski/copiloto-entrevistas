# Backend sênior · Pagamentos

## Vaga
Pessoa desenvolvedora backend sênior pro time de pagamentos (Pix, cartão e conciliação). Stack em Java e Kotlin, eventos em Kafka, Postgres. O time tem 6 pessoas e está saindo de um monólito.

## Competências

### Modelagem de dados e consistência
Forte: explica idempotência, transações e o que acontece quando um evento chega duas vezes, com um caso real.
Fraco: fala de "garantir consistência" sem dizer como; não pensa em falha parcial.

### Sistemas distribuídos e mensageria
Forte: discute ordenação, reprocessamento, dead letter, backpressure; sabe o custo de cada escolha.
Fraco: usa fila como caixa preta; não sabe o que faria se o consumidor caísse no meio.

### Qualidade e operação
Forte: testes no nível certo, observabilidade desde o começo, já esteve de plantão e mudou algo depois de um incidente.
Fraco: "o QA testa"; nunca olhou métrica de produção.

### Comunicação com negócio
Forte: traduz restrição técnica em impacto pro cliente; negociou escopo com produto.
Fraco: só descreve tarefas técnicas; não sabe por que o projeto existia.

### Liderança técnica
Forte: puxou decisão de arquitetura, escreveu proposta, ajudou outras pessoas a crescer.
Fraco: tudo no plural ("a gente fez") sem dizer qual foi a parte dela.

## Roteiro

### Abertura (5 min)
- Me conta rapidamente sua trajetória e o que te trouxe até aqui.

### Experiência técnica (25 min)
- Me conta sobre um projeto difícil em que você trabalhou recentemente.
- Como vocês garantiam que um pagamento não fosse processado duas vezes?
- O que acontecia quando um consumidor caía no meio do processamento?
- Como vocês sabiam que o sistema estava saudável em produção?

### Comportamental (15 min)
- Conta de uma vez em que você discordou de uma decisão técnica. O que fez?
- Como você explicou um atraso ou um risco técnico pra área de negócio?

### Perguntas do candidato (5 min)
- O que você gostaria de saber sobre o time?

## Perguntas proibidas
- (a lista padrão do app já vale; acrescente aqui o que for específico da empresa)

## Termos
Kafka, Pix, Postgres, idempotência, idempotente, Kotlin, conciliação, dead letter, backpressure, observabilidade
