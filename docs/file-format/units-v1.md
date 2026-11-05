# Capa de unidades v1: claves y valores

| Campo | Valor |
|---|---|
| Estado | Borrador normativo (E3-01); lo fija el propietario al cerrar E3-01 |
| Versión de la capa | `layer_version = 1`, `envelope_version = 1` |
| Relacionado con | [diseño de la Etapa 3](../architecture/etapa3-design.md) §2, [acta E3-00](../architecture/etapa3-acta-e3-00.md), [ADR-0004](../adr/0004-historial-de-revisiones.md), [ADR-0007](../adr/0007-consistencia-de-indices.md) (índice primario, E4-03), [ADR-0008](../adr/0008-serializacion-documental-y-hashes.md) (aceptado), [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) (relaciones, E4-02), [formato v1 del motor](v1.md) |

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
| `relation_type` de una relación (E4-02) | 1..=64 B, sin NUL | UNIT-12 |
| `origin` de la procedencia de una relación | ≤ 256 B | UNIT-11 |
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
| `0x40` | `REL` (E4-02) | `edge_id[16]` | 17 |
| `0x41` | `REL_OUT` (E4-02) | `unit_id[16]` ‖ `relation_type` (1..=64 B de UTF-8 sin NUL) ‖ `0x00` ‖ `edge_id[16]` | 35..=98 |
| `0x42` | `REL_IN` (E4-02) | igual que `REL_OUT`, con `unit_id` = el extremo `to` | 35..=98 |
| `0x50` | `IDX_META` (E4-03) | ninguno: hay un solo registro, el del índice primario | 1 |
| `0x51` | `IDX_PRIMARY` (E4-03) | `unit_id[16]` | 17 |
| `0x52` | `IDX_TYPE` (E4-04) | `len: u8` (1..=64) ‖ `unit_type` (`len` B de UTF-8) ‖ `schema_version: u32` ‖ `unit_id[16]` | 23..=86 (= 22 + `len`) |

Reglas de clave, evaluadas **en este orden**:

- **UNIT-1** (`Corruption`): un tag desconocido cuyo nibble alto es `0x0`, `0x1`, `0x2` o `0x3` (`0x00`, `0x02..=0x0F`, `0x11..=0x1F`, `0x22..=0x2F`, `0x32..=0x3F`) está dentro del rango de una etiqueta conocida y es corrupción. Una clave vacía también (no tiene tag).
- **UNIT-2** (`IncompatibleLayer`): un tag con nibble alto `0x4..=0xF`, salvo `0x40..=0x42` (E4-02), `0x50..=0x51` (E4-03) y `0x52` (E4-04), lo escribió una capa más nueva. Una capa anterior a E4-02 rechaza por esta regla un almacén con relaciones (véase ADR-0014, «Consecuencias»), una anterior a E4-03 uno con el índice primario construido, y una anterior a E4-04 uno construido con `IDX_TYPE`. Un almacén sin ninguno de esos registros se abre igual con todas ellas: el índice solo existe tras una llamada a `rebuild_index`.
- **UNIT-3** (`Corruption`): la longitud de la clave no es la de la tabla (en `IDX_TYPE`, un `len` fuera de 1..=64 o una longitud distinta de 22 + `len`).
- **UNIT-12** también se aplica a la clave de `IDX_TYPE`: un `unit_type` que no es UTF-8.
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
| `purpose` | `u8` | `Application=0`, `Retention=1`, `Maintenance=2`, `Relation=3`, `Index=4` (UNIT-13). `Relation` lo escribe la capa para un `ChangeSet` de aplicación que solo crea o borra relaciones (sin miembros, E4-02). `Index` lo escribe para el `ChangeSet` que construye el índice primario o anota que su construcción falló (sin miembros, E4-03) |
| `reason`, `actor_ref`, `source_ref`, `correlation` | `bytes(1024)` cada uno | |

