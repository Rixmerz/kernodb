# Diseño de la Fase 1: núcleo persistente

| Campo | Valor |
|---|---|
| Estado | Propuesta de diseño para ejecutar la Fase 1 (SAD §16). Las decisiones que interpretan ADR aceptados se registran en ADR-0013 (WP06) |
| Fecha | 2026-10-28 |
| Base | M0 cerrado en `0cba925`; ADR-0001, ADR-0002 y ADR-0003 aceptados el 2026-10-28 |
| Formato | [`docs/file-format/v1.md`](../file-format/v1.md) (borrador; se congela al cerrar la fase) |
| Relacionado | [roadmap](../roadmap/README.md) «Fase 1: plan de trabajo», [matriz de fallos](failure-matrix.md), [invariantes](invariants.md), [plan de pruebas](../testing/test-plan.md), [informe M0](../testing/m0-report.md), ADR-0012 |

## 0. Resumen

La Fase 1 convierte el prototipo `spike-lsm` en el primer motor del producto, `kerno-storage`, con un formato físico v1 especificado y probado, y cumple las condiciones 1–3 de ADR-0001 y los seguimientos de Fase 1 de ADR-0002. Su salida es la de SAD §16: un motor experimental que demuestra las invariantes de persistencia y pasa la matriz de recuperación actualizada.

Este diseño sintetiza tres propuestas evaluadas por dos jueces (formato primero, riesgo primero, incremental). De cada una toma lo mejor valorado:

- **Del enfoque «formato primero»:** el formato v1 completo (cabecera común con `db_id`, tramas de WAL por grupo con `synced_len`, MANIFEST como log de ediciones con `SESSION_OPEN` y checkpoint `CLEAN`, tabla de decisión de recuperación, invariantes del escritor W-1..W-13, vectores de estructura antes que el motor, `spec_hex_matches_vectors`, rechazo de directorios ajenos).
- **Del enfoque «riesgo primero»:** el **orden de ejecución**: las colas de latencia de la condición 1 se diagnostican y se corrigen *antes* de invertir en el formato, con objetivos registrados antes de medir y un punto de decisión cuantitativo; las suites nuevas se validan primero contra el motor de referencia; un decodificador independiente escrito desde la especificación; CRC con contexto de archivo; *flush split* con métrica de profundidad de L0; la lista de lo que nunca se recorta.
- **Del enfoque incremental:** el puerto «copiar primero» verificado con un `diff` normalizado; un interruptor por cada cambio grande cuyo valor «apagado» reproduce el M0, con la suite compartida ejecutándose en ese perfil; un único hilo de compactación por defecto; la huella exacta de `Outcomes` en cada paquete; `base_seq` en la cabecera del WAL; pasos de escenario extensibles (`Step::Ext`).

Además cubre lo que ninguna propuesta trataba: disco lleno (ENOSPC), determinismo del presupuesto de cierre, protección contra bases creadas con el borrador (bit `DRAFT`), borrado diferido de archivos obsoletos, bloqueo entre dos procesos reales, prueba con dispositivo real como criterio de salida, memoria acotada en la recuperación, la plataforma soportada, la carrera entre instantáneas huérfanas y `verify()`, la **reentrancia** de la recuperación cuando el WAL nuevo se crea antes de su registro en el MANIFEST, un comprobador de concurrencia (`loom`) para el pipeline de commits, la contrapresión de la cola de commits, el presupuesto de tiempo de CI, el proceso para cambiar bytes del borrador y la comparación de cobertura entre el modo con hilos y el modo determinista.

## 1. Principios de ejecución

1. **Paquetes en un grafo de dependencias y siempre en verde.** Los paquetes de trabajo (WP, §11) se ejecutan en paralelo cuando sus dependencias ya están integradas; cada integración deja la rama con `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` y `cargo test --workspace --all-features --release` en verde (puerta **G0**).
2. **Huella de no-regresión del harness (puerta FP).** Todo paquete ejecuta:

   ```text
   SPIKE_SEEDS=400 cargo test --release -p spike-btree --test shared -- shared_corruption --nocapture 2>&1 | grep Outcomes
   SPIKE_SEEDS=400 cargo test --release -p kerno-testkit --test reference_engine -- shared_corruption --nocapture 2>&1 | grep Outcomes
   SPIKE_SEEDS=400 cargo test --release -p spike-lsm --test shared -- shared_corruption --nocapture 2>&1 | grep Outcomes
   ```

   y debe obtener exactamente `btree BitFlip: Outcomes { detected_at_open: 28, detected_on_read: 191, harmless: 181 }`, `btree Truncate: Outcomes { detected_at_open: 400, detected_on_read: 0, harmless: 0 }` y `log BitFlip: Outcomes { detected_at_open: 397, detected_on_read: 0, harmless: 3 }` (sin hilos: deterministas), y para `lsm` `Truncate` 400/0/0 y `BitFlip` con `harmless: 0` (tiene hilos; el reparto 36/364 puede variar). Antes de WP01 el segundo comando usa `-p spike-core`.
3. **Copiar primero, cambiar un subsistema por paquete.** `kerno-storage` nace como copia de `spike-lsm` con solo renombres. `spike-lsm` queda **congelado** como línea base de rendimiento en la misma sesión (ADR-0012); la única edición permitida son los brazos de `match` que obliga `Op::DeleteRange` (WP10).
4. **Interruptores con perfil M0.** Cada cambio de comportamiento grande (palancas de compactación, presupuesto de cierre, group commit) tiene un interruptor en `Config` cuyo valor «apagado» reproduce el M0. La suite compartida se ejecuta también en el perfil `M0` para poder bisecar regresiones.
5. **Pruebas antes que funciones.** Cada suite nueva se valida primero contra `LogEngine` (nativo) o `Emulated<E>` (emulación en el testkit); solo después se ejecuta contra `kerno-storage`. Si falla contra la referencia, la prueba está mal.
6. **Determinismo.** `Background::Inline` ejecuta flush y compactación en línea; con el presupuesto de cierre solo en bytes (`max_duration: None`), la secuencia de E/S es determinista: los barridos `every_point` numeran igual en cada ejecución y los vectores se regeneran byte a byte.
7. **El riesgo que puede tumbar la fase se mide primero.** La condición 1 (colas de latencia) se diagnostica en WP04 y se ataca en WP05, antes del formato; si no se cumple, el propietario lo sabe al principio, no al final.
8. **Nunca se recortan** (§10.2): las pruebas de modelo y propiedades, los barridos de caídas filtrados sobre el MANIFEST, la suite de caídas con varios escritores, la prueba dirigida de reentrancia, los vectores y su guardia, y la huella FP.

## 2. Crates y dependencias

```text
crates/kerno-core       (sin dependencias externas)
  error     Error, Result                       (+ Incompatible, ReadOnly en WP07)
  batch     Op, WriteBatch, MAX_KEY_LEN, MAX_VALUE_LEN  (+ Op::DeleteRange en WP10)
  Durability
  vfs       File, OpenMode, Vfs, dir_size, StdVfs
  probe     etiqueta de fase por hilo para las trazas de prueba (WP03)
  contract  #[doc(hidden)] Engine, Snapshot, Options (+ small_for_tests);
            extensiones: TestHooks (WP03), SyncEngine (WP12), TxnEngine/Txn (WP13), RangeDeleteEngine (WP10)

crates/kerno-testkit    (publish = false; kerno-core, crc32c, parking_lot)
  rng (Rng, sin cambios byte a byte), reference (LogEngine), vfs::{SimVfs, CrashKind, CountingVfs,
  IoCounters, IoSnapshot, HookVfs}, harness::*, emulated (Emulated<E>), process (re-ejecución del binario
  de pruebas), macros: engine_test_suite!, extended_failure_suite!, range_delete_test_suite!,
  sync_test_suite!, group_commit_test_suite!, txn_test_suite!

crates/kerno-storage    (kerno-core, crc32c, parking_lot, quick_cache, crossbeam-skiplist)
  dev: kerno-testkit, proptest; [target.'cfg(kerno_loom)'.dev-dependencies] loom
  dev (solo para generar vectores M0): spike-lsm, spike-btree

spikes/storage/spike-core    → shim: re-exporta kerno-core y kerno-testkit con las rutas de siempre
spike-lsm, spike-btree       → spike-core (sin cambios de código salvo WP10)
spike-bench                  → spike-core; [candidates] spike-lsm, spike-btree; [kerno] kerno-storage
```

Reglas y guardias de CI:

