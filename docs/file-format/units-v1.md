# Capa de unidades v1: claves y valores

| Campo | Valor |
|---|---|
| Estado | Borrador normativo (E3-01); lo fija el propietario al cerrar E3-01 |
| Versión de la capa | `layer_version = 1`, `envelope_version = 1` |
| Relacionado con | [diseño de la Etapa 3](../architecture/etapa3-design.md) §2, [acta E3-00](../architecture/etapa3-acta-e3-00.md), [ADR-0004](../adr/0004-historial-de-revisiones.md), [ADR-0008](../adr/0008-serializacion-documental-y-hashes.md) (propuesto), [formato v1 del motor](v1.md) |

Este documento especifica, byte a byte, las claves y los valores que la capa de unidades guarda como **claves y valores ordinarios** del motor. No cambia ningún byte del formato v1 del motor ([v1.md](v1.md)): para el motor, todo lo de aquí son bytes opacos. Una implementación de la capa y el decodificador independiente de `crates/kerno-units/tests/spec_decoder` se escriben **solo** desde este texto. Las palabras «debe» y «no debe» son normativas.

## 1. Convenciones

| Tipo | Codificación |
|---|---|
| `u8` | 1 byte |
| `u32`, `u64` | 4 u 8 bytes, **big-endian** (el orden lexicográfico de la clave es el numérico) |
| `i64` | 8 bytes, big-endian, complemento a dos (µs desde la época Unix; descriptivo, sin restricción de valor) |
| `str(max)` | `len: u32` ‖ `len` bytes de UTF-8 válido; `len ≤ max` |
| `bytes(max)` | `len: u32` ‖ `len` bytes opacos; `len ≤ max`; **`len = 0` significa ausente** (campos opcionales) |
| `unit_id` | 16 bytes opacos |
| `ContentHash` | 33 bytes: `alg: u8` ‖ `digest: [u8; 32]` (§4) |

No hay relleno de alineación ni campos implícitos. Todo byte de un valor pertenece a un campo de este documento.

### 1.1 Cabecera común de los valores

Todo valor, salvo `REV_PAYLOAD` (§3.4), empieza por:

| Desplazamiento | Campo | Tipo | Valor requerido |
|---|---|---|---|
| 0 | `envelope_version` | `u8` | `1`. `0` es corrupción (UNIT-6); mayor que `1` es `IncompatibleLayer` (UNIT-7) |
| 1 | `reserved_flags` | `u8` | `0`. Cualquier otro valor es corrupción (UNIT-8) |

### 1.2 Límites

| Límite | Valor | Regla |
|---|---|---|
| Nombre de `LAYER_META` | 1..=64 B | UNIT-3, UNIT-4 |
| `unit_type` | 1..=64 B | UNIT-11, UNIT-12 |
| `metadata` | ≤ 4096 B | UNIT-11 |
| `actor_ref`, `source_ref` de `RevisionMeta` | ≤ 256 B c/u | UNIT-11 |
| `reason`, `actor_ref`, `source_ref`, `correlation` de `ChangeSetRecord` | ≤ 1024 B c/u | UNIT-11 |
| `payload_len` | ≤ 2^26 = 67 108 864 | UNIT-22 |
| `member_count` | ≤ 2^20 = 1 048 576 | UNIT-24 |
| `layer_version` soportada | 1 | UNIT-15, UNIT-16 |
| Longitud de clave | la de §2 | UNIT-3 |

## 2. Claves

Una clave es `tag: u8` seguido de los campos de la tabla. Longitudes en bytes, tag incluido.

| `tag` | Nombre | Campos tras el tag | Longitud |
|---|---|---|---|
| `0x01` | `LAYER_META` | `name`: UTF-8, 1..=64 B (sin prefijo de longitud: es el resto de la clave) | 2..=65 |
| `0x10` | `UNIT_HEAD` | `unit_id[16]` | 17 |
| `0x20` | `REV_META` | `unit_id[16]` ‖ `rev_no: u64` | 25 |
| `0x21` | `REV_PAYLOAD` | `unit_id[16]` ‖ `rev_no: u64` | 25 |
| `0x30` | `CHANGESET` | `commit_seq: u64` | 9 |
| `0x31` | `CS_MEMBER` | `commit_seq: u64` ‖ `ordinal: u32` | 13 |

Reglas de clave, evaluadas **en este orden**:

