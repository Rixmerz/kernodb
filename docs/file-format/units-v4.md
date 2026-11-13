# Capa de unidades v4: contadores de adyacencia

| Campo | Valor |
|---|---|
| Estado | Normativo desde el paquete P1 posterior a la Etapa 9 (cierra el defecto K1 de [etapa9-k-e3-7.md](../architecture/etapa9-k-e3-7.md)). Decisión de diseño tomada por delegación; revocable |
| Versión de la capa | `layer_version = 4`, `envelope_version = 1` |
| Relacionado con | [units-v1](units-v1.md), [v2](units-v2.md) y [v3](units-v3.md) (no se editan salvo una nota en v3 §3), [formato v1 del motor](v1.md) (congelado, sin cambios), [fixtures](../architecture/etapa9-fixtures.md) §6 |

Este documento especifica, byte a byte, lo que la capa de unidades añade en la versión 4: un registro de conteo por cada `(unidad, dirección, tipo de relación)` que tiene entradas de adyacencia. Como en las versiones anteriores son **claves y valores ordinarios** del motor: no cambia ningún byte del formato v1 del motor. «Debe» y «no debe» son normativos.

## 1. Para qué

El índice de adyacencia (`REL_OUT` `0x41`, `REL_IN` `0x42`) se lee con un recorrido de rango. Una entrada perdida no deja huella, así que, preguntado desde el extremo que la perdió, `relations_of` devolvía una respuesta corta sin error y `query_relations` sin `Fallback` (defecto K1). Nada une una unidad con sus relaciones salvo las propias entradas. El conteo es la marca contra la que el lector compara lo que recorrió.

## 2. Qué es la versión 4 de la capa

Se sigue el precedente de [units-v3.md](units-v3.md) §2:

- La marca `LAYER_META "layer"` lleva `layer_version = 4` **si y solo si** la base tiene o tuvo un registro `0x72`. Una base sin ninguno conserva la marca que tenía (1, 2 o 3).
- La versión 4 se escribe **en la misma transacción** que el primer registro `0x72` y **nunca se baja**. Quien reescribe la marca conserva su `history_policy`. La versión 4 incluye todo lo de la 3.
- **Consecuencia para las builds anteriores.** Un lector de capa 1, 2 o 3 lee la marca antes que ninguna otra clave y rechaza la base completa: `IncompatibleLayer` con la regla UNIT-16 y `found_layer_version = 4`. No escribe nada. Una base con una sola relación creada por esta build deja de abrirse con una build sin esta versión. Un lector de esta versión rechaza `layer_version > 4`.
- Una marca con `layer_version < 4` y claves `0x72` es corrupción (UNIT-46).
- Abrir no escribe nada: la primera relación creada o borrada que escribe un conteo eleva la marca.

## 3. Claves

| Tag | Nombre | Campos tras el tag | Longitud |
|---|---|---|---|
| `0x72` | `REL_COUNT` | `unit_id[16]` ‖ `direction[1]` ‖ `relation_type` (1..=64 B) ‖ `0x00` | 20..=83 |

- `direction`: `0` para las entradas `REL_OUT` de la unidad, `1` para las `REL_IN`. Otro valor es UNIT-12.
- `relation_type` con las mismas reglas que en la clave de adyacencia (UTF-8, sin NUL, 1..=64 bytes: UNIT-12). El NUL final lo separa; falta de NUL final o longitud fuera de rango es UNIT-3.
- El orden de las claves agrupa por unidad y dirección: **todos los conteos de una unidad y dirección son un rango**.
- Tags `0x73`–`0x7F`: reservados (UNIT-2). Las claves quedan fuera de `Key` de `codec.rs`, igual que las de v3; `verify_units` las lee aparte.

## 4. Valores

