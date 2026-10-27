# ADR-0007 — Consistencia y reconstrucción de índices

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §9, D-07, invariantes I-21, I-23, I-24 |

## Contexto

Los índices secundarios y derivados deben ser coherentes con el contenido fuente o declarar explícitamente su frescura.

**Pregunta:** Qué índices participan en transacciones y cuáles son derivados.

## Opciones consideradas

1. **Transaccionales**: se actualizan en el mismo batch atómico que el contenido.
2. **Derivados asíncronos**: se actualizan después, con estado (`ready`/`building`/`stale`/`failed`) y watermark de secuencia.
3. **Mixto por definición de índice** (propuesta).

## Propuesta inicial (SAD §15)

Índice primario consistente con el commit; índices secundarios críticos con frontera atómica clara; índices costosos con estado de frescura y reconstrucción.

## Criterios de decisión

Los índices reconstruidos producen resultados equivalentes a un índice nuevo sobre los mismos datos.

## Lo que aportó M0

El contrato del spike (batch atómico + secuencia de commit monotónica y persistente) es suficiente para ambos modos: un índice transaccional son claves adicionales en el mismo batch; un índice derivado guarda su watermark como una clave más y compara con `Snapshot::seq()`.

## Dependencias

Depende de ADR-0002 (secuencia de commit como watermark).

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