- `kerno-testkit` nunca depende de `kerno-storage` (evita el ciclo y la doble compilación en pruebas unitarias).
- El producto no depende en tiempo de ejecución de crates de prueba ni de spikes:
  `tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"` y `test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1`.
- Todo acceso a archivos pasa por `Vfs` (CONTRIBUTING): `if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi`.
- Las guardias fallan cerradas: un error de `cargo tree` o de `grep` (crate renombrado, directorio inexistente) hace fallar la guardia. La forma `! orden | grep -q …` pasaba en ese caso, porque `!` invierte también el fallo de la orden.
- El contrato del harness vive en `kerno_core::contract` (`#[doc(hidden)]`): `kerno-storage` lo implementa directamente, así que el harness prueba el tipo del producto sin adaptadores, y `spike-bench` y la prueba diferencial añaden `kerno` sin envoltorios. La futura fachada pública `kerno` (ADR-0010, pendiente) no lo re-exporta.
- `Op` no se marca `#[non_exhaustive]` (obligaría a brazos comodín en los spikes); `Error` sí (fuera de su crate solo se usa con `matches!`). `Options` no (dos sitios usan sintaxis de actualización funcional).

## 3. API pública de `kerno-storage` (Fase 1, explícitamente inestable)

Sin promesa de semver (C-17). Los nombres se fijan aquí para que los paquetes coincidan.

```rust
pub enum AccessMode { ReadWrite, ReadOnly }

pub struct Config {
    pub access: AccessMode,                    // ReadWrite
    pub create_if_missing: bool,               // true
    pub durability: Durability,                // Sync (por base; por commit: diferido, C-05)
    pub cache_bytes: usize,                    // 64 MiB
    pub write_buffer_bytes: usize,             // 8 MiB
    pub max_immutable_memtables: usize,        // 4
    pub limits: Limits,                        // key 512 B, value 1 MiB, batch 64 MiB, 1<<20 ops (≤ límites de formato)
    pub compaction: CompactionConfig,          // §5.4; CompactionConfig::m0(write_buffer) = M0
    pub close_budget: CloseBudget,             // Bounded{max_bytes: 256 MiB, max_duration: Some(200 ms)} | Unbounded (= M0)
    pub group_commit: GroupCommitConfig,       // {enabled: true, max_group_bytes: 1 MiB, max_group_commits: 1024,
                                               //  max_queued_bytes: 64 MiB}
    pub allow_unverified_platform: bool,       // false (§5.10)
    #[doc(hidden)] pub test: TestConfig,       // {background: Threads | Inline, db_id: Option<[u8; 16]>}
}
impl From<&kerno_core::contract::Options> for Config { /* durability, cache, write buffer; resto por defecto */ }

pub struct KernoDb;   // Send + Sync
impl KernoDb {
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &Config) -> Result<KernoDb>;
    pub fn is_read_only(&self) -> Option<ReadOnlyReason>;   // Requested | UnknownRoCompat { file, bits }
    pub fn commit(&self, batch: &WriteBatch) -> Result<u64>;            // lote ciego; se agrupa (WP12)
    pub fn begin_write(&self) -> Result<WriteTxn<'_>>;                  // RMW exclusivo (WP13)
    pub fn snapshot(&self) -> Result<KernoSnapshot>;
    pub fn sync(&self) -> Result<u64>;                                  // secuencia duradera (WP12)
    pub fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<CompactRangeReport>; // WP10
    pub fn metrics(&self) -> Metrics;                                   // tipado; stats() lo aplana
    pub fn close(self) -> Result<CloseReport>;  // {duration, flushed_bytes, compacted_bytes, budget_exhausted, l0_files, l0_depth}
    #[doc(hidden)] pub fn hooks(&self) -> &dyn TestHooks;               // wait_idle, flush, compact_once
}
impl WriteTxn<'_> {
    pub fn read_seq(&self) -> u64;
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;          // lee sus propias escrituras
    pub fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;     // límites comprobados aquí
    pub fn delete(&mut self, key: &[u8]) -> Result<()>;
    pub fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()>;   // [start, end)
    pub fn commit(self) -> Result<u64>;    // vacía: secuencia actual, no consume ninguna
    pub fn abort(self);                    // Drop == abort
}
impl KernoSnapshot { seq(), get(), scan() }   // paginado y reanudable con Excluded(last) (C-27)

pub fn verify(vfs: Arc<dyn Vfs>, dir: &Path, opts: &VerifyOptions) -> Result<VerifyReport>; // nunca escribe
pub fn inspect(vfs: Arc<dyn Vfs>, dir: &Path) -> Result<FormatInfo>;                       // solo cabeceras
#[doc(hidden)] pub mod testing;   // perfiles Profiled<P>: Inline, M0, Serial (cada uno implementa Engine)
#[doc(hidden)] pub mod format;    // códecs v1, parches de cabecera, generador de vectores
```

**`Metrics`** (todas se exportan por `stats()` con nombres estables; se conservan los nombres del M0):

| Grupo | Claves |
|---|---|
| Forma | `l{0..6}_files`, `l{0..6}_bytes`, `l0_depth`, `range_fragments`, `memtable_bytes`, `immutable_memtables`, `last_seq`, `durable_seq` |
| Escritura | `commits`, `commit_groups`, `group_commits_max`, `wal_bytes`, `wal_syncs`, `memtable_rotations`, `queued_bytes_max` |
| Fases del commit | `commit_{queue,make_room,wal_write,wal_sync,apply}_p99_micros` (histograma log2) |
| Bloqueos | `stall_episodes_{memtable,l0}`, `stall_micros_{memtable,l0}`, `stall_micros_max`, `slowdown_commits`, `slowdown_micros`; los antiguos `write_stalls` (ahora episodios), `write_stall_micros`, `write_slowdowns` |
| Segundo plano | `flushes` (incluidos cierre y recuperación), `flush_micros_max`, `compactions`, `compaction_micros_max`, `compaction_bytes_{read,written}`, `l0_compaction_input_bytes_max`, `l0_compaction_l1_files_max`, `pending_compaction_bytes`, `trivial_moves`, `tombstones_dropped`, `range_tombstones_dropped` |
| Ciclo de vida | `obsolete_bytes_pending`, `obsolete_files_deleted`, `delete_failures` |
| Cierre y apertura | `close_compaction_micros`, `close_budget_exhausted`, `last_open_clean`, `last_open_replayed_bytes` |
| MANIFEST | `manifest_records`, `manifest_checkpoints`, `manifest_bytes` |
| Instantáneas | `snapshots_live`, `snapshot_oldest_seq`, `snapshot_oldest_age_ms`, `snapshot_retained_bytes` |
| Transacciones | `txns_committed`, `txns_aborted`, `txn_gate_wait_micros` |
| Lecturas | contadores de bloom y caché (repartidos en 16 ranuras con relleno de línea de caché para eliminar la contención del M0, `stats.rs:44`) |

**`VerifyReport`** (WP09): `files` (versión y bits por archivo leído), `clean`, `manifest_torn_tail`, `last_seq`, `levels` (por nivel archivos/bytes/entradas/fragmentos), `wals` (número, tramas, commits, cabecera válida, cola rota), `orphans` (nombre y longitud), `unknown`, `manifest_tmp`, «huérfanos desaparecidos» (`vanished_orphans`, RO-4) y `issues: Vec<Issue { file, offset: Option<u64>, rule: &'static str, detail: String }>`; `is_ok()`. `VerifyOptions { deep, check_wal }` (ambas a `true` por defecto). El daño se informa en `issues`; `Err` queda para `Io`, `Locked`, `Incompatible` e `InvalidArgument` (directorio que no es una base). `inspect()` devuelve `FormatInfo { files: Vec<FileInfo> }`.

## 4. Modelo de errores

`kerno_core::Error` conserva las cinco variantes del M0 (ADR-0002, ADR-0010) y añade dos (C-16); se marca `#[non_exhaustive]`. El helper del harness `assert_damage_error` sigue siendo estricto (`Corruption | Io | Poisoned`): un bit flip no puede producir una cabecera con CRC válido y versión desconocida.

