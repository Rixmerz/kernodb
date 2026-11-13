# Capa de unidades v2: datos derivados persistidos

| Campo | Valor |
|---|---|
| Estado | Normativo desde E8-04. Copia las secciones A–D de [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md), que manda si hay una discrepancia |
| Versión de la capa | `layer_version = 2` (característica `DERIVED_VECTORS`), `envelope_version = 1`. La [versión 3](units-v3.md) (identificadores públicos) la incluye y eleva el máximo conocido |
| Relacionado con | [capa de unidades v1](units-v1.md) (no se edita; este documento solo añade), [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md), [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md), [ADR-0008](../adr/0008-serializacion-documental-y-hashes.md) (BLAKE3-256), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) (formato v1 del motor, congelado), [formato v1 del motor](v1.md) |

Este documento especifica, byte a byte, lo que la capa de unidades añade en la versión 2: los derivados persistidos (hoy, vectores). Como en la versión 1, son **claves y valores ordinarios** del motor: no cambia ningún byte del formato v1 del motor ([v1.md](v1.md)), ni su lista de bits de característica (`KNOWN_INCOMPAT` no cambia). La frontera de compatibilidad es la marca de la capa, no el motor. Una implementación y el decodificador independiente `spec_decoder_agrees_derived_v2` (`crates/kerno-units/tests/derived_vectors.rs`) se escriben **solo** desde este texto y el de la versión 1. Las palabras «debe» y «no debe» son normativas.

## 1. Convenciones

Las de [units-v1.md](units-v1.md) §1, sin cambios: enteros **big-endian**, `str(max)` = `len: u32` ‖ bytes UTF-8, la cabecera común de dos bytes (`envelope_version = 1`, `reserved_flags = 0`; UNIT-6, UNIT-7, UNIT-8), `unit_id` de 16 bytes. Hay **una excepción**, explícita porque es lo que hace el código de E8-02 y E8-03: las **preimágenes de los hashes** (§5) usan enteros **little-endian**.

## 2. Qué es la versión 2 de la capa

- La marca `LAYER_META "layer"` (units-v1 §3.1) lleva `layer_version = 2` **si y solo si** la base tiene o tuvo un derivado persistido. Una base sin ninguno sigue en `layer_version = 1` y la abre cualquier build de capa 1 o 2.
- La versión 2 se escribe **en la misma transacción** que la primera clave `0x60`–`0x62`. **Nunca se baja**: ni `set_policy`, ni ningún `ChangeSet` posterior, ni el borrado de todos los derivados. Quien reescribe la marca conserva su `history_policy`.
- Un lector de capa 1 lee la marca antes que ninguna otra clave y la rechaza: `IncompatibleLayer` con la regla UNIT-16 y `found_layer_version = 2`. No escribe nada. Un lector de esta versión rechaza `layer_version > 2` con la misma regla. (Desde E9-02, la [versión 3](units-v3.md) eleva el máximo a 3: la fila `mark_unit16_v3` de §8 pasó a `mark_unit16_v4`, y los lectores de capa 2 rechazan una marca 3.)
- Una marca con `layer_version = 1` y claves `0x60`–`0x62` es corrupción (DV-11).

## 3. Claves

| `tag` | Nombre | Campos tras el tag | Longitud |
|---|---|---|---|
| `0x60` | `DERIVED_DESC` | `derived_id[32]` | 33 |
| `0x61` | `DERIVED_VEC` | `derived_id[32]` | 33 |
| `0x62` | `DERIVED_SRC` | `unit_id[16]` ‖ `rev_no: u64` ‖ `derived_id[32]` | 57 |

- `derived_id` es el identificador de §5.1. Cada derivado es un descriptor, como mucho un vector y exactamente una entrada `DERIVED_SRC`.
- `DERIVED_SRC` ordena por unidad y luego por revisión: «todos los derivados de una unidad» es el rango con prefijo `0x62 ‖ unit_id`, y «todos los de una revisión» el de `0x62 ‖ unit_id ‖ rev_no`.
- `rev_no = 0` en una clave `0x62` es UNIT-5. Una longitud distinta es DV-3.
- Tags `0x63`–`0x6F`: reservados. Una capa 2 que los encuentre los rechaza con UNIT-2 (`IncompatibleLayer`). En una capa 1, todo el nibble `0x6` cae en UNIT-2.

## 4. Valores

