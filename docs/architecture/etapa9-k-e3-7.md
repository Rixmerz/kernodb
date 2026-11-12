# Revisión de K-E3-7: lecturas de `kerno-units` frente a `verify_units`

**Paquete:** E9-16 de la [acta de la Etapa 9](etapa9-acta.md) (§3, fila E9-16; H-13 de §9; D20 (iii) de §8.1).
**Base de la revisión:** commit `a2248ab` (HEAD al empezar el paquete).
**Alcance:** solo lectura de código. No hay cambio de código, ni de pruebas, ni de dependencias. Ninguna prueba se ejecutó: los tests citados se leyeron, no se corrieron. Las afirmaciones marcadas *inferencial* se deducen del código y están pendientes de una prueba que las confirme (§5).

## 1. Qué es K-E3-7

[Cierre de la Etapa 3](etapa3-cierre.md), tabla de límites, fila K-E3-7: «Las lecturas comprueban menos de lo que comprueba `verify_units`. Por ejemplo, un `REV_META` intermedio dañado solo se ve cuando una lectura pasa por él». Quedó **aceptado por diseño** el 2026-11-04, con la recomendación de revisarlo en la Etapa 9. El diseño que se acepta es: las lecturas comprueban lo que tocan, y `verify_units` lo recorre todo.

Esta revisión responde a una sola pregunta: para cada lectura pública o interna de `kerno-units` que devuelve datos de un snapshot, qué comprueba, qué comprueba `verify_units` que ella no, y si la diferencia es la que el diseño acepta o es un defecto.

**Qué es `verify_units`:** `verify_snapshot` (`crates/kerno-units/src/verify.rs:1141`), que recorre cada clave de un snapshot y aplica las reglas UNIT-1 a UNIT-38 (cabecera de `verify.rs:1-60`). Es de solo lectura (I-18).

## 2. Método

Se leyó cada función de lectura citada en §3 hasta su cuerpo, y las reglas de `verify.rs` que cubren lo mismo. Las líneas se obtuvieron con `grep -n` sobre el árbol de `a2248ab`. Los dos tests de daño que aparecen en §5 se leyeron para ver qué afirman sobre la lectura, no sobre `verify_units`.

No se revisaron, y quedan fuera de esta revisión: la lógica de decisión del planificador (`planner.rs` salvo `query`, `query_relations`, `check_units`, `check_relations`, `check_adjacent`, `execute_relations`), la caché de resultados (`cache.rs`), la retención (`retention.rs`), la escritura de vectores (`vector_store.rs` salvo `vector` y la carga), y la parte final de `unit_at_head` (`history.rs:487` en adelante).

## 3. Tabla de lecturas

Leyenda de la última columna: **de diseño** = lo que el diseño acepta y se documenta; **K** = hallazgo con número en §4.