| Situación | Error | Estado después |
|---|---|---|
| Lote o transacción fuera de límites; `start > end`; secuencia agotada (2^63−1) | `InvalidArgument` | nada escrito, ninguna secuencia consumida, handle sano |
| Directorio abierto por otro handle o proceso | `Locked` | sin efectos (solo la creación de `LOCK`) |
| Escritura o `fsync` del WAL fallidos, `fsync` de un grupo fallido, ENOSPC, error o pánico en segundo plano o en el líder de commits | `Io`, y después `Poisoned` | resultado indeterminado hasta reabrir; tras reabrir, el commit está entero o ausente; todos los miembros del grupo fallido reciben `Io`; los encolados no escritos, `Poisoned` |
| Daño (CRC, estructura, archivo fabricado con CRC válido) | `Corruption` | nunca se repara automáticamente; en lecturas no envenena; en segundo plano sí |
| `format_version` > 1, bit `incompat` desconocido, `AEAD`, formato M0, borrador tras la congelación | `Incompatible` | comprobado antes de cualquier escritura; directorio idéntico byte a byte |
| Directorio ajeno o inexistente sin `create_if_missing` | `InvalidArgument` | nada tocado |
| Escritura en un handle de solo lectura | `ReadOnly` | — |
| Plataforma sin garantías verificadas y `Sync` sin `allow_unverified_platform` | `InvalidArgument` | — |

Los mensajes citan archivo, desplazamiento e id de regla, nunca bytes de claves o valores (T-08, R-88). Un pánico en código de usuario dentro de una transacción solo libera la ranura de escritura: el motor no tocó estado compartido.

## 5. Cambios del motor

### 5.1 Puerto y modo determinista (WP02, WP03)

- `kerno-storage` = copia de `spike-lsm/src` y `tests` con `spike_core` → `kerno_core`/`kerno_testkit`, `LsmEngine` → `KernoDb`, `LsmSnapshot` → `KernoSnapshot`, `name()` = `"kerno"`, hilos `kerno-flush` y `kerno-compact-0`. La documentación del crate dice «formato de desarrollo; nunca para datos de usuario» hasta que exista v1.
- `Background::Inline` (WP03): sin hilos; `make_room` rota y ejecuta en línea el flush de la inmutable más vieja y las compactaciones que elija `pick` hasta que no quede ninguna; `close` y `compact_range` también en línea. Respeta el orden de bloqueo `writer → manifest → state`.
- Perfiles `testing::Profiled<P>` (`Inline`, `M0`, `Serial`): tipos nuevos que implementan `Engine` con otra `Config`, para ejecutar las macros del harness en varias configuraciones sin tocar `Options`. `Engine::name()` devuelve `kerno-inline`, `kerno-m0`, `kerno-serial`.
- **Sonda de fase** (`kerno_core::probe`, WP03): `probe::phase("flush")` devuelve un guard que fija una etiqueta por hilo; `SimVfs::trace()` la registra con cada operación. Coste: una escritura en un `thread_local`. Etiquetas: `create`, `session`, `commit`, `rotate`, `flush`, `compact`, `compact_range`, `close`, `checkpoint`, `delete`, `sync`.

### 5.2 Formato v1 en el motor (WP06, WP07)

Todo el formato está en [v1.md](../file-format/v1.md). WP06 lo implementa como **código puro** (`crates/kerno-storage/src/format/`, sin E/S, con vectores de estructura); WP07 hace que el motor escriba y lea solo v1, todavía con el MANIFEST como instantánea única reescrita por rename en cada cambio (protocolo M0 sobre registros v1). Puntos que cambian el motor:

- Cabecera común en los tres tipos de archivo, `db_id` y número de archivo verificados (HDR-10).
- WAL por tramas (una trama por commit hasta WP12, una por grupo después), `synced_len` (W-11) y `base_seq`; reproducción en flujo de 1 MiB.
- Lote canónico (WAL-C1) aplicado en la memtable.
- SST v1 (encapsulado de 44 bytes con ranura AEAD, CRC con contexto, propiedades TLV, pie de 80 bytes); `TableIter` y `Table::get` exponen la secuencia de cada entrada.
- Límites de API en `Config::limits`, comprobados en `kerno-storage` (no en `WriteBatch::validate`, que siguen usando los spikes). Aritmética comprobada en todo valor leído del disco; números de archivo ≤ 2^48−1 (DIR-1).
- Protocolo de creación DIR-3 y rechazo de directorios ajenos DIR-2. `create_dir_all` y `sync_dir(padre)` solo al crear.
- Bit `DRAFT` en todo archivo escrito.

### 5.3 Condición 3 de ADR-0001, primera mitad: MANIFEST como log de ediciones (WP08)

- `ManifestState` guarda el manejador de anexado y el `edit_seq` vigente. `write_manifest` pasa a `append_record`: un `write_at` + `sync_data` (antes: dos sincronizaciones más creación y rename). El flush cuesta 3 sincronizaciones (SST, directorio, registro) en lugar de 4.
- `log_number` pasa a ser el **número exacto del WAL vivo más antiguo**: el flush de la memtable `m` lo fija al WAL de la siguiente memtable más vieja (o al de la activa). El M0 usaba `wal_number + 1`, que podía no ser un WAL.
- Arranque de sesión S-1..S-6 (v1.md §9.4): el WAL nuevo se crea **antes** de su `SESSION_OPEN`, y REC-10c hace que esa ventana sea reentrante. Una reapertura limpia cuesta 2 sincronizaciones.
- `close()` sigue W-9: rota si la memtable activa recibió commits, flush de todas, compactación obligatoria y con presupuesto, checkpoint `CLEAN` y borrado de WAL. Tras un cierre limpio el directorio contiene solo `LOCK`, `MANIFEST` (un único registro) y SST, como en el M0, así que ningún daño posterior es ambiguo.
- Tabla de decisión REC-1..REC-13 implementada como una función pura `recovery::classify(listado, manifest, cabeceras) -> Plan` que comparten la apertura de lectura-escritura, la de solo lectura y `verify()`; las escrituras solo las ejecuta `recovery::apply(plan)` (I-18 por construcción).
- Memoria de recuperación acotada: S-2 escribe una SST de L0 cada vez que la memtable reproducida alcanza `write_buffer_bytes`.
- **Borrado diferido (W-13):** `Table::drop` de una tabla obsoleta encola su ruta en un `Deleter` compartido; si el motor vive, un hilo `kerno-delete` (o la ejecución en línea en `Inline`) la borra y reintenta los fallos; si el handle ya se cerró, se borra en el acto (comportamiento M0). Métricas `obsolete_bytes_pending`, `delete_failures`. Las lecturas ya no pagan borrados.

### 5.4 Condición 1 de ADR-0001: colas de latencia acotadas (WP04, WP05)

**Diagnóstico primero (WP04).** Hipótesis a confirmar o descartar con métricas antes de corregir nada:

- **H1, flushes multi-prefijo.** `agent_memory` escribe `mem/<id>` y `emb/<id>` en cada lote con ids crecientes en el tiempo (`workloads.rs:598-658`); LiveSpec hace lo mismo con `cs/` y `sym/`. Cada SST de L0 abarca desde `emb/<id viejo>` hasta `mem/<id nuevo>`, que contiene **todas** las SST de L1 con `mem/` antiguos, y `compaction::pick` toma todo L0 más todo L1 solapado: compactaciones enormes, L0 crece, cada commit duerme 1 ms con L0 ≥ 8 y se bloquea con 12.
- **H2, deuda tras la carga.** La carga inicial deja trabajo pendiente que se drena durante la fase medida; un único hilo y L0 sin prioridad.
- **H3, interferencia de escritura.** Las SST se escriben en trozos de 256 KiB y un único `sync_data` final; el `fdatasync` del WAL espera detrás.
- **H4, rotación en primer plano.** Crear el WAL y `sync_dir` ocurren dentro del commit.

**Palancas (WP05), cada una con interruptor** en `CompactionConfig` (el perfil `CompactionConfig::m0()` las apaga todas):

| Palanca | Interruptor | Hipótesis |
|---|---|---|
| *Flush split*: al escribir una memtable se corta la salida entre dos claves consecutivas siempre que algún archivo de L1 quede entero entre ellas; los fragmentos de rango se recortan en el corte | `flush_split` | H1 |
| Métrica de L0 por **profundidad** (máximo de archivos de L0 que se solapan en una clave) para puntuación, frenado y parada | `l0_metric = Depth \| Files` | H1 |
| Prioridad de L0: con métrica ≥ `l0_slowdown`, L0 gana siempre | `l0_priority` | H2 |
| Compactación parcial de L0: el **sufijo más viejo** de L0 hasta `max_compaction_bytes` (25 × tamaño objetivo) incluyendo L1 solapado, mínimo un archivo (siempre seguro: los archivos que quedan son más nuevos) | `l0_partial` | H1, H2 |
| Corte por solape con el nivel abuelo (10 × tamaño objetivo) | `grandparent_overlap_bytes` | H2 |
| Movimiento trivial desde L0 cuando el archivo más viejo no solapa L1 | `l0_trivial_move` | H2 |
| Frenado por tasa: retraso = bytes del lote / tasa (16 MiB/s inicial, se divide entre 2 mientras la métrica crece) en lugar de 1 ms fijo | `slowdown = Rate \| Sleep(1 ms)` | H2 |
| `sync_data` cada 1 MiB escrito de SST | `bytes_per_sync` | H3 |
| Fusión con montículo con más de 8 hijos | `heap_merge_threshold` | lecturas |
| Condicional: subcompactaciones (división en límites de L1, `std::thread::scope`, una sola edición) y 2 hilos con exclusión `being_compacted` | `max_subcompactions`, `max_background_compactions` (1 por defecto) | si WP05 lo exige |