### 3.6 `CS_MEMBER` (`0x31`)

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_id` | `[u8; 16]` | ninguna |
| `rev_no` | `u64` | ≥ 1 (UNIT-25) |
| `kind` | `u8` | como `REV_META.kind` (UNIT-13) |

Tamaño fijo: 27 B.

### 3.7 `REL` (`0x40`, E4-02)

Una relación dirigida de `from` a `to` ([ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md), opción 2).

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `from`, `to` | `[u8; 16]` | ninguna (las unidades extremo deben ser `Active`: UNIT-36) |
| `relation_type` | `str(64)` | 1..=64 B, sin NUL (UNIT-12) |
| `created_seq` | `u64` | ≥ 1 (UNIT-23); el `CHANGESET` de esa secuencia guarda actor, origen y motivo |
| `provenance` | `u8` | `Declared=0`, `Imported=1`, `Algorithmic=2`, `Inferred=3` (UNIT-13). **Obligatoria**: no hay valor por defecto ni forma de omitirla |
| `origin` | `bytes(256)` | vacío con `Declared`; no vacío con las demás (UNIT-13) |
| `confidence` | `u16` | milésimas, `0..=1000`; `0` salvo con `Inferred` (UNIT-13) |

Un `REL` no tiene revisiones propias: borrarlo borra el registro y sus dos entradas, y el `ChangeSet` que lo hizo es la única huella.

### 3.8 `REL_OUT` y `REL_IN` (`0x41`, `0x42`, E4-02)

Entradas del índice de adyacencia, escritas y borradas en el mismo `ChangeSet` que el `REL` (I-21). La clave ordena por unidad, tipo y `edge_id`, de modo que los vecinos de una unidad (con o sin un tipo) son un rango. El NUL tras el tipo evita que un tipo que es prefijo de otro (`cite`, `cites`) coincida con las entradas del segundo.

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `other` | `[u8; 16]` | el otro extremo de la relación (`to` en `REL_OUT`, `from` en `REL_IN`); permite leer el vecino sin abrir el `REL` |

Tamaño fijo: 18 B.

### 3.9 `IDX_META` (`0x50`, E4-03)

El estado de frescura del índice primario que se guarda ([ADR-0007](../adr/0007-consistencia-de-indices.md), I-23). Hay un solo registro. Lo escribe el `ChangeSet` de propósito `Index` que construye el índice (`UnitDb::rebuild_index`), en la misma transacción que las entradas y el borrado de las anteriores (I-21): un almacén tiene la generación anterior entera o la nueva entera.

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `outcome` | `u8` | `Succeeded=0`, `Failed=1` (UNIT-13): cómo acabó el último intento de construcción. `Building` y `Stale` **no se guardan**: el primero es de un manejador en marcha y el segundo se calcula contra una instantánea |
| `watermark` | `u64` | la secuencia de commit que reflejan las entradas: la del `ChangeSet` que las escribió (no cambia ninguna unidad, así que las entradas son función del estado en ese commit). `0` solo si nunca se construyó nada; con `Succeeded` es ≥ 1 (UNIT-23) |
| `entries` | `u64` | número de entradas `IDX_PRIMARY` de esa generación. Con `watermark = 0` es `0` (UNIT-13) |

Tamaño fijo: 19 B. Con `Failed`, `watermark` y `entries` son los de la generación que sigue guardada (o `0` y `0` si no hay ninguna): un intento fallido no cambia ninguna entrada. El estado que ve una consulta es función de este registro y de su instantánea: `Failed` si `outcome = Failed`; si no, `Ready` cuando `watermark` es la secuencia de la instantánea y `Stale` cuando es anterior; sin registro, `Stale` sin watermark. Un `watermark` posterior a la instantánea es corrupción.

### 3.10 `IDX_PRIMARY` (`0x51`, E4-03)

La entrada del índice primario de una unidad `Active`: los campos de `UNIT_HEAD` que lee un filtro básico. Solo hay entrada para las unidades `Active` (las archivadas y las borradas no la tienen). La clave ordena por `unit_id`, igual que `UNIT_HEAD`, así que recorrer el índice da las unidades en el mismo orden que la ruta de referencia.

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_type` | `str(64)` | no vacío (UNIT-12); el de la cabeza de la unidad |
| `schema_version` | `u32` | ninguna; el de la cabeza de la unidad |

### 3.11 `IDX_TYPE` (`0x52`, E4-04)

El índice secundario por `(unit_type, schema_version)`, justificado por el workload W1 ([etapa4-workloads.md](../architecture/etapa4-workloads.md)). Una entrada por cada entrada `IDX_PRIMARY` y ninguna más: la escribe la misma construcción, en la misma transacción y bajo el mismo `IDX_META` (§3.9), así que comparte watermark, estado y comportamiento ante caídas, y una reconstrucción las reemplaza a la vez (I-21, I-23, I-24). Todo está en la clave; el prefijo de longitud hace que el rango de un tipo no contenga el de otro tipo que lo tenga por prefijo. El orden es longitud, tipo, versión de esquema y `unit_id`: un filtro con tipo y versión recorre su rango en orden de id; con tipo solo, el resultado se reordena por id.