| # | Lectura (archivo:línea) | Qué comprueba | Qué comprueba `verify_units` y la lectura no | Veredicto |
|---|---|---|---|---|
| R1 | `UnitDb::get_unit` (`db.rs:585`), `UnitSnapshot::get_unit` (`db.rs:721`), `load_unit` (`db.rs:403-429`) | `UNIT_HEAD` decodifica (UNIT-8). Su `REV_META` existe (si no, corrupción). Cabeza y `REV_META` de su revisión coinciden en hash, longitud, esquema, metadata, `updated_seq` y ciclo de vida (`db.rs:415-420`, `lifecycle_after`). El `REV_PAYLOAD` de la cabeza se verifica por hash y longitud (`db.rs:425`). | No mira los `REV_META` intermedios (UNIT-31 en el rango, UNIT-32 cadena, UNIT-33 procedencia, UNIT-34 política). No valida `oldest_rev` en `1..=head_rev` (eso lo hace `read_head`, ver R2; ver K3). No mira `created_seq` (UNIT-31). | **De diseño** para los intermedios. K3 para `oldest_rev`. |
| R2 | `get_revision` (`history.rs:552`), `history` (`history.rs:562`), `history_page` (`history.rs:699`); `read_head` (`history.rs:355-367`), `check_bounds` (`history.rs:371`), `read_meta` (`history.rs:385`), `read_payload` (`history.rs:401`) | Cabecera con `oldest_rev` en `1..=head_rev`. Límites: `RevisionNotFound`, `Pruned`, `HistoryUnavailable`. Cada `REV_META` del tramo pedido: clave exacta, decodificación, `commit_seq` estrictamente creciente hasta `updated_seq` (`history.rs:590-612`). Payload verificado de cada revisión pedida. | Los `REV_META` fuera del tramo pedido. UNIT-32 (`parent_rev`, `Create` en la revisión 1) y UNIT-33 (procedencia) no se comprueban en la lectura. | **De diseño**: un `REV_META` intermedio dañado aparece solo cuando la lectura pasa por él. Es exactamente el caso de K-E3-7. |
| R3 | `unit_as_of` (`history.rs:789`), `state_as_of` (`history.rs:799`), `unit_at` (`history.rs:439`), `unit_at_head` (`history.rs:447-486`) | `created_seq` frente al `s` pedido. `updated_seq` frente al `REV_META` de la cabeza (`history.rs:457-464`). Con varias revisiones, búsqueda sobre `commit_seq`. Con `oldest_rev = 1`, `created_seq` frente a la revisión 1 (`history.rs:481-486`). La política se lee con `policy_of`. | Lo mismo que R2 para las revisiones que no toca la búsqueda. UNIT-32 y UNIT-33. | **De diseño**. Mismo límite que R2. |
| R4 | `get_relation` (`db.rs:726`) | El `REL` decodifica (`RelationRecord::decode`). | `created_seq` no posterior al snapshot y extremos `Active` (UNIT-36). Presencia de las dos entradas de adyacencia (UNIT-36). | **K5** (menor). |
| R5 | `relations_of` (`db.rs:741-773`) | Cada entrada del índice de adyacencia del lado pedido tiene su `REL` (`db.rs:763`), y coincide en extremos y tipo (`db.rs:767`). Daño en una entrada → corrupción. | La **entrada del extremo contrario** (`REL_IN` para `Out`, `REL_OUT` para `In`). Un `REL` con una sola de sus dos entradas es invisible desde el lado que la ha perdido, sin error (K1). UNIT-36 sí lo reporta. | **K1**. |
| R6 | `reference_relations` (`reference.rs:94`), `reference_relations_of` (`reference.rs:113`) | Decodifica cada `REL` del snapshot y filtra. | No recorre la adyacencia: es la ruta de referencia que I-32 toma como verdad. Por eso es la comparación de K1: en un almacén con una entrada faltante, R5 y R6 dan respuestas distintas sin que nadie lo señale. | Referencia. |
| R7 | `query` (`planner.rs:434`), `indexed_scan` (`index.rs:337-342`), `scan_type` (`index.rs:362-387`), `scan_entries` (`index.rs:389-413`), `check_units` (`planner.rs:283-297`) | Por unidad devuelta: `get_unit` (R1) y la entrada coincide con la cabeza (ciclo activo, tipo, esquema; `index.rs:372-380`, `index.rs:400-410`). `check_units`: solo activas, que cumplen el filtro, y en orden estrictamente creciente de lo **devuelto**. | La **completitud**: que cada unidad activa que cumple el filtro tenga su entrada `IDX_PRIMARY` o `IDX_TYPE`. Que el número de entradas de `IDX_META` coincida con el recorrido: `index.rs:328` lo devuelve, y el planificador lo usa solo para costes (`planner.rs:236`, `planner.rs:338`). UNIT-37 y UNIT-38 lo verifican. | **K2**. |
| R8 | `indexed_assoc_query` (`assoc.rs:344`), `indexed_traverse` (`assoc.rs:374-392`), `check_adjacent` (`planner.rs:761-774`), `is_mirrored` (`planner.rs:776-788`) | Por salto: `check_relations`, `created_seq` no posterior al snapshot y entrada espejo del otro extremo (`planner.rs:761-774`). | Las relaciones que no llegan al recorrido, por ejemplo una `REL_IN` faltante cuando el recorrido va hacia fuera desde el otro extremo. `planner.rs:758-760` lo declara como límite («not a completeness check»). | **De diseño** en lo declarado. Ver K1 para lo que no se declara. |
| R9 | `query_relations` (`planner.rs:609`), `execute_relations` (`planner.rs:791-812`) | Por la ruta de adyacencia, solo `check_relations` (`planner.rs:800`) y R5. | **No** llama a `check_adjacent` ni a `is_mirrored`. La comprobación de espejo que sí hace la ruta de asociativas (R8) no se aplica a las consultas de relaciones (K1). | **K1**. |
| R10 | `read_input` (`derived_stale.rs:354-358`), `valid_inputs` (`derived_stale.rs:370-372`) | Lee la fuente con `get_unit` (R1). | Una fuente archivada o borrada devuelve `payload = None`, y `and_then` la **omite sin error** (`derived_stale.rs:355-358`). | **Ya asignado**: F-03 de E8, en E9-11 (acta §9). No se reabre aquí. |
| R11 | `open_persisted` (`vector_store.rs:1128`), `load_persisted` (`vector_store.rs:1134`), `vector` (`vector_store.rs:1166`) | Al abrir, `analyze` (`vector_store.rs:720`), la misma análisis que usa `verify_findings` (`vector_store.rs:903-906`). Un vector se sirve solo si su descriptor, fuente y hash de salida coinciden. `vector` devuelve lo que quedó en memoria. | Un daño en disco posterior a la apertura no aparece en `vector` hasta reabrir. | **De diseño**: `vector_store.rs:1124-1127` lo describe («Every `Ready` descriptor … is `Ready` and served»). Inferencial en cuanto a que no hay relectura. |
| R12 | `policy_of` (`db.rs:346-360`), `UnitSnapshot::policy` (`db.rs:685`), `layer_version` (`db.rs:709`), `read_mark` (`db.rs:388`) | La `LAYER_META` de la política o de la marca decodifica con `decode_upto`. Sin marca, cae en el valor por defecto. | Nada relevante: UNIT-29 cubre los nombres y la marca (`verify.rs:763`). | Sin hallazgo. |
| R13 | `change_set` (`history.rs:851`), `change_sets_since` (`history.rs:863`), `change_set_info` (`history.rs:879-915`) | El registro decodifica. `member_count` coincide con el número de `CS_MEMBER` (`history.rs:888`). Ordinales en orden. El motivo es UTF-8. | Que cada miembro coincida con su `REV_META` (UNIT-33, `verify.rs:1078-1140`). | **K4** (menor). |