**Presupuesto de cierre (C-10).** `compact_for_close`: primero, obligatorio, compactar hasta que la métrica de L0 quede por debajo de `l0_trigger` (conserva la intención de la regresión M0 `short_sessions`); después, mientras quede presupuesto de bytes y de tiempo, una compactación acotada más (el exceso está acotado por `max_compaction_bytes`). `max_duration: None` hace el cierre determinista (pruebas, barridos y vectores). Lo pendiente lo retoma la puntuación tras la siguiente apertura; el formato no registra «deuda».

**Métricas de bloqueo.** Un episodio se cuenta una vez al entrar en la espera (el M0 contaba despertares), por motivo, con su duración y el máximo; `spike-bench` registra `stat_*` desde `Engine::stats()` al final de cada carga.

**Punto de decisión (WP05).** Con los objetivos de §7 registrados en WP04 antes de medir:

- T1 cumplido ⇒ **GO**.
- T1 incumplido por ≤ 25 % ⇒ GO, con el desvío documentado.
- T1 incumplido por > 25 % ⇒ se implementan dentro de WP05 las subcompactaciones y el segundo hilo, y se vuelve a medir; si sigue incumplido, se escribe `docs/testing/f1-decision-cond1.md` (candidato a NO-GO, con las siguientes palancas y la contingencia R-10/R-11) y se señala al propietario. La ejecución secuencial continúa salvo que el propietario la detenga: el formato v1 no depende de esta decisión.

### 5.5 Condición 2 de ADR-0001: lápidas de rango y compactación dirigida (WP10)

- **Semántica** (v1.md §11): `delete_range(start, end)` borra `[start, end)`; `start > end` ⇒ `InvalidArgument`; `start == end` no tiene efecto. Lote canónico (WAL-C1). **Visibilidad solo por secuencias** (SEM-4): un punto `p@s` está oculto en la instantánea `S` si algún fragmento que lo cubre tiene `s < u ≤ S`; con la misma secuencia gana el punto.
- **Memtable:** lista de rangos `(start, end, seq)` solo de anexado tras un `RwLock`; rotación forzada al llegar a 4 096 lápidas de rango por memtable (acota memoria y coste de lectura).
- **SST:** bloque de rangos fragmentado (una secuencia por fragmento, la mayor), residente como el índice y el filtro. Los límites del archivo incluyen la extensión de los fragmentos (`largest_exclusive`).
- **`get(k)`:** recorre las fuentes de más nueva a más vieja manteniendo `max_tomb` (máxima secuencia `≤ S` de los fragmentos que cubren `k`, comprobada en cada fuente **antes** del filtro bloom); el primer punto encontrado con `s ≥ max_tomb` se devuelve (`None` si es lápida de punto); si `s < max_tomb`, `None`; salida temprana en cuanto `largest_seq` de la siguiente fuente < `max_tomb`. (La propuesta incremental devolvía el punto antes de mirar los rangos de la misma memtable: incorrecto.)
- **`scan`:** `MergeIter` produce la versión más nueva de cada clave con su secuencia; un agregador de fragmentos por fuente, con cursores que solo avanzan, oculta la clave si la máxima secuencia de cobertura supera la del punto. Sin fragmentos en ninguna fuente no se crea ningún cursor (el coste del M0).
- **Flush y compactación:** descartan puntos cubiertos por un fragmento de entrada con secuencia mayor; fusionan y recortan fragmentos en cada corte de salida (tamaño, abuelo, *flush split*); descartan un fragmento cuando ningún archivo más profundo que la salida lo solapa (`DeeperLevels::range_may_overlap`; siempre en L6). Se permiten SST de solo rangos.
- **`compact_range(start, end)`:** (1) si la memtable activa recibió commits, rota y espera a que se vacíen las inmutables (los datos salen del WAL, que se borra tras su `FLUSH`); (2) envía una petición al compactador (en `Inline`, en línea) que compacta nivel a nivel todo archivo que solape el rango hasta que ningún nivel más profundo lo solape, con lo que las lápidas desaparecen; (3) escribe un checkpoint para que las claves del rango salgan del historial de ediciones; (4) purga de la caché los bloques de las tablas eliminadas; (5) devuelve `CompactRangeReport { files_rewritten, bytes_read, bytes_written, pinned_files, pinned_bytes }`, donde lo retenido son tablas de entrada aún referenciadas por versiones antiguas (instantáneas vivas).
- **Solo mecanismo** (C-11, R-52, ADR-0009 pendiente): sin política de retención ni garantía de borrado físico. Lo que puede sobrevivir se documenta en `DESIGN.md`: archivos retenidos por instantáneas, bloques liberados de archivos borrados, memoria de la caché.

### 5.6 Condición 3 de ADR-0001, segunda mitad: group commit; ADR-0002: `sync()` (WP12)

- **Ranura de escritura** `txn_gate: Mutex<()>` (D-03). Un `commit` ciego la toma solo para encolar; una `WriteTxn` la mantiene toda su vida.
- **Cola** `Mutex<CommitQueue>` + `Condvar` por petición. La petición de cabeza es la **líder**: (1) toma `writer`; (2) forma un grupo FIFO hasta `max_group_bytes`/`max_group_commits`; (3) ejecuta `make_room` una vez (un grupo nunca cruza dos WAL); (4) asigna secuencias consecutivas y codifica **una trama** con `synced_len` vigente; (5) un `write_at`; (6) `sync_data` si la base es `Sync` o hay una petición `sync()` en el grupo; (7) aplica los lotes a la memtable en orden; (8) publica `last_seq` y `durable_seq` **una vez**, después del `fsync` en `Sync` (W-12, I-16); (9) completa a todos y cede el liderazgo. Un solo grupo en vuelo.
- **Fallos:** escritura o `fsync` fallidos envenenan; los miembros del grupo reciben `Io` (se construye un `io::Error` por miembro: no es `Clone`), los encolados `Poisoned`. El cuerpo del líder va dentro de `catch_unwind`: un pánico envenena y despierta a todos con error (nunca se cuelgan).
- **Contrapresión:** si los bytes encolados superan `max_queued_bytes`, `commit` espera antes de encolar.
- **Camino rápido:** con la cola vacía, el líder no espera a nadie (sin regresión de un escritor).
- **`sync()`:** encola una petición sin lote; el líder sincroniza solo si `synced_len < bytes escritos`; devuelve `durable_seq`. En `Buffered`, con la semántica de Linux tras un `fsync` fallido, los commits `Buffered` desde el último `sync()` correcto pueden perderse: el estado recuperado sigue siendo un prefijo ≥ el último `sync()` correcto (fila FL).
- **`GroupCommitConfig { enabled: false }`** (perfil `Serial`) reproduce el camino de un commit por trama.
- **Comprobación de concurrencia:** el protocolo de la cola es una máquina de estados sin E/S (`commit_queue.rs`) escrita sobre una fachada `crate::sync` que usa `parking_lot`/`std` normalmente y `loom` con `--cfg kerno_loom`. Modelos `loom` con ≤ 3 hilos: relevo de líder, propagación del envenenamiento, espera de vaciado de `begin_write`, contrapresión. Además, una prueba de estrés con `HookVfs` que inyecta retrasos y `yield` aleatorios (10^4 iteraciones) y la detección de interbloqueos de `parking_lot` en una prueba de depuración.

### 5.7 Seguimientos de ADR-0002: transacciones RMW y métricas de instantáneas (WP13)

