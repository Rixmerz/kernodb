# ADR-0007 — Consistencia y reconstrucción de índices

| Campo | Valor |
|---|---|
| Estado | Aceptado |
| Fecha | 2026-10-27 (apertura); 2026-11-04 (aceptado por el propietario, E4-00) |
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

## Decisión (2026-11-04)

Aceptada la opción recomendada: el índice primario es consistente con el commit; los índices secundarios son derivados asíncronos con estado (`ready`/`building`/`stale`/`failed`) y watermark de secuencia. I-23 e I-24 se promueven en E4-03.

## Evidencia (E4-03, 2026-11-05)

E4-03 implementa el índice primario de `kerno-units` ([`index.rs`](../../crates/kerno-units/src/index.rs), [units-v1.md](../file-format/units-v1.md) §3.9–§3.10) con el estado `ready`/`building`/`stale`/`failed` y el watermark que esta decisión pide. Lo que se decidió al implementarlo:

- **El watermark es la secuencia de commit del `ChangeSet` que escribió las entradas.** Ese `ChangeSet` (propósito `Index`) no cambia ninguna unidad, así que las entradas son función del estado en ese mismo commit y la capa puede escribir su propia secuencia (se predice, como en cualquier `ChangeSet`). Comparar con `Snapshot::seq()` es entonces exacto: `Ready` si son iguales, `Stale` si la instantánea es posterior.
- **Cualquier commit posterior deja el índice `Stale`**, también uno que no toca ninguna unidad (una política, una relación). Es conservador a propósito: una consulta nunca declara una frescura que no puede demostrar. `UnitSnapshot::query` cae a la ruta de referencia y dice por qué; `indexed_scan` se niega.
- **`Ready` y `Failed` se guardan; `Stale` se calcula; `Building` es del manejador.** Una construcción es un solo `ChangeSet` atómico, así que un estado `Building` guardado no existiría nunca de forma veraz tras una caída.
- **La construcción lee solo las cabezas.** El daño más profundo es de `verify_units` (K-E3-7) y cada unidad que devuelve una consulta por índice se lee y verifica como en `get_unit`.

**Punto abierto para el propietario.** Esta decisión dice que el índice primario es «consistente con el commit». E4-03 lo implementa como derivado con frescura explícita, porque el acta (§3, fila E4-03) y la invariante I-23 piden el estado y el watermark para él. La consecuencia es que, tras cualquier escritura, las consultas usan la ruta de referencia hasta la siguiente `rebuild_index`. Si un workload medido lo justifica, el siguiente paso es mantenerlo dentro de cada `ChangeSet` (siempre `Ready`, con el coste de escritura que K-E3-9 ya señala) o reconstruirlo en segundo plano; el formato y el estado ya lo admiten. Propuesta: decidirlo en E4-04, con un workload escrito, y no antes.

## Evidencia (E4-04, 2026-11-05)

El workload W1 ([etapa4-workloads.md](../architecture/etapa4-workloads.md)), escrito antes de medir, justificó un índice secundario por `(unit_type, schema_version)`: con 100 000 unidades y selectividad del 1 %, la ruta primaria quedaba a 4,8–12,3× de la cota ideal con p95 de 57–71 ms. `IDX_TYPE` ([units-v1.md](../file-format/units-v1.md) §3.11) es derivado como dice esta decisión y comparte generación con `IDX_PRIMARY`: misma construcción, misma transacción, mismo `IDX_META`, mismo estado y watermark. W1 mide lecturas sobre un índice `Ready`; no mide escrituras intercaladas, así que **no** aporta evidencia para mantener los índices dentro de cada `ChangeSet`. El punto abierto de arriba sigue abierto, a la espera de un workload que lo mida.

## Decisión del propietario sobre el punto abierto (2026-11-05)

Se aprueba la implementación de E4-03/E4-04: los índices de unidades (`IDX_PRIMARY`, `IDX_TYPE`) siguen siendo **derivados** con frescura explícita y solo los escribe `rebuild_index`; tras una escritura, `query` usa la ruta de referencia hasta la siguiente reconstrucción (I-32). Mantenerlos dentro de cada `ChangeSet` o reconstruirlos en segundo plano **no** se hace sin un workload escrito antes de medir que incluya escrituras intercaladas y muestre que la ruta de referencia no basta. Los índices de adyacencia de relaciones no cambian: son transaccionales (ADR-0014, I-21).

**Límite conocido.** La construcción guarda todas las entradas en memoria y las escribe en un solo lote, así que está limitada por los límites del lote del motor (1 Mi operaciones y 64 MiB por defecto); por encima falla con `ChangeSetTooLarge` y el estado queda `Failed`.