## 4. Hallazgos

Cada hallazgo tiene su evidencia y su clase. *Inferencial* quiere decir que sale de leer el código; no hay prueba que lo haya reproducido. El estado de todos es **abierto**: ninguno se corrige en E9-16 (acta §3, fila E9-16).

**K1. Una relación con una sola entrada de adyacencia es invisible desde el lado que la pierde.** *Inferencial.*
- Dónde: `db.rs:741-773` (R5), `planner.rs:791-812` (R9).
- Qué pasa: si falta `REL_IN` de una relación, `relations_of(to, In)` no la devuelve (no hay entrada que recorrer), pero `reference_relations_of(to, In)` sí (R6). `query_relations` por la ruta de adyacencia no compara con la referencia, porque `execute_relations` solo aplica `check_relations`. La ruta de asociativas sí compara el espejo (`planner.rs:776-788`), pero solo para las relaciones que su recorrido llega a ver.
- Prueba de daño existente: `crates/kerno-units/tests/relations.rs:534-572`, en la lista `damages` incluye «REL_IN missing». Para ese caso solo se afirma `UNIT-36` (`relations.rs:562`). El bloque de resultados de la lectura tiene `_ => {}` (`relations.rs:568`): **ningún test afirma qué devuelve la lectura** con `REL_IN` faltante.
- Lo que sí cubre el diseño: `planner.rs:758-760` declara que la comprobación de espejo no es completa. No dice que `query_relations` no la aplica.
- Clase de defecto: divergencia de I-32 en un almacén dañado, sin error ni `Fallback`.
- Causa: pre-existente en el código base (no lo introduce ningún cambio de este paquete).