- **Pesimistas (C-03, opción b).** `begin_write`: toma `txn_gate`, espera a que `published_seq == assigned_seq` (vaciado del pipeline; en `Sync` puede costar un `fsync`), toma una instantánea registrada. Superposición: `BTreeMap<Vec<u8>, Option<Vec<u8>>>` + lista de rangos + registro ordenado de operaciones. `get`: punto de la superposición, si no rangos de la transacción, si no la instantánea. `scan`: fusiona páginas de la instantánea (filtradas por los rangos propios, repaginando hasta llenar `limit`) con la superposición. `commit`: canonicaliza, valida límites, encola con la ranura tomada, libera la ranura y espera. Una transacción vacía devuelve la secuencia actual; `abort`/`Drop` no consumen secuencia. La superposición está acotada por `Limits` (`put` devuelve `InvalidArgument` al superar el lote máximo).
- Serializable por construcción: nadie más escribe entre las lecturas de la transacción y su commit (D-03). Los commits ciegos de otros hilos esperan; las transacciones no se encadenan entre sí (documentado; `txn_gate_wait_micros` lo expone).
- **Registro de instantáneas:** `Arc<SnapshotRegistry>` compartido por `Inner` y cada instantánea (sobrevive al handle); `BTreeMap<(seq, id), Instant>`; `Drop` da de baja. `snapshot_retained_bytes` = bytes de tablas marcadas obsoletas aún vivas (sube en `mark_obsolete`, baja en `Table::drop`) + bytes de memtables retiradas aún retenidas (sube al desinstalar, baja en `Memtable::drop`) (C-30).

### 5.8 Solo lectura, `verify()` e `inspect()` (WP08, WP09)

- **Solo lectura** (WP08): por `AccessMode::ReadOnly` o por un bit `ro_compat` desconocido. Toma el `LOCK` exclusivo, ejecuta `classify` y reproduce los WAL en memoria; sin hilos, sin `create_dir_all`, `sync_dir`, borrados ni checkpoints. `commit`, `begin_write`, `sync`, `compact_range` ⇒ `ReadOnly`; `close` no escribe nada.
- **`verify()`** (WP09, biblioteca; la CLI llega en la Fase 2, C-01): toma el `LOCK`, reutiliza `classify` con un sumidero nulo, abre cada tabla con `fill_cache = false`, recorre **todos** los bloques (SST-R*, SST-V*), comprueba MAN-S4, reproduce en seco cada WAL vivo, comprueba ranuras y bits, e informa huérfanos y desconocidos sin borrarlos (RO-4 para los que desaparecen durante la verificación). En flujo, sin búferes de archivo completo.
- **`inspect()`**: cabeceras de todos los archivos (versión, bits, `db_id`, número, tamaño); toma el `LOCK`.

### 5.9 Bloqueo y orden de bloqueo

- `txn_gate → queue → writer → manifest → state`; la cola del `Deleter` y el registro de instantáneas son hojas. Se documenta en `DESIGN.md` y se comprueba con `debug_assert` de orden en compilaciones de prueba.
- `LOCK` exclusivo para todo modo de apertura, `verify()` e `inspect()` (`Vfs::lock` solo ofrece exclusivo; coherente con SAD §4.2).

### 5.10 Plataforma (C-28)

Las garantías de `Sync` se verifican solo en **Linux** (ext4/xfs). En otra plataforma, `KernoDb::open` con `Durability::Sync` devuelve `InvalidArgument("durability not verified on this platform")` salvo `allow_unverified_platform = true`; macOS (`F_FULLFSYNC`) y Windows quedan sin verificar. La CI solo ejecuta Linux; una prueba `platform_gate_matches_target` fija el comportamiento por `cfg!(target_os = "linux")`.

## 6. Plan de verificación

### 6.1 Extensiones del testkit (WP03, opcionales: el comportamiento por defecto no cambia ni consume más números aleatorios)

| Extensión | Qué añade |
|---|---|
| `CrashKind::ProcessKillKeepPending` | la imagen conserva `durable` y `pending`; un corte de energía posterior sobre ella pierde lo no sincronizado (el M0 marcaba todo como duradero, `sim.rs:204-205, 241`) |
| `FsyncFailure::{KeepPending, LinuxDropPages}` | con `LinuxDropPages`, un `sync_data` fallido descarta para siempre las escrituras pendientes de ese inodo (los datos visibles no cambian) y el siguiente `sync_data` devuelve `Ok` (R-66) |
| `DirDurability::{Global, PerDir}` | con `PerDir`, `sync_dir(d)` solo hace duraderas las operaciones bajo `d` y un directorio nuevo solo es duradero tras sincronizar su padre |
| `set_space_limit(Some(bytes))` | escrituras y `set_len` que harían crecer el total fallan con `ErrorKind::StorageFull`, sin efecto; `remove_file` libera espacio (ENOSPC) |
| `crash_when(pred, nth, kind)`, `fail_when(pred, nth, then)` | disparadores filtrados por `OpInfo { kind, path, offset, len, phase }` |
| `start_trace()`/`trace()` | `(n, kind, path, offset, len, phase, thread)` por operación mutante |
| `dump()`/`restore()`/`assert_dir_unchanged(&a, &b, ignore)` | imagen completa en bytes (incluidos inodos desenlazados aún alcanzables) y comparación byte a byte |
| `set_sync_latency(d)` | `sync_data` duerme `d` **fuera** del mutex global (para que se formen grupos) |
| `corrupt_byte_live(path, off, mask)` | daño con escrituras pendientes, durante el uso |
| `HookVfs` | envoltorio genérico con ganchos antes/después de cada operación (errores, pánicos, retrasos); sustituye a los cuatro envoltorios escritos a mano |
| `crash::{Step<E>, scenario, run_scenario}` públicos, `Step::Ext(nombre, Arc<dyn Fn(&E) -> Result<()>>)` | escenarios por paquete sin nuevas restricciones de tipo en la macro compartida |
| `extended_failure_suite!(E)` (módulo `shared_ext_failure`) | barrido de muerte con pendientes + corte posterior (`Sync`, `Buffered`); barrido EIO con `LinuxDropPages` (`Sync`; `Buffered` con reinicio); barrido EIO en `Buffered` (falta en el M0); barrido de caídas con `PerDir`; barrido ENOSPC |
| `process` | re-ejecuta el binario de pruebas como proceso hijo (`std::env::current_exe()`, `--exact <prueba>`, variable `KERNO_CHILD`) para bloqueo entre procesos y `kill -9` |

### 6.2 Suites contra `kerno-storage`

| Objetivo (`crates/kerno-storage/tests/…`) | Contenido | WP |
|---|---|---|
| `shared.rs` | `engine_test_suite!(KernoDb, buffered_power_loss_prefix = true)`; módulos `inline` (perfil `Inline`) y `m0_profile` (perfil `M0`, desde WP05); `serial` (desde WP12) | 02, 03, 05, 12 |
| `ext_failure.rs` | `extended_failure_suite!` con `KernoDb` y con el perfil `Inline` | 03 |
| `enospc.rs`, `model.rs` | disco lleno; modelo `proptest` (put, delete, instantánea, reapertura, corte; ampliado con rangos, `compact_range` y transacciones) | 03, 05, 10, 13 |
| `metrics.rs`, `compaction.rs`, `close_budget.rs` | métricas y palancas de la condición 1 | 04, 05 |
| `format.rs` | cabeceras, bits, versiones, M0, directorios ajenos, límites, plataforma | 07 |
| `struct_vectors.rs`, `spec_decoder/` | vectores de estructura, decodificador independiente, hex de la especificación | 06 |
| `recovery_rules.rs`, `clean_shutdown.rs`, `read_only.rs`, `manifest_log.rs`, `file_lifecycle.rs`, `coverage.rs` | log de ediciones, tabla de decisión, solo lectura, borrado diferido, cobertura de la matriz | 08 |
| `verify.rs`, `crafted.rs`, `damage.rs`, `process_lock.rs` | integridad, archivos fabricados, daño durante el uso y tras caídas, bloqueo entre procesos | 09 |
| `shared_ranges.rs`, `range_delete.rs`, `compact_range.rs` | lápidas de rango y compactación dirigida | 10 |
| `dir_vectors.rs` | vectores de directorio, guardia, daño exhaustivo | 11 |
| `group_commit.rs`, `sync.rs` (+ `loom` en `src/commit_queue.rs`) | grupos, `sync()`, concurrencia | 12 |
| `txn.rs`, `snapshot_metrics.rs` | RMW y métricas de instantáneas | 13 |
| `engine.rs`, `regressions.rs` | pruebas del M0 portadas; las acopladas al formato se reescriben (`wal_damage` con ayudantes de `format`, `format_version` byte a byte esperando `Incompatible`, `short_sessions` con la métrica de L0) | 02, 05, 07 |
| `process_kill.rs` | `kill -9` real de un escritor con `StdVfs` | 14 |