| Campo | Tipo | Restricción |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1; no hay más campos (UNIT-10 si sobran bytes) |

Tamaño fijo: 2 B.

## 4. `ContentHash`

| `alg` | Algoritmo | `digest` |
|---|---|---|
| `0` | CRC-32C provisional, **sin garantía criptográfica** | `crc: u32` big-endian ‖ 28 bytes `0x00` (UNIT-14) |
| `1` | BLAKE3-256 (ADR-0008, aceptado) | 32 bytes cualesquiera |

El hash cubre `payload_codec ‖ payload` (un byte de códec seguido de la carga útil, sin longitud). Cualquier otro `alg` es corrupción (UNIT-13). Los escritores de esta capa usan `alg = 1`. El decodificador de estructuras no comprueba el hash; lo hace la verificación de carga útil de §5.1 (E3-02, `kerno_units::verify_payload`). El verificador offline de E3-07 la reutiliza.

## 5. Reglas de validación

Cada regla lleva su clase de error: `Corruption` (los bytes no son lo que un escritor de esta capa produce) o `IncompatibleLayer` (los escribió una capa más nueva; no se modifica nada).

| Id | Regla | Clase |
|---|---|---|
| UNIT-1 | Tag desconocido dentro del rango de una etiqueta conocida, o clave vacía | `Corruption` |
| UNIT-2 | Tag con nibble alto `0x4..=0xF`, salvo `0x40..=0x42` y `0x50..=0x52` | `IncompatibleLayer` |
| UNIT-3 | Longitud de clave distinta de la de §2 | `Corruption` |
| UNIT-4 | `name` de `LAYER_META` no es UTF-8 | `Corruption` |
| UNIT-5 | `rev_no = 0` o `commit_seq = 0` en la clave | `Corruption` |
| UNIT-6 | `envelope_version = 0` | `Corruption` |
| UNIT-7 | `envelope_version > 1` | `IncompatibleLayer` |
| UNIT-8 | `reserved_flags ≠ 0` | `Corruption` |
| UNIT-9 | Un campo sobrepasa el final del valor (valor truncado, incluido el vacío) | `Corruption` |
| UNIT-10 | Bytes sobrantes tras el último campo | `Corruption` |
| UNIT-11 | El prefijo de longitud de un `str`/`bytes` supera el máximo del campo | `Corruption` |
| UNIT-12 | `unit_type` o `relation_type` vacío o no UTF-8 (también el de `IDX_PRIMARY`); `relation_type` con NUL (también en la clave de `REL_OUT`/`REL_IN`) | `Corruption` |
| UNIT-13 | Valor de enumeración fuera de la tabla: `lifecycle`, `kind`, `purpose`, `history_policy`, `alg`, `provenance`, `outcome`; `payload_codec ≠ 0`; una procedencia de `REL` incoherente (§3.7); o un `IDX_META` con `entries ≠ 0` y `watermark = 0` (§3.9) | `Corruption` |
| UNIT-14 | `alg = 0` con los 28 bytes finales del `digest` distintos de cero | `Corruption` |
| UNIT-15 | `layer_version = 0` | `Corruption` |
| UNIT-16 | `layer_version > 1` | `IncompatibleLayer` |
| UNIT-17 | `UNIT_HEAD`: no se cumple `1 ≤ oldest_rev ≤ head_rev` | `Corruption` |
| UNIT-18 | `UNIT_HEAD`: no se cumple `1 ≤ created_seq ≤ updated_seq` | `Corruption` |
| UNIT-19 | `REV_META`: `parent_rev` incoherente con `kind` y `rev_no` (§3.3) | `Corruption` |
| UNIT-20 | `REV_META`: `restored_from` incoherente con `kind` y `rev_no` (§3.3) | `Corruption` |
| UNIT-21 | `REV_META`: `payload_present ∉ {0,1}`, o `Delete` con carga útil | `Corruption` |
| UNIT-22 | `payload_len > 2^26` | `Corruption` |
| UNIT-23 | `REV_META` o `REL`: `commit_seq` / `created_seq` = 0; `IDX_META` con `outcome = Succeeded` y `watermark` = 0 | `Corruption` |
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

Esta verificación no repite UNIT-14. Con `alg = 0`, un relleno distinto de cero en el `digest` se rechaza aquí como UNIT-28, porque el digest calculado siempre lleva ese relleno a cero. El decodificador de estructuras ya lo rechaza antes como UNIT-14.

