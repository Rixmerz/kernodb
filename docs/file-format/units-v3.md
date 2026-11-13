# Capa de unidades v3: identificadores públicos

| Campo | Valor |
|---|---|
| Estado | Normativo desde E9-02, **hecho en el árbol y no fusionado**. Copia las decisiones de [ADR-0005](../adr/0005-identificadores-publicos.md) (opción 1), que manda si hay una discrepancia. Decisiones D-a a D-f del 2026-11-13, tomadas por delegación expresa del propietario |
| Versión de la capa | `layer_version = 3`, `envelope_version = 1` |
| Relacionado con | [capa de unidades v1](units-v1.md) (no se edita) y [v2](units-v2.md) (editada solo en el encabezado y en una fila de §8, por la nueva marca máxima; este documento solo añade), [ADR-0005](../adr/0005-identificadores-publicos.md), [exportación lógica](export-v0-borrador.md), [formato v1 del motor](v1.md) (congelado, sin cambios) |

Este documento especifica, byte a byte, lo que la capa de unidades añade en la versión 3: el mapeo entre un identificador público (un token opaco de 128 bits) y la unidad que nombra. Como en las versiones 1 y 2, son **claves y valores ordinarios** del motor: no cambia ningún byte del formato v1 del motor ni su lista de bits de característica. La frontera de compatibilidad es la marca de la capa. La implementación (`crates/kerno-units/src/ids.rs`) y la prueba `vectors_of_the_document_match_the_codec_byte_for_byte` (`crates/kerno-units/tests/public_ids.rs`) se escriben solo desde este texto y los de las versiones anteriores. «Debe» y «no debe» son normativos.

## 1. Convenciones

Las de [units-v1.md](units-v1.md) §1: enteros **big-endian**, la cabecera común de dos bytes de los valores (`envelope_version = 1`, `reserved_flags = 0`; UNIT-6, UNIT-7, UNIT-8) y `unit_id` de 16 bytes. Un `token` son 16 bytes (128 bits) de un generador aleatorio del sistema operativo, sin relación con el `unit_id`, el reloj ni el orden de creación.

**Texto del token** (no se guarda; lo muestra y valida la API): `k1_` seguido de 26 caracteres, los 128 bits en base32 RFC 4648, minúsculas, sin relleno. El dígito es la versión del texto (`PUBLIC_ID_TEXT_VERSION = 1`), una constante distinta de la versión del registro (`PUBLIC_ID_RECORD_VERSION`, la `envelope_version` de §4). Los 26 caracteres llevan 130 bits: los dos últimos deben ser cero, y un texto con ellos puestos se rechaza (un texto por id). Orden de validación, de lo más barato a lo más caro y antes de cualquier búsqueda: prefijo `k`, dígitos y `_`; versión; longitud del cuerpo; alfabeto; bits finales. Cada fallo es su propio error (`PublicIdError`).

## 2. Qué es la versión 3 de la capa

Se sigue el precedente de [units-v2.md](units-v2.md) §2 (una capa que añade registros eleva la marca):

- La marca `LAYER_META "layer"` (units-v1 §3.1) lleva `layer_version = 3` **si y solo si** la base tiene o tuvo un registro `0x70`/`0x71`. Una base sin ninguno conserva la marca que tenía (1 o 2): una de marca 1 la abre cualquier build de capa 1, 2 o 3; una de marca 2 la abre cualquier build de capa 2 o 3 (una de capa 1 la rechaza por su marca, units-v2 §2).
- La versión 3 se escribe **en la misma transacción** que el primer registro `0x70`/`0x71`. **Nunca se baja**: ni `set_policy`, ni borrar la unidad con token, ni ningún `ChangeSet` posterior. Quien reescribe la marca conserva su `history_policy`. La versión 3 incluye todo lo de la 2: un lector de capa 3 lee derivados persistidos.
- **Consecuencia para las builds anteriores.** Un lector de capa 1 o 2 lee la marca antes que ninguna otra clave y rechaza la base completa: `IncompatibleLayer` con la regla UNIT-16 y `found_layer_version = 3`. No escribe nada. No hay lectura parcial: una base con un solo token deja de abrirse con una build sin esta versión. Un lector de esta versión rechaza `layer_version > 3` con la misma regla.
- Una marca con `layer_version < 3` y claves `0x70`/`0x71` es corrupción (UNIT-44).
- Abrir no escribe nada: el primer token de una base de capa 1 o 2 es el que eleva la marca.

## 3. Claves

| Tag | Nombre | Campos tras el tag | Longitud |
|---|---|---|---|
| `0x70` | `PUBLIC_ID` | `token[16]` | 17 |
| `0x71` | `PUBLIC_ID_OF` | `unit_id[16]` | 17 |

- `0x70` es el índice `token → unidad`; `0x71` el inverso `unidad → token`, que permite mostrar de nuevo el token de una unidad sin recorrer nada.
- Tags `0x72`–`0x7F`: reservados. Una capa 3 que los encuentre los rechaza con UNIT-2 (`IncompatibleLayer`). En una capa 2 o anterior, todo el nibble `0x7` cae en UNIT-2.
- Las claves quedan fuera de `Key` de `codec.rs`, igual que los derivados persistidos (`0x60`–`0x62`): `verify_units` las lee aparte.