`LogEngine` ejecuta todas las macros nuevas (`kerno-testkit/tests/reference_engine.rs`) con `sync()` y `DeleteRange` nativos; las transacciones, con `Emulated<LogEngine>`. Los spikes solo ejecutan `engine_test_suite!` y sus regresiones.

### 6.3 Cobertura de la matriz de fallos

`coverage.rs` (WP08, ampliado en cada paquete) ejecuta el escenario B en modo `Inline` con traza y asigna cada punto a una fila por `(fase, tipo de archivo, operación)`:

| Fila | Paso del protocolo | Fase / archivo / operación |
|---|---|---|
| L1–L4 | commit | `commit` / WAL / `write_at`, `sync_data` |
| L5′ | rotación (cabecera de WAL aún no duradera) | `rotate` / WAL / crear, `write_at`; dir / `sync_dir` |
| L6, L7′, L8 | flush: SST, registro `FLUSH`, borrado del WAL | `flush` / SST; `flush` / MANIFEST; `delete` / WAL |
| L9, L10′, L11 | compactación: salidas, registro `COMPACTION`, borrado diferido | `compact` / SST; `compact` / MANIFEST; `delete` / SST |
| L12′ | arranque de sesión S-2..S-5 | `session` / * |
| L13′ | cierre W-9 | `close` / * |
| CK | checkpoint | `checkpoint` / `MANIFEST.tmp`, rename |
| CR | creación DIR-3 | `create` / * |
| R0 | reapertura tras `CLEAN` (`SESSION_OPEN`) | `session` / MANIFEST |
| G1–G3, S1–S2 | grupo, `sync()` | `commit`, `sync` (escenario C, varios escritores) |
| RT, DC, SC | rangos, `compact_range`, subcompactaciones | `compact_range` / *; `compact` (si se construyen) |
| RO | solo lectura y `verify()`: **cero** operaciones mutantes | — |
| FL, NS, KP | `fsync` fallido de Linux, ENOSPC, muerte + corte | modos de `SimVfs` |

`failure_matrix_rows_are_covered` falla si una fila no recibe ningún punto. `threads_mode_rows_superset_of_inline` ejecuta el mismo escenario con hilos y comprueba que cubre las mismas filas (los errores que solo existen con hilos se cubren con `crash::run` y la suite compartida en modo `Threads`, que sigue siendo el perfil por defecto).

### 6.4 Escenarios `every_point` nuevos

- **A** (el del M0): creación, 25 commits, cierre, apertura, 20 commits, `Drop`, apertura, 10 commits, cierre.
- **B «formato»:** creación → 30 commits (10 % rangos) → `compact_range` → cierre → apertura → 10 transacciones → `sync` (`Buffered`) → `Drop` → apertura → checkpoint forzado → cierre. Se amplía en cada paquete con `Step::Ext`.
- **C «grupo»:** 4 escritores × 10 commits con `set_sync_latency`.
- Cada escenario bajo `PowerLoss`, `ProcessKill`, `ProcessKillKeepPending` (`Sync` y `Buffered`) y los barridos EIO en `Sync` y `Buffered` con ambos modelos de `fsync`, más ENOSPC. Además, barridos filtrados sobre las operaciones de `MANIFEST*` (`crash_when`) y la prueba dirigida `chained_crash_during_session_start_after_torn_wal` (REC-10c).

### 6.5 Prueba diferencial (`spike-bench/tests/differential.rs`)

Añade `kerno` (secuencia comprobada), 8 semillas en CI (64 en la semanal), observaciones también en instantáneas antiguas, un camino `Buffered`, e historias con `DeleteRange` (WP10) y RMW (WP13): `LogEngine` nativo como oráculo, spike-btree, spike-lsm, redb, fjall y SQLite a través de `Emulated<E>` (rango = recorrido + borrados de punto bajo el mutex de commit; RMW = lecturas de instantánea + lote).

### 6.6 Procesos reales y dispositivo real

- `process_lock.rs` (WP09): un hijo abre la base con `StdVfs` y espera; el padre comprueba `Locked` en apertura, solo lectura, `verify()` e `inspect()`; tras matar al hijo, la apertura funciona.
- `process_kill.rs` (WP14): el hijo escribe con `StdVfs` y publica por una tubería cada secuencia confirmada; el padre lo mata con `Child::kill` (SIGKILL) en momentos aleatorios, reabre y exige `s ≥ confirmada` y el modelo; 200 iteraciones en `Sync` y en `Buffered`.
- **LazyFS** (WP14): `spikes/storage/run-lazyfs.sh` y `.github/workflows/lazyfs.yml` (manual y semanal). Ejecuta el escenario A sobre un montaje LazyFS con pérdida de páginas no sincronizadas. **Su registro es criterio de salida** (R-84); si el entorno no permite FUSE, la salida queda pendiente de ese registro, no se da por cumplida.

### 6.7 CI y presupuesto de tiempo

- Trabajos de PR en paralelo: `checks` (fmt, clippy, guardias), `test-spikes` (spikes, testkit y diferencial; objetivo ≤ 15 min), `test-kerno` (`kerno-storage` con todos los perfiles; objetivo ≤ 25 min), `bench-smoke` (`tiny`, incluye `kerno`).
- Los perfiles adicionales (`inline`, `m0_profile`, `serial`, rangos) usan las semillas por defecto en PR.
- Semanal: `SPIKE_SEEDS=1000` en todo el espacio de trabajo, dividido en una matriz por objetivo de prueba para no superar el límite de 6 h por trabajo; `loom` (`RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue`); LazyFS.
- Si un paquete supera el presupuesto, reduce semillas por defecto de sus pruebas nuevas, no las de la suite compartida.

## 7. Criterios de aceptación

### 7.1 Puerta de corrección (ADR-0012 umbral 1, C-25), bloqueante

1. Cero fallos con `SPIKE_SEEDS=1000` en todas las suites de `kerno-storage` y en todos sus perfiles.
2. Cero fallos en todos los barridos `every_point` (escenarios A, B, C) para muerte de proceso, corte de energía y muerte + corte, en `Sync` y `Buffered` donde tenga sentido; en los barridos EIO (`Sync`, `Buffered` × ambos modelos de `fsync`) y ENOSPC; y en los barridos filtrados del MANIFEST.
3. Corrupción de `kerno` a 1000 semillas: **0 datos erróneos, 0 retrocesos, 0 inocuos** en `BitFlip`; 100 % de truncamientos detectados al abrir. Vectores limpios: daño exhaustivo con 0 inocuos.
4. `kill -9` (200 iteraciones × 2 modos) y el registro de LazyFS.
5. Todas las filas de la matriz cubiertas (`failure_matrix_rows_are_covered`).

### 7.2 Objetivos de rendimiento (se registran en `docs/testing/f1-objetivos-rendimiento.md` en WP04, **antes** de medir; los aprueba el propietario, C-06)

Escala `medium` (200 000 registros), `Sync`, mediana de 3 repeticiones intercaladas, misma sesión y misma máquina con `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` (ADR-0012). Valores M0 del 2026-10-27 (VM, 4 núcleos, ext4, `fdatasync` 4 KiB p50 113 / p99 568 µs).

| Id | Métrica | M0 | Objetivo |
|---|---|---|---|
| T1 | `agent_memory` p99 de escritura de memoria | LSM 5 837 µs; B+ tree 1 573; fjall 1 090 (3,71× / 5,36×) | ≤ 2,0× fjall **y** ≤ 1,5× spike-btree |
| T2 | p99 de commit: LiveSpec / Cauce / `append_seq` inserción / `append_seq` actualización / `history_sim` | 4 764 / 713 / 4 139 / 6 840 / 9 118 µs | ≤ 1,2× spike-lsm; además `append_seq` actualización ≤ 2× fjall (M0 2,60×); incumplimientos documentados |
| T3 | p99 de lectura: LiveSpec module-scan / symbol-get, Cauce, recall, recent-scan | 657 / 43,7 / 98,0 / 100 / 234 µs | ≤ 1,2× spike-lsm; LiveSpec module-scan ≤ 1,5× spike-btree (M0 1,49×), si no, revisión de la condición 5 |
| T4 | Cierre limpio tras `bulk_load` | 759 ms (`medium`), 3 587 ms (`large`); fjall 96 / 919 | ≤ 250 ms y ≤ 1 000 ms con el presupuesto por defecto |
| T5 | Rendimiento de cada carga | p. ej. 4 044 commits/s, carga 162 kops/s, lecturas 230 kops/s | ≥ 0,9× spike-lsm |
| T6 | `fsync` por commit | 1,00 | 1 escritor = 1,00; `concurrent_commits` con 8 escritores ≤ 0,5 y ≥ 2× el rendimiento de 1 escritor |
| T7 | Amplificación de escritura `small_commits` / `bulk_load` | 1,09× / 3,4× | ≤ 1,20× (la cabecera de trama añade 16 B por commit) / ≤ 3,6× |
| T8 | Reapertura limpia / recuperación sucia | 3,4 ms / 9,73 ms | ≤ 1,0× / ≤ 1,2× spike-lsm |
| T9 | Coste RMW (`rmw_counters`) | — | se informa (R-S6) |
| T10 | Rendimiento frente a fjall | 0,8–1,3× | se informa; incumplimientos de 2× documentados (C-07) |

