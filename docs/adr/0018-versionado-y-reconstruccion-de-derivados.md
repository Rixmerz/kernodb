# ADR-0018 — Versionado y reconstrucción de artefactos derivados

| Campo | Valor |
|---|---|
| Estado | Propuesto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §10, §11, §12, §14; SAD §9.2, §10.2–10.3; ADR-0007, ADR-0009, ADR-0011, ADR-0016; invariantes I-23, I-24, I-25, I-38 |

## Contexto

Índices no transaccionales, embeddings, resúmenes, agrupaciones, relaciones inferidas, cachés y optimizaciones aprendidas son **artefactos derivados**: se calculan a partir de datos fuente y pueden quedar obsoletos. ADR-0007 cubre la frescura de índices y ADR-0011 los metadatos de embeddings; falta un contrato común para todos los derivados que diga cómo se versionan, cuándo quedan obsoletos y cómo se reconstruyen.

## Opciones consideradas

1. **Contrato por tipo de derivado**, cada uno con sus reglas. Flexible, pero repite lógica de procedencia y supresión en cada sitio.
2. **Registro común de derivados** (propuesta): todo derivado lleva un descriptor con fuentes (unidad y revisión exactas o rango de secuencia), método o modelo y versión, parámetros que afectan al resultado, fecha, hash de la entrada, estado (`ready`, `building`, `stale`, `failed`, `superseded`) y si es reconstruible.
3. **Derivados inmutables solo por recálculo completo**. Simple, pero inviable para embeddings y resúmenes costosos.

## Criterios de decisión

1. Cada derivado se puede vincular a sus fuentes y marcar obsoleto cuando cambian (I-38, I-25).
2. Un derivado reconstruible produce el mismo resultado que uno nuevo sobre los mismos datos, salvo métodos no deterministas declarados como tales (I-24).
3. Suprimir una fuente invalida o suprime sus derivados según ADR-0009.
4. Un resumen nunca reemplaza el original cuando se requiere auditoría (spec §10).
5. No se mezclan modelos ni dimensiones incompatibles (spec §11).

## Evidencia

Ninguna todavía. M0 y la Fase 1 no generan derivados de este tipo.

## Decisión

Pendiente. Propuesta: opción 2. Los derivados generados por modelos externos se producen detrás de adaptadores o jobs fuera del núcleo transaccional (I-31); el núcleo solo guarda el resultado y su descriptor. Un cambio de modelo o versión crea derivados nuevos (`superseded` para los anteriores) en lugar de reescribirlos. La exportación lógica distingue datos fuente de derivados reconstruibles (spec §14).

## Consecuencias

- El descriptor es parte del formato lógico y se versiona (ADR-0003, ADR-0008).
- Detectar obsolescencia exige que la escritura de una fuente sea barata de cruzar con sus derivados: hace falta un índice inverso fuente → derivados.
- Los métodos no deterministas (por ejemplo, algunos modelos de embeddings) deben declarar el hash de la salida, no solo el de la entrada, para la verificación.

## Condiciones para reabrir

Un tipo de derivado cuyo coste de descriptor o de índice inverso sea desproporcionado frente al derivado mismo.

## Seguimiento

- Aplicar el contrato a los índices de la Etapa 4 y a las optimizaciones de la Etapa 6 antes de los embeddings de la Etapa 8.
- Prueba de extremo a extremo: modificar y suprimir una fuente y comprobar el estado de cada derivado.