### 4.1 `DERIVED_DESC` (`0x60`)

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `kind` | `u8` | `1` = `Vector`. `0` y `2..=255`: DV-1 |
| 4 | `state` | `u8` | `1` = `Ready`, `2` = `Stale`, `3` = `Superseded`. Los demás: DV-2 |
| 5 | `source_unit` | `[u8; 16]` | ninguna |
| 6 | `source_rev` | `u64` | ≥ 1 (DV-4) |
| 7 | `method` | `str(64)` | 1..=64 B, UTF-8 (DV-4) |
| 8 | `method_version` | `u32` | ninguna |
| 9 | `profile` | `str(64)` | 1..=64 B, UTF-8 (DV-4). Perfil de modelo, texto opaco |
| 10 | `param_count` | `u8` | 0..=16 (DV-5) |
| 11 | `params` | `param_count` × (`key: str(64)` ‖ `value: str(256)`) | `key` 1..=64 B, `value` 0..=256 B, claves en **orden creciente de bytes**, sin repetidas, UTF-8 (DV-5) |
| 12 | `input_hash` | `[u8; 32]` | §5.2 |
| 13 | `output_hash` | `[u8; 32]` | §5.3 |

Sin bytes sobrantes (UNIT-10). **Precedencia de las longitudes:** una longitud de texto fuera de su máximo (o cero donde no se admite) es la regla DV de su campo (DV-4, DV-5), no UNIT-11; si la longitud es válida y faltan bytes, UNIT-9. Dentro de un parámetro se evalúa la clave, luego el valor, luego el orden respecto de la clave anterior. El descriptor **no lleva fecha**: la de un derivado es el `created_at` del `CHANGESET` de su commit.

### 4.2 `DERIVED_VEC` (`0x61`)

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `dim` | `u32` | 1 ≤ `dim` ≤ 4096 (DV-6) |
| 4 | `elem_type` | `u8` | `1` = `f32` IEEE 754 binary32. Otro valor: DV-7. Un tipo nuevo exige una versión de capa nueva |
| 5 | `data` | `dim` × 4 B | cada elemento con sus bits en **big-endian**; finito (DV-8) |

Longitud total: `7 + 4·dim` bytes; otra longitud es DV-6. `NaN`, `+∞`, `−∞` y `−0.0` (bits `0x80000000`) son DV-8: el escritor escribe `−0.0` como `+0.0` para que el hash de salida no dependa del signo del cero.

### 4.3 `DERIVED_SRC` (`0x62`)

Dos bytes: `envelope_version = 1`, `reserved_flags = 0`. Sin más campos (UNIT-10). La clave lleva la fuente; el valor no repite nada.

### 4.4 Marca de capa

El valor de units-v1 §3.1 con `layer_version = 2` (`u32` big-endian). Mismo registro, sin cambio de forma. UNIT-15 y UNIT-16 de la versión 1 se leen así: `0` es UNIT-15, y un valor mayor que el máximo que conoce el lector es UNIT-16 con el valor encontrado.

### 4.5 `CHANGESET`, propósito `5` (`Derived`)

`purpose = 5` lo escribe la capa para un `ChangeSet` que solo escribe claves `0x60`–`0x62`, sin miembros, igual que `Relation` (3) e `Index` (4). Un lector de capa 1 lo rechaza como fuera de la tabla (UNIT-13). Con `member_count ≠ 0` es corrupción (DV-13). Un `ChangeSet` que escribe unidades y derivados es de propósito `Application`.

## 5. Identidad y hashes

Todas las funciones usan BLAKE3-256 (ADR-0008). `F(x)` es el campo de longitud `u64` **little-endian** (8 bytes) seguido de `x`.

### 5.1 `derived_id` (clave de `0x60`, `0x61` y `0x62`)

```
derived_id = BLAKE3-256(
    "kerno.derived.identity.v2"                      -- ASCII, sin F(·)
    ‖ F(u64le(n))                                    -- n = número de fuentes (1 en vectores)
    ‖ por cada fuente, en orden (unit, rev):
        F(unit[16]) ‖ F(u64le(rev))
    ‖ F(method) ‖ F(u32le(method_version))
    ‖ F(profile)
    ‖ F(u64le(m))                                    -- m = número de parámetros
    ‖ por cada parámetro, en orden creciente de clave:
        F(key) ‖ F(value)
)
```

Fuera del identificador: `input_hash`, `output_hash` y la fecha. El dominio `…v1` de E8-02 (sin `profile`) nunca se persistió.

### 5.2 `input_hash` (sin `F(·)`, little-endian)

```
input_hash = BLAKE3-256(
    "kerno.derived.input.v1"                          -- ASCII
    ‖ u64le(n)                                        -- n = número de fuentes
    ‖ por cada fuente, en orden (unit, rev):
        unit[16] ‖ u64le(rev)
)
```

### 5.3 `output_hash`

`BLAKE3-256` de todos los bytes del valor `DERIVED_VEC` (`7 + 4·dim` bytes, cabecera incluida). Lo fija el escritor; el lector lo comprueba antes de servir el vector.

## 6. Reglas de validación

