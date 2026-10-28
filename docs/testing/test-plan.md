# Plan de pruebas

Estado: vigente para M0; se amplía por fase · Relacionado con: [SAD §17.6, §18](../architecture/SAD.md#18-calidad-pruebas-y-definición-de-terminado), [invariantes](../architecture/invariants.md), [matriz de fallos](../architecture/failure-matrix.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md)

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
| `engine_test_suite!` | Genera la misma batería de pruebas para cada motor. |
| `extended_failure_suite!` | Genera el módulo `shared_ext_failure`: los modelos opcionales de `SimVfs` en cada operación de E/S del escenario A (ver §2.2). |
| `LogEngine` | Motor trivialmente correcto que valida el propio harness (si falla una prueba compartida, la prueba está mal). |

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
| Una base nueva no sincronizaba el padre de su directorio: con durabilidad por directorio, la base entera podía desaparecer tras un commit confirmado. Ahora sincroniza todos los ancestros (una primera versión solo sincronizaba el padre inmediato y seguía fallando con rutas anidadas como `/data/db`). | `per_dir_durability_power_loss_sync` | `new_database_directory_survives_power_loss_with_per_dir_durability` |
| En `Buffered`, `close` sincronizaba la marca de cierre limpio junto con los registros que avala: un corte de energía podía conservar la marca y perder un registro anterior, y la recuperación informaba corrupción. Ahora sincroniza primero los registros. | `single_io_failure_at_every_point_buffered` | `buffered_close_makes_records_durable_before_the_clean_marker` |

La revisión de la suite corrigió además dos defectos del propio modelo, cada uno con su autoprueba de `SimVfs`: un `fsync` fallido con semántica de Linux descartaba también los `set_len` pendientes, de modo que un truncamiento cuyo `fsync` fallaba nunca llegaba al disco (`linux_drop_pages_keeps_length_changes`; `recovery_truncation_survives_a_failed_sync` muestra el efecto sobre `LogEngine`: un registro obsoleto reaparecía tras un commit confirmado), y con durabilidad por directorio un `rename` entre directorios podía resucitar el nombre de origen (`per_dir_cross_dir_rename_does_not_resurrect_source`). También endureció el modelo de disco lleno: un archivo borrado o reemplazado conserva su espacio mientras siga abierto, como en Linux (`space_limit_freed_by_remove`).

## 3. Pruebas compartidas (SAD §17.6)

| Requisito de SAD §17.6 | Prueba |
|---|---|
| La misma secuencia de operaciones produce el mismo estado lógico en cada candidato | `spike-bench/tests/differential.rs` (prototipos, `LogEngine`, redb, fjall y SQLite) |
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
cargo test --workspace --all-features --release              # todo (lo que ejecuta la CI)

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
| Crash testing | `SimVfs` (energía y proceso, todos los puntos de E/S; un EIO aislado en cada punto) | + modelos opcionales de `SimVfs` en cada punto (`extended_failure_suite!`: semántica Linux de páginas descartadas tras `fsync` fallido, durabilidad por directorio, disco lleno, muerte del proceso sin pérdida de lo pendiente); kill -9 real de procesos y dispositivo con pérdida de escrituras (LazyFS / `dm-log-writes`) |
| Corrupción | bit flips y truncamientos tras cierre limpio | + daño durante uso, archivos fabricados |
| Fuzzing | — | `cargo fuzz` sobre lectores de formato, WAL/manifest y parser documental (Fase 6) |
| Compatibilidad | — | vectores de prueba por versión de formato, migraciones con interrupción simulada |
| Rendimiento | `spike-bench` (ADR-0012) | CI de rendimiento con umbrales de regresión de ADR-0012 |

## 5. Integración continua

[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) ejecuta en cada PR y en `main`: `cargo fmt --check`, `clippy -D warnings`, todas las suites en `release` (con un resumen de los resultados de corrupción en la página del job) y una pasada de benchmarks a escala `tiny` como prueba de humo. Una ejecución semanal programada sube las suites aleatorias a 1000 semillas.

## 6. Gate de M0

M0 solo se cierra si todas las pruebas compartidas pasan para el candidato elegido con `SPIKE_SEEDS ≥ 400` en `release`, el barrido de todos los puntos de E/S pasa para ambos tipos de fallo y los resultados de benchmark están publicados con su entorno. Los resultados de cada ejecución se registran en el [informe de M0](m0-report.md).
