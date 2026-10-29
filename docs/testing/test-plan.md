# Plan de pruebas

Estado: vigente; ampliado al cierre de la Fase 1 (2026-10-29, §7) · Relacionado con: [SAD §17.6, §18](../architecture/SAD.md#18-calidad-pruebas-y-definición-de-terminado), [invariantes](../architecture/invariants.md), [matriz de fallos](../architecture/failure-matrix.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md)

## 1. Principio

Las garantías de Kerno solo existen si una prueba automatizada las demuestra. Cada invariante de [`invariants.md`](../architecture/invariants.md) cita la prueba que la verifica; una invariante sin prueba figura como pendiente.

## 2. Infraestructura de M0

El contrato, los errores y `Vfs`/`StdVfs` viven en [`crates/kerno-core`](../../crates/kerno-core); los simuladores, las suites compartidas y `LogEngine`, en [`crates/kerno-testkit`](../../crates/kerno-testkit). [`spikes/storage/spike-core`](../../spikes/storage/spike-core) solo los re-exporta con las rutas que usan los spikes:

| Pieza | Qué hace |
|---|---|
| `Engine` / `Snapshot` | Contrato común a ambos candidatos (batches atómicos, secuencia de commit, snapshots, apertura exclusiva, errores tipados). |
| `Vfs` | Todo acceso a archivos pasa por esta abstracción; los motores nunca usan `std::fs` directamente. |
| `SimVfs` | Sistema de archivos en memoria que simula **corte de energía** (escrituras no sincronizadas descartadas, reordenadas o rotas a 512 B o a nivel de byte; metadatos de directorio como prefijo journaled) y **muerte del proceso** (sobrevive todo lo escrito). Puede provocar la caída justo antes de la *n*-ésima operación de E/S mutante, o **un único error de E/S** (EIO) en esa operación con el dispositivo vivo. Desde la Fase 1, con modelos opcionales desactivados por defecto (ver la [matriz de fallos](../architecture/failure-matrix.md)): muerte del proceso que deja pendiente lo no sincronizado (`CrashKind::ProcessKillKeepPending`), semántica de Linux tras un `fsync` fallido (`FsyncFailure::LinuxDropPages`), durabilidad por directorio (`DirDurability::PerDir`) y disco lleno (`set_space_limit`, `fill_disk_after`); además, caídas y errores filtrados por tipo de operación, ruta y fase (`crash_when`, `fail_when`, con la fase de `kerno_core::probe`), trazas de E/S, imágenes byte a byte del directorio (`dump`, `restore`, `assert_dir_unchanged`), latencia de `fsync` y daño del medio durante el uso (`corrupt_byte_live`). |
| `HookVfs` | Envuelve otro `Vfs` y llama a un *hook* antes y después de cada operación, que decide si sigue, falla, entra en pánico o se retrasa. |
| `process` | Relanza el binario de pruebas como proceso hijo con un rol (`spawn_child`, `child_role`), para pruebas entre procesos. |
| `CountingVfs` | Cuenta bytes, operaciones y `fsync` para medir amplificación. |
| `engine_test_suite!` | Genera la misma batería de pruebas para cada motor. Con `config = SuiteConfig::with_range_deletes(n)`, la misma batería con `n` milésimas de las operaciones aleatorias convertidas en borrados de rango (también el escenario de los barridos `every_point`); sin `config` es exactamente la del M0 (`WorkloadCfg` por defecto no consume ningún número aleatorio adicional). |
| `range_delete_test_suite!` | Genera el módulo `shared_range` para motores que implementan `RangeDeleteEngine`: límites `[start, end)` y rangos vacíos, orden dentro del lote (SEM-3), lecturas y escaneos paginados a través de lápidas de varias generaciones, aislamiento de instantáneas, modelo aleatorio con rangos, persistencia tras cierre, caída y corte de energía, y espacio. Validada sobre `LogEngine` y `Emulated<LogEngine>` (ver §2.3). |
| `Emulated<E>` | Da borrados de rango a cualquier motor: bajo un mutex de commit, cada rango se convierte, en su posición del lote, en borrados de punto de las claves que existen en ese punto (instantánea más claves del propio lote). La reescritura nunca introduce la clave vacía, que algunos motores rechazan (fjall aborta con un pánico): solo la borra cuando ya la contienen las operaciones del propio lote o el estado guardado, cosa imposible en un motor que la rechaza. Es un oráculo de pruebas, no una implementación. |
| `extended_failure_suite!` | Genera el módulo `shared_ext_failure`: los modelos opcionales de `SimVfs` en cada operación de E/S del escenario A (ver §2.2). |
| `LogEngine` | Motor trivialmente correcto que valida el propio harness (si falla una prueba compartida, la prueba está mal). Implementa los borrados de rango de forma nativa (operación 3 del log). |

### 2.1 Validación del harness (mutation testing manual)

Antes de confiar en el harness se le inyectaron fallos conocidos (en `LogEngine` y, para las filas 6 y 7, en una copia del prototipo B+ tree). Las filas marcadas en negrita obligaron a reforzar el harness:

| Mutación | Detectada por |
|---|---|
| Omitir `fsync` en commits `Sync` | `power_loss_sync`, `power_loss_at_every_io_point` |
| Ignorar el error de un `fsync` fallido y confirmar el commit | `single_io_failure_at_every_point` (variante con corte de energía inmediato; la primera versión, sin ella, no lo detectaba) |
| Sincronizar el directorio solo si el archivo «acaba de crearse» | `single_io_failure_at_every_point` — **error real** de `LogEngine`, corregido |
| `rename` del archivo reescrito sin `fsync` previo | `power_loss_sync`, `power_loss_buffered` (tras reducir el espacio de claves del workload para forzar reescrituras del log; la primera versión del harness no lo detectaba) |
| Tratar corrupción a mitad del log como cola rota | `bit_flips_are_detected_or_harmless` |
| B+ tree: `close()` no escribe el segundo slot del meta | `bit_flips_are_detected_or_harmless` (semilla 7) — **solo tras dirigir la mitad de los daños a cabeceras/metas**; con daño uniforme sobrevivía > 40 semillas |
| B+ tree: el commit sobrescribe el slot del meta **vigente** | `power_loss_sync`, `power_loss_at_every_io_point` — **solo tras añadir desgarro a nivel de byte**; con desgarro a 512 B el meta (92 B) se escribía de forma atómica y el fallo era invisible |
| (Log truncado tras cierre limpio) | `truncations_are_detected_or_harmless` — por eso `LogEngine` declara `detects_truncation = false` |

### 2.2 Suite de fallos extendida (Fase 1)

`extended_failure_suite!` ([`harness/ext.rs`](../../crates/kerno-testkit/src/harness/ext.rs)) se ejecuta sobre `LogEngine` en `kerno-testkit/tests/reference_engine.rs`. Encontró tres **errores reales** de `LogEngine`, corregidos con una prueba de regresión cada uno (en `kerno-testkit/src/reference.rs`; cada prueba falla sin su corrección):

| Error real de `LogEngine` | Detectado por (`shared_ext_failure::`) | Regresión |
|---|---|---|
| La recuperación daba por bueno lo que podía leer: los registros no sincronizados (sesión `Buffered`, caída antes del `fsync`) y las páginas descartadas por un `fsync` fallido nunca se hacían durables, y un corte de energía posterior perdía o corrompía estado confirmado. Ahora reescribe el prefijo recuperado y lo sincroniza antes de aceptar commits. | `power_loss_after_kill_keep_pending_{sync,buffered}`, `single_io_failure_at_every_point_buffered`, `linux_fsync_drop_faults_{sync,buffered_restart}` | `recovery_makes_an_unsynced_tail_durable`, `recovery_rewrites_pages_dropped_by_a_failed_fsync` |
| Una base nueva no sincronizaba el padre de su directorio: con durabilidad por directorio, la base entera podía desaparecer tras un commit confirmado. Ahora sincroniza el directorio que contiene cada directorio que creó, también `.` en una ruta relativa (una primera versión solo sincronizaba el padre inmediato y seguía fallando con rutas anidadas como `/data/db`; la siguiente sincronizaba todos los ancestros hasta `/`, saltaba `.` y no podía crear la base bajo un ancestro de solo travesía). | `per_dir_durability_power_loss_sync` | `new_database_directory_survives_power_loss_with_per_dir_durability` |
| En `Buffered`, `close` sincronizaba la marca de cierre limpio junto con los registros que avala: un corte de energía podía conservar la marca y perder un registro anterior, y la recuperación informaba corrupción. Ahora sincroniza primero los registros. | `single_io_failure_at_every_point_buffered` | `buffered_close_makes_records_durable_before_the_clean_marker` |

La revisión de la suite corrigió además dos defectos del propio modelo, cada uno con su autoprueba de `SimVfs`: un `fsync` fallido con semántica de Linux descartaba también los `set_len` pendientes, de modo que un truncamiento cuyo `fsync` fallaba nunca llegaba al disco (`linux_drop_pages_keeps_length_changes`; `recovery_truncation_survives_a_failed_sync` muestra el efecto sobre `LogEngine`: un registro obsoleto reaparecía tras un commit confirmado), y con durabilidad por directorio un `rename` entre directorios podía resucitar el nombre de origen (`per_dir_cross_dir_rename_does_not_resurrect_source`). También endureció el modelo de disco lleno: un archivo borrado o reemplazado conserva su espacio mientras siga abierto, como en Linux (`space_limit_freed_by_remove`).

### 2.3 Borrados de rango (Fase 1, WP10a)

`range_delete_test_suite!` ([`harness/range.rs`](../../crates/kerno-testkit/src/harness/range.rs)) se valida primero sobre `LogEngine` (nativo) y `Emulated<LogEngine>` en `kerno-testkit/tests/reference_engine.rs`; `engine_test_suite!` con `config = SuiteConfig::with_range_deletes(20)`, sobre los mismos dos motores en `kerno-testkit/tests/reference_ranges.rs` (un objetivo aparte para que la huella FP de `reference_engine` solo imprima la suite por defecto). Los spikes y los adaptadores del benchmark rechazan los rangos con `InvalidArgument` antes de escribir nada (`engines_without_range_deletes_reject_them`). `kerno-storage` los implementa desde WP10b y ejecuta lo mismo sobre sí mismo: `tests/shared_ranges.rs` (`engine_test_suite!` con `SuiteConfig::with_range_deletes(20)` y `range_delete_test_suite!`, sobre `KernoDb` y los perfiles `Inline` y `M0`), `tests/range_delete.rs` (lectura en cada fuente, SST de solo fragmentos, fragmentos cortados en las salidas, extremos exclusivos, descarte en el último nivel, rotación a las 4 096 lápidas, filtro bloom y barrido de caídas en cada punto del escenario B con rangos), `tests/compact_range.rs` (la marca no queda en ningún archivo vivo, instantáneas retenidas, checkpoint del MANIFEST, caché y barrido de caídas), el modelo MVCC `prop_mvcc_model_*` de `tests/model.rs` (el oráculo principal: puts, deletes, rangos, instantáneas, `flush`, compactaciones y `compact_range`, reaperturas y cortes de energía; `PROPTEST_CASES=5000` en la puerta) y `differential_range_deletes`, que añade `kerno` al oráculo nativo `LogEngine`.

Para comprobar que la suite detecta errores se le inyectaron once mutaciones, una cada vez, y se ejecutó `shared_range` sobre los dos motores de referencia; la duodécima, propia de un LSM, se modela con un motor de prueba (última fila). Todas se detectaron:

| Mutación | Detectada por (`shared_range::`) |
|---|---|
| `LogEngine`: el extremo final del rango incluido | todas salvo `space_is_reclaimed_after_range_deletes` |
| `LogEngine`: los rangos se aplican después de los puntos de su lote | `order_within_batch_range_then_put`, `scan_across_tombstones_in_all_sources`, `range_survives_reopen`, modelo aleatorio, aislamiento |
| `LogEngine`: los rangos se aplican antes de los puntos de su lote | `order_within_batch_put_then_range`, modelo aleatorio, aislamiento |
| `LogEngine`: la recuperación ignora los rangos del log | `range_survives_reopen` y las demás pruebas que reabren |
| `LogEngine`: un lote de rangos vacíos no consume secuencia | `delete_range_bounds_and_degenerate`, aislamiento |
| `LogEngine`: nunca reescribe su log (no recupera espacio) | `space_is_reclaimed_after_range_deletes` (nativo, en el ciclo 6, y `Emulated`) |
| `Emulated`: el rango ignora las claves escritas antes en su lote | `order_within_batch_put_then_range`, modelo aleatorio, aislamiento |
| `Emulated`: el lote reescrito puede quedar vacío (sin secuencia) | `delete_range_bounds_and_degenerate`, `scan_across_tombstones_in_all_sources`, aislamiento |
| `Emulated`: solo se borra la primera página de un rango | `space_is_reclaimed_after_range_deletes` |
| `Emulated`: el extremo final del rango incluido | todas salvo `space_is_reclaimed_after_range_deletes` |
| `WriteBatch::validate` acepta `start > end` | `delete_range_bounds_and_degenerate` (ambos motores) |
| Nunca descarta los puntos cubiertos solo por un rango (las sobrescrituras sí se recuperan) | `space_is_reclaimed_after_range_deletes`, en el ciclo 6 (autoprueba `space_check_fails_for_an_engine_that_keeps_range_deleted_points`) |

`space_is_reclaimed_after_range_deletes` escribe en cada ciclo claves que ningún ciclo anterior escribió (todas dentro de `["c", "d")`): ningún valor se sobrescribe ni se borra con una operación de punto, así que solo los borrados de rango convierten en basura los valores antiguos. Una primera versión rellenaba siempre las mismas claves; cada relleno ocultaba los valores borrados por rango con un punto más nuevo de la misma clave, y un motor que recupera las versiones sobrescritas pero conserva los puntos cubiertos solo por un rango (el error de compactación que WP10b debe excluir) pasaba la prueba y la suite entera. `space_check_fails_for_an_engine_that_keeps_range_deleted_points` lo comprueba con un motor así: `LogEngine` más un archivo que conserva los bytes de valor de cada punto cubierto por un rango hasta que esa clave se vuelve a escribir; falla en el ciclo 6, y pasaba la versión anterior. La prueba no tiene un número fijo de ciclos: un borrado de rango ocupa pocos bytes, así que un motor que no recupera nada conserva al menos todos los bytes de valor escritos (aleatorios, incompresibles), y la prueba sigue (mínimo seis ciclos, máximo 64) hasta que esos bytes por sí solos superan la cota `4 × primer llenado + 1 MiB` en dos ciclos. Con seis ciclos fijos, el `LogEngine` nativo que nunca reescribe su log pasaba (crece un llenado por ciclo y queda bajo la cota); ahora falla en el ciclo 6. `space_check_fails_for_an_engine_that_reclaims_nothing` lo comprueba con un motor que guarda en su directorio, para siempre, las claves y valores de cada lote.

La revisión del paquete añadió tres regresiones. `Emulated<E>` convertía un lote formado solo por rangos vacíos en el borrado de la clave vacía, y fjall aborta con un pánico ante una clave vacía; la semilla 2027 no tiene un lote así, la 1 sí (commit 162). Ahora la reescritura nunca introduce la clave vacía (`the_rewrite_never_introduces_the_empty_key`, con un motor que aborta ante ella; `an_existing_empty_key_is_deleted_like_any_other` fija que solo la borra cuando ya está guardada o la pone el propio lote; `emulated_engines_commit_empty_ranges_and_ranges_from_the_empty_key`, sobre todos los motores del diferencial) y `differential_range_deletes` recorre las historias de las semillas 2027 y 1. La prueba de rechazo de `kerno-storage` llena antes el búfer de escritura, de modo que un lote que pasara la comprobación de la entrada de `commit` cambiaría de memtable antes de llegar al rechazo del WAL: sin esa comprobación, la prueba falla (el disco registra operaciones), y antes pasaba igual.

Sobre `kerno-storage` (WP03b), la suite extendida se ejecuta con hilos de segundo plano y con el perfil `Inline` (`tests/ext_failure.rs`, módulos `threads` e `inline`), junto con la suite compartida en ese perfil (`tests/shared.rs`, módulo `inline`), el determinismo del modo en línea (`tests/inline.rs`), disco lleno dirigido en el commit, en el flush y en cada operación de la apertura (`tests/enospc.rs`) y la prueba de modelo `proptest` con instantáneas, reaperturas y cortes de energía (`tests/model.rs`; casos: `PROPTEST_CASES`, si no `SPIKE_SEEDS`, si no 256). En el escenario A no encontró errores. El modelo de durabilidad por directorio, con una ruta anidada, sí encontró uno **real**, el mismo que tenía `LogEngine`: una base nueva solo sincronizaba el padre inmediato de su directorio, así que con `/data/apps/db` sobre un disco vacío un corte de energía podía borrar la base entera tras commits `Sync` confirmados. Ahora una base nueva sincroniza el directorio que contiene cada directorio que creó (regresión `nested_new_directory::new_nested_database_directory_survives_power_loss_with_per_dir_durability` en `tests/regressions.rs`; falla sin la corrección). La revisión de WP03b encontró dos defectos en la primera corrección, que sincronizaba todos los ancestros hasta `/`: un ancestro que solo se puede atravesar (modo 0711) impedía para siempre crear la base, y el directorio actual de una ruta relativa (`db`, `data/apps/db`) nunca se sincronizaba. Cada caso tiene su regresión en el mismo módulo (las rutas relativas, en la anterior): `traverse_only_ancestors_do_not_prevent_creating_a_database`, `unsyncable_parent_fails_the_open`, `retried_open_syncs_the_ancestors_an_earlier_open_created`, `retried_open_after_a_partial_create_dir_all_syncs_the_ancestors` (una apertura anterior dejó `/data/apps` sin `db`: los ancestros por encima de los creados también se sincronizan, sin garantía, en toda base nueva), `retried_open_does_not_swallow_an_io_error_syncing_an_ancestor` (esa sincronización sin garantía solo se detiene en silencio ante `PermissionDenied`, `ReadOnlyFilesystem` o `Unsupported`; un `EIO` hace fallar la apertura) y `execute_only_ancestor_on_the_real_file_system` (sistema de archivos real; se salta si el proceso no está sujeto a los bits de permiso, como root, y ahí se ejecuta con `setpriv --bounding-set=-dac_override,-dac_read_search`). `SimVfs` trata `.` y la ruta vacía como el mismo directorio (`current_dir_aliases_name_the_same_directory`). Con hilos, `TestHooks::compact_once` devuelve si terminó alguna compactación mientras esperaba (`threads_compact_once_reports_a_running_compaction` en `tests/inline.rs`).

## 3. Pruebas compartidas (SAD §17.6)

| Requisito de SAD §17.6 | Prueba |
|---|---|
| La misma secuencia de operaciones produce el mismo estado lógico en cada candidato | `spike-bench/tests/differential.rs` (prototipos, `LogEngine`, redb, fjall y SQLite); con borrados de rango, `differential_range_deletes` (historias de las semillas 2027 y 1; `LogEngine` nativo como oráculo; prototipos, redb, fjall y SQLite a través de `Emulated<E>`; también instantáneas tomadas a mitad de cada tramo) |
| Insert/update/delete/reopen recupera el estado esperado | `functional::*` (reapertura limpia y sucia, modelo aleatorio de 1200 commits, aislamiento, concurrencia, espacio) |
| Detecta o rechaza archivos truncados | `corruption::truncations`, `corruption::bit_flips` |
| Fallos en puntos predefinidos comparados con la matriz esperada | `crash::every_point` (caída antes de **cada** operación de E/S de un escenario fijo, para corte de energía y muerte de proceso), `faults::single_failure_at_every_point` (un EIO aislado en cada punto) y `crash::run` (puntos aleatorios en historias largas, incluso durante la recuperación) |
| Benchmarks repetibles con un único comando | [`spikes/storage/run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh) |
| Al menos una medición de rango, recuperación y memoria/disco | `range_scans`, `bulk_load`/`cauce` (`recovery_ms`), `peak_rss_mb`, `space_amp*` |

### 3.0 Revisión adversarial y regresiones

Además de las suites compartidas, cada candidato pasó por una revisión adversarial (cinco revisores por clase de fallo —protocolo de caídas, concurrencia, integridad frente a archivos dañados, semántica del contrato, rendimiento— y dos verificadores independientes por hallazgo, que debían refutarlo o reproducirlo con una prueba). Los defectos confirmados quedaron como pruebas en `tests/regressions.rs` del motor correspondiente, salvo tres del LSM de severidad baja que solo se verificaron con la reproducción del revisor: lotes de más de 4 GiB (la prueba exigiría asignar más de 4 GiB), aritmética sin comprobar con SST fabricadas y la doble lectura de bloque en `seek` (rendimiento). La lista completa está en el [informe de M0 §2.2](m0-report.md#22-defectos-encontrados-y-cómo-se-encontraron).

### 3.1 Cómo ejecutarlas

```bash
cargo test -p kerno-testkit                 # harness + LogEngine
cargo test -p spike-btree                   # candidato CoW B+ tree
cargo test -p spike-lsm                     # candidato LSM
cargo test -p spike-bench --all-features --release           # prueba diferencial (con kerno)
cargo test --workspace --all-features --release              # todo (la CI lo reparte en dos trabajos paralelos:)
cargo test --workspace --exclude kerno-storage --all-features --release   #   test-spikes
cargo test -p kerno-storage --all-features --release                      #   test-kerno (todos los perfiles)

# Más semillas (las suites aleatorias usan entre 4 y 150 por defecto):
SPIKE_SEEDS=1000 cargo test -p spike-btree --release

# Reproducir un fallo concreto:
SPIKE_SEED=123 cargo test -p spike-lsm shared_crash::power_loss_sync
SPIKE_POINT=57 cargo test -p spike-lsm shared_crash::power_loss_at_every_io_point
```

## 4. Pirámide de pruebas por fase (SAD §18.1)

| Nivel | M0 (spike) | Fase 1 en adelante |
|---|---|---|
| Unitarias | codificación de nodos/bloques, filtros bloom, manifest, WAL | + serialización documental, claves, políticas |
| Propiedades / modelo | modelo `BTreeMap` con historias aleatorias | `proptest` con reducción de casos; modelos de revisiones e índices |
| Integración | contrato `Engine` | API pública, CLI, documentos, revisiones |
| Crash testing | `SimVfs` (energía y proceso, todos los puntos de E/S; un EIO aislado en cada punto) | + modelos opcionales de `SimVfs` en cada punto (`extended_failure_suite!`: semántica Linux de páginas descartadas tras `fsync` fallido, durabilidad por directorio, disco lleno, muerte del proceso sin pérdida de lo pendiente); `kill -9` real de un escritor (`process_kill.rs`, hecho en la Fase 1); dispositivo con pérdida de escrituras (LazyFS: preparado, **registro pendiente**; `dm-log-writes`: no intentado) |
| Corrupción | bit flips y truncamientos tras cierre limpio | + daño durante uso, archivos fabricados con CRC válido, daño exhaustivo de los vectores de directorio (hecho en la Fase 1) |
| Fuzzing | — | `cargo fuzz` sobre lectores de formato, WAL/manifest y parser documental (Fase 6) |
| Compatibilidad | — | vectores de prueba por versión de formato (hechos en la Fase 1: 194 de estructura y 29 de directorio, con guardia de regeneración), migraciones con interrupción simulada |
| Rendimiento | `spike-bench` (ADR-0012) | CI de rendimiento con umbrales de regresión de ADR-0012 |

## 5. Integración continua

[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) ejecuta en cada PR y en `main`: `cargo fmt --check`, `clippy -D warnings`, todas las suites en `release` (con un resumen de los resultados de corrupción en la página del job) y una pasada de benchmarks a escala `tiny` como prueba de humo. Una ejecución semanal programada sube las suites aleatorias a 1000 semillas y los `kill -9` a 200 iteraciones por modo. El detalle de los trabajos de la Fase 1 está en §7.5.

## 6. Gate de M0

M0 solo se cierra si todas las pruebas compartidas pasan para el candidato elegido con `SPIKE_SEEDS ≥ 400` en `release`, el barrido de todos los puntos de E/S pasa para ambos tipos de fallo y los resultados de benchmark están publicados con su entorno. Los resultados de cada ejecución se registran en el [informe de M0](m0-report.md).

## 7. Fase 1: pruebas de `kerno-storage`

Lo que se añadió al plan de M0 con el motor del producto. Los números de las puertas de salida y su resultado están en el [informe de la Fase 1](f1-report.md); el diseño de cada suite, en [phase1-design.md §6](../architecture/phase1-design.md).

### 7.1 Macros del harness y dónde se ejecutan

Cada macro genera un módulo de pruebas para cualquier motor que implemente el contrato correspondiente. Se validan **primero** sobre `LogEngine` o `Emulated<E>` (si fallan allí, la prueba está mal) y después sobre `kerno-storage`.

| Macro (módulo generado) | Contrato | Contenido | Se ejecuta en `kerno-storage` |
|---|---|---|---|
| `engine_test_suite!` (`shared_functional`, `shared_crash`, `shared_faults`, `shared_corruption`) | `Engine` | funcionales, caídas en puntos aleatorios y en cada punto de E/S, EIO aislado, bit flips y truncamientos | `shared.rs` (perfiles `KernoDb`, `inline`, `m0_profile`, `serial`) |
| `extended_failure_suite!` (`shared_ext_failure`) | `Engine` | los modelos opcionales de `SimVfs` en cada punto del escenario A | `ext_failure.rs` (hilos e `Inline`) |
| `range_delete_test_suite!` (`shared_range`) | `RangeDeleteEngine` | `[start, end)`, orden dentro del lote, instantáneas, espacio | `shared_ranges.rs` |
| `sync_test_suite!` (`shared_sync`) | `SyncEngine` | `sync()`, envenenamiento, prefijo ≥ último `sync()` | `sync.rs` |
| `group_commit_test_suite!` (`shared_concurrency`) | `Engine` | escenario C (varios escritores), visibilidad tras durabilidad | `sync.rs` |
| `txn_test_suite!` (`shared_txn`) | `TxnEngine` | transacciones read-modify-write, escrituras propias, abortar | `txn.rs` (nativo en `KernoDb` e `Inline`; `Emulated<LogEngine>` valida la suite) |

Las suites que fijan un detalle del motor (formato, recuperación, métricas) no son macros: `format.rs`, `recovery_rules.rs`, `clean_shutdown.rs`, `read_only.rs`, `manifest_log.rs`, `file_lifecycle.rs`, `coverage.rs`, `verify.rs`, `crafted.rs`, `damage.rs`, `process_lock.rs`, `compact_range.rs`, `range_delete.rs`, `group_commit.rs`, `snapshot_metrics.rs`, `compaction.rs`, `close_budget.rs`, `metrics.rs`, `wal_prealloc.rs`, `write_amp_levers.rs`, `enospc.rs`, `inline.rs`, `model.rs`, `engine.rs`, `regressions.rs` y los vectores (`struct_vectors.rs`, `dir_vectors.rs`, `spec_decoder/`).

### 7.2 Modos de `SimVfs`

Todos están desactivados por defecto (el comportamiento por defecto, incluidas las extracciones del generador aleatorio, lo fija `default_mode_crash_images_unchanged`); la [matriz de fallos](../architecture/failure-matrix.md) dice qué sobrevive en cada uno.

| Modo | Para qué |
|---|---|
| `CrashKind::{PowerLoss, ProcessKill}` | los del M0 |
| `CrashKind::ProcessKillKeepPending` | la muerte del proceso deja pendiente lo no sincronizado; un corte de energía posterior aún puede perderlo (fila KP) |
| `FsyncFailure::{KeepPending, LinuxDropPages}` | `fsync` fallido: pendiente (por defecto) o descartado para siempre, como en Linux (filas FL, FLb) |
| `DirDurability::{Global, PerDir}` | `sync_dir(d)` global o solo bajo `d` |
| `set_space_limit`, `fill_disk_after` | disco lleno (fila NS) |
| `crash_when`, `fail_when`, `start_trace` | caídas y errores filtrados por tipo, ruta y fase; trazas de E/S (`coverage.rs`, barridos del MANIFEST) |
| `dump`, `restore`, `assert_dir_unchanged`, `corrupt_byte_live` | imágenes byte a byte, comprobación de que no se mutó nada (I-18), daño durante el uso |
| `set_sync_latency` | `sync_data` duerme fuera del mutex global, para que se formen grupos |

### 7.3 Escenarios y barridos

- **A** (el del M0): creación, 25 commits, cierre, apertura, 20 commits, `Drop`, apertura, 10 commits, cierre.
- **B «formato»:** creación → 30 commits (10 % rangos) → `compact_range` → cierre → apertura → 10 transacciones → `sync` → `Drop` → apertura → checkpoint forzado → cierre. Lo recorre `coverage.rs` en modo `Inline` con traza.
- **C «grupo»:** 4 escritores × 10 commits con `set_sync_latency`.
- Cada escenario, bajo muerte de proceso, corte de energía, muerte con pendientes + corte, EIO con ambos modelos de `fsync` y ENOSPC, en `Sync` y `Buffered` donde tiene sentido; más barridos filtrados sobre las operaciones de `MANIFEST*` y la prueba dirigida `chained_crash_during_session_start_after_torn_wal` (REC-10c).
- `failure_matrix_rows_are_covered` falla si una fila de la matriz no recibe ningún punto; `threads_mode_rows_superset_of_inline` exige lo mismo con hilos de segundo plano.

### 7.4 Procesos reales y dispositivo real

| Prueba | Qué hace | Estado |
|---|---|---|
| `process_lock.rs` | un hijo mantiene la base abierta; el padre comprueba `Locked` en apertura, solo lectura, `verify()` e `inspect()`; el candado se libera al morir el hijo | en la CI |
| `process_kill.rs` | un hijo (el mismo binario de pruebas, `kerno_testkit::process`) escribe con `StdVfs` una secuencia de lotes y publica por stdout cada secuencia confirmada; el padre lo mata con SIGKILL en un momento aleatorio, ejecuta `verify()`, reabre y exige `s ≥` última confirmada y contenido igual al modelo. Cada iteración continúa desde el estado de la anterior. `KERNO_KILL_ITERS` fija las iteraciones (20 por defecto, 200 en la ejecución semanal y de salida) | en la CI |
| `lazyfs.rs` + `spikes/storage/run-lazyfs.sh` + `.github/workflows/lazyfs.yml` | escenario A sobre un montaje LazyFS (commit fijado), con una prueba de control de que el montaje pierde lo no sincronizado y `lazyfs::clear-cache` entre el escritor y el comprobador; el script devuelve 2 (**pendiente**, nunca verde) si faltan FUSE o herramientas | manual y semanal; **registro pendiente** (ver el informe de la Fase 1) |

Los dos tests de `process_kill.rs` no se ejecutan a la vez (un `fork` en un hilo copia los descriptores que el otro tiene abiertos, entre ellos el `LOCK` de un `verify()` en curso, y el candado `flock` no se libera hasta el `exec` del hijo: la reapertura del otro test veía `Locked`).

### 7.5 Puertas de salida y presupuesto de CI

| Puerta | Comando | Dónde |
|---|---|---|
| G0 | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features --release` | cada PR (`checks`, `test-spikes`, `test-kerno`) |
| FP (huella del harness) | las tres órdenes de [phase1-design.md §1](../architecture/phase1-design.md) con `SPIKE_SEEDS=400`; deben dar exactamente las cifras de `Outcomes` de ese apartado | cada paquete y la salida |
| Corrección, 1000 semillas | `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture` | semanal (`SPIKE_SEEDS` y `KERNO_KILL_ITERS` los fija el evento `schedule`) y salida |
| `kill -9` | `KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill` | semanal y salida |
| `loom` | `RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue` | semanal y a demanda (trabajo `loom`) |
| LazyFS | `./spikes/storage/run-lazyfs.sh` | manual y semanal (`lazyfs.yml`) |
| Benchmarks | `ENGINES=btree,lsm,kerno,redb,fjall,sqlite ./spikes/storage/run-benchmarks.sh` y `spike-bench report --baseline … --baseline-label m0` | salida de la fase y cada PR que cambie rendimiento (CONTRIBUTING) |
| Vectores | `struct_vectors_encode_exact`, `dir_vectors_are_reproducible` (normales, en la CI) | cada PR; un vector que falla no se regenera para pasar ([CONTRIBUTING](../../CONTRIBUTING.md)) |

Presupuesto de tiempo (objetivos de [phase1-design.md §6.7](../architecture/phase1-design.md)): `test-spikes` ≤ 15 min y `test-kerno` ≤ 25 min por PR. Medido en la VM de salida (4 núcleos), la suite normal de `kerno-storage` con `--all-features --release` tarda TIEMPO_SUITE; los tiempos en los ejecutores de GitHub no se han medido. Si un paquete supera el presupuesto, reduce las semillas por defecto de sus pruebas nuevas, no las de la suite compartida.