**K2. Una consulta por índice puede devolver menos unidades que la referencia cuando falta una entrada.** *Inferencial.*
- Dónde: `index.rs:389-413` (`scan_entries`), `index.rs:362-387` (`scan_type`), `planner.rs:283-297` (`check_units`), `index.rs:328` (uso del conteo).
- Qué pasa: una unidad activa sin su `IDX_PRIMARY` no se recorre, y ninguna comprobación del camino de consulta lo nota. `check_units` solo mira lo devuelto. El conteo de `IDX_META` no se compara con el recorrido.
- Prueba de daño existente: `crates/kerno-units/tests/index.rs:783-820`. Las dos listas (`type_damages`, `damages`) afirman solo `UNIT-37` o `UNIT-38`. No hay ninguna afirmación sobre lo que devuelve `indexed_scan`.
- Lo que sí declara el código: `index.rs:52-55` dice que «on a damaged store the index path can answer where the reference path … fails». Eso describe el caso en que la ruta de índice responde y la de referencia falla. No dice nada del caso contrario, en que la ruta de índice responde menos y sin error.
- Clase de defecto: igual que K1.
- Cause: pre-existente.

**K3. `get_unit` no valida el rango de `oldest_rev`, que `read_head` sí valida.** *Inferencial.*
- Dónde: `db.rs:403-429` frente a `history.rs:355-367`.
- Qué pasa: `load_unit` no mira `oldest_rev`; `read_head` exige `1 <= oldest_rev <= head_rev`. Un `UNIT_HEAD` con `oldest_rev = 0` se lee con `get_unit` sin error. UNIT-31 lo reporta (`verify.rs:809-830`).
- Consecuencia que se ha comprobado: ninguna en la lectura. Es una diferencia entre dos lecturas de la misma cabecera.
- Clase: menor.

**K4. `change_set_info` no comprueba que los miembros coincidan con su `REV_META`.** *Inferencial.*
- Dónde: `history.rs:879-915`, frente a `verify.rs:1078-1140` (UNIT-33).
- Qué pasa: `change_set` y `change_sets_since` devuelven los miembros con su revisión y su tipo sin cruzarlos con `REV_META`.
- Clase: menor. Es el límite que el diseño acepta («lo que tocan»), y la lectura no lo oculta: solo no lo comprueba.

**K5. `get_relation` no comprueba `created_seq` ni los extremos.** *Inferencial.*
- Dónde: `db.rs:726-733`, frente a UNIT-36 (`verify.rs:694-761`).
- Clase: menor. Se mezcla con K1 si la relación además pierde una entrada.

## 5. Pruebas que faltan para cerrar K1 y K2

Esta revisión no ejecuta nada, así que K1 y K2 no están demostrados. El paquete que los trate debe:

1. Para K1: afirmar, con `REL_IN` faltante, que `relations_of(to, In)` y `query_relations` devuelven el mismo resultado que `reference_relations_of` **o** un error. Hoy `relations.rs:568` no afirma nada.
2. Para K2: con `IDX_PRIMARY` faltante de una unidad activa, afirmar que `indexed_scan` y `reference_filter` coinciden o que `indexed_scan` da error.
3. Cada prueba, en los dos motores (`emulated_log` y `kerno_db`, criterio §4.3 de la acta), vista fallar con una mutación en una copia.

## 6. Propuesta, no decidida

K1 y K2 son lo que la acta llama «defecto encontrado» (§3, fila E9-16: «un defecto encontrado abre un paquete nuevo»). Abrir ese paquete cambia el plan de la etapa, así que lo decide el propietario (D20 (iii), revocable). La propuesta es un paquete nuevo con este criterio: las lecturas de relaciones e índice, con una entrada de adyacencia o de índice faltante, devuelven lo de la referencia o un error; y la prueba afirma el resultado de la lectura, no solo la regla de `verify_units`. K3, K4 y K5 son menores: se pueden anotar en el cierre (E9-CL) o en el mismo paquete.

## 7. Registro de revisión (acta §4.10)

| Hallazgo | Estado | Evidencia |
|---|---|---|
| K1 | Abierto. Propuesto como paquete nuevo | Lectura de código; prueba pendiente (§5) |
| K2 | Abierto. Propuesto como paquete nuevo | Lectura de código; prueba pendiente (§5) |
| K3 | Abierto, menor | Lectura de código |
| K4 | Abierto, menor | Lectura de código |
| K5 | Abierto, menor | Lectura de código |
| F-03 (R10) | Asignado a E9-11 | Acta §9 |

**Revisión independiente:** pendiente. El autor de este documento no puede ser su revisor (§4.10). Este paquete no se fusiona con K1 o K2 abiertos sin decisión del propietario.
