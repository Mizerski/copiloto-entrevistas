# Ana Souza

## Resumo
Desenvolvedora backend há 7 anos, Java e Kotlin, os últimos 2 em pagamentos numa fintech. Gosto de sistemas que não podem errar: idempotência, conciliação, observabilidade.

## Experiências

### PagaJá · Backend sênior (2023–hoje)
- Time de pagamentos, 6 pessoas. Kotlin, Spring Boot, Kafka, Postgres.
- Responsável pelos consumidores de conciliação na migração do processamento de pagamentos para eventos. Entrei no projeto na metade; quem liderou foi meu gestor.
- Plantão uma semana por mês.

### LojaFácil · Backend pleno (2019–2023)
- E-commerce, checkout e estoque. Java 11, Spring, MySQL.
- Reduzi o tempo do fechamento de pedido de 2,1 s para 600 ms tirando chamadas síncronas ao estoque.

### Agência Pixel · Desenvolvedora júnior (2017–2019)
- Sites e integrações em PHP e Java para clientes de varejo.

## Histórias

### Pagamento duplicado no Pix
Situação: o mesmo evento de confirmação chegava duas vezes e gerava cobrança dupla (uns 40 casos por semana). Ação: propus guardar a chave de idempotência no Postgres com restrição de unicidade e escrevi o teste que reproduzia o problema. Resultado: os casos de duplicidade foram a zero em duas semanas. Competências: modelagem de dados, consistência.

### Fila em memória nos estornos
Situação: o time queria uma fila em memória para eventos de estorno. Ação: escrevi um documento mostrando que perderíamos eventos se o serviço reiniciasse, com um teste que reproduzia, e levei para a reunião de arquitetura. Resultado: a decisão mudou para Kafka; no trimestre seguinte nenhum estorno perdido. Competências: discordar com dados, influência.

### Checkout lento na Black Friday
Situação: o fechamento de pedido levava 2,1 s e caía em pico. Ação: medi com tracing, tirei as chamadas síncronas ao estoque e passei a reservar por evento. Resultado: 600 ms no p95 e nenhuma queda na Black Friday seguinte. Competências: performance, observabilidade.

### Erro meu em produção
Situação: subi uma migração de banco sem índice e a conciliação ficou 3 horas atrasada. Ação: reverti, avisei o time e o financeiro, criei um checklist de migração com revisão obrigatória. Resultado: nenhum incidente parecido desde então. Competências: responsabilidade, aprendizado.

## Habilidades
Kotlin, Java, Spring Boot, Kafka, Postgres, MySQL, Docker, Kubernetes básico, Grafana, OpenTelemetry.

## Pontos a melhorar
Nunca liderei time formalmente; mentorei dois juniores. Inglês intermediário (leio bem, converso devagar).

## Jeito de falar
Falo de forma direta e meio informal, uso "a gente" no lugar de "nós" e começo explicação com "então". Exemplos:
- "Então, o problema era que o evento chegava duas vezes, e a gente cobrava duas vezes."
- "Eu não liderei, quem liderou foi meu gestor, mas os consumidores eram comigo."

## Perguntas pra fazer
- Como funciona o plantão e quantos incidentes por mês?
- Como o time decide arquitetura?

## Termos
Kotlin, Spring Boot, Kafka, Postgres, Pix, idempotência, OpenTelemetry, Grafana, p95