Todo PR que cambie rendimiento adjunta la comparación con la línea base de la misma máquina (CONTRIBUTING) hasta que exista una CI de rendimiento dedicada (C-32). Para comparar con el M0, `spike-bench report --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` etiqueta esas filas como `lsm@m0`, etc., y nunca las mezcla con las nuevas.

## 8. Invariantes nuevas y matriz de fallos

Propuestas para `invariants.md` (WP08–WP13, promovidas en WP14):

| Id | Invariante | Pruebas |
|---|---|---|
| I-16 | Visibilidad tras durabilidad: en `Sync`, ningún lector observa datos que una caída pueda perder | `no_reader_sees_undurable_data`, `sync_mode_readers_never_see_undurable_seq` |
| I-17 | Tras `sync()` → `Ok(s)`, todo commit ≤ `s` sobrevive a un corte; un fallo envenena | `shared_sync::*` |
| I-18 | La apertura de solo lectura, `verify()`, `inspect()` y toda apertura rechazada no mutan el directorio (salvo crear `LOCK`) | `read_only_open_writes_nothing`, `verify_is_read_only`, `*_rejected_untouched` |
| I-19 | Semántica de rangos: orden dentro del lote, visibilidad por secuencias (SEM-3, SEM-4) | `shared_range::*`, `prop_mvcc_model_*` |

Cambios en las existentes: I-01 (atomicidad de cada miembro de un grupo), I-03 (el prefijo es obligatorio para `kerno`, C-24), I-04 (desbordamiento: error, nunca vuelta a 0, C-31), I-09 (checkpoint `CLEAN`), I-11 (REC-10c), I-12 (presupuesto de cierre a lo largo de sesiones cortas y borrado diferido), I-15 (de «parcial» a verificada).

La matriz de fallos gana una sección «LSM v1 (`kerno-storage`)» con las filas de §6.3 (L1–L4, L5′, L6, L7′, L8, L9, L10′, L11, L12′, L13′, CK, CR, R0, G1–G3, S1–S2, RT, DC, SC, RO, FL, NS, KP) y el presupuesto de `fsync` nuevo: commit `Sync` = 1 (< 1 con concurrencia), rotación = 1 `sync_dir` (+1 `sync_data` del WAL viejo en `Buffered`), flush = 3, compactación = salidas + directorio + 1, checkpoint = 2 + rename, reapertura limpia = 2. La sección M0 queda como histórica.

## 9. Fuera de alcance de la Fase 1