- **UNIT-1** (`Corruption`): un tag desconocido cuyo nibble alto es `0x0`, `0x1`, `0x2` o `0x3` (`0x00`, `0x02..=0x0F`, `0x11..=0x1F`, `0x22..=0x2F`, `0x32..=0x3F`) está dentro del rango de una etiqueta conocida y es corrupción. Una clave vacía también (no tiene tag).
- **UNIT-2** (`IncompatibleLayer`): un tag con nibble alto `0x4..=0xF` lo escribió una capa más nueva.
- **UNIT-3** (`Corruption`): la longitud de la clave no es la de la tabla.
- **UNIT-4** (`Corruption`): el `name` de `LAYER_META` no es UTF-8 válido.
- **UNIT-5** (`Corruption`): `rev_no = 0` en `REV_META`/`REV_PAYLOAD`, o `commit_seq = 0` en `CHANGESET`/`CS_MEMBER`. Los números de revisión y las secuencias de commit empiezan en 1.

`unit_id` y `ordinal` no tienen restricción de valor.

## 3. Valores

Los campos se listan en orden de byte; los desplazamientos de los campos de ancho variable dependen de las longitudes anteriores.

### 3.1 `LAYER_META` (`0x01`)

| Campo | Tipo |
|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` (§1.1) |
| `layer_version` | `u32` |
| `history_policy` | `u8`: `None=0`, `Audit=1`, `Versioned=2` |

Tamaño fijo: 7 B. `layer_version = 0` es corrupción (UNIT-15); `layer_version > 1` es `IncompatibleLayer` (UNIT-16). La semántica de `history_policy` por nombre la fija E3-05; el registro de nombre `layer` es la marca de capa (diseño §2.6).

### 3.2 `UNIT_HEAD` (`0x10`)

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_type` | `str(64)` | no vacío (UNIT-12) |
| `schema_version` | `u32` | ninguna |
| `lifecycle` | `u8` | `Active=0`, `Archived=1`, `Deleted=2` (UNIT-13) |
| `head_rev` | `u64` | ≥ 1 (UNIT-17) |
| `oldest_rev` | `u64` | `1 ≤ oldest_rev ≤ head_rev` (UNIT-17) |
| `created_seq` | `u64` | ≥ 1 (UNIT-18) |
| `updated_seq` | `u64` | ≥ `created_seq` (UNIT-18) |
| `created_at`, `updated_at` | `i64` | ninguna |
| `content_hash` | `ContentHash` | §4 |
| `payload_len` | `u32` | ≤ 2^26 (UNIT-22) |
| `payload_codec` | `u8` | `0` (UNIT-13) |
| `metadata` | `bytes(4096)` | |

### 3.3 `REV_META` (`0x20`)

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `kind` | `u8` | `Create=1`, `Update=2`, `Archive=3`, `Unarchive=4`, `Delete=5`, `Restore=6`; `0` y `7..=255` son corrupción (UNIT-13). `7..=15` están reservados para versiones futuras de la capa |
| `parent_rev` | `u64` | `0` si y solo si `kind = Create`; si no, `1 ≤ parent_rev < rev_no` de la clave (UNIT-19) |
| `commit_seq` | `u64` | ≥ 1 (UNIT-23) |
| `created_at` | `i64` | ninguna |
| `schema_version` | `u32` | ninguna |
| `content_hash` | `ContentHash` | §4 |
| `payload_len` | `u32` | ≤ 2^26 (UNIT-22) |
| `payload_codec` | `u8` | `0` (UNIT-13) |
| `payload_present` | `u8` | `0` o `1`; `Delete` exige `payload_present = 0` y `payload_len = 0` (UNIT-21) |
| `restored_from` | `u64` | `0` salvo en `Restore`; en `Restore`, `1 ≤ restored_from < rev_no` (UNIT-20) |
| `metadata` | `bytes(4096)` | |
| `actor_ref` | `bytes(256)` | |
| `source_ref` | `bytes(256)` | |

Los valores de `REV_META` son inmutables: una clave se escribe una vez (diseño §3.1).

### 3.4 `REV_PAYLOAD` (`0x21`)