Orden de evaluación, como en units-v1 §6: clave, envelope, campos en orden, reglas entre campos. Las reglas UNIT-6 a UNIT-10 se aplican a los valores nuevos.

| Id | Regla | Clase |
|---|---|---|
| DV-1 | `kind ∉ {1}` | `Corruption` |
| DV-2 | `state ∉ {1, 2, 3}` | `Corruption` |
| DV-3 | Longitud de clave ≠ 33 (`0x60`, `0x61`) o ≠ 57 (`0x62`) | `Corruption` |
| DV-4 | `source_rev = 0`; `method` o `profile` vacío, mayor de 64 B o no UTF-8 | `Corruption` |
| DV-5 | `param_count > 16`; clave vacía o mayor de 64 B; valor mayor de 256 B; claves no crecientes o repetidas; no UTF-8 | `Corruption` |
| DV-6 | `dim = 0`, `dim > 4096`, o longitud del valor ≠ `7 + 4·dim` | `Corruption` |
| DV-7 | `elem_type ≠ 1` | `Corruption` |
| DV-8 | Elemento `NaN`, `±∞` o `−0.0` | `Corruption` |
| DV-9 | Vector o entrada `0x62` sin descriptor; descriptor sin entrada `0x62`; o entrada `0x62` cuyo `unit`/`rev` no son los del descriptor | `Corruption` |
| DV-10 | El `derived_id` recalculado (§5.1) ≠ la clave | `Corruption` |
| DV-11 | Claves `0x60`–`0x62` en una base cuya marca tiene `layer_version = 1` | `Corruption` |
| DV-12 | `Ready` sin vector, o con `output_hash` distinto del hash del valor; `Stale` o `Superseded` con vector | `Corruption` |
| DV-13 | `purpose = 5` con `member_count ≠ 0`. La otra mitad de ADR-0020 (un `ChangeSet` de propósito `Derived` que toca claves de unidades o relaciones) no se puede ver desde los datos, porque las claves no llevan el commit que las escribió: la capa no escribe esa forma | `Corruption` |
| DV-14 | Un descriptor cuya unidad fuente no existe, o cuya cabeza está en `Deleted` | `Corruption` |
| UNIT-2 (extendido) | Tags de nibble `0x4`–`0xF` fuera de `0x40`–`0x42`, `0x50`–`0x52` y `0x60`–`0x62` | `IncompatibleLayer` |
| UNIT-16 (extendido) | `layer_version` mayor que el que conoce el lector (2 en esta versión, 1 en una capa 1) | `IncompatibleLayer` |

Un descriptor que falla DV-10 (o cualquier regla de su valor) no se carga: no se sirve su vector ni ocupa el registro. Sus claves hijas quedan huérfanas y no se informan por separado.

## 7. Atomicidad, reapertura y supresión (normativo)

1. Descriptor, vector, entrada `0x62` y, si hace falta, la marca de capa, se escriben en **una** transacción del motor. Escribir un derivado solo es válido si su revisión fuente es la cabeza de su unidad en esa transacción; si no, se rechaza (`InvalidArgument`) y no se escribe nada.
2. Un cambio de fuente (cualquier revisión que no sea `Delete`) recorre `0x62 ‖ unit_id` y, para cada descriptor `Ready`, escribe `state = Stale` y borra su vector, en la misma transacción. `Stale` y `Superseded` no se tocan.
3. Un borrado de unidad, y una retención que purga una revisión (con el rango `0x62 ‖ unit_id ‖ rev_no` de las revisiones purgadas), borran descriptores, vectores y entradas `0x62` de esos derivados en la misma transacción. No los dejan como obsoletos.
4. Abrir no escribe. El registro sale de los descriptores; un `Ready` cuya fuente ya no es la cabeza (o cuyo `input_hash` no cuadra), o cuyo vector falta, no se decodifica o no cuadra con su `output_hash`, se sirve como `Stale` y no se sirve su vector. Reparar es una llamada explícita que escribe un `ChangeSet` de propósito `Derived`.
5. Sobrevive en disco hasta la compactación lo que ADR-0009 (opción 3) declara; no hay borrado físico ni criptográfico. Los vectores van en claro.

## 8. Vectores

`crates/kerno-units/tests/derived_vectors.rs` (`t01_vectors_of_the_document_match_the_codec_byte_for_byte`) construye cada fila con el códec de producción y la compara **byte a byte** con esta tabla, y decodifica el hex de esta tabla y comprueba el resultado. `ok` es un valor que se decodifica y se vuelve a codificar a los mismos bytes; otro valor es la regla que lo rechaza. Los tipos `mark1` y `changeset1` se leen con el lector de capa 1 (el de una build anterior a E8-04); `mark` y `changeset` con el de esta versión. Entradas comunes: unidad `0102…10`, revisión 2, método `embed` versión 3, perfil `model-a`, parámetros `dim=4` y `norm=l2`, vector `[1.0, −2.5, 0.0, 3.75]`.