| Elemento | Fase |
|---|---|
| CLI (`kerno verify`, `kerno export --salvage`) | P2 |
| API tipada estable, semver, fachada `kerno` | P2 / ADR-0010 |
| Documentos, índice primario, filtros | P2 |
| Revisiones, ChangeSets, historial, políticas | P3 (ADR-0004) |
| Identificadores públicos | P4 (ADR-0005) |
| Cifrado y gestión de claves (v1 solo reserva la ranura) | P4 (ADR-0006) |
| Exportación lógica, supresión con garantías, copias, migraciones (v1 reserva el tipo de registro 5 y bits) | P4 (ADR-0003 §5, ADR-0009) |
| Índices secundarios, texto, vectores | P5 |
| Fuzzing con `cargo fuzz`, `cargo audit`/`cargo deny`, benchmarks beta | P6 |
| Durabilidad por commit (`commit_with`) | diferida; el formato ya la admite (C-05) |
| Compresión, compresión de prefijos, índice o filtro particionados, nivel base dinámico | no requeridos; el formato los admite vía bits `incompat` |
| Reciclado y preasignación del WAL | añadidos tras WP05 como palanca de la condición 1 (`Config::wal_preallocate`, [resultados](../testing/f1-diagnostico-colas.md#8-preasignación-y-reciclado-del-wal)); sin cambio de formato: usan el relleno de ceros de v1.md §6.1 |
| Escritura multiproceso, servidor, replicación, SQL | no objetivos (SAD §1.4) |
| Garantías en macOS y Windows | sin verificar (§5.10) |

## 10. Riesgos

### 10.1 Registro

| # | Riesgo | Señal temprana | Mitigación |
|---|---|---|---|
| K1 | La condición 1 no se cumple | WP04 no atribuye la p99 a mecanismos corregibles; T1 falla en WP05 | Diagnóstico con métricas por fase; palancas con interruptores; regla de decisión cuantitativa; memorando al propietario |
| K2 | Las lápidas de rango devuelven datos erróneos | contraejemplo del modelo MVCC | Visibilidad solo por secuencias; modelo `proptest` como oráculo principal; suite validada antes en `LogEngine`; diferencial; barridos |
| K3 | Group commit rompe la regla de cola rota o el envenenamiento | barrido con varios escritores | Trama = grupo con `synced_len`; un grupo en vuelo; `catch_unwind`; `loom` |
| K4 | La recuperación del log de ediciones retrocede o pierde commits | inocuos o retrocesos en la suite de corrupción | Regla del WAL requerido, `CLEAN` en el checkpoint, W-8, barridos filtrados, REC-10c con prueba dirigida |
| K5 | Deriva entre especificación y código; vectores inestables | guardia de regeneración | Vectores de estructura antes del motor; `spec_decoder`; `spec_hex_matches_vectors`; proceso de §15 de v1.md |
| K6 | RMW: pérdidas de actualizaciones, interbloqueos | prueba de contadores concurrentes | Pesimista con vaciado; orden de bloqueo comprobado; `loom` |
| K7 | La extracción del harness pierde cobertura | cambian las huellas FP | WP01 movimiento puro; FP en todos los paquetes |
| K8 | Barridos inestables por hilos | semillas no reproducibles | `Inline`, presupuesto solo en bytes, `threads_mode_rows_superset_of_inline` |
| K9 | Benchmarks no comparables | dispersión > 15 % | Misma sesión siempre; objetivos en proporciones; máquina objetivo nombrada antes de WP14 |
| K10 | Alcance mayor que el calendario (C-33) | velocidad de los paquetes | Lista de recortes (§10.2) |
| K11 | Dependencias nuevas no disponibles sin red (`proptest` está en la caché local; `loom` no, y Cargo la resuelve en `Cargo.lock` aunque solo se compile con `--cfg kerno_loom`) | fallo de `cargo fetch` al añadirla | Si `loom` no se puede resolver, WP12 la sustituye por un explorador de intercalados propio y exhaustivo para la máquina de estados de la cola (≤ 3 hilos, permutaciones explícitas de pasos) y lo documenta; las licencias se justifican en el PR (R-21) |

### 10.2 Recortes y lo que nunca se recorta

Se recorta, en este orden: subcompactaciones y segundo hilo (ya condicionales), WAL precreado (solo si H4 se confirma), automatización de LazyFS (la ejecución manual de salida se mantiene), `inspect()` como función separada, pulido de `report --baseline`. **Nunca:** pruebas de modelo, barridos filtrados del MANIFEST, suite de caídas con varios escritores, prueba de reentrancia REC-10c, vectores y su guardia, huella FP.

## 11. Paquetes de trabajo

Los paquetes forman un **grafo de dependencias**, no una cadena. Cada uno se implementa en un *worktree* propio, partiendo de la rama con todas sus dependencias ya integradas; pasa una revisión desde dos enfoques (durabilidad y formato; contrato y pruebas) y sus correcciones, y se integra en la rama con un único commit (*squash*), de uno en uno, tras pasar G0 completo y FP (§1). Cuando dos paquetes paralelos tocan el mismo archivo, sus cambios se mantienen aditivos y locales, y los métodos nuevos de los traits del contrato llevan cuerpo por defecto para no romper a quien los implementa en paralelo.

Para paralelizar, tres paquetes se dividen en una mitad de **contrato y oráculo** (`kerno-core`, `kerno-testkit`, `LogEngine`, `Emulated<E>`, spikes, bench), que se valida primero contra la referencia (principio 5), y una mitad de **motor** (`kerno-storage`): WP10a/WP10b, WP12a/WP12b y WP13a/WP13b. WP03 se divide igual (WP03a, testkit; WP03b, motor). `Config` y el módulo `testing` (`Profile`, `Profiled<P>`) se crean en WP02 para que WP03b, WP04 y WP05 los amplíen sin chocar. Los vectores de directorio de group commit pasan de WP12 a WP11.

| WP | Título | Depende de | Nivel | Resultado |
|---|---|---|---|---|
| WP01 | Extraer `kerno-core` y `kerno-testkit`; `spike-core` como shim | — | 1 | movimiento puro; huellas idénticas |
| WP02 | Portar `spike-lsm` a `kerno-storage` (copia), `Config` y `testing`; conectar bench, diferencial y CI | WP01 | 2 | 24 + 7 + 7 pruebas verdes con `kerno` |
| WP03a | Testkit: modelo de fallos, sonda de fase, `HookVfs`, procesos, `extended_failure_suite!` sobre `LogEngine` | WP01 | 2 | suites extendidas verdes en la referencia |
| WP03b | Modo `Inline`, perfil `Inline`, fases en el motor, suites extendidas, ENOSPC y modelo `proptest` sobre `kerno`; CI dividida | WP02, WP03a | 3 | suites extendidas verdes en `kerno` |
| WP04 | Observabilidad, integración en el bench, diagnóstico de colas y objetivos registrados | WP02 | 3 | `f1-objetivos-rendimiento.md`, `f1-diagnostico-colas.md` |
| WP06 | Códec v1, vectores de estructura, decodificador independiente, ADR-0013 | WP02 | 3 | formato como código puro |
| WP10a | `DeleteRange` en el contrato, spikes y oráculo; `range_delete_test_suite!` sobre la referencia | WP02, WP03a | 3 | suite de rangos validada en `LogEngine` y `Emulated<LogEngine>` |
| WP12a | `SyncEngine`, `sync_test_suite!`, `group_commit_test_suite!`, escenario C sobre `LogEngine` | WP03a | 3 | suites de `sync()` y concurrencia validadas |
| WP05 | Colas acotadas, presupuesto de cierre, perfil M0 y punto de decisión | WP03b, WP04 | 4 | T1–T5 medidos (provisionales); GO o memorando |
| WP07 | El motor escribe y lee solo v1 (MANIFEST como instantánea) | WP06, WP03b | 4 | suites verdes sobre v1, 0 inocuos |
| WP13a | `TxnEngine`, transacciones en `Emulated<E>`, `txn_test_suite!`, `rmw_counters`, `differential_rmw` (referencias) | WP10a, WP12a | 4 | suite de transacciones validada |
| WP08 | MANIFEST como log, sesión, `CLEAN`, tabla de recuperación, solo lectura, borrado diferido | WP07, WP05 | 5 | condición 3a, matriz cubierta |
| WP12b | Group commit, `sync()`, `loom`, contrapresión en `kerno` | WP07, WP12a, WP04, WP03b | 5 | condición 3b, T6 |
| WP09 | `verify()`, `inspect()`, archivos fabricados, daño durante el uso, bloqueo entre procesos | WP08 | 6 | integridad (R-69, R-85, R-67) |
| WP10b | Lápidas de rango en `kerno` y `compact_range` | WP08, WP05, WP10a | 6 | condición 2 |
| WP11 | Vectores de directorio (también los de group commit), guardia de regeneración, candidato a congelación | WP09, WP10b, WP12b | 7 | formato completo en disco y protegido |
| WP13b | Transacciones RMW y métricas de instantáneas en `kerno` | WP12b, WP10b, WP13a | 7 | ADR-0002 completo |
| WP14 | `kill -9`, LazyFS, puerta de 1000 semillas, benchmarks, congelación y documentación | WP11, WP13b | 8 | salida de la Fase 1 |

El camino crítico tiene 8 niveles (WP01 → WP02 → WP03b → WP07 → WP08 → WP10b → WP11/WP13b → WP14) frente a los 14 de la cadena original. Las mediciones de WP04 y WP05 se hacen mientras otros paquetes compilan y prueban en la misma máquina: se apoyan en métricas estructurales del motor (episodios de bloqueo, profundidad de L0, tamaño de las compactaciones), registran la carga del sistema y son **provisionales**; la medición de referencia es la de WP14, con la máquina en reposo.

## 12. Ambigüedades resueltas

| Id | Resolución |
|---|---|
| C-01 | `verify()` de biblioteca en la Fase 1; CLI en la Fase 2 |
| C-02 | Group commit obligatorio (ADR-0001 manda); se demuestra con `fsync` por commit < 1 |
| C-03 | Ranura de escritura: la transacción la mantiene hasta encolar; los commits ciegos solo para encolar; el líder agrupa lotes ciegos |
| C-04, C-05 | `synced_len` por trama sustituye a `SYNCED`; la durabilidad por commit se difiere (el formato ya la admite) |
| C-06, C-07, C-08 | Objetivos T1–T10 registrados antes de medir; el «2× fjall» se aplica a rendimiento y a las p99 nombradas; se propone aceptar ADR-0012 con ellos |
| C-09 | Comparaciones en la misma sesión; el propietario nombra la máquina objetivo antes de WP14 (propuesta: la clase de VM del M0 y una máquina Linux ext4 NVMe de consumo) |
| C-10 | Parte de L0 obligatoria en el cierre; el resto con presupuesto |
| C-11 | Solo mecanismo; rangos en la base de v1 (sin bit) |
| C-12 | Ranura de 40 B por bloque de SST y registro de MANIFEST; en el WAL, por archivo (pendiente de firma en ADR-0013) |
| C-13 | Cabecera en el desplazamiento 0 de todos los archivos; la SST conserva el pie; `LOCK` exento; `.tmp` sigue a su tipo final |
| C-14 | Solo un WAL sin tramas tras un cierre sucio (o mayor que `next_file` tras uno limpio) puede tener la cabecera rota |
| C-15 | Las rutas de solo lectura toman el `LOCK` exclusivo y solo pueden crear `LOCK` |
| C-16 | `Incompatible` y `ReadOnly` |
| C-17 | API inestable; contrato oculto |
| C-18 | Contrato en `kerno-core`; harness en `kerno-testkit`; `spike-core` como shim |
| C-19 | «Congelar» = bytes estables + vectores + apertura y recuperación probadas; migraciones con v2 (tipo de registro 5 reservado) |
| C-21 | ADR-0013 nuevo; los ADR aceptados solo reciben notas de seguimiento |
| C-22 | SAD 0.2.0 en WP14 |
| C-23 | I-16 |
| C-24 | `buffered_power_loss_prefix = true` para `kerno` |
| C-25 | La puerta cubre todos los barridos y el corte aleatorio en `Buffered` |
| C-26 | Límites de formato y de API separados (v1.md §1.4) |
| C-27 | `scan` paginado y reanudable; iteración en flujo en la Fase 2 |
| C-28 | Linux verificado; el resto, sin verificar y con puerta en `open` |
| C-29 | La salida se juzga contra la matriz actualizada (§8) |
| C-30 | §5.7 |
| C-31 | Error, nunca vuelta a 0 |
| C-32 | Regla manual por PR hasta tener una máquina dedicada |
| C-33 | Orden de §11; la contingencia (R-10) en el punto de decisión de WP05 |

## 13. Decisiones del propietario

1. Aprobar T1–T10 antes de la medición de WP05 (WP04 los registra como propuesta).
2. Aceptar ADR-0013 antes de la congelación (WP14), en particular: ranura AEAD del WAL por archivo, sin archivo `CURRENT`, `CLEAN` dentro del checkpoint, `synced_len`, `Incompatible`/`ReadOnly`, solo lectura automática ante `ro_compat` desconocido, bit `DRAFT`.
3. Aceptar ADR-0012 o adoptar sus umbrales como criterio de salida.
4. Nombrar la máquina o máquinas objetivo antes de WP14.
5. Decidir sobre el memorando de WP05 si T1 queda incumplido.