El valor es la carga útil **tal cual**: sin `envelope_version`, sin longitud, sin relleno. Su longitud y su integridad no se validan al decodificar el valor aislado: las fijan `payload_len` (≤ 2^26, UNIT-22) y `content_hash` del `REV_META` correspondiente (diseño §2.7), y la capa comprueba ambos al leer la revisión (§5.1, E3-02). Por eso no hay regla `UNIT-n` de decodificación para él: todo valor, de cualquier longitud que el motor admita, es sintácticamente válido como `REV_PAYLOAD`, y un valor cuya longitud no coincide con `payload_len` es corrupción detectada por esa comprobación cruzada, no por este decodificador.

### 3.5 `CHANGESET` (`0x30`)

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `created_at` | `i64` | ninguna |
| `member_count` | `u32` | ≤ 2^20 (UNIT-24) |
| `purpose` | `u8` | `Application=0`, `Retention=1`, `Maintenance=2` (UNIT-13) |
| `reason`, `actor_ref`, `source_ref`, `correlation` | `bytes(1024)` cada uno | |

### 3.6 `CS_MEMBER` (`0x31`)

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_id` | `[u8; 16]` | ninguna |
| `rev_no` | `u64` | ≥ 1 (UNIT-25) |
| `kind` | `u8` | como `REV_META.kind` (UNIT-13) |

Tamaño fijo: 27 B.

## 4. `ContentHash`

| `alg` | Algoritmo | `digest` |
|---|---|---|
| `0` | CRC-32C provisional, **sin garantía criptográfica** | `crc: u32` big-endian ‖ 28 bytes `0x00` (UNIT-14) |
| `1` | BLAKE3-256 (ADR-0008, propuesto) | 32 bytes cualesquiera |

El hash cubre `payload_codec ‖ payload` (un byte de códec seguido de la carga útil, sin longitud). Cualquier otro `alg` es corrupción (UNIT-13). Los escritores de esta capa usan `alg = 1`. El decodificador de estructuras no comprueba el hash; lo hace la verificación de carga útil de §5.1 (E3-02, `kerno_units::verify_payload`). El verificador offline de E3-07 la reutiliza.

## 5. Reglas de validación

Cada regla lleva su clase de error: `Corruption` (los bytes no son lo que un escritor de esta capa produce) o `IncompatibleLayer` (los escribió una capa más nueva; no se modifica nada).

| Id | Regla | Clase |
|---|---|---|
| UNIT-1 | Tag desconocido dentro del rango de una etiqueta conocida, o clave vacía | `Corruption` |
| UNIT-2 | Tag con nibble alto `0x4..=0xF` | `IncompatibleLayer` |
| UNIT-3 | Longitud de clave distinta de la de §2 | `Corruption` |
| UNIT-4 | `name` de `LAYER_META` no es UTF-8 | `Corruption` |
| UNIT-5 | `rev_no = 0` o `commit_seq = 0` en la clave | `Corruption` |
| UNIT-6 | `envelope_version = 0` | `Corruption` |
| UNIT-7 | `envelope_version > 1` | `IncompatibleLayer` |
| UNIT-8 | `reserved_flags ≠ 0` | `Corruption` |
| UNIT-9 | Un campo sobrepasa el final del valor (valor truncado, incluido el vacío) | `Corruption` |
| UNIT-10 | Bytes sobrantes tras el último campo | `Corruption` |
| UNIT-11 | El prefijo de longitud de un `str`/`bytes` supera el máximo del campo | `Corruption` |
| UNIT-12 | `unit_type` vacío o no UTF-8 | `Corruption` |
| UNIT-13 | Valor de enumeración fuera de la tabla: `lifecycle`, `kind`, `purpose`, `history_policy`, `alg`; o `payload_codec ≠ 0` | `Corruption` |
| UNIT-14 | `alg = 0` con los 28 bytes finales del `digest` distintos de cero | `Corruption` |
| UNIT-15 | `layer_version = 0` | `Corruption` |
| UNIT-16 | `layer_version > 1` | `IncompatibleLayer` |
| UNIT-17 | `UNIT_HEAD`: no se cumple `1 ≤ oldest_rev ≤ head_rev` | `Corruption` |
| UNIT-18 | `UNIT_HEAD`: no se cumple `1 ≤ created_seq ≤ updated_seq` | `Corruption` |
| UNIT-19 | `REV_META`: `parent_rev` incoherente con `kind` y `rev_no` (§3.3) | `Corruption` |
| UNIT-20 | `REV_META`: `restored_from` incoherente con `kind` y `rev_no` (§3.3) | `Corruption` |
| UNIT-21 | `REV_META`: `payload_present ∉ {0,1}`, o `Delete` con carga útil | `Corruption` |
| UNIT-22 | `payload_len > 2^26` | `Corruption` |
| UNIT-23 | `REV_META`: `commit_seq = 0` | `Corruption` |
| UNIT-24 | `CHANGESET`: `member_count > 2^20` | `Corruption` |
| UNIT-25 | `CS_MEMBER`: `rev_no = 0` | `Corruption` |

### 5.1 Integridad de la carga útil (E3-02)

Al leer una revisión, la capa compara el `REV_PAYLOAD` almacenado (o su ausencia) con los campos `payload_codec`, `payload_len`, `payload_present` y `content_hash` de su `REV_META`. Las reglas se aplican en este orden y se rechaza la primera violada. La longitud se compara antes de calcular el hash, así que un valor truncado o sobredimensionado nunca se procesa.

| Id | Regla | Clase |
|---|---|---|
| UNIT-13 | `payload_codec ≠ 0` (repetida aquí porque la declaración puede llegar de otra fuente) | `Corruption` |
| UNIT-22 | `payload_len > 2^26` (repetida) | `Corruption` |
| UNIT-26 | `payload_present = 1` sin `REV_PAYLOAD`; `REV_PAYLOAD` presente con `payload_present = 0`; o `payload_present = 0` con `payload_len ≠ 0` | `Corruption` |
| UNIT-27 | La longitud del `REV_PAYLOAD` difiere de `payload_len` (truncado o extendido) | `Corruption` |
| UNIT-28 | El hash de `payload_codec ‖ payload` con el `alg` declarado difiere de `content_hash` | `Corruption` |

La verificación es pura: no escribe, no depende de nada fuera de sus argumentos y devuelve el mismo error para la misma entrada. Un rechazo no modifica la base y se puede repetir. Solo una carga útil verificada (`VerifiedPayload`) llega al resto de la capa. Vectores de hash conocidos y pruebas de corrupción: `crates/kerno-units/tests/payload_integrity.rs`.

## 6. Entrada truncada o corrupta

Un decodificador recibe el par `(key, value)` y devuelve los campos o **un** rechazo `(UNIT-n, clase)`. Debe:

1. **No entrar en pánico, no leer fuera del búfer y no reservar memoria a partir de una longitud sin validarla**: un prefijo de longitud se compara con el máximo del campo (UNIT-11) y con los bytes que quedan (UNIT-9) antes de leer el cuerpo. El máximo se comprueba primero: un prefijo `0xFFFFFFFF` es UNIT-11, no UNIT-9.
2. **Rechazar la primera regla violada** en este orden: reglas de clave (§2) → cabecera (UNIT-6, UNIT-7, UNIT-8) → campos en orden de byte, cada uno con sus comprobaciones propias (UNIT-9, UNIT-11, UNIT-12, UNIT-13, UNIT-14, UNIT-15, UNIT-16, UNIT-22, UNIT-23, UNIT-24, UNIT-25 en cuanto se lee el campo) → bytes sobrantes (UNIT-10) → reglas entre campos (UNIT-17..UNIT-21, tras UNIT-10).
3. **Distinguir truncamiento de corrupción solo por la regla**, no por la clase: ambas son `Corruption`. Un valor vacío es UNIT-9, y nunca una base nueva.
4. No «reparar», rellenar ni ignorar nada. Un rechazo no modifica la base (I-18 si es de solo lectura).
5. Un tag con nibble alto `0x4..=0xF` (UNIT-2) o `envelope_version`/`layer_version` mayores (UNIT-7, UNIT-16) se rechaza como `IncompatibleLayer` **sin interpretar el resto**.

## 7. Vectores

`crates/kerno-units/tests/vectors/v1/structs/<name>.bin` guarda cada vector como `key_len: u32` (big-endian) ‖ `key` ‖ `value`. Los positivos se llaman `<estructura>_<caso>.bin`; los negativos, `neg_unit<n>_<caso>.bin`, y hay al menos uno por regla `UNIT-n`. La tabla de campos esperados (positivos) y de regla y clase esperadas (negativos) vive en `tests/struct_vectors.rs`; cambiar un byte de un vector existente sigue [v1.md §15](v1.md) en espíritu: versión de la capa nueva y ADR. Para regenerar:

```text
cargo test -p kerno-units --test struct_vectors -- --ignored regenerate_struct_vectors
```
