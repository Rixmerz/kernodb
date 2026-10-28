# ADR-0017 — Semántica de la «unidad neuronal»

| Campo | Valor |
|---|---|
| Estado | Propuesto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §2, §3, §6.1, §10, §21; SAD §5; ADR-0004, ADR-0005, ADR-0008, ADR-0014, ADR-0016; invariantes I-22, I-39 |

## Contexto

La especificación usa el cerebro como inspiración (§3) y advierte que las correspondencias son hipótesis de diseño, no equivalencias científicas, y que los módulos se definen por contratos técnicos y no por nombres anatómicos. Hace falta fijar qué significa «unidad» en el modelo de datos y qué significan los términos inspirados en el cerebro (conexión, plasticidad, consolidación, olvido), para que nadie los implemente con otra semántica.

## Opciones consideradas

1. **Unidad = documento con identidad y ciclo de vida** (propuesta). La «unidad neuronal» es solo un nombre de la metáfora; técnicamente es una `Unit` (spec §6.1): identidad estable, tipo, versión de esquema, revisión vigente, estado de ciclo de vida, metadatos y referencia de contenido. Las «conexiones» son `Edge` (ADR-0014); la «plasticidad», ajustes de estadísticas o prioridades (ADR-0016); la «consolidación», artefactos derivados con procedencia (ADR-0018).
2. **Unidad con estado dinámico propio** (activación, pesos que cambian al leer). Rechazada en la propuesta: haría que las lecturas muten datos (spec §2) y mezclaría aprendizaje operacional con contenido (spec §10).
3. **Sin término «unidad»**, solo «documento» como en el SAD. Pierde la generalidad que pide la especificación (documento, recuerdo, evento, hecho, recurso).

## Criterios de decisión

1. Cada término de la metáfora se define con un contrato técnico verificable y un caso de uso (spec §2).
2. Ninguna lectura cambia datos de unidad.
3. Compatibilidad con el modelo documental del SAD (§5) y con el historial de revisiones (ADR-0004).

## Evidencia

No aplica una medición: es una decisión de vocabulario y contrato. Se valida con ejemplos y pruebas cuando exista la API de la Etapa 3.

## Decisión

Pendiente. Propuesta: opción 1, con estas reglas:

- `Unit` equivale al documento del SAD; `unit_type` es la colección o tipo lógico.
- `lifecycle_state` admite al menos `active`, `archived` y `deleted`; las transiciones las define ADR-0009.
- Un término de la metáfora no da nombre a crates ni módulos (spec §3, §13).
- El «olvido» nunca borra contenido por baja frecuencia de acceso; solo políticas explícitas de retención (spec §10).
- Toda «conexión» inferida guarda origen y confianza y no se presenta como declarada (I-39).

## Consecuencias

- La documentación y la API usan `Unit`, `Revision`, `ChangeSet` y `Edge`; los términos cerebrales solo aparecen en la documentación de visión.
- Un mecanismo inspirado en el cerebro que no tenga caso de uso y prueba de valor no entra en el producto.

## Condiciones para reabrir

Un caso de uso validado que necesite estado dinámico por unidad; en ese caso se modelaría como dato derivado, no como parte de la unidad.

## Seguimiento

- Aplicar el vocabulario en ADR-0010 (API pública) al iniciar la Etapa 3.