## 4. Valores

Ambos valores tienen **18 bytes**: la cabecera común y 16 bytes.

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` (`PUBLIC_ID_RECORD_VERSION`) |
| 2 | `reserved_flags` | `u8` | `0`. Reservado para un estado futuro de token revocado o reemitido; hoy cualquier otro valor es UNIT-8 |
| 3 | `other` | `[u8; 16]` | en `0x70`, el `unit_id` que el token nombra; en `0x71`, el `token` de la unidad |

Sin bytes sobrantes (UNIT-10).

**Orden de evaluación (excepción declarada a units-v2 §6).** Para estos registros se lee la **cabecera del valor antes que la longitud de la clave**: `envelope_version` (UNIT-6, UNIT-7), `reserved_flags` (UNIT-8), longitud de la clave (UNIT-3) y por último la longitud del valor (UNIT-9, UNIT-10). Así un registro futuro con otra longitud de token se lee como una capa más nueva (UNIT-7), no como daño. Un valor de menos de dos bytes es UNIT-9.

## 5. Reglas de validación

Las reglas por registro reutilizan las de units-v1: UNIT-3 (longitud de clave distinta de 17), UNIT-6, UNIT-7, UNIT-8, UNIT-9, UNIT-10 y UNIT-2 (extendido: tags de nibble `0x4`–`0xF` fuera de `0x40`–`0x42`, `0x50`–`0x52`, `0x60`–`0x62` y `0x70`–`0x71`). UNIT-16 (extendido): `layer_version` mayor que el que conoce el lector (3 en esta versión, 2 en una capa 2, 1 en una capa 1).

Reglas entre registros, comprobadas por `verify_units` (cada una es un hallazgo propio; un registro que no decodifica es un hallazgo de estructura y su pareja no se vuelve a informar como ausente):

| Id | Regla | Clase |
|---|---|---|
| UNIT-39 | Registro `0x70` sin el `0x71` de su unidad | `Corruption` |
| UNIT-40 | Registro `0x71` sin el `0x70` del token que nombra | `Corruption` |
| UNIT-41 | Un par que nombra una unidad sin `UNIT_HEAD` (una unidad borrada lógicamente conserva su cabeza; el borrado quita el par, §7) | `Corruption` |
| UNIT-42 | Dos tokens para una unidad: otro `0x70` nombra la misma unidad, o el `0x71` de la unidad nombra otro token | `Corruption` |
| UNIT-43 | Un token para dos unidades: el `0x70` del token nombra otra unidad, o dos `0x71` nombran el mismo token | `Corruption` |
| UNIT-44 | Registros `0x70`/`0x71` en una base cuya marca tiene `layer_version < 3` | `Corruption` |

Los textos de los hallazgos nombran el registro, nunca un token junto con el `unit_id` de su unidad (D-d). La resolución aplica la misma regla en línea: `resolve_public_id` lee `0x70` y después `0x71`, y `public_id` lee `0x71` y después `0x70`; un par que no coincide es `Corruption`, y ninguna de las dos llamadas devuelve un token que no resuelve (D-c).

## 6. Vectores

`crates/kerno-units/tests/public_ids.rs` (`vectors_of_the_document_match_the_codec_byte_for_byte`) lee esta tabla, construye las filas `ok` con el códec de producción y las compara **byte a byte**, y decodifica cada fila negativa y comprueba la regla. Entradas comunes: unidad `0102…10`, token `000102…0f` (texto `k1_aaaqeayeaudaocajbifqydiob4`). La prueba fija los recuentos (2 positivos, 15 negativos, 1 marca positiva y 3 negativas).

| Nombre | Clave (hex) | Valor (hex) | Resultado |
|---|---|---|---|
| `key_token` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f10` | ok |
| `key_reverse` | `710102030405060708090a0b0c0d0e0f10` | `0100000102030405060708090a0b0c0d0e0f` | ok |
| `key_unit3_token_short` | `70000102030405060708090a0b0c0d0e` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_token_long` | `70000102030405060708090a0b0c0d0e0f00` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_token_empty` | `70` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_reverse_short` | `710102030405060708090a0b0c0d0e0f` | `0100000102030405060708090a0b0c0d0e0f` | UNIT-3 |
| `key_unit3_reverse_long` | `710102030405060708090a0b0c0d0e0f1000` | `0100000102030405060708090a0b0c0d0e0f` | UNIT-3 |
| `value_unit6_env0` | `70000102030405060708090a0b0c0d0e0f` | `00000102030405060708090a0b0c0d0e0f10` | UNIT-6 |
| `value_unit7_env2` | `70000102030405060708090a0b0c0d0e0f` | `02000102030405060708090a0b0c0d0e0f10` | UNIT-7 |
| `value_unit7_env2_other_length` | `70000102030405060708090a0b0c0d0e0f` | `02000102030405060708090a0b0c0d0e0f1000000000` | UNIT-7 |
| `value_unit7_env2_bad_key` | `70000102030405060708090a0b0c0d0e` | `02000102030405060708090a0b0c0d0e0f10` | UNIT-7 |
| `value_unit8_flags` | `70000102030405060708090a0b0c0d0e0f` | `01010102030405060708090a0b0c0d0e0f10` | UNIT-8 |
| `value_unit9_empty` | `70000102030405060708090a0b0c0d0e0f` | `` | UNIT-9 |
| `value_unit9_one_byte` | `70000102030405060708090a0b0c0d0e0f` | `01` | UNIT-9 |
| `value_unit9_envelope_only` | `70000102030405060708090a0b0c0d0e0f` | `0100` | UNIT-9 |
| `value_unit9_short` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f` | UNIT-9 |
| `value_unit10_trailing` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f1000` | UNIT-10 |

Marcas de capa (units-v1 §3.1: `envelope_version`, `reserved_flags`, `layer_version: u32`, política). La columna «Lector» es el `layer_version` máximo que conoce el lector:

| Nombre | Bytes (hex) | Lector | Resultado |
|---|---|---|---|
| `mark_v3` | `01000000000302` | `3` | ok |
| `mark_v3_layer2_reader` | `01000000000302` | `2` | UNIT-16 |
| `mark_v3_layer1_reader` | `01000000000302` | `1` | UNIT-16 |
| `mark_v4_layer3_reader` | `01000000000402` | `3` | UNIT-16 |

## 7. Ciclo de vida, atomicidad y exportación (normativo)

1. **Creación.** El token y sus dos registros se escriben en la transacción de la unidad (`ChangeSet::create_public`); se confirman o desaparecen con ella. La marca sube a 3 en esa misma transacción si estaba por debajo. Si el generador aleatorio falla, no se crea nada.
2. **Borrado (D-a).** Borrar una unidad (`delete`, también a través de `DerivedDb`) quita **ambos** registros en la misma transacción, de modo que su token resuelve después exactamente como uno nunca emitido (`None`). Una retención que poda revisiones antiguas no toca el mapeo de una unidad viva. La capa no tiene otra operación que olvide una unidad.
3. **Exportación e importación (D-b).** La [exportación lógica](export-v0-borrador.md) lleva el token de cada unidad que lo tiene (su `format_version` pasa a 1) y `import` lo restaura en la transacción de la primera revisión conservada de la unidad: un token es estable tras exportar e importar. Un fichero que daría un token a dos unidades se rechaza antes de la primera escritura con `UnitError::PublicIdTaken`.
4. **Sin token.** Una unidad creada con `ChangeSet::create`, o escrita antes de este módulo, no tiene token; `public_id` devuelve `None`. Un token solo se emite al crear la unidad.
5. **Permisos.** Un token válido no autoriza nada (I-27): solo dice qué unidad nombra. La capa que expone Kerno verifica permisos en cada operación.

## 8. Fuente de aleatoriedad (D-e)

Los 16 bytes salen de `/dev/urandom`, leído con `std` (sin dependencias nuevas). Ese fichero existe en Linux, macOS, los BSD y otros Unix. En cualquier otra plataforma (Windows, WASI) abrirlo falla y `create_public` devuelve ese error (`UnitError::Storage(Io)`) sin crear nada; leer y resolver tokens ya emitidos sigue funcionando. Un error de lectura o una lectura corta es un error. Un token de 16 bytes todos iguales (una fuente constante) también se rechaza, como error y sin reintentar, de modo que una fuente rota no puede dejar la creación en un bucle. Solo una colisión con un token ya emitido se reintenta, con un máximo de 4 sorteos. Camino de mejora: `getrandom` si algún día se admite un destino que no sea Unix.

## 9. Fixture

`crates/kerno-units/tests/vectors/public-ids-v3/db-with-token/` es una base de `kerno_db` cerrada con tres unidades: la 1 con token, la 2 sin él y la 3 con token y borrada (sin mapeo). Marca de capa 3. `MANIFEST` lista el SHA-256, el tamaño y el nombre de cada fichero y `TOKEN` el texto del token de la unidad 1. Los bytes no se reproducen entre compilaciones (el motor sortea la identidad de la base y el token sale del sistema operativo): la integridad la da `MANIFEST`. Se regenera con `cargo test --release -p kerno-units --test public_ids -- --ignored regenerate_fixture`. Lo comprueban `fixture_directory_opens_resolves_and_verifies`, `fixture_directory_is_refused_by_a_layer2_reader` y `fixture_manifest_lists_the_committed_files`. El detalle está en [etapa9-fixtures.md](../architecture/etapa9-fixtures.md) §5.

## 10. Historial

| Fecha | Cambio |
|---|---|
| 2026-11-13 | E9-02: primera versión (valor `versión ‖ 16 bytes`, marca sin elevar). |
| 2026-11-13 | Corrección tras revisión independiente, por delegación expresa del propietario (la rama no está fusionada y nadie tiene datos con estos registros, así que el formato en disco se corrige ahora): valor con la cabecera común (B1), marca de capa 3 (B2), comprobaciones entre registros UNIT-39 a UNIT-44 (B3), borrado y exportación (D-a, D-b). |