Constantes de esos vectores:

| Constante | Valor |
|---|---|
| `derived_id` | `bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` |
| `input_hash` | `0c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5d` |
| `output_hash` | `a23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` |
| `vector_value` | `010000000004013f800000c02000000000000040700000` |

Se listan como `` `derived_id` ``, `` `input_hash` ``, `` `output_hash` `` y `` `vector_value` ``; una prueba (`t01_hashes_of_the_document`) las recalcula.

| Nombre | Tipo | Bytes (hex) | Resultado |
|---|---|---|---|
| `key_desc` | key | `60bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `key_vec` | key | `61bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `key_src` | key | `620102030405060708090a0b0c0d0e0f100000000000000002bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `desc_ready` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_stale` | desc | `010001020102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_superseded` | desc | `010001030102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_no_params` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61000c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `vec_dim4` | vec | `010000000004013f800000c02000000000000040700000` | ok |
| `vec_dim1` | vec | `010000000001013f000000` | ok |
| `src_entry` | src | `0100` | ok |
| `mark_v2` | mark | `01000000000202` | ok |
| `changeset_derived` | changeset | `010000060a24181e4000000000000500000000000000000000000000000000` | ok |
| `key_dv3_short` | key | `60bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0f` | DV-3 |
| `key_dv3_src_short` | key | `620102030405060708090a0b0c0d0e0f100000000000000002bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0f` | DV-3 |
| `key_unit5_rev0` | key | `620102030405060708090a0b0c0d0e0f100000000000000000bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | UNIT-5 |
| `key_unit2_reserved` | key | `63bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | UNIT-2 |
| `desc_unit6_env0` | desc | `000001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-6 |
| `desc_unit7_env2` | desc | `020001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-7 |
| `desc_unit8_flags` | desc | `010101010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-8 |
| `desc_dv1_kind0` | desc | `010000010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-1 |
| `desc_dv1_kind2` | desc | `010002010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-1 |
| `desc_dv2_state0` | desc | `010001000102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-2 |
| `desc_dv2_state4` | desc | `010001040102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-2 |
| `desc_dv4_rev0` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000000000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_empty` | desc | `010001010102030405060708090a0b0c0d0e0f1000000000000000020000000000000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_65` | desc | `010001010102030405060708090a0b0c0d0e0f100000000000000002000000416d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d00000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_not_utf8` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005ff6d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_profile_empty` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d6265640000000300000000020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv5_count17` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61110000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_key_empty` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000064696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_not_increasing` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d6102000000037a696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_repeated_key` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d00000001340000000364696d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_value_257` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d000001017676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_count_255` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61ff0000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_unit9_truncated` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6ca` | UNIT-9 |
| `desc_unit10_trailing` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab400` | UNIT-10 |
| `vec_dv6_dim0` | vec | `010000000000013f800000c02000000000000040700000` | DV-6 |
| `vec_dv6_dim4097` | vec | `010000001001013f800000c02000000000000040700000` | DV-6 |
| `vec_dv6_length` | vec | `010000000004013f800000c020000000000000407000` | DV-6 |
| `vec_dv7_elem2` | vec | `010000000004023f800000c02000000000000040700000` | DV-7 |
| `vec_dv8_nan` | vec | `010000000004017fc00000c02000000000000040700000` | DV-8 |
| `vec_dv8_inf` | vec | `010000000004017f800000c02000000000000040700000` | DV-8 |
| `vec_dv8_negative_zero` | vec | `010000000004013f800000c02000008000000040700000` | DV-8 |
| `vec_unit9_short` | vec | `0100000000` | UNIT-9 |
| `vec_unit7_env2` | vec | `020000000004013f800000c02000000000000040700000` | UNIT-7 |
| `src_unit10_trailing` | src | `010000` | UNIT-10 |
| `src_unit8_flags` | src | `0101` | UNIT-8 |
| `src_unit9_empty` | src | `` | UNIT-9 |
| `mark_unit16_v4` | mark | `01000000000402` | UNIT-16 |
| `mark_v1_reader_unit16` | mark1 | `01000000000202` | UNIT-16 |
| `mark_unit15_v0` | mark | `01000000000002` | UNIT-15 |
| `changeset_v1_reader_unit13` | changeset1 | `010000060a24181e4000000000000500000000000000000000000000000000` | UNIT-13 |

## 9. Historial

| Fecha | Cambio |
|---|---|
| 2026-11-12 | E8-04: primera versión, desde ADR-0020 §A–§D. |