La verificación es pura: no escribe, no depende de nada fuera de sus argumentos y devuelve el mismo error para la misma entrada. Un rechazo no modifica la base y se puede repetir. Solo una carga útil verificada (`VerifiedPayload`) llega al resto de la capa. Vectores de hash conocidos y pruebas de corrupción: `crates/kerno-units/tests/payload_integrity.rs`.

### 5.2 Reglas entre registros (E3-07)

El verificador offline (`verify_units`, `crates/kerno-units/src/verify.rs`) recorre todas las claves de una instantánea y, además de UNIT-1..UNIT-28, comprueba estas reglas entre registros. No cambian ningún byte: describen qué combinaciones de registros válidos por separado produce un escritor de esta capa. Todas son `Corruption`. Un registro que no decodifica se informa una sola vez (por su regla de §5) y se excluye de estas comprobaciones, para no informarlo de nuevo como «ausente».

| Id | Regla | Invariante |
|---|---|---|
| UNIT-29 | Un almacén con claves no tiene la marca `LAYER_META` `layer`, o un nombre de `LAYER_META` no es `layer` ni `policy/<unit_type>` (1..=57 B) | §2.6 del diseño |
| UNIT-30 | Registro huérfano: `REV_META`/`REV_PAYLOAD` sin `UNIT_HEAD`, `REV_PAYLOAD` sin su `REV_META`, `CS_MEMBER` sin su `CHANGESET` | IU-2, IU-4 |
| UNIT-31 | Cabeza incoherente: falta un `REV_META` en `[oldest_rev, head_rev]`, hay `REV_META`/`REV_PAYLOAD` fuera de ese rango, la cabeza difiere del `REV_META` de `head_rev` (hash, longitud, esquema, metadatos, `updated_seq = commit_seq`, ciclo de vida deducido de `kind`), la revisión vigente no lápida no tiene carga útil, o `created_seq` no es el `commit_seq` de la revisión 1 (si está retenida; si no, es menor que el de `oldest_rev`) | IU-2 |
| UNIT-32 | Cadena imposible: `Create` fuera de la revisión 1 o revisión 1 que no es `Create`, `parent_rev ≠ rev_no − 1`, `commit_seq` no estrictamente creciente o posterior a la instantánea, transición de ciclo de vida que ningún escritor produce (`Update`/`Archive`/`Restore` sobre no `Active`, `Unarchive` sobre no `Archived`, nada tras `Delete`), o `Archive`/`Unarchive`/`Restore` cuyo hash, longitud, esquema o metadatos difieren de los de su origen retenido | IU-3 |
| UNIT-33 | Procedencia: un `REV_META` sin `CHANGESET` en su `commit_seq`, con un `CHANGESET` cuyo `purpose` no es `Application`, o sin un `CS_MEMBER` de ese `ChangeSet` que lo nombre (`unit_id`, `rev_no`, `kind`); miembros que no son exactamente los ordinales `0..member_count`; una unidad nombrada dos veces en un `ChangeSet`; `Application`/`Retention` sin miembros o `Maintenance`/`Relation`/`Index` con miembros; un miembro de `Application` que nombra una unidad inexistente, una revisión posterior a la cabeza, o una revisión retenida con otro `commit_seq` u otro `kind`. Los miembros de `Retention` solo se comprueban en el sentido `REV_META → miembro` (nombran la primera revisión retenida, no una que escribieran) | IU-4 |
| UNIT-34 | Política no aplicada, solo donde la política vigente al escribir se conoce (revisiones confirmadas después del último `ChangeSet` `Maintenance`): bajo `none`, `oldest_rev ≠ head_rev`; bajo `audit`, una revisión sustituida que conserva `payload_present = 1` | IU-9 |
| UNIT-35 | Entre dos instantáneas, una clave `REV_META`/`REV_PAYLOAD` presente en ambas cambió de valor, salvo el paso de `payload_present` de 1 a 0 que hace una política (`verify_immutable`) | IU-1 |
| UNIT-36 | Relaciones (E4-02): un `REL` sin su `REL_OUT` o sin su `REL_IN` (con el mismo tipo y el otro extremo correcto), una entrada `REL_OUT`/`REL_IN` sin su `REL` o que no concuerda con él, un `REL` con `created_seq` posterior a la instantánea, o un extremo que no existe o no está `Active` (archivar o borrar una unidad con relaciones se rechaza) | I-21 |
| UNIT-37 | Índice primario (E4-03): entradas `IDX_PRIMARY` sin `IDX_META`; un `watermark` posterior a la instantánea; un `entries` que no es el número de entradas guardadas; una unidad que no cambió desde el watermark (`updated_seq ≤ watermark`) y que, siendo `Active`, no tiene entrada o la tiene con otro `unit_type` o `schema_version`, o que, no siendo `Active`, la tiene; una entrada de una unidad que no existe o que se creó después del watermark. Una unidad que cambió después del watermark puede discrepar de su entrada: es la obsolescencia que declara el estado, no un daño | I-23 |
| UNIT-38 | Índice secundario por tipo (E4-04): una entrada `IDX_PRIMARY` sin la entrada `IDX_TYPE` con su mismo `unit_type`, `schema_version` y `unit_id`, o una entrada `IDX_TYPE` sin esa entrada `IDX_PRIMARY`. Las unidades con una entrada que no decodifica (en cualquiera de los dos) ya son un hallazgo de estructura y se omiten | I-23, I-24 |

