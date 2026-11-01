# Diseño de la Etapa 3: unidades, revisiones y procedencia

Estado: borrador para revisión del propietario · Relacionado con: [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.1–6.3 y §16, [SAD §8](SAD.md#8-revision-engine-historial-diferencias-y-procedencia), [ADR-0001](../adr/0001-motor-de-almacenamiento.md), [ADR-0002](../adr/0002-transacciones-y-durabilidad.md), [ADR-0003](../adr/0003-formato-fisico-y-versionado.md), [ADR-0004](../adr/0004-historial-de-revisiones.md) (aceptado), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) (aceptado), [diseño de revisiones de M0](revision-model-design.md), [diseño de la Fase 1](phase1-design.md), [invariantes](invariants.md), [roadmap](../roadmap/README.md#etapa-3--unidades-revisiones-y-procedencia)

Este documento es **solo diseño**: no hay código, no se acepta ningún ADR y no se cambia ningún byte del formato v1. Los identificadores de tipos, claves y funciones están en inglés; la prosa, en español. Todo lo que dependa de un ADR todavía no aceptado (0005, 0008, 0017) se presenta como **propuesta** y se lista en §12 para que el propietario decida.

## 0. Resumen

1. **Capa nueva, formato intacto.** `Unit`, `Revision` y `ChangeSet` se representan como **claves y valores ordinarios** del LSM de `kerno-storage` (que ya da batches atómicos, snapshots, `delete_range` y transacciones read-modify-write). La Etapa 3 no toca el formato v1 (ADR-0013 «inmutable mientras exista v1»): ni cabecera, ni WAL, ni SST, ni MANIFEST. La capa tiene su **propio** número de versión, guardado como un registro de metadatos dentro del espacio de claves (§2.1). Si el propietario quiere además que un motor anterior se niegue a abrir una base con unidades, eso se hace con un **bit de característica** (`incompat`, rango 1..62 reservado) y nunca con un cambio de `format_version` (§12, decisión 3).
2. **Un `ChangeSet` = un commit del motor.** Todo cambio lógico (contenido, revisión, registro de miembros del `ChangeSet`) va en **una** `WriteTxn` y, por tanto, en una sola trama del WAL: I-01 e I-21 se heredan del motor en lugar de reimplementarse (§4).
3. **Revisiones inmutables.** La clave de una revisión se escribe una sola vez y no se reescribe nunca; el estado vigente (`head`) es lo único mutable. El historial vive en claves propias, no en el WAL (I-20, §3, §5).
4. **Lecturas «as-of»** de dos clases: contra una instantánea del motor (consistente con I-05) y contra una `commit_sequence` histórica (búsqueda binaria sobre las revisiones de la unidad, §3.5).
5. **Primera API pública tipada** (`UnitDb`, `ChangeSet`, `UnitSnapshot`, `UnitError`; §6), genérica sobre `TxnEngine` de `kerno-core` para poder ejecutar las mismas pruebas sobre `LogEngine`/`Emulated<E>` y sobre `KernoDb`.
6. **Objetivos T1–T10 sin cambios.** La capa vive en otro crate y no modifica `kerno-storage`; sus costes se miden con métricas **nuevas** (serie `U`, §9.4) que el propietario aprueba antes de medir (ADR-0012), sin reabrir T1–T10 (§9.5).

## 1. Qué hay y qué no hay debajo

Hechos del código en el commit base (comprobados, no supuestos):

| Capacidad | Dónde | Lo que la Etapa 3 usa |
|---|---|---|
| `Engine::commit(&WriteBatch) -> seq`, atómico (I-01), secuencia `+1` por commit no vacío (I-04) | `kerno-core::contract` | la `commit_sequence` de un `ChangeSet` es la que devuelve el commit |
| `TxnEngine::begin_write` → `Txn` con `get`, `scan`, `put`, `delete`, `delete_range`, `commit`, `abort` | `kerno-core::contract`, `kerno-storage::txn::WriteTxn` | el `ChangeSet` **es** una `Txn`: toma la única ranura de escritura, lee lo que escribe y no deja rastro si se aborta o se suelta |
| `Snapshot::{seq, get, scan}` | `KernoSnapshot` | lecturas consistentes de cabeza, revisión e historial (I-05) |
| `delete_range` con lápidas de rango y `compact_range` | `Config`/WP10 | retención y GC de historial (§5) |
| `Durability::{Sync, Buffered}`, `sync()` | `Config::durability`, `SyncEngine` | la capa hereda la durabilidad; no define otra |
| Límites: clave 512 B, valor 1 MiB, lote 64 MiB / 2^20 operaciones (ajustables hasta 16 KiB / 64 MiB) | `Config::limits` | tope de la carga útil de una unidad y de un `ChangeSet` (§2.4, §6.5) |
| `Error::{Io, Corruption, Locked, InvalidArgument, Poisoned, Incompatible, ReadOnly}` | `kerno-core::error` | se envuelven sin perder la clase (§6.6) |
| `SimVfs` con corte de energía, escritura rota, fallo de `fsync`, disco lleno, trazas | `kerno-testkit::vfs` | barridos de caída de la capa (§8) |

Lo que **no** hay y esta etapa no añade: lecturas hacia atrás (los `scan` son hacia delante; se diseña para ello, §3.5), claves compuestas de primera clase (se codifican a mano, §2), tipos de índice (Etapa 4), cifrado (AEAD sigue `Incompatible`, ADR-0006).

Consecuencia de diseño: la capa no necesita nada del motor que ya no exista, salvo exponer en el trait `Txn` la secuencia leída (`read_seq`, ya presente en `WriteTxn`). Si una necesidad real aparece más adelante, entra por la vía de ADR-0013: bit de característica + vectores + ADR, nunca reescribiendo bytes v1.

## 2. Modelo de datos y codificación de claves

### 2.1 Principios

- **Un byte de etiqueta por tipo de registro**, seguido de campos de ancho fijo **big-endian** para que el orden lexicográfico sea el numérico (como en el diseño de M0, §2). Ninguna clave excede 512 B (la mayor mide 25 B).
- **Los registros de una unidad son contiguos**: `unit_id` va antes de `rev_no`, así que «todo el historial de una unidad» es un único `scan` de prefijo y la retención es un único `delete_range`.
- **Sin colecciones en la clave.** La spec §6.1 identifica la unidad por `unit_id` y `unit_type` (campo de la cabeza). El filtrado por tipo es un índice (Etapa 4, ADR-0007/0014); la Etapa 3 recorre la cabeza.
- **El orden histórico lo da `commit_sequence`**, no el reloj ni el UUIDv7 (I-26). Los `created_at` son datos descriptivos.

### 2.2 Espacio de claves

| Etiqueta | Clave | Valor | Mutable | Para qué |
|---|---|---|---|---|
| `0x01` `LAYER_META` | `01 ‖ name` (UTF-8, ≤ 64 B) | `layer_version` y, por nombre, la política de historial por tipo | sí (raro) | marca que la base tiene capa de unidades; ver §2.6 |
| `0x10` `UNIT_HEAD` | `10 ‖ unit_id[16]` (17 B) | `UnitHead` (§2.3) | **sí** | estado vigente en **una** búsqueda |
| `0x20` `REV_META` | `20 ‖ unit_id[16] ‖ rev_no:u64be` (25 B) | `RevisionMeta` (§2.4) | **no** | historial de una unidad = un rango contiguo, sin leer cargas útiles |
| `0x21` `REV_PAYLOAD` | `21 ‖ unit_id[16] ‖ rev_no:u64be` (25 B) | carga útil de la revisión (bytes opacos) | **no** | separa los bytes grandes de los metadatos |
| `0x30` `CHANGESET` | `30 ‖ commit_seq:u64be` (9 B) | `ChangeSetRecord` (§2.5) | **no** | «qué operación produjo este cambio» y recorrido por orden de commit |
| `0x31` `CS_MEMBER` | `31 ‖ commit_seq:u64be ‖ ordinal:u32be` (13 B) | `(unit_id, rev_no, kind)` | **no** | miembros de un `ChangeSet`, sin límite práctico de tamaño |

Las etiquetas `0x02..0x0F`, `0x11..0x1F`, `0x22..0x2F` y `0x32..0xFF` quedan reservadas para etapas posteriores (relaciones, derivados). Un registro con etiqueta desconocida **dentro del rango de una etiqueta conocida** es `Corruption`; una etiqueta desconocida de primer nivel es `IncompatibleLayer` (una capa más nueva la escribió).

Por qué `REV_META` y `REV_PAYLOAD` separados: listar el historial (`history`, `compare` por metadatos, GC) lee solo `20…`, de modo que una unidad de 800 KiB con 1 000 revisiones no obliga a leer 800 MiB para pintar una lista. El LSM guarda los dos rangos en bloques distintos. El coste es una clave pequeña más por revisión en el mismo batch.

### 2.3 `UnitHead` (estado vigente)

Campos (codificación: byte `envelope_version`, enteros de ancho fijo big-endian, cadenas y bytes con prefijo de longitud `u32be`; la especificación byte a byte irá a `docs/file-format/units-v1.md`, WP-E3-01):

| Campo | Tipo | Nota |
|---|---|---|
| `unit_type` | cadena ≤ 64 B | spec §6.1 |
| `schema_version` | `u32` | del esquema del contenido, no de la capa |
| `lifecycle` | `u8` | `Active=0`, `Archived=1`, `Deleted=2` |
| `head_rev` | `u64` | revisión vigente (`current_revision_id`); ≥ 1 |
| `oldest_rev` | `u64` | primera revisión **retenida** (1 si no hubo GC); lo escribe el GC, §5 |
| `created_seq`, `updated_seq` | `u64` | `commit_sequence` de la primera y de la última revisión |
| `created_at`, `updated_at` | `i64` µs Unix | del reloj inyectado (§6.7); descriptivos |
| `content_hash` | `alg:u8 ‖ [u8;32]` | hash de la carga útil de `head_rev`; ver §2.7 |
| `payload_len` | `u32` | de la carga útil de `head_rev` |
| `payload_codec` | `u8` | `0` bytes opacos; el resto lo asigna ADR-0008 |
| `metadata` | bytes ≤ 4 KiB | metadatos de la unidad (spec §6.1) |

La **carga útil vigente no está en la cabeza**: está en `REV_PAYLOAD(unit, head_rev)`. `get_unit` hace dos `get` puntuales (cabeza y carga útil) y verifica el hash. Se eligió así, y no duplicar los bytes en la cabeza como hacía el diseño de M0, porque (a) la escritura de una actualización sube de «2× carga útil» a «1× carga útil», (b) no existe un estado en el que cabeza y revisión vigente divergen, y (c) T1 está dominado por el `fdatasync` del WAL (nota del proyecto #88), de modo que bytes extra en el WAL cuestan, pero un `get` extra de una clave en caché no. Es la decisión 5 de §12 (alternativa: duplicar para leer en una sola búsqueda).

### 2.4 `RevisionMeta` (inmutable)

| Campo | Tipo | Nota |
|---|---|---|
| `kind` | `u8` | `Create=1`, `Update=2`, `Archive=3`, `Unarchive=4`, `Delete=5` (lápida), `Restore=6`; `7..=15` reservados (p. ej. `Delta`, §3.3) |
| `parent_rev` | `u64` | `0` solo en `Create` |
| `commit_seq` | `u64` | `commit_sequence` (spec §6.2); también la clave de su `ChangeSet` |
| `created_at` | `i64` µs | |
| `schema_version` | `u32` | |
| `content_hash`, `payload_len`, `payload_codec` | como en §2.3 | `payload_len = 0` y hash del vacío en `Delete`; bajo política `audit` se conservan hash y longitud con `payload_present = 0` (ver «Nota de E3-05») |
| `payload_present` | `u8` | `1` si existe `REV_PAYLOAD` para esta revisión (cambia a ausente por política o GC, §3.4/§5) |
| `restored_from` | `u64` | `0` salvo en `Restore` |
| `metadata` | bytes ≤ 4 KiB | los metadatos **versionados** con la revisión (decisión 6) |
| `actor_ref`, `source_ref` | bytes ≤ 256 B c/u, opcionales | «sin contexto inventado» (revision-model §1.5) |

`revision_id` público es `(unit_id, rev_no)`: contador por registro (decisión ya inclinada en revision-model §7.1 por claves más cortas y orden trivial). No existe un `revision_id` UUID propio en la Etapa 3.

`Archive`/`Unarchive`/`Restore` **copian** la carga útil completa (`REV_PAYLOAD` propio): ninguna revisión depende de que otra siga existiendo, lo que hace segura la retención por rangos (§5). Las revisiones por referencia o por delta son una optimización posterior detrás del byte `kind` (§3.3).

### 2.5 `ChangeSetRecord`

| Campo | Tipo | Nota |
|---|---|---|
| `change_set_id` | implícito | **es** la `commit_sequence` (clave `30 ‖ commit_seq`) |
| `created_at` | `i64` µs | |
| `member_count` | `u32` | número de claves `CS_MEMBER` |
| `purpose` | `u8` | `Application=0`, `Retention=1` (GC, §5), `Maintenance=2` |
| `reason`, `actor_ref`, `source_ref`, `correlation` | bytes ≤ 1 KiB c/u, opcionales | spec §6.3: motivo, actor/fuente, correlación |

Que `change_set_id == commit_sequence` evita un contador aparte y hace que «recorrer cambios por orden de commit» (auditoría, sincronización de derivados de la Etapa 8) sea un `scan` de `0x30`. El hueco entre secuencias (commits que no son de la capa, o `Retention` no escritos) es normal: los `ChangeSet` no son contiguos.

**Cómo se conoce la secuencia antes del commit.** La `Txn` mantiene la ranura de escritura; el contrato garantiza que nada más commitea entre `begin_write` y `commit`, y que un commit no vacío recibe `seq + 1` (I-04). Por tanto `commit_seq = txn.read_seq() + 1` se calcula **antes** de escribir `REV_META` y `CHANGESET`, y el commit devuelve el mismo valor. La capa lo comprueba tras el commit; una discrepancia no puede deshacerse y se trata como violación de invariante (`UnitError::Storage(Error::Poisoned)` y el handle se invalida), con una prueba que la hace imposible en la práctica (§8, `seq_prediction`). **Cambio de contrato requerido:** `read_seq()` hoy es un método inherente de `kerno_storage::WriteTxn`; la Etapa 3 añade `fn read_seq(&self) -> u64` al trait `kerno_core::Txn` (implementado por `WriteTxn` y por `EmulatedTxn`), única modificación fuera de `kerno-units`, hecha en E3-01. Alternativa descartada: escribir primero y anotar la secuencia en un segundo commit (rompe I-21).

### 2.6 Marca de capa y compatibilidad

`LAYER_META["layer"]` guarda `layer_version` (`1` en la Etapa 3). El primer `ChangeSet` de una base nueva lo crea en el mismo commit. En `open`:

- base vacía y `create_if_missing` → se crea la marca con el primer commit;
- base con claves pero sin marca → `UnitError::NotAUnitDatabase` (no se adivina);
- `layer_version` mayor que la soportada → `UnitError::IncompatibleLayer` sin modificar nada (misma filosofía que I-15/HDR-4);
- modo solo lectura: nunca crea nada (I-18); una base vacía en solo lectura se abre como vacía.

Esto no sustituye un bit de característica: un motor antiguo (o un acceso directo a `KernoDb`) puede escribir claves sin pasar por la capa. Mientras `kerno-storage` sea la API inestable de la Fase 1 y el único consumidor sea la capa, se acepta; la decisión 3 de §12 pide al propietario que fije si hace falta el bit `incompat` «UNITS».

### 2.7 Hash de contenido y serialización

`content_hash` = hash de `payload_codec ‖ payload` (la carga útil tal como se guarda). La Etapa 3 trata la carga útil como **bytes opacos**: la serialización canónica (RFC 8785 u otra) y el algoritmo son ADR-0008 (pendiente). El campo lleva un byte `alg` precisamente para no codificar el algoritmo en el formato de la capa. Propuesta: BLAKE3-256 si se acepta una dependencia nueva (justificación pendiente: no existe en la biblioteca estándar ni en el manifiesto actual), o SHA-256 en su defecto; en ambos casos la dependencia entra con ADR-0008 y no antes (decisión 7). **Resuelto (2026-10-31):** ADR-0008 aceptado con BLAKE3-256 (`alg = 1`) y la dependencia `blake3 = "=1.8.7"` fijada; la verificación está en `units-v1.md` §5.1 (E3-02). Mientras tanto la capa puede usar `alg = 0` con CRC-32C ya disponible en el proyecto, marcado como «sin garantía criptográfica», solo para detectar discordancia extremo a extremo. El hash **se verifica en cada lectura de carga útil**: discordancia = `Corruption` (I-08), nunca datos distintos de los confirmados.

### 2.8 Ejemplo de disposición

Unidad `U`, tres cambios (crear, actualizar, archivar), políticas `versioned`:

```
10 U                    -> head{head_rev=3, oldest_rev=1, lifecycle=Archived, ...}
20 U 0000000000000001   -> meta{Create,  commit_seq=41}
20 U 0000000000000002   -> meta{Update,  commit_seq=57}
20 U 0000000000000003   -> meta{Archive, commit_seq=90}
21 U 0000000000000001   -> payload v1
21 U 0000000000000002   -> payload v2
21 U 0000000000000003   -> payload v2 (copia)
30 0000000000000029     -> changeset{41}      31 0000000000000029 00000000 -> (U, 1, Create)
30 0000000000000039     -> changeset{57}      ...
```

## 3. Revisiones, historial y lecturas as-of

### 3.1 Inmutabilidad

Una clave `REV_META`/`REV_PAYLOAD` se **crea** una vez. Dentro de la `Txn` el `ChangeSet` calcula `rev_no = head_rev + 1` leyendo la cabeza, por lo que la clave no puede existir; aun así la capa hace un `get` de comprobación en modo de pruebas (`debug_assert`) y `verify_units` (§8.5) lo comprueba offline. Solo dos operaciones eliminan claves de revisión: la retención (§5) y la supresión de ADR-0009 (fuera de esta etapa, §10); **ninguna** las reescribe. Esto convierte «revisión inmutable» en una propiedad comprobable por barrido: dos instantáneas cualesquiera deben coincidir en todas las claves `0x20/0x21` que ambas contienen.

### 3.2 Escrituras por operación

Todas ocurren en la `Txn` del `ChangeSet`; `s = txn.read_seq() + 1`.

| Operación | Lee | Escribe |
|---|---|---|
| `create(type, payload, meta)` | nada (comprueba que `unit_id` no existe) | `UNIT_HEAD`, `REV_META(1)`, `REV_PAYLOAD(1)`, `CS_MEMBER` |
| `update(id, payload, expected)` | `UNIT_HEAD` | `UNIT_HEAD` (nuevo), `REV_META(n+1)`, `REV_PAYLOAD(n+1)`, `CS_MEMBER` |
| `archive` / `unarchive` | `UNIT_HEAD` + carga útil vigente | ídem `update` con `kind` propio y copia de la carga útil |
| `delete(id)` | `UNIT_HEAD` | `UNIT_HEAD{lifecycle=Deleted}`, `REV_META(n+1){Delete}`, `CS_MEMBER` (sin carga útil) |
| `restore(id, rev)` | `UNIT_HEAD`, `REV_META(rev)`, `REV_PAYLOAD(rev)` | ídem `update` con `kind=Restore`, `restored_from=rev` y copia completa |
| (una vez por `ChangeSet`) | | `CHANGESET(s)` con `member_count` |

`Delete` es lógico: la cabeza permanece (con `lifecycle=Deleted`) para que el historial y las lecturas as-of sigan funcionando. Recrear con el mismo `unit_id` está prohibido (`AlreadyExists`); se crea otro `unit_id`. El borrado físico es ADR-0009 (§10).

`expected` es una `RevNo` opcional (comprobación optimista): si la cabeza ya no está en esa revisión, `Conflict { expected, found }`. Con una `Txn` serializable no hay carrera dentro del `ChangeSet`, pero sí entre la lectura de la aplicación y el `begin_changeset`.

**Restaurar** (I-22): crea una revisión nueva cuyo contenido es el de `rev`; las revisiones posteriores a `rev` **permanecen**. Restaurar una revisión podada es `Pruned`.

### 3.3 Snapshots y deltas

La Etapa 3 guarda **una carga útil completa por revisión** (cada `REV_PAYLOAD` es un snapshot). Razones: (a) el formato de delta es ADR-0008 (pendiente) y no debe codificarse algo difícil de revertir antes de aceptarlo (ADR-0004 lo exige); (b) sin deltas, la reconstrucción es O(1) y la retención por rangos no necesita reescribir la primera revisión retenida; (c) el coste es espacio, medible con el `history_sim` de M0. El byte `kind` y la reserva `7..=15` dejan sitio para `Delta` con profundidad máxima `K` y fracción `F` (revision-model §3) cuando haya benchmarks que los fijen; añadirlo es un cambio de `layer_version`, no de formato de almacenamiento. Decisión 4 de §12.

### 3.4 Políticas de historial (SAD §8.4)

Por tipo de unidad, guardadas en `LAYER_META` (`policy/<unit_type>`; por defecto la de `UnitConfig`). Cambiar la política escribe el registro dentro de un `ChangeSet` `Maintenance`; no reescribe revisiones existentes.

| Política | `REV_META` | `REV_PAYLOAD` | `CHANGESET` + miembros | Lecturas posibles |
|---|---|---|---|---|
| `none` | no | solo la vigente: cada actualización borra la carga útil anterior en el mismo batch | sí (provenance mínima) | solo estado vigente (y as-of contra una instantánea del motor) |
| `audit` | sí, `payload_present=0` salvo la vigente | solo la vigente | sí | historial de metadatos y hashes; no contenido pasado |
| `versioned` | sí | todas (hasta la retención) | sí | todo |

Una lectura que exige contenido pasado bajo `none`/`audit` devuelve `UnitError::HistoryUnavailable { policy }`, no un resultado vacío.

### 3.5 Lecturas

Todas las lecturas pasan por una instantánea del motor (la de la `UnitDb` en cada llamada suelta, o la fijada por `UnitSnapshot`), de modo que heredan I-05 y una cabeza leída junto a sus revisiones es coherente aunque un `ChangeSet` o la retención confirmen entre medias.

- **`get_unit(id)`**: `get(10‖id)` + `get(21‖id‖head_rev)` + verificación de hash. `Deleted` devuelve la cabeza sin carga útil.
- **`get_revision(id, rev_no)`**: `get(20…)` + `get(21…)`; fuera de `[oldest_rev, head_rev]` → `Pruned` (por debajo) o `RevisionNotFound` (por encima).
- **`history(id, range, order, limit)`**: `scan` de `20‖id‖[from, to)` solo de metadatos. `Order::Desc` se resuelve con una página hacia delante desde `max(oldest, head - limit + 1)` y se invierte en memoria (el motor no tiene escaneo inverso; el límite acota la lectura).
- **`unit_as_of(id, AsOf::Seq(s))`**: dado que `commit_seq` crece con `rev_no` dentro de una unidad, busca la mayor revisión con `commit_seq ≤ s` por **búsqueda binaria con `get` de `REV_META`** en `[oldest_rev, head_rev]` (O(log n) lecturas de ~100 B). Resultado: `Absent` (`s` anterior a la creación), `Present(Revision)`, `DeletedAt(rev)`, o `Pruned` si `s` cae antes de `oldest_rev`. Una `s` mayor que la secuencia de la instantánea es `InvalidArgument`. *Alternativa descartada por ahora:* índice secundario `22‖id‖(!commit_seq)` con la secuencia invertida, que resuelve «el último ≤ s» con un solo `scan` hacia delante; añade una clave por revisión y se reconsidera si la búsqueda binaria no cumple los objetivos `U` (§9.4).
- **`state_as_of(AsOf::Seq(s))`** (estado de **todas** las unidades en `s`): recorre `10…` y aplica `unit_as_of` a cada una cuya `updated_seq > s`; si `updated_seq ≤ s` la cabeza ya es la respuesta. Es lineal en unidades; se documenta como tal. El recorrido por `CHANGESET` desde `s+1` permite la variante incremental («qué cambió desde `s`»): `scan` de `30‖(s+1)…` y sus `31`.
- **`compare(id, a, b)`**: devuelve ambas `Revision` y `same_content` (comparación de hash y bytes). La **diferencia estructural** (JSON Patch u otra) la define ADR-0008; la Etapa 3 no la inventa.
- **`AsOf::Time`** (instante de reloj) **queda fuera**: el reloj no es monótono y I-26 prohíbe ordenar con él. Si se quiere, se define como «último `ChangeSet` cuyo `created_at ≤ t` escaneando por secuencia» y se documenta que depende del reloj (decisión 8).

### 3.6 Orden y visibilidad

Visibilidad transaccional (spec §6.2): un `ChangeSet` es visible completo o nada, y solo tras `commit` (el motor publica `last_seq` después del `fsync` en `Sync`, I-16). Un lector con una `UnitSnapshot` en `s` ve exactamente los `ChangeSet` con `commit_seq ≤ s`.

## 4. `ChangeSet` como commit atómico

- `begin_changeset(meta)` llama a `begin_write()`: espera la ranura de escritura (los `ChangeSet` y los commits ciegos se serializan, nota #2: un segundo `begin_*` en el mismo hilo se bloquea; el `ChangeSet` es `!Send` solo si lo es la `Txn` y la documentación lo advierte).
- Cada operación valida (tamaños, estado de ciclo de vida, `expected`) y escribe en la `Txn`; **ninguna toca estado compartido** hasta `commit` (contrato del motor): abortar o soltar no deja rastro ni consume secuencia (I-01).
- `commit()`: escribe `CHANGESET(s)` con `member_count` y devuelve `CommitInfo { change_set_id, commit_seq, revisions }`. Un `ChangeSet` sin operaciones no escribe nada y devuelve `CommitInfo::empty(seq_actual)` (el motor no consume secuencia).
- **I-21** se cumple por construcción: contenido (`10`, `21`), revisión (`20`) y registro (`30`, `31`) están en la misma `Txn`, esto es, en la misma trama del WAL. No hay protocolo de commit propio; si en una etapa posterior los índices son transaccionales (Etapa 4), entran en la misma `Txn`.
- Límites: el motor rechaza un lote que excede `max_batch_ops`/`max_batch_bytes` con `InvalidArgument` **al escribir**, antes de cualquier efecto; la capa lo traduce a `ChangeSetTooLarge { ops, bytes }`. Una unidad con carga útil > `max_value_len - 0` es `PayloadTooLarge` (sin troceado en esta etapa; subir `Config::limits.max_value_len` hasta 64 MiB es ajuste, no cambio de formato).
- Un `ChangeSet` largo retiene la ranura de escritura: se documenta y se ofrece `UnitConfig::max_changeset_ops` (por defecto 10 000 operaciones) como protección. Los trabajos de retención usan lotes pequeños (§5).
- Durabilidad: la de `Config::durability` del motor. `CommitInfo` indica `durable: bool` (`true` en `Sync`; en `Buffered`, hasta `sync()`). La capa expone `sync()` tal cual (I-17).
- Envenenamiento: un `fsync` fallido devuelve `UnitError::Storage(Poisoned)`; la `UnitDb` queda inutilizable hasta reabrir, y la recuperación (I-10, I-11) decide el estado. Nunca se reintenta el commit.

## 5. Almacenamiento del historial, retención y GC

### 5.1 Qué ocupa espacio y cuándo se libera

Las revisiones son claves normales: ocupan espacio hasta que se borran **y** el LSM las reescribe. Hay tres capas, cada una con su responsabilidad:

1. **Retención lógica** (la capa): decide qué revisiones dejan de ser alcanzables y las borra con `delete_range`.
2. **Compactación del LSM** (el motor): descarta las claves tapadas por lápidas cuando ya no las ve ninguna instantánea (I-05). La capa no lo controla, salvo pedir `compact_range` sobre el rango (`CompactRangeReport`).
3. **WAL**: se recicla con el flush; no contiene historial que alguien necesite (I-20: el WAL solo sirve para recuperación).

Consecuencia que debe ser visible en la API: **una instantánea larga retiene el historial podado** (I-05). `UnitDb::metrics()` reexporta las métricas de instantáneas del motor (cuántas hay, qué retienen) y `RetentionReport` informa `bytes_logically_freed` aparte de `bytes_reclaimed` (esto último solo tras `compact`).

### 5.2 Políticas de retención

`RetentionPolicy` por tipo de unidad: `KeepAll` (por defecto), `KeepLast(n)`, `KeepFor(duration)` (usa `created_at` de la revisión; descriptivo, no ordena), o `KeepSince(commit_seq)`. Reglas:

- **Siempre se conserva** la revisión vigente y la última revisión previa a un `Delete`; la retención nunca deja una unidad sin cabeza válida.
- Podar = en **una** `Txn` por unidad: `delete_range(20‖id‖[oldest, k))`, `delete_range(21‖id‖[oldest, k))`, `UNIT_HEAD{oldest_rev=k}` y un `CHANGESET{purpose=Retention}` con un miembro por unidad. Cabeza y rangos cambian juntos (I-21); una caída deja la unidad podada entera o sin podar.
- No hay reescritura de la primera revisión retenida (a diferencia del diseño de M0 con deltas) porque cada revisión es un snapshot completo (§3.3).
- Los `CHANGESET` y `CS_MEMBER` **no se podan** en la Etapa 3: son la procedencia (pequeños: ~60 B por revisión). Un miembro cuya revisión fue podada se informa como `pruned` (la cabeza de la unidad dice `oldest_rev`). Su borrado pertenece a la supresión de ADR-0009 (decisión 9).
- La retención se ejecuta **a petición** (`UnitDb::run_retention(budget)`), no en un hilo propio, en lotes acotados por `budget { max_units, max_ops }` y reanudable: al ser cada lote atómico y autosuficiente, no hay estado intermedio que recuperar y repetirla es idempotente. Un programador automático es posterior.

### 5.3 Lo que no promete

El borrado lógico **no** es borrado físico: las versiones podadas siguen en SST y WAL hasta la compactación y el reciclado, y en copias de seguridad (revision-model §5). Las garantías de supresión (ADR-0009, ADR-0006) no se declaran en esta etapa; el informe de la etapa lo dice explícitamente.

## 6. Primera API pública de Rust

Sujeta a ADR-0010 (crates y API pública, pendiente): **propuesta** de crate nuevo `kerno-units` (nombre provisional) que depende de `kerno-core` y, para el tipo concreto, de `kerno-storage`. Inestable hasta aceptar ADR-0010. Las firmas son orientativas; los nombres de tipos son los que se prueban.

### 6.1 Apertura

```rust
pub struct UnitConfig {
    pub storage: kerno_storage::Config,      // durabilidad, límites, perfil
    pub create_if_missing: bool,
    pub default_policy: HistoryPolicy,
    pub max_changeset_ops: usize,            // 10_000 por defecto
    pub clock: Arc<dyn Clock>,               // inyectable (determinismo en SimVfs)
    pub ids: Arc<dyn UnitIdSource>,          // inyectable
}

pub struct UnitDb<E: TxnEngine = KernoDb> { /* engine, config, cache de políticas */ }
pub type KernoUnits = UnitDb<KernoDb>;

impl<E: TxnEngine> UnitDb<E> {
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &UnitConfig) -> Result<Self, UnitError>;
    pub fn close(self) -> Result<(), UnitError>;
    pub fn sync(&self) -> Result<u64, UnitError>;        // I-17 (requiere E: SyncEngine)
}
```

`open` abre el motor, valida la marca de capa (§2.6) y no escribe nada en solo lectura. Un parámetro por defecto de tipo permite el uso normal sin genéricos y las pruebas con `Emulated<LogEngine>`.

### 6.2 Tipos de valor

```rust
pub struct UnitId([u8; 16]);                    // interno (D-05); el id público es ADR-0005
pub struct RevNo(u64);                          // 1-based
pub struct RevisionId { pub unit: UnitId, pub rev: RevNo }
pub struct ChangeSetId(u64);                    // == commit_sequence
pub struct CommitSeq(u64);

pub enum Lifecycle { Active, Archived, Deleted }
pub enum ChangeKind { Create, Update, Archive, Unarchive, Delete, Restore }
pub enum HistoryPolicy { None, Audit, Versioned }

pub struct Unit { pub id: UnitId, pub unit_type: String, pub schema_version: u32,
                  pub lifecycle: Lifecycle, pub head: RevNo, pub oldest: RevNo,
                  pub created_at: Timestamp, pub updated_at: Timestamp,
                  pub metadata: Vec<u8>, pub payload: Option<Vec<u8>>, pub content_hash: ContentHash }
pub struct Revision { pub id: RevisionId, pub kind: ChangeKind, pub parent: Option<RevNo>,
                      pub commit_seq: CommitSeq, pub change_set: ChangeSetId, pub created_at: Timestamp,
                      pub schema_version: u32, pub content_hash: ContentHash,
                      pub restored_from: Option<RevNo>, pub provenance: Provenance,
                      pub payload: Option<Vec<u8>> }       // None en historial de metadatos
pub struct Provenance { pub actor: Option<Vec<u8>>, pub source: Option<Vec<u8>> }
```

`Timestamp` es un `i64` de microsegundos Unix con nombre propio. Los payloads son `Vec<u8>` (bytes opacos, §2.7); ADR-0008 añadirá codecs tipados encima, sin cambiar la capa.

### 6.3 Escritura

```rust
pub struct ChangeMeta { pub reason: Option<String>, pub actor: Option<Vec<u8>>,
                        pub source: Option<Vec<u8>>, pub correlation: Option<Vec<u8>>,
                        pub purpose: ChangePurpose }

impl<E: TxnEngine> UnitDb<E> {
    pub fn begin_changeset(&self, meta: ChangeMeta) -> Result<ChangeSet<'_, E>, UnitError>;
    /// Atajos de una sola operación (un ChangeSet de un miembro).
    pub fn create_unit(&self, new: NewUnit, meta: ChangeMeta) -> Result<(UnitId, CommitInfo), UnitError>;
}

impl<'db, E: TxnEngine> ChangeSet<'db, E> {
    pub fn create(&mut self, new: NewUnit) -> Result<UnitId, UnitError>;
    pub fn update(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn archive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn unarchive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn delete(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn restore(&mut self, id: UnitId, from: RevNo, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;   // lee sus propias escrituras
    pub fn commit(self) -> Result<CommitInfo, UnitError>;
    pub fn abort(self);                                                       // y Drop
}

pub struct CommitInfo { pub change_set: ChangeSetId, pub commit_seq: CommitSeq,
                        pub revisions: Vec<RevisionId>, pub durable: bool }
```

`create` y las demás **no** son `async`; bloquean como `begin_write` (contrato del motor). Un `ChangeSet` vacío en `commit` no escribe nada.

### 6.4 Lectura e historial

```rust
impl<E: TxnEngine> UnitDb<E> {
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;
    pub fn snapshot(&self) -> Result<UnitSnapshot<E>, UnitError>;
}

impl<E: TxnEngine> UnitSnapshot<E> {
    pub fn seq(&self) -> CommitSeq;
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;
    pub fn get_revision(&self, id: UnitId, rev: RevNo) -> Result<Revision, UnitError>;
    pub fn history(&self, id: UnitId, q: HistoryQuery) -> Result<Vec<Revision>, UnitError>;
    pub fn unit_as_of(&self, id: UnitId, at: AsOf) -> Result<UnitAt, UnitError>;
    pub fn compare(&self, id: UnitId, a: RevNo, b: RevNo) -> Result<Comparison, UnitError>;
    pub fn change_set(&self, id: ChangeSetId) -> Result<ChangeSetInfo, UnitError>;
    pub fn change_sets_since(&self, after: CommitSeq, limit: usize) -> Result<Vec<ChangeSetInfo>, UnitError>;
    pub fn scan_units(&self, after: Option<UnitId>, limit: usize) -> Result<Vec<Unit>, UnitError>;
}

pub struct HistoryQuery { pub from: Option<RevNo>, pub to: Option<RevNo>,
                          pub order: Order, pub limit: usize, pub with_payload: bool }
pub enum AsOf { Seq(CommitSeq) }                        // Time queda fuera (decisión 8)
pub enum UnitAt { Absent, Present(Revision), DeletedAt(Revision), Pruned { oldest: RevNo } }
```

Los resultados paginados llevan `limit` explícito y devuelven el último `RevNo`/`UnitId` para continuar; ninguna consulta carga un historial entero sin tope. Los tipos de retorno compuestos se nombran con `type` alias (regla del proyecto: `clippy -D warnings` rechaza `clippy::type_complexity`, constraint #17).

### 6.5 Mantenimiento

```rust
impl<E: TxnEngine> UnitDb<E> {
    pub fn set_policy(&self, unit_type: &str, p: HistoryPolicy, meta: ChangeMeta) -> Result<CommitInfo, UnitError>;
    pub fn run_retention(&self, policy: &RetentionPolicy, budget: RetentionBudget) -> Result<RetentionReport, UnitError>;
    pub fn compact(&self) -> Result<CompactReport, UnitError>;        // delega en compact_range
    pub fn verify_units(&self) -> Result<UnitVerifyReport, UnitError>; // §8.5, solo lectura
    pub fn metrics(&self) -> UnitMetrics;                              // incluye las del motor
}
```

### 6.6 Errores

```rust
#[non_exhaustive]
pub enum UnitError {
    Storage(kerno_core::Error),                 // Io, Corruption, Locked, Poisoned, Incompatible, ReadOnly, InvalidArgument
    NotFound { unit: UnitId },
    AlreadyExists { unit: UnitId },
    Deleted { unit: UnitId },                   // operación inválida sobre una unidad en lápida
    Conflict { unit: UnitId, expected: RevNo, found: RevNo },
    RevisionNotFound { unit: UnitId, rev: RevNo },
    Pruned { unit: UnitId, oldest: RevNo },
    HistoryUnavailable { policy: HistoryPolicy },
    PayloadTooLarge { len: usize, max: usize },
    ChangeSetTooLarge { ops: usize, bytes: usize },
    InvalidArgument(String),
    NotAUnitDatabase,
    IncompatibleLayer { found: u32, supported: u32 },
    ChangeSetClosed,
}
```

Reglas: (a) todo error del motor se conserva **sin cambiar de clase** dentro de `Storage` (en particular `Corruption` nunca se degrada a un error de la capa y `Poisoned` sigue implicando «reabrir»); (b) un registro de la capa que no decodifica o cuyo hash no coincide es `Storage(Corruption)` con contexto (qué clave, qué regla `UNIT-n`); (c) ningún mensaje contiene contenido ni metadatos de la unidad, solo ids y números (I-29, I-37).

### 6.7 Determinismo

`Clock` y `UnitIdSource` son traits inyectables (`SystemClock`/`UuidV7Source` por defecto; `FixedClock`/`CounterIds` en pruebas). Sin ellos, los barridos de `SimVfs` no serían reproducibles por semilla. La capa no usa `std::time` ni aleatoriedad directamente, igual que `kerno-storage` no usa `std::fs` (todo pasa por `Vfs`).

## 7. Invariantes

Reutiliza I-01…I-19 del motor y concreta I-20…I-22, I-26 y I-31. Las nuevas son propuestas (`IU-n`) para promoverse a `invariants.md` cuando haya prueba, con la regla de ese documento: una invariante sin prueba automatizada se marca *pendiente*.

| ID | Invariante | Verificación (§8) |
|---|---|---|
| I-20 | El historial no depende del WAL: tras flush y reciclado del WAL, `history`/`get_revision` devuelven lo mismo; truncar o reciclar el WAL nunca borra revisiones. | `wal_independence`, barridos con perfil `Inline` (WAL reciclado, nota #27) |
| I-21 | Cabeza, `REV_META`, `REV_PAYLOAD`, `CHANGESET` y `CS_MEMBER` de un `ChangeSet` aparecen todos o ninguno tras cualquier caída. | `changeset_every_point`, `model_crash` |
| I-22 | Restaurar crea una revisión nueva; ninguna revisión posterior a la restaurada cambia ni se borra. | `restore_keeps_later`, comparación de instantáneas antes/después |
| I-26 | El orden histórico es `commit_sequence`; `rev_no` y `commit_seq` crecen juntos por unidad; ni UUIDv7 ni el reloj participan. | `ordering_ignores_clock` (reloj hacia atrás) |
| IU-1 | **Revisión inmutable**: una clave `0x20/0x21` existente nunca cambia de valor. | `immutability_sweep` entre instantáneas |
| IU-2 | **Cabeza coherente**: `head_rev` existe, su `REV_META` coincide en hash/longitud/esquema con la cabeza y `oldest_rev ≤ head_rev`; `REV_META` cubre `[oldest_rev, head_rev]` sin huecos (bajo `versioned`). | `verify_units` |
| IU-3 | **Cadena y orden**: `parent_rev = rev-1`, `commit_seq` estrictamente creciente en la unidad. | `verify_units` |
| IU-4 | **Procedencia total**: toda `REV_META` con `commit_seq = s` tiene `CHANGESET(s)` y un `CS_MEMBER` que la nombra; `member_count` = nº de miembros. | `verify_units` |
| IU-5 | **Hash extremo a extremo**: el hash de cada carga útil leída coincide; si no, `Corruption`. | `payload_hash_checked`, barrido de daños (§8.4) |
| IU-6 | **As-of exacto**: `unit_as_of(id, s)` coincide con el modelo en todo `s` ≤ `seq` de la instantánea, incluidos anteriores a la creación y posteriores al borrado. | `model_as_of` |
| IU-7 | **Retención atómica y segura**: tras `run_retention` (y tras una caída en cualquier punto) cada unidad tiene un `oldest_rev` coherente con las claves existentes y conserva su revisión vigente. | `retention_every_point` |
| IU-8 | **Aislamiento del historial**: una `UnitSnapshot` ve el historial de su `seq` aunque la retención o nuevos `ChangeSet` confirmen después (revision-model §7.5). | `snapshot_vs_retention` |
| IU-9 | **Política respetada**: bajo `none` no queda ninguna `REV_META`/`REV_PAYLOAD` antigua; bajo `audit`, ninguna carga útil antigua. | `policy_matrix` |
| IU-10 | **Sin contenido en errores ni métricas** (I-29/I-37 aplicados a la capa). | `sentinel_no_content` |

## 8. Plan de pruebas

Marco: el existente (`kerno-testkit`, `SimVfs`, semillas por `SPIKE_SEEDS`, suites compartidas). Cada suite nueva se valida primero contra el motor trivialmente correcto (`LogEngine` con `Emulated<E>`) y después contra `KernoDb`, como manda phase1-design §1.5. Un test que nunca se vio fallar no cuenta: cada suite incluye su mutación (invertir una aserción o saltarse una escritura de la capa) y se comprueba que se pone en rojo.

### 8.1 Modelo y funcionales

- **Modelo de referencia** (`UnitModel`): `BTreeMap<UnitId, Vec<ModelRevision>>` más una lista de `ChangeSet`; aplica las mismas operaciones y define la respuesta correcta de `get_unit`, `history`, `unit_as_of`, `state_as_of`, `change_sets_since` y `compare`.
- `model_ops`: operaciones aleatorias (crear, actualizar, archivar, borrar, restaurar, `ChangeSet` de 1..50 miembros, abortos) con comparación completa tras cada `ChangeSet` y `UnitSnapshot` antiguas guardadas que se re-leen al final (IU-6, IU-8).
- `model_as_of`: para cada `s` en `[0, seq]` y cada unidad, `unit_as_of` = modelo.
- `restore_keeps_later`, `delete_then_history`, `conflict_expected_rev`, `payload_limits`, `changeset_limits` (justo en el límite y uno por encima: sin efectos), `policy_matrix` (3 políticas × operaciones), `ordering_ignores_clock` (reloj inyectado que retrocede).
- `seq_prediction`: con 1..N hilos que intentan commits ciegos y `ChangeSet`, el `commit_seq` escrito en cada `REV_META` es el devuelto por `commit` (§2.5); un `ChangeSet` abortado no consume secuencia.

### 8.2 Caídas y recuperación con `SimVfs`

La capa se prueba con los mismos barridos que el motor, pero con **escenarios de `ChangeSet`** en lugar de lotes:

| Suite | Qué hace | Aserción tras recuperar |
|---|---|---|
| `changeset_every_point` | ejecuta un escenario fijo por semilla; `crash_after(n, kind)` para **cada** operación de E/S mutante `n` (como `crash::every_point`), con `CrashKind::{PowerLoss, ProcessKill}` y `Durability::{Sync, Buffered}` | `last_seq` = prefijo de `ChangeSet` completos; en `Sync`, `s ≥` último confirmado; el estado = modelo tras exactamente `k` `ChangeSet`; `verify_units` sin hallazgos (I-21, IU-2…IU-4) |
| `model_crash` | `model_ops` con caída aleatoria por ronda (`crash_now`, `take_crash_image`, `restore_seeded`) y continuación sobre la imagen recuperada | igual que arriba, y la base **sigue funcionando** (`keeps_working`) |
| `retention_every_point` | `run_retention` sobre unidades con historial largo, caída en cada punto | cada unidad podada entera o sin podar (IU-7); revisión vigente intacta |
| `recovery_reentrant` | caída durante la recuperación de una base con `ChangeSet` en el WAL (I-11) | segunda recuperación idéntica |
| `fsync_failure` | `set_fsync_failure` en el commit de un `ChangeSet` | `Poisoned`; tras reabrir, el `ChangeSet` o está entero o no está; nunca a medias (I-10) |
| `enospc` | `set_space_limit` / `fill_disk_after` durante un `ChangeSet` | error limpio sin efectos parciales; la capa no deja una cabeza sin revisión. Reservar espacio de cabecera de WAL de 96 B (nota #11) |
| `torn_write` | `set_tear_unit` en la trama de un `ChangeSet` grande | la trama rota se descarta entera (WAL-R) |
| `wal_independence` | flush, rotación y reciclado del WAL (perfil `Inline`, nota #27), reapertura | historial idéntico (I-20) |

Los cuatro tipos de caída (`PowerLoss`, `ProcessKill`, escritura rota, fallo de `fsync`) y ambos modos de durabilidad se barren como en la matriz de fallos ([failure-matrix](failure-matrix.md)); se añade una fila por punto específico de la capa (p. ej. «caída entre la cabeza y el `CHANGESET`» no existe como punto observable porque van en la misma trama, y la prueba lo demuestra: ninguna imagen de caída contiene uno sin el otro).

El envoltorio `TxnCommits<E>` (testkit) no se reutiliza: aquí la unidad de commit es el `ChangeSet`, no un `WriteBatch`; el barrido envuelve `UnitDb` en un `Step` propio (`Step::ChangeSet`) en un módulo `harness::units`.

### 8.3 Procesos reales

`kill -9` real sobre un proceso hijo (`kerno-testkit::process`) que ejecuta `ChangeSet` en bucle, con verificación del prefijo tras cada muerte (como `process_kill.rs`), 200 iteraciones en CI nocturna. Sin LazyFS (sigue pendiente de la Fase 1).

### 8.4 Corrupción

`bit_flips` y `truncations` sobre una base con unidades: todo error debe ser `Corruption`, `Io` o `Poisoned` (I-08); además, cambiar un byte de una `REV_PAYLOAD` **con CRC de bloque válido** (parche de SST con recálculo) debe detectarlo el hash de contenido (IU-5, `Corruption`). Marca de capa alterada → `NotAUnitDatabase`/`IncompatibleLayer`, sin escribir (I-18).

### 8.5 `verify_units`

Función de solo lectura sobre una instantánea: recorre `10`, `20`, `21`, `30`, `31` y comprueba IU-2, IU-3, IU-4 y la inmutabilidad entre dos instantáneas (IU-1) sin pasar por la API de lectura. Se ejecuta tras **cada** caída de los barridos de §8.2 y está pensada para exponerse como CLI en la Fase 2/Etapa 9. Cada hallazgo lleva el id de la regla `UNIT-n`, como `verify()` del motor.

### 8.6 Rendimiento

Banco `unit_bench` (en `spikes/storage/spike-bench` o crate de bancos nuevo, según ADR-0010): cargas `unit_updates` (1 unidad, N actualizaciones), `history_scan`, `as_of_point`, `retention`. Se informa p50/p95/p99 y amplificación de escritura frente a escribir la misma carga útil como clave simple en `KernoDb`.

### 8.7 Presupuesto de CI

`SPIKE_SEEDS` por defecto (rápido) en PR; semanal con 1 000 semillas para `changeset_every_point`, `model_crash` y `retention_every_point`, divididos en matriz para no superar el límite de 6 h por trabajo (phase1-design §6.7).

## 9. Compatibilidad, rendimiento y lo que no se toca

### 9.1 Formato v1

| Elemento | Etapa 3 |
|---|---|
| Cabecera común, WAL, SST, MANIFEST, recuperación | **sin cambios** (inmutables mientras exista v1, ADR-0013) |
| Bits de característica | **ninguno nuevo por defecto**; opcional `incompat` «UNITS» solo si el propietario lo decide (§12.3), con vectores y nota en ADR-0013 |
| Byte `layer_version` | dentro de un valor de usuario; versionado propio |
| SST multiversión / compresión / valores separados | no se usan; siguen siendo bits reservados |
| Vectores de directorio de la Fase 1 | siguen válidos byte a byte |

Prueba de guarda: los `dir_vectors` y `struct_vectors` existentes pasan sin regenerar (`cargo test -p kerno-storage --test dir_vectors --test struct_vectors`). Si algún WP necesitara regenerarlos, el WP está mal.

### 9.2 Cómo se evitan los cambios de formato en el futuro

Tres vías, por orden: (1) claves nuevas bajo etiquetas reservadas y `layer_version`; (2) bit `compat`/`ro_compat` del motor si un lector antiguo puede ignorarlo; (3) bit `incompat` (1..62) para lo que un lector antiguo no puede tratar como opaco, con ADR. Las dos primeras no tocan `kerno-storage`.

### 9.3 Coste esperado de la capa (a medir, no a prometer)

Un `update` suelto escribe 5 claves en un commit: `10`, `20`, `21`, `31`, `30` (la última compartida por todos los miembros). Con T1 dominado por el `fdatasync` (≈ 92–98 % de la latencia del commit, nota #88), el coste adicional previsto está en bytes del WAL y CPU de codificación, no en `fsync`. La hipótesis se contrasta en WP-E3-08, no se da por buena.

### 9.4 Serie de métricas `U` (propuesta; el propietario las aprueba **antes** de medir, ADR-0012)

| Id | Métrica | Referencia | Propuesta de objetivo |
|---|---|---|---|
| U1 | p99 de commit de un `update` de 1 KiB, `Sync` | clave simple de 1 KiB en `KernoDb`, misma máquina y sesión | se informa; cota sugerida ≤ 1,5× |
| U2 | `history(id, 100 rev)` p99, solo metadatos | `scan` del mismo nº de claves de 100 B | ≤ 1,5× |
| U3 | `unit_as_of` p99 con 1 000 revisiones | — | se informa; si > 5× un `get`, valorar el índice invertido (§3.5) |
| U4 | amplificación de escritura de `update` 1 KiB y 64 KiB | bytes lógicos escritos | se informa |
| U5 | espacio por revisión tras N actualizaciones (`history_sim`) | M0 | se informa |
| U6 | `run_retention` rendimiento y p99 de escritura concurrente durante la poda | — | sin ampliar T1–T3 en más de lo medido en baja carga |

### 9.5 T1–T10 no cambian

T1–T10 (phase1-design §7.2, [f1-objetivos-rendimiento](../testing/f1-objetivos-rendimiento.md)) miden `kerno-storage` y no se editan, no se reinterpretan y no se relajan por esta etapa. Esta etapa **no depende** de que T1 se cumpla: el estado de T1 y la revisión de contingencia de ADR-0001 son decisiones abiertas del propietario aparte. Lo único que la capa debe garantizar es no empeorarlas: no hay ningún cambio en `kerno-storage` por defecto, y si un WP lo exigiera (p. ej. una métrica nueva) debe aislarse en su propio WP con la comparación de T1–T10 adjunta (CONTRIBUTING).

## 10. Fuera de alcance de la Etapa 3

Relaciones e índices (Etapa 4: sin índice por tipo ni por campo; `scan_units` recorre cabezas); consultas por filtro y planificador (5); supresión/borrado físico y retención legal (ADR-0009; la etapa solo deja el gancho de §5.3); cifrado (ADR-0006: `Incompatible`); IDs públicos y tokens (ADR-0005); diff estructural y canonicalización (ADR-0008); deltas; troceado de cargas útiles grandes; `AsOf::Time`; CLI; exportación/importación (I-30, Etapa 9); sincronización o réplica; múltiples escritores (la ranura de escritura única es una propiedad del motor, I-06).

## 11. Paquetes de trabajo y criterios de aceptación

Todos son secuenciales salvo donde se indica. «Hecho» incluye: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, la suite del WP y la suite completa del espacio de trabajo en verde, el conteo de tests observado antes y después (la suite nueva está dentro de las globs del runner), y cada test nuevo visto fallar por mutación.

| WP | Contenido | Depende de | Aceptación |
|---|---|---|---|
| **E3-00** Puertas de entrada | Confirmar la condición de inicio del roadmap: WP14 integrado, formato v1 congelado (bit `DRAFT` retirado) y ADR-0013 aceptado; decisiones 1–3 de §12 cerradas | — | Acta del propietario; `docs/file-format/v1.md` sin `DRAFT`; ningún WP posterior arranca antes |
| **E3-01** ADR y especificación de la capa | Aceptar ADR-0004 (modelo de revisión), ADR-0010 (crate/API), y proponer ADR-0008 mínimo (hash y bytes opacos); añadir `Txn::read_seq` al trait de `kerno-core` (con `impl` en `WriteTxn` y `EmulatedTxn`); `docs/file-format/units-v1.md` con codificaciones byte a byte, reglas `UNIT-n` y vectores | E3-00 | ADR en estado Propuesto→Aceptado por el propietario; vectores de estructura generados y un decodificador independiente (como `spec_decoder`) los reproduce |
| **E3-02** Codecs y claves (el propietario acotó el 2026-10-31 la primera entrega a la integridad de `REV_PAYLOAD`, `units-v1.md` §5.1; los codecs restantes siguen pendientes) | Crate `kerno-units`: codificación de claves/valores, `UnitModel`, `Clock`, `UnitIdSource`, errores | E3-01 | Round-trip y orden lexicográfico = numérico (propiedad); decodificación rechaza cada regla `UNIT-n` con la clase de error esperada; 0 cambios en `kerno-storage` salvo el `impl` de `Txn::read_seq` añadido en E3-01 (`git diff --stat` lo demuestra) |
| **E3-03** `ChangeSet` y CRUD | `open`, `begin_changeset`, `create/update/archive/unarchive/delete`, `get_unit`, límites, predicción de secuencia | E3-02 | `model_ops` y `seq_prediction` en verde sobre `Emulated<LogEngine>` **y** `KernoDb`; I-21 por construcción demostrado en `changeset_every_point` (versión rápida) |
| **E3-04** Historial y as-of | `history`, `get_revision`, `unit_as_of`, `state_as_of`, `change_sets_since`, `compare`, `restore` | E3-03 | `model_as_of`, `restore_keeps_later` (I-22), `ordering_ignores_clock` (I-26), `snapshot_vs_retention` parcial |
| **E3-05** Políticas de historial | `none`/`audit`/`versioned` por tipo, `set_policy`, `HistoryUnavailable` | E3-03 | `policy_matrix` (IU-9); los errores de política no son resultados vacíos |
| **E3-06** Retención y GC | `RetentionPolicy`, `run_retention`, `compact`, `RetentionReport` | E3-04, E3-05 | `retention_every_point`, `snapshot_vs_retention` (IU-8), espacio recuperado medible tras `compact` en `history_sim` |
| **E3-07** `verify_units` y corrupción | verificador offline, hash de contenido en lectura, daños | E3-03 | `verify_units` sin hallazgos tras cada barrido; los daños de §8.4 dan `Corruption`/`IncompatibleLayer`; I-18 |
| **E3-08** Barridos de caída | `harness::units`, `changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure`, `enospc`, `torn_write`, `wal_independence`, `kill -9` real | E3-03…E3-07 | todos en verde con `SPIKE_SEEDS` por defecto y con 1 000 semillas; nuevas filas en `failure-matrix.md`; I-20 y I-21 marcadas verificadas en `invariants.md` |
| **E3-09** Bancos y métricas `U` | `unit_bench` y objetivos U1–U6 (aprobados antes) | E3-03…E3-06 | Informe `docs/testing/etapa3-report.md` con la comparación contra la referencia; T1–T10 sin tocar (`git diff` de `docs/testing/f1-objetivos-rendimiento.md` vacío) |
| **E3-10** Cierre | Revisión adversarial de la etapa, promoción de IU-1…IU-10, actualización del roadmap y del SAD §8 | todos | Informe de revisión sin bloqueantes abiertos; roadmap actualizado con el estado real; criterios de aceptación del roadmap cumplidos (reconstruir la evolución, restaurar sin destruir posteriores, historial sin WAL, misma frontera atómica en la matriz de fallos) |

Camino crítico: E3-00 → 01 → 02 → 03 → 04; 05, 06, 07 se pueden paralelizar tras 03/04; 08 tras todos; 09 puede empezar tras 06.

**Nota de E3-03 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/src/db.rs`):**

- `UnitConfig` no lleva `kerno_storage::Config`: `kerno-units` no depende de `kerno-storage` (solo como dependencia de desarrollo, para las pruebas sobre `KernoDb`). Lleva `engine: Options` de `kerno-core`, y `UnitDb::from_engine` acepta un motor ya abierto con su configuración propia. Por lo mismo no existe el alias `KernoUnits` ni el parámetro de tipo por defecto `E = KernoDb`.
- Un `ChangeSet` crea como mucho **una revisión por unidad** (I-26, IU-3): una segunda operación sobre la misma unidad es `InvalidArgument` y deja el `ChangeSet` utilizable.
- Ciclo de vida: `update` exige `Active` (sobre `Archived` es `InvalidArgument`); `archive` exige `Active`; `unarchive`, `Archived`; `delete` admite `Active` y `Archived`. Orden de comprobación: `NotFound` → `Conflict` (`expected`) → `Deleted` → ciclo de vida.
- Cada operación valida y codifica todo antes de su primera escritura, así que un rechazo no tiene efecto. Si el motor rechaza una escritura a mitad de operación (límite de lote, E/S), el `ChangeSet` queda roto: devuelve `ChangeSetClosed` y `commit` lo aborta. `InvalidArgument` del motor se traduce a `ChangeSetTooLarge`.
- `ChangeMeta`: `actor` y `source` se copian también en cada `REV_META`, por lo que su límite es 256 B; `Some` vacío se rechaza (el formato no distingue vacío de ausente); `purpose` solo admite `Application` (los demás son de E3-05/E3-06).
- `default_policy` solo admite `Versioned` hasta E3-05 (levantado en E3-05, ver su nota). Con `create_if_missing = false`, una base vacía se abre y se lee vacía, y `begin_changeset` es `InvalidArgument`.
- Una lectura del reloj por `ChangeSet`, usada por el registro y todas sus revisiones. `get_unit` compara la cabeza con el `REV_META` de `head_rev` (IU-2) además de `verify_payload`; una discrepancia es `Storage(Corruption)`.
- Tras una predicción de secuencia fallida el manejador rechaza también las lecturas con `Storage(Poisoned)` hasta reabrir.

**Nota de E3-04 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/src/history.rs` y `ChangeSet::restore` en `db.rs`):**

- `restore(id, from, expected)` exige una unidad `Active` (sobre `Archived` es `InvalidArgument`, sobre `Deleted` es `Deleted`; no «desborra»), con el mismo orden de comprobación que E3-03. `from` debe estar en `[oldest_rev, head_rev]` (`RevisionNotFound` por encima o con `0`, `Pruned` por debajo) y tener carga útil: restaurar una lápida es `InvalidArgument`. Se admite restaurar la propia cabeza o una revisión `Restore`. La revisión nueva copia carga útil, `schema_version` y `metadata` de `from` (decisión 6) y deja la unidad `Active`; la carga útil de origen se verifica antes de copiarla.
- `Revision` lleva además `lifecycle` (deducido de `kind`: `Archive` → `Archived`, `Delete` → `Deleted`, el resto → `Active`), `payload_len` y `metadata`.
- `history`: `from` inclusivo, `to` exclusivo (§3.5); un `to` más allá de la cabeza se recorta, pero un `from` por debajo de `oldest_rev` es `Pruned` y no se recorta en silencio. `limit = 0` es `InvalidArgument` en `history`, `state_as_of` y `change_sets_since`. Cada página comprueba que las `REV_META` sean contiguas y con `commit_seq` estrictamente creciente (IU-3); si no, `Storage(Corruption)`.
- `unit_as_of`: búsqueda binaria por `commit_seq`; cada sonda comprueba que su `commit_seq` esté estrictamente entre las de sus vecinas conocidas y que la cabeza coincida con `REV_META(head_rev)` (y con la revisión 1 si no hubo poda). Una `REV_META` ausente dentro del rango retenido es `Corruption`, nunca `Absent`. `UnitAt::Pruned` es un resultado, no un error.
- `state_as_of(at, after, limit)` se pagina por `unit_id` y **omite** las unidades aún no creadas en `at` (no las devuelve como `Absent`).
- `change_set(id)` devuelve `Option` (`None` si no hay registro con esa secuencia) en lugar de un error nuevo. `change_set`/`change_sets_since` comprueban que haya exactamente `member_count` miembros con ordinales `0..n`; `reason` no UTF-8 es `Corruption`.
- `compare` devuelve ambas revisiones con carga útil verificada y `same_content` (hash y bytes iguales); sin diff estructural (ADR-0008).
- `snapshot_vs_retention` es parcial: comprueba que una `UnitSnapshot` antigua sigue leyendo su historial mientras confirman otros `ChangeSet` (incluidas restauraciones y borrados); la mitad de retención llega con E3-06.

**Nota de E3-05 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/src/db.rs`, `src/history.rs`, `src/model.rs`, `tests/policies.rs`):**

- Se levanta la restricción de E3-03: `default_policy` admite las tres políticas. Es la política escrita en la marca de capa por el primer `ChangeSet`; después cuenta la de la marca (reabrir con otra `default_policy` no la cambia). La política de un tipo es su registro `LAYER_META` `policy/<unit_type>` (valor `LayerMeta`, mismo formato que la marca) y, si no lo tiene, la de la marca.
- El nombre de `LAYER_META` mide como mucho 64 B, así que solo los tipos de 1..=57 B (`MAX_POLICY_UNIT_TYPE_LEN`) admiten `set_policy`; uno más largo es `InvalidArgument` y sigue la política por defecto.
- `set_policy` es un `ChangeSet` `Maintenance` sin miembros (`member_count = 0`) que solo escribe ese registro; acepta `purpose` `Application` o `Maintenance` (registra `Maintenance`), rechaza `Retention`, y fijar la política vigente también confirma. No reescribe revisiones.
- La política vigente al escribir una revisión decide qué pasa con las anteriores, en el mismo lote: `audit` reescribe la `REV_META` de la cabeza sustituida con `payload_present = 0` **conservando hash y longitud** (se sigue §3.4, «historial de metadatos y hashes», y no la nota de §2.4 que pedía hash del vacío) y borra su `REV_PAYLOAD`; `none` borra con dos `delete_range` todas las `REV_META`/`REV_PAYLOAD` de la unidad por debajo de la nueva y pone `oldest_rev` en la nueva revisión. Las cargas útiles de un periodo `versioned` anterior a un cambio a `audit` no se borran (solo la cabeza sustituida); quedan inaccesibles por la regla de lectura y las recogerá la retención (E3-06). Así IU-9 se cumple para toda revisión sustituida bajo `none`/`audit`.
- Las lecturas consultan la política vigente **ahora** del tipo. `HistoryUnavailable { policy }`: `get_revision`/`compare`/`restore` de una revisión por debajo de la cabeza bajo `none`/`audit` (tras `RevisionNotFound` y antes de `Pruned`); `history` siempre bajo `none`, y con `with_payload` siempre bajo `audit` (sin metadatos sí funciona); `unit_as_of`/`state_as_of` entre la creación y la última revisión bajo `none`/`audit` (antes de la creación sigue siendo `Absent`; desde la última revisión responde la cabeza). `state_as_of` falla entera si falla una unidad de su página. Bajo cualquier política, leer el contenido de una revisión no lápida con `payload_present = 0` es `HistoryUnavailable { policy: Audit }`.
- Tras volver de `none` a `versioned`, las revisiones borradas quedan por debajo de `oldest_rev` y se informan como `Pruned` (no se distingue de la retención). La cabeza siempre puede restaurarse (copia la vigente) bajo cualquier política.
- `UnitModel` modela las políticas (`with_default_policy`, `set_policy`, `oldest`, `ModelRevision::content_kept`) y responde las lecturas con política (`read_revision`, `read_history`, `read_as_of`, `read_state_as_of`, `read_restore_source`); las consultas anteriores siguen respondiendo desde la historia lógica completa.

**Nota de E3-06 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/src/retention.rs`, `ChangeSet::restore` en `src/db.rs`, `src/history.rs`, `src/model.rs`, `tests/retention.rs`, `tests/history.rs`, `tests/policies.rs`):**

- API: `RetentionPolicy { default, by_type }` asigna a cada tipo una `RetentionRule` (`KeepAll` por defecto, `KeepLast(n ≥ 1)`, `KeepFor(Duration)`, `KeepSince(CommitSeq)`); `KeepLast(0)` es `InvalidArgument`. `run_retention(&policy, RetentionBudget { max_units ≥ 1, max_ops ≥ 4 })` no recibe `ChangeMeta`: el `CHANGESET` lleva `purpose = Retention` sin motivo ni actor, y ningún `ChangeSet` de aplicación ni `set_policy` puede declarar ese propósito.
- El lote también respeta `UnitConfig::max_changeset_ops` (§4): `max_units` efectivo = `min(max_units, max_changeset_ops)`; si quedan unidades, `complete = false` y la siguiente llamada continúa.
- **Una llamada = un `ChangeSet` = una `Txn`** con todas las unidades del lote (no una `Txn` por unidad): cada unidad sigue podándose entera o nada (IU-7) y el lote entero es atómico. El presupuesto cuenta las escrituras por unidad (4 si pierde revisiones: dos `delete_range`, cabeza y miembro; 1 si solo pierde cargas útiles, más 2 por carga útil), no la marca ni el `CHANGESET`. Sin cursor: cada llamada recorre las cabezas desde el principio y las unidades sin trabajo solo cuestan lecturas; `complete = false` pide repetir. Una base vacía o sin nada que podar no escribe nada (`change_set = None`).
- Qué se conserva: siempre la cabeza; si la cabeza es una lápida, también la revisión anterior (salvo bajo `none`, que ya solo guarda la cabeza). `KeepSince(s)` y `KeepFor(d)` conservan además la revisión **vigente en el punto de corte** (la última con `commit_seq ≤ s`, o la anterior a la primera con `created_at ≥ now − d`), para que las lecturas as-of desde el corte sigan respondiendo; solo se poda un prefijo, aunque el reloj no sea monótono (`created_at` es descriptivo, I-26). `now` es la lectura de reloj del propio `ChangeSet` de retención.
- Miembros de un `CHANGESET` `Retention`: uno por unidad tocada, `(unit, k, kind de k)`, donde `k` es el nuevo `oldest_rev` (la primera revisión retenida). Su `commit_seq` no es el del `ChangeSet`: IU-4 se comprueba en el sentido `REV_META → miembro`, y `verify_units` (E3-07) debe tratar así los miembros de `Retention`. La cabeza solo cambia en `oldest_rev` (`updated_seq` no se mueve: la retención no crea revisiones).
- Política de historial aplicada a lo almacenado (pendiente de E3-05): bajo `audit`, toda revisión retenida por debajo de la cabeza que aún tiene `REV_PAYLOAD` (de un periodo `versioned` anterior) lo pierde y su `REV_META` se reescribe con `payload_present = 0` conservando hash y longitud, igual que hace `audit` con la cabeza sustituida (es, como ella, la única reescritura de `REV_META`; IU-1 queda acotada a «solo la política cambia `payload_present`»). Bajo `none`, todo lo que está por debajo de la cabeza se poda (`oldest_rev = head_rev`). Ambas limpiezas ocurren aunque la regla sea `KeepAll`; las de `audit` pueden repartirse entre llamadas (cada carga útil es atómica con su `REV_META`).
- `history` con `from = None` y `to ≤ oldest_rev` cuando hubo poda (`oldest_rev > 1`) devuelve `Pruned` en lugar de una página vacía (también al paginar `Desc` hasta el final de lo retenido). Con `oldest_rev = 1` sigue siendo vacía.
- `restore` aplica `check_payload` contra el `max_payload_len` **vigente**: la revisión restaurada es contenido nuevo y obedece el límite actual, como `update` (`PayloadTooLarge` sin efectos). `archive`/`unarchive` no lo aplican: copian la cabeza ya aceptada y no deben bloquear el ciclo de vida.
- Errores: una `REV_META` ausente o con `commit_seq` desordenado dentro de `[oldest_rev, head_rev]` es `Storage(Corruption)` y la llamada no escribe nada; nunca «nada que podar».
- `compact()` exige `E: TestHooks` (el único acceso genérico a `compact_range` en `kerno-core`; lo implementa `KernoDb`, no `Emulated<LogEngine>`) y compacta `[0x20, 0x22)`. `CompactReport` da los bytes de tablas vivas antes y después (suma de las estadísticas `l<N>_bytes` del motor) y `bytes_reclaimed()`; por eso `bytes_reclaimed` no está en `RetentionReport`, que da `bytes_logically_freed` (claves y valores de `REV_META`/`REV_PAYLOAD` eliminados, medidos sin leer cargas útiles). Lo que una instantánea aún ve no se reclama (I-05). `UnitDb::metrics()` no entra en este paquete.
- El «espacio recuperado medible en `history_sim`» se comprueba con una carga equivalente dentro de `kerno-units` (`history_sim_reclaims_space`, solo `KernoDb`) en lugar de tocar `spike-bench`. `retention_every_point` barre la confirmación de la retención y el cierre que la vuelca a tablas.
- `UnitModel::retain`/`retain_within` decide la retención de forma independiente de la base (mismas reglas y mismo presupuesto) y `policy_model` la compara en cada ronda; `snapshot_vs_retention` queda completo (IU-8).

**Nota de E3-07 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/src/verify.rs`, `docs/file-format/units-v1.md` §5.2, `tests/verify.rs` y las comprobaciones añadidas a `tests/unit_db.rs`, `tests/history.rs`, `tests/policies.rs` y `tests/retention.rs`):**

- API: `UnitDb::verify_units()` y `UnitSnapshot::verify_units()` sobre la instantánea de la capa; `verify_engine(&E)` y `verify_snapshot(&S)` sobre cualquier motor, también uno cuya marca está dañada (que `open` rechaza) o abierto en solo lectura; `verify_immutable(older, newer)` para IU-1. Devuelven `UnitVerifyReport { seq, counts, findings, findings_omitted }`; cada `VerifyFinding` lleva la regla (`VerifyRule::{Structure, Payload, Cross}`, con id `UNIT-n`), su clase, la clave del registro y un detalle sin contenido (IU-10). `into_result()` convierte el informe en el error que daría la API de lectura: `Storage(Corruption)`, `Storage(Incompatible)` o `IncompatibleLayer` (se prefiere el primer hallazgo `IncompatibleLayer`: nada de una capa más nueva se interpreta). Los daños son hallazgos, no errores; solo los errores del motor (`Io`, `Corruption` del propio motor, `Poisoned`) salen como `Err`. Se guardan como mucho 1 024 hallazgos y el resto se cuenta.
- §8.5 pide que cada hallazgo lleve una regla `UNIT-n`, pero las invariantes entre registros (IU-1…IU-4, IU-9) no tenían número: se añaden UNIT-29…UNIT-35 a `units-v1.md` §5.2. No cambian ningún byte ni el comportamiento de los decodificadores; todas son `Corruption`.
- Un registro que no decodifica se informa una vez y se excluye de las comprobaciones entre registros (no se vuelve a informar como «ausente»). Las claves con etiqueta desconocida se informan (UNIT-1 `Corruption`, UNIT-2 `IncompatibleLayer`) y se cuentan; el recorrido abarca todo el espacio de claves, también los huecos entre etiquetas.
- Revisión no lápida con `payload_present = 0`: es una que `audit` vació conservando hash y longitud (nota de E3-05), así que para ella no se aplica la parte «`payload_present = 0` con `payload_len ≠ 0`» de UNIT-26; solo se exige que no tenga `REV_PAYLOAD`. La revisión vigente no lápida siempre debe tener carga útil (UNIT-31).
- Políticas (IU-9): los `LAYER_META` no guardan cuándo cambió la política, y un cambio no reescribe nada (E3-05), así que lo que dejó un periodo anterior es legítimo hasta la retención. Solo se comprueba donde la política vigente al escribir es segura: las revisiones confirmadas **después del último `ChangeSet` `Maintenance`** (los únicos que cambian políticas) se escribieron con la política vigente ahora. Ahí, `none` exige `oldest_rev = head_rev` y `audit` exige `payload_present = 0` en toda revisión sustituida. Antes de ese punto no se afirma nada (podría ser un falso positivo).
- Procedencia (IU-4): los miembros de un `ChangeSet` `Retention` solo se comprueban en el sentido `REV_META → miembro`, como pide la nota de E3-06 (nombran la primera revisión retenida); de ellos solo se comprueba la forma (ordinales `0..member_count`, sin unidad repetida, al menos uno). Un `REV_META` cuyo `commit_seq` es un `ChangeSet` que no es `Application` es hallazgo. `Maintenance` debe tener 0 miembros; `Application` y `Retention`, al menos uno (un `ChangeSet` vacío no escribe nada). Un miembro de `Application` cuya revisión está por debajo de `oldest_rev` es procedencia de una revisión podada (retención o `none`) y no es hallazgo.
- Cadenas (IU-3): además de `parent_rev = rev − 1` y `commit_seq` creciente, se comprueban las transiciones de ciclo de vida de las notas de E3-03/E3-04 y que `Archive`/`Unarchive`/`Restore` copien hash, longitud, esquema y metadatos de su origen cuando está retenido. Los `created_at`/`updated_at` son descriptivos (I-26) y no se comprueban.
- Inmutabilidad (IU-1): `verify_immutable` admite solo el paso de `payload_present` de 1 a 0 (nota de E3-06); cualquier otro cambio de una clave `0x20/0x21` presente en ambas instantáneas es UNIT-35. Las cargas útiles se comparan por BLAKE3 para no guardar sus bytes.
- Hash en lectura: ya lo hacían todas las lecturas de carga útil (`get_unit`, `get_revision`, `history` con carga útil, `unit_as_of`, `compare`, `restore`, `archive`/`unarchive`); E3-07 no cambia código de lectura y lo prueba con cada daño.
- §8.4: el «parche de SST con recálculo de CRC» se hace en el plano lógico: el daño se escribe con un lote del motor, de modo que todas las sumas de comprobación del motor son válidas y solo la capa puede detectarlo; es equivalente para la capa y no exige tocar `kerno-storage`. Los `bit_flips`/`truncations` de archivos se hacen sobre una base con unidades (`media_damage`): todo error es `Corruption`, `Io` o `Poisoned`. `Emulated<LogEngine>` puede perder la cola de su registro truncado (no es un motor durable); se admite solo en él, y lo que queda debe verificar sin hallazgos. En `KernoDb` no se admite ninguna vuelta atrás.
- I-18: el verificador solo lee una instantánea. Se prueba en ambos motores (claves y secuencia sin cambios tras verificar y tras una apertura rechazada) y, en `KernoDb` abierto con `AccessMode::ReadOnly`, sobre el directorio byte a byte (solo puede aparecer `LOCK`). No se ha tocado `kerno-storage` ni `kerno-core`; en `kerno-units` solo se añade `Hash` a `PayloadRule`.
- El recorrido lee página a página (32 claves por `scan`) y nunca guarda cargas útiles, pero guarda decodificados y completos todos los `UNIT_HEAD`, `REV_META`, `CHANGESET` y `CS_MEMBER` del almacén (metadatos incluidos, hasta 4096 B cada uno) y la clave de cada `REV_PAYLOAD`: en el peor caso, unos 4,6 KB por revisión y 2,5 KB por conjunto de cambios. `verify_immutable` guarda además cada `REV_META` de la instantánea antigua (codificado) y un resumen BLAKE3 de 32 B por `REV_PAYLOAD`. El trabajo depende solo de los registros que existen, nunca de los números que un registro declara: un `head_rev` dañado (p. ej. 2^63) se informa como UNIT-31 sin recorrer su rango. La CLI de la Fase 2/Etapa 9 queda fuera.

**Nota de E3-08 (ambigüedades resueltas con la opción más conservadora; `crates/kerno-units/tests/harness/units.rs`, `tests/crash.rs`, `tests/process_kill.rs`, [matriz de fallos §7](failure-matrix.md), [invariantes](invariants.md) I-20/I-21, `.github/workflows/ci.yml`):**

- `harness::units` vive en las pruebas de `kerno-units` (`tests/harness/units.rs`, incluido como módulo por cada archivo de prueba) y no en `kerno-testkit`: el testkit no puede depender de `kerno-units` sin un ciclo (la capa ya depende del testkit en desarrollo), y así no se toca `kerno-storage`, `kerno-core` ni `kerno-testkit`. Ofrece `Step::{ChangeSet(n), Close, Drop}`, un `Target` por motor y las comprobaciones comunes.
- Oráculo, en dos partes, porque cada una prueba algo distinto. (1) **Byte a byte**: cada recuperación se compara con todo el almacén que deja, tras exactamente `s` `ChangeSet`, una ejecución sin fallos **del mismo código de la capa** sobre `Emulated<LogEngine>` (todas las familias: cabeza, `REV_META`, `REV_PAYLOAD`, `CHANGESET`, `CS_MEMBER` y `LAYER_META`). Esto prueba I-21 en la frontera del motor (ninguna imagen contiene parte de un `ChangeSet`) y que `KernoDb` y el motor de referencia son intercambiables, pero **no** que la capa escriba lo correcto: un error de `UnitDb` está en los dos lados y la comparación no lo ve. (2) **Modelo**: el oráculo también aplica al `UnitModel` independiente los mismos `ChangeSet` confirmados (las revisiones se construyen en la prueba a partir de la operación y del propio estado del modelo, nunca leyendo registros de la capa), y tras cada recuperación las lecturas públicas de la base deben coincidir con él: los ids de unidad, cada cabeza (tipo, esquema, ciclo de vida, revisión, `oldest`, metadatos, carga útil), cada revisión retenida con `get_revision` (tipo, secuencia, ciclo de vida, origen de restauración, metadatos, hash y carga útil, o el mismo rechazo), cada `ChangeSet` con su propósito y miembros, y la política de cada tipo. Así se detectan errores de la propia capa: omitir la escritura de `CHANGESET` en la secuencia 3 de `db.rs` lo detecta la comprobación con el modelo (además de `verify_units`). El modelo no comprueba los campos descriptivos (`created_at`, `reason`, procedencia), que solo cubre la comparación byte a byte. Para que exista ese almacén de referencia, la carga es una función determinista de `(semilla, secuencia, cabezas almacenadas)`: cada paso confirma **exactamente un** `ChangeSet` (paso `k` = secuencia `k`; si `run_retention` no tiene nada que podar, un `ChangeSet` de relleno con una creación), el reloj es fijo y los ids se reinician por paso (`StepIds`: `k·2^16 + i`). Un `ChangeSet` perdido en una caída se rehace idéntico tras recuperar. La carga mezcla `create/update/archive/unarchive/delete/restore` (de la cabeza, que siempre tiene carga útil), `set_policy` (las tres políticas), `run_retention` con `KeepLast(1..3)` y, cada 7 pasos, un `ChangeSet` grande (3–5 cargas útiles de 8–24 KB) que ocupa una trama de muchos sectores y fuerza un flush dentro del commit.
- Tras **cada** recuperación de cada barrido: secuencia dentro de los límites, almacén = referencia, `verify_units` sin hallazgos, lecturas = `UnitModel` (incluye `get_unit` de todas las cabezas, que verifica el hash) y `keeps_working`: un `ChangeSet` más, cierre limpio, corte de energía y de nuevo las mismas comprobaciones.
- Motor: `KernoDb` se abre en los barridos con `Background::Inline` (mismas operaciones de E/S en cada ejecución, puntos reproducibles, como los barridos del motor); el `kill -9` real usa los hilos de fondo por defecto. `changeset_every_point`, `model_crash` y `recovery_reentrant` se ejecutan sobre `Emulated<LogEngine>` y `KernoDb`; `fsync_failure`, `enospc`, `torn_write` y `wal_independence` solo sobre `KernoDb`, porque prueban la semántica de fallos del motor real (`Poisoned`, WAL-R, reciclado), que el motor de referencia no modela.
- Límites de la secuencia recuperada: `acked ≤ s ≤ attempted` (con `attempted = acked + 1` si el paso falló), salvo corte de energía en `Buffered`, donde solo se exige un prefijo no anterior al último **cierre limpio** (el barrido `every_point` del testkit no admite esa combinación; aquí se incluye con esa cota más débil, que sigue exigiendo `ChangeSet` completos).
- `changeset_every_point` (completo; la versión rápida de E3-03 sigue en `unit_db.rs`): por semilla, un escenario de 12 `ChangeSet` con cierre limpio y cierre sucio intermedios, caída antes de cada operación mutante (+5 de margen), 4 combinaciones de tipo de caída y durabilidad, desgarro a 1 B en los puntos impares. `model_crash`: 5 rondas de hasta 20 `ChangeSet` con caída aleatoria (1 de cada 5 rondas, también durante la recuperación) y continuación sobre la imagen, como `crash::run`.
- `recovery_reentrant`: base con 6 `ChangeSet` solo en el WAL, caída (cada tipo) y, para cada operación mutante de su recuperación (+2), una caída en ese punto; la segunda recuperación debe dar el mismo almacén que la recuperación sin interrupciones. En `KernoDb` se exige que la recuperación escriba algo (si no, el barrido no probaría nada).
- `fsync_failure`: falla, por turno, **cada** `sync_data` de un escenario (apertura, 10 `ChangeSet`, cierre), del WAL, de SST o del MANIFEST, con `KeepPending` y `LinuxDropPages`. Se exige `Storage(Io)` en la operación que lo sufre y `Storage(Poisoned)` en el siguiente `ChangeSet` y en `begin_changeset`; después, caída (alternando corte de energía y muerte del proceso) y `N` o `N+1`.
- `enospc`: (a) `fill_disk_after` antes de cada operación mutante del mismo escenario; (b) «reservar espacio de cabecera de WAL de 96 B (nota #11)» se interpreta como un `set_space_limit` igual al espacio usado más los 96 B de la cabecera de un WAL nuevo más una holgura de 0, 64, 512, 4 096 o 20 000 B, fijado tras 0, 3, 6 o 9 `ChangeSet`: cabe la cabecera de un WAL pero no el `ChangeSet`. En ambos, el primer error debe ser `Storage(Io(StorageFull))` y el siguiente `ChangeSet` `Poisoned`; al liberar espacio se reabre el mismo dispositivo (el proceso sigue vivo) y se exige `N` o `N+1`. Una operación que no necesita espacio puede dejar terminar el escenario; eso no es fallo. Se exige que al menos un caso de (b) llene el disco.
- `torn_write`: corte de energía justo después de la mayor escritura del WAL del `ChangeSet` grande 7 y antes de su `fsync`, con desgarro a 512 B y a 1 B; el resultado debe ser 6 o 7 completos, y el conjunto de semillas debe descartar la trama al menos una vez (con las semillas por defecto: 22 descartes y 10 tramas enteras de 32).
- `wal_independence`: historial pequeño de 4 unidades (actualizaciones, archivar/desarchivar, borrar, restaurar) que al escribirse solo está en el WAL; luego solo se escriben **otras** unidades hasta que ninguno de los WAL de ese momento existe (flush y borrado; con `wal_preallocate`, reciclado), y 30 `ChangeSet` más. `history` y `get_revision` (con carga útil verificada) de las 4 unidades deben ser idénticos tras cada `ChangeSet`, tras `compact`, tras reabrir y tras un corte de energía, con `verify_units` limpio.
- `kill -9` real (`tests/process_kill.rs`, solo Linux): como el del motor, 20 iteraciones por defecto (`KERNO_KILL_ITERS`, 200 en la ejecución semanal), `Sync` y `Buffered`, `verify()` del motor antes de reabrir y `N` o `N+1` con el almacén de referencia, las lecturas iguales al `UnitModel` y `verify_units` limpio. El trabajo de CI `test-spikes` ya ejecutaba `kerno-units` (todo el espacio de trabajo menos `kerno-storage`) con `SPIKE_SEEDS=1000` en la ejecución semanal; solo se le añade `KERNO_KILL_ITERS`.
- Semillas: valores por defecto 4 (`changeset_every_point`), 16 (`model_crash`, `torn_write`), 4 (`recovery_reentrant`) y 2 (`fsync_failure`, `enospc`); `SPIKE_SEEDS`, `SPIKE_SEED` y `SPIKE_POINT` funcionan como en los barridos del motor. Comprobación de 1 000 semillas hecha en local: `SPIKE_SEEDS=1000 cargo test --release -p kerno-units --test crash` (27 pruebas en verde en 7 min 21 s de reloj en local, 441 s de ejecución; `torn_write` descartó la trama 1 274 veces y la encontró entera 726 de 2 000) y `KERNO_KILL_ITERS=200 cargo test --release -p kerno-units --test process_kill` (2 pruebas en verde en 2 min 20 s; secuencias recuperadas 818 en `Sync` y 942 en `Buffered`). Con las semillas por defecto, `crash.rs` tarda unos 2–5 s y `process_kill.rs` unos 3–9 s. `retention_every_point` no depende de semillas (escenario fijo). El tiempo cabe holgadamente en un trabajo de 6 h, así que no hace falta dividir en matriz (§8.7).
- No se mide aquí el coste en bytes de WAL de un `update` (§9, «se contrasta en WP-E3-08»): es una medida de rendimiento y queda para los bancos de E3-09. `retention_every_point` (E3-06) ya ejecutaba `verify_units` tras cada caída y no cambia.

**Nota de E3-09 (`crates/kerno-units/examples/unit_bench.rs`, [informe](../testing/etapa3-report.md), salida bruta en `spikes/storage/results/2026-11-01-e3/`):**

- Umbrales de la decisión 12, fijados antes de medir y no ajustados. Comando: `cargo run --release -p kerno-units --example unit_bench` (completo, ~7,5 min) o `-- --quick` (~3 s, añadido al trabajo `bench-smoke` de CI; informa, no hace fallar el trabajo por un umbral).
- Resultados (VM de 4 vCPU, `KernoDb` por defecto, `Sync`, `StdVfs`): **U1 cumple** (p99 0,557 ms frente a 0,428 ms, 1,30×); **U2 incumple** (0,1061 ms frente a 0,0580 ms, 1,83×); **U3 aviso** (0,0326 ms frente a 0,0063 ms, 5,13×; 4,37× en otra pasada; no se añade índice invertido); U4: +339 B de WAL por `update` (1 416 B frente a 1 077 B con 1 KiB; 1,01× con 64 KiB), que es la medida aplazada de E3-08; U5: 1 257,7 B por revisión de 1 KiB frente a 1 068,0 B por clave (1,178×); **U6 incumple**: p99 de escritura concurrente de ~2,4 ms antes a 5 693 ms durante la poda (217,7 ms con presupuesto de 25 unidades), porque `run_retention` recorre todas las cabezas dentro de la ranura de escritura en cada llamada.
- `docs/testing/f1-objetivos-rendimiento.md` sin cambios. Las mejoras de U2 y U6 quedan como trabajo posterior; no se han hecho en este WP.

## 12. Decisiones del propietario

Las decisiones 1–8 quedaron **decididas** el 2026-10-31 ([acta E3-00](etapa3-acta-e3-00.md)), con las condiciones que ahí constan; la 12, el 2026-11-01; las 9–11 y 13 siguen abiertas.

| # | Decisión | Recomendación de este diseño | Si se elige otra cosa |
|---|---|---|---|
| 1 | **Condición de inicio**: la Etapa 3 exige Fase 1 cerrada (formato v1 congelado, ADR-0013 aceptado y WP14 completado). ¿Se empieza con el borrador `DRAFT` o se espera a la congelación?<br>**Decidido (2026-10-31): satisfecha (congelación hecha).** | Esperar: el diseño no depende del formato, pero las bases de prueba creadas con `DRAFT` se rechazan tras congelar | Empezar en paralelo solo con E3-01/E3-02 (puro código y documentación), sin crear bases persistentes |
| 2 | **Crate y API** (ADR-0010): ¿crate nuevo `kerno-units` genérico sobre `TxnEngine`, o integrar en `kerno-storage`?<br>**Decidido (2026-10-31): crate nuevo `kerno-units`, separación estricta de `kerno-storage`, sin regresiones en T1–T10.** | Crate nuevo: aísla T1–T10 de la capa y permite probarla sobre `LogEngine` | Integrar: más simple, pero toda la capa queda bajo los objetivos de la Fase 1 |
| 3 | **Bit de característica**: ¿basta la marca de capa o se reserva un `incompat` «UNITS» para que un motor antiguo no abra la base?<br>**Decidido (2026-10-31): marca de capa en disco, sin bit `incompat` para UNITS mientras `kerno-storage` sea interno.** | Solo marca de capa mientras `kerno-storage` sea interno; bit si se ofrece acceso KV directo al usuario | Bit en 1..62 con vectores y nota en ADR-0013 (no cambia `format_version`) |
| 4 | **Snapshots frente a deltas** (ADR-0004/0008): ¿carga útil completa por revisión hasta que haya benchmark?<br>**Decidido (2026-10-31): copia completa por revisión, sin deltas hasta que los benchmarks los justifiquen.** | Sí; `Delta` reservado en `kind` | Delta desde el inicio: exige fijar formato de delta y `K`/`F` antes de medir |
| 5 | **Duplicar o no la carga útil vigente en la cabeza** (§2.3)<br>**Decidido (2026-10-31): una sola copia, se acepta el coste de 2 `get`.** | No duplicar (una copia; 2 `get`) | Duplicar: 1 `get`, ~2× bytes de escritura en `versioned` |
| 6 | **Metadatos versionados**: ¿los `metadata` y la clasificación forman parte de la revisión?<br>**Decidido (2026-10-31): metadatos y clasificación versionados y auditados en la revisión.** | Sí, en `REV_META` | No: pasan a mutar solo en la cabeza y no se auditan |
| 7 | **Hash**: algoritmo y dependencia (ADR-0008)<br>**Decidido (2026-10-31): BLAKE3-256 con su ADR (ADR-0008); CRC-32C provisional hasta completar esa decisión.** | BLAKE3-256 con ADR; `alg=0` CRC-32C provisional | SHA-256; o postponer el hash criptográfico a la Etapa 9 |
| 8 | **`AsOf::Time`**<br>**Decidido (2026-10-31): `AsOf::Time` fuera de la Etapa 3.** | Fuera de la Etapa 3 (I-26) | Añadirlo con definición documentada que depende del reloj |
| 9 | **Poda de `CHANGESET`**: ¿se conservan siempre (procedencia) hasta ADR-0009? | Sí, ~60 B por revisión | Podarlos con la revisión: pierde procedencia |
| 10 | **`unit_id`**: UUIDv7 interno inyectable (D-05) y mapeo a IDs públicos en ADR-0005 | UUIDv7 | UUIDv4 o contador por base: pierde localidad temporal de claves (efecto en escritura al LSM a medir) |
| 11 | **Límite de carga útil**: 1 MiB por defecto, ajustable a 64 MiB; ¿troceado? | Sin troceado en esta etapa | Troceado: nueva etiqueta de clave y reglas de GC |
| 12 | **Umbrales U1–U6** y si se comparan con SQLite además de con la referencia interna<br>**Decidido (2026-11-01):** U1 (p99 de commit de un `update` de 1 KiB, `Sync`, frente a un put simple de 1 KiB en `KernoDb`) ≤ 1,5×; U2 (`history(id, 100 rev)` p99 solo metadatos frente a `scan` del mismo nº de claves de 100 B) ≤ 1,5×; ambas con el mismo hardware, carga, calentamiento y configuración, informando latencias absolutas en ms además de razones. U3 (aviso si > 5× un `get`; no se añade el índice invertido solo por superarlo, su coste debe justificarse antes), U4 y U5 informativas pero completas y reproducibles (comandos, configuración, semillas, salida bruta). U6: rendimiento de `run_retention` y p99 de escritura concurrente medidos antes y durante la poda con la misma carga, comparando p99 y rendimiento; T1–T3 no deben empeorar más allá de lo medido en baja carga. Solo referencia interna; SQLite queda para la Etapa 9. T1–T10 sin tocar; nunca se bajan umbrales para declarar éxito. | Aprobar antes de medir (ADR-0012) | — |
| 13 | **Orden de seguridad frente a IA** (decisión ya pendiente del roadmap): no bloquea la Etapa 3, pero fija si la supresión (ADR-0009) entra antes de que se acumule historial real | Adelantar ADR-0009 a antes de la Etapa 8 | — |

## 13. Riesgos

| ID | Riesgo | Mitigación |
|---|---|---|
| K-E3-1 | La predicción `commit_seq = read_seq + 1` falla (p. ej. un futuro commit interno del motor que consuma secuencia) | Comprobación tras commit con envenenamiento; prueba `seq_prediction`; documentarla en el contrato de `TxnEngine`; si se rompe, cambiar el motor con bit de característica y no la capa |
| K-E3-2 | Ranura de escritura única: un `ChangeSet` largo bloquea a todos | `max_changeset_ops`, retención en lotes, documentación; multiescritor fuera de alcance |
| K-E3-3 | Doble escritura por revisión (REV_META + REV_PAYLOAD + miembro) eleva amplificación | Métricas U4/U5; el índice y el delta son optimizaciones posteriores detrás de `layer_version` |
| K-E3-4 | Instantáneas largas anulan la retención | Métricas de instantáneas visibles; informe separa «liberado lógicamente» de «reclamado» |
| K-E3-5 | Una capa de usuarios construida sobre ADR pendientes se vuelve difícil de revertir | Bytes opacos, `alg` y `kind` reservados, `layer_version`; nada de ADR-0008 codificado |
| K-E3-6 | El borrado lógico se confunde con supresión | Documentación explícita (§5.3) y `docs` de seguridad; ADR-0009 antes de prometer nada |