El valor tiene **10 bytes**: la cabecera común y un `u64` big-endian.

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` (otro valor: UNIT-8) |
| 3 | `count` | `u64` | `>= 1`. Un conteo que llega a cero **se borra**: no se guarda `0` (UNIT-12) |

Sin bytes sobrantes (UNIT-10). Orden de evaluación: la cabecera del valor antes que la clave (UNIT-6, UNIT-7, UNIT-8), después la clave (UNIT-3, UNIT-12), por último la longitud del valor (UNIT-9, UNIT-10) y el rango del conteo (UNIT-12).

## 5. Semántica

**Escritor.** Crear una relación suma 1 al conteo de `(from, Out, tipo)` y de `(to, In, tipo)`; borrarla resta 1 a los mismos. Se hace en la misma transacción que la relación y sus entradas, sobre el conteo **tal como lo ve el conjunto de cambios** (varias operaciones en uno se componen). Si el registro no existe, el escritor cuenta primero las entradas que hay en el rango de ese `(unidad, dirección, tipo)` y parte de ese número (es el caso de una base de antes de la capa 4). Coste: una lectura puntual y una escritura por entrada de adyacencia escrita o borrada; el recorrido de arranque ocurre una sola vez por `(unidad, dirección, tipo)`. La importación lógica escribe las relaciones por el mismo camino y deja los mismos conteos.

**Lector.** `relations_of` lee los conteos que esperan sus entradas y los compara con lo que recorrió, por tipo:

- con un tipo dado: **una lectura puntual** más (el registro de ese tipo);
- sin tipo: **un recorrido de rango** más, sobre los conteos de esa unidad y dirección (uno por tipo distinto, normalmente una sola llamada de recorrido).

Un tipo con registro cuyo número de entradas recorridas es distinto (más o menos, **cero incluido**) es `Corruption`. `query_relations` responde entonces por la ruta de referencia con `Fallback` `Corrupt`, y la respuesta es completa. Un tipo con entradas y sin registro **no es hallazgo**: es indistinguible de una base de antes de la capa 4.

**Verificador** (`verify_units`):

| Id | Regla | Clase |
|---|---|---|
| UNIT-45 | Un registro `0x72` cuyo conteo no es el número de entradas de adyacencia de su unidad, dirección y tipo (también un registro sin ninguna entrada) | `Corruption` |
| UNIT-46 | Registros `0x72` en una base cuya marca tiene `layer_version < 4` | `Corruption` |

UNIT-36 sigue siendo la regla de una relación sin sus dos entradas; UNIT-45 es la que explica por qué la lectura se niega. Un registro que no decodifica es un hallazgo de estructura y no se compara.

## 6. Vectores

`crates/kerno-units/tests/rel_counts.rs` (`vectors_of_the_document_match_the_codec_byte_for_byte`) lee esta tabla, construye las filas `ok` con el escritor de producción y las compara **byte a byte**, y decodifica cada fila negativa y comprueba la regla. Entradas comunes: unidad `0102…10`. La prueba fija los recuentos (3 positivos, 13 negativos, 1 marca positiva y 2 negativas).

| Nombre | Clave (hex) | Valor (hex) | Resultado |
|---|---|---|---|
| `key_out_cites_1` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01000000000000000001` | ok |
| `key_in_cites_2` | `720102030405060708090a0b0c0d0e0f1001636974657300` | `01000000000000000002` | ok |
| `key_out_refines_300` | `720102030405060708090a0b0c0d0e0f1000726566696e657300` | `0100000000000000012c` | ok |
| `key_unit3_tag_only` | `72` | `01000000000000000001` | UNIT-3 |
| `key_unit3_no_type` | `720102030405060708090a0b0c0d0e0f100000` | `01000000000000000001` | UNIT-3 |
| `key_unit3_no_nul` | `720102030405060708090a0b0c0d0e0f10006369746573` | `01000000000000000001` | UNIT-3 |
| `key_unit3_long_type` | `720102030405060708090a0b0c0d0e0f1000616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616100` | `01000000000000000001` | UNIT-3 |
| `key_unit12_direction` | `720102030405060708090a0b0c0d0e0f1002636974657300` | `01000000000000000001` | UNIT-12 |
| `key_unit12_type_nul` | `720102030405060708090a0b0c0d0e0f1000636900657300` | `01000000000000000001` | UNIT-12 |
| `key_unit12_type_not_utf8` | `720102030405060708090a0b0c0d0e0f1000ff00` | `01000000000000000001` | UNIT-12 |
| `value_unit12_zero_count` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01000000000000000000` | UNIT-12 |
| `value_unit6_env0` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `00000000000000000001` | UNIT-6 |
| `value_unit7_env2` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `02000000000000000001` | UNIT-7 |
| `value_unit8_flags` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01010000000000000001` | UNIT-8 |
| `value_unit9_short` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `0100000000` | UNIT-9 |
| `value_unit10_trailing` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `0100000000000000000100` | UNIT-10 |

Marcas de capa. La columna «Lector» es el `layer_version` máximo que conoce el lector:

| Nombre | Bytes (hex) | Lector | Resultado |
|---|---|---|---|
| `mark_v4` | `01000000000402` | `4` | ok |
| `mark_v4_layer3_reader` | `01000000000402` | `3` | UNIT-16 |
| `mark_v4_layer2_reader` | `01000000000402` | `2` | UNIT-16 |

## 7. Límites (normativo: lo que el conteo no ve)

1. **Base escrita antes de la capa 4.** Sus relaciones no tienen conteo hasta que se crea o borra una relación de ese `(unidad, dirección, tipo)`. Hasta entonces la lectura se comporta como antes: una entrada perdida **sigue siendo invisible** desde su extremo. No se escribe nada al abrir (I-18), así que no hay migración en bloque; es decisión de diseño, y una migración tendría que recorrer todas las relaciones.
2. **Doble daño.** Una entrada perdida **junto con** su registro de conteo, o un conteo reescrito para coincidir con la entrada perdida. `verify_units` ve la entrada que falta (UNIT-36) pero no que el conteo desapareció con ella.
3. **Un registro de conteo perdido** con sus entradas intactas no es hallazgo (es el caso 1).
4. Un daño dentro del valor de una entrada lo detecta la comprobación contra su `REL`, como antes.

## 8. Fixture

`crates/kerno-units/tests/vectors/rel-counts-v4/db-legacy-relations/` es una base de `kerno_db` cerrada con tres unidades y tres relaciones tal como las dejaba el escritor **anterior** a esta capa: `REL` y sus dos entradas de adyacencia, sin registro de conteo, marca 1. Se generó escribiendo esos tres registros por relación con el mismo medio de daño que usan las pruebas del crate (`raw_commit`), no con una build anterior: los bytes del escritor antiguo son exactamente esos. `MANIFEST` lista el SHA-256, el tamaño y el nombre de cada fichero. Los bytes no se reproducen entre compilaciones; la integridad la da `MANIFEST`. Se regenera con `cargo test --release -p kerno-units --all-features --test rel_counts -- --ignored regenerate_fixture`. Detalle en [etapa9-fixtures.md](../architecture/etapa9-fixtures.md) §6.

## 9. Historial

| Fecha | Cambio |
|---|---|
| 2026-11-13 | P1: primera versión (registro `0x72`, marca 4, UNIT-45 y UNIT-46). |