Para la integridad de la carga útil el verificador aplica §5.1 a cada `REV_META`, con una excepción: una revisión no lápida con `payload_present = 0` es una que un periodo `audit` vació conservando hash y longitud (nota de E3-05 del diseño); para ella solo se exige que no exista `REV_PAYLOAD` (UNIT-26) y no se aplica la parte «`payload_present = 0` con `payload_len ≠ 0`» de UNIT-26.

## 6. Entrada truncada o corrupta

Un decodificador recibe el par `(key, value)` y devuelve los campos o **un** rechazo `(UNIT-n, clase)`. Debe:

1. **No entrar en pánico, no leer fuera del búfer y no reservar memoria a partir de una longitud sin validarla**: un prefijo de longitud se compara con el máximo del campo (UNIT-11) y con los bytes que quedan (UNIT-9) antes de leer el cuerpo. El máximo se comprueba primero: un prefijo `0xFFFFFFFF` es UNIT-11, no UNIT-9.
2. **Rechazar la primera regla violada** en este orden: reglas de clave (§2) → cabecera (UNIT-6, UNIT-7, UNIT-8) → campos en orden de byte, cada uno con sus comprobaciones propias (UNIT-9, UNIT-11, UNIT-12, UNIT-13, UNIT-14, UNIT-15, UNIT-16, UNIT-22, UNIT-23, UNIT-24, UNIT-25 en cuanto se lee el campo) → bytes sobrantes (UNIT-10) → reglas entre campos (UNIT-17..UNIT-21 y la de `IDX_META` con `watermark = 0`, tras UNIT-10).
3. **Distinguir truncamiento de corrupción solo por la regla**, no por la clase: ambas son `Corruption`. Un valor vacío es UNIT-9, y nunca una base nueva.
4. No «reparar», rellenar ni ignorar nada. Un rechazo no modifica la base (I-18 si es de solo lectura).
5. Un tag con nibble alto `0x4..=0xF`, salvo `0x40..=0x42` y `0x50..=0x52` (UNIT-2), o `envelope_version`/`layer_version` mayores (UNIT-7, UNIT-16) se rechaza como `IncompatibleLayer` **sin interpretar el resto**.

Los códecs de producción (E3-02) están en `crates/kerno-units/src/codec.rs` (`Key`, `Record` y un tipo por valor) y siguen este orden; los errores, con la regla y su clase, en `src/error.rs`. La prueba `production_codec_matches_committed_vectors` de `tests/struct_vectors.rs` exige que decodifiquen cada vector de §7 como indica la tabla y que reproduzcan byte a byte los positivos; `spec_decoder` sigue sin depender de ellos.

## 7. Vectores

`crates/kerno-units/tests/vectors/v1/structs/<name>.bin` guarda cada vector como `key_len: u32` (big-endian) ‖ `key` ‖ `value`. Los positivos se llaman `<estructura>_<caso>.bin`; los negativos, `neg_unit<n>_<caso>.bin`, y hay al menos uno por regla `UNIT-n`. La tabla de campos esperados (positivos) y de regla y clase esperadas (negativos) vive en `tests/struct_vectors.rs`; cambiar un byte de un vector existente sigue [v1.md §15](v1.md) en espíritu: versión de la capa nueva y ADR. Para regenerar:

```text
cargo test -p kerno-units --test struct_vectors -- --ignored regenerate_struct_vectors
```
