# Plan de pruebas

Estado: vigente para M0; se amplía por fase · Relacionado con: [SAD §17.6, §18](../architecture/SAD.md#18-calidad-pruebas-y-definición-de-terminado), [invariantes](../architecture/invariants.md), [matriz de fallos](../architecture/failure-matrix.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md)

## 1. Principio

Las garantías de Kerno solo existen si una prueba automatizada las demuestra. Cada invariante de [`invariants.md`](../architecture/invariants.md) cita la prueba que la verifica; una invariante sin prueba figura como pendiente.

## 2. Infraestructura de M0

Todo vive en [`spikes/storage/spike-core`](../../spikes/storage/spike-core):

| Pieza | Qué hace |
|---|---|
| `Engine` / `Snapshot` | Contrato común a ambos candidatos (batches atómicos, secuencia de commit, snapshots, apertura exclusiva, errores tipados). |
| `Vfs` | Todo acceso a archivos pasa por esta abstracción; los motores nunca usan `std::fs` directamente. |
| `SimVfs` | Sistema de archivos en memoria que simula **corte de energía** (escrituras no sincronizadas descartadas, reordenadas o rotas a 512 B o a nivel de byte; metadatos de directorio como prefijo journaled) y **muerte del proceso** (sobrevive todo lo escrito). Puede provocar la caída justo antes de la *n*-ésima operación de E/S mutante, o **un único error de E/S** (EIO) en esa operación con el dispositivo vivo. |
| `CountingVfs` | Cuenta bytes, operaciones y `fsync` para medir amplificación. |
| `engine_test_suite!` | Genera la misma batería de pruebas para cada motor. |
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
cargo test -p spike-core                    # harness + LogEngine
cargo test -p spike-btree                   # candidato CoW B+ tree
cargo test -p spike-lsm                     # candidato LSM
cargo test -p spike-bench --features candidates --release   # prueba diferencial
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
| Crash testing | `SimVfs` (energía y proceso, todos los puntos de E/S; un EIO aislado en cada punto) | + kill -9 real de procesos y dispositivo con pérdida de escrituras (LazyFS / `dm-log-writes`); semántica Linux de páginas descartadas tras `fsync` fallido |
| Corrupción | bit flips y truncamientos tras cierre limpio | + daño durante uso, archivos fabricados |
| Fuzzing | — | `cargo fuzz` sobre lectores de formato, WAL/manifest y parser documental (Fase 6) |
| Compatibilidad | — | vectores de prueba por versión de formato, migraciones con interrupción simulada |
| Rendimiento | `spike-bench` (ADR-0012) | CI de rendimiento con umbrales de regresión de ADR-0012 |

## 5. Integración continua

[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) ejecuta en cada PR y en `main`: `cargo fmt --check`, `clippy -D warnings`, todas las suites en `release` (con un resumen de los resultados de corrupción en la página del job) y una pasada de benchmarks a escala `tiny` como prueba de humo. Una ejecución semanal programada sube las suites aleatorias a 1000 semillas.

## 6. Gate de M0

M0 solo se cierra si todas las pruebas compartidas pasan para el candidato elegido con `SPIKE_SEEDS ≥ 400` en `release`, el barrido de todos los puntos de E/S pasa para ambos tipos de fallo y los resultados de benchmark están publicados con su entorno. Los resultados de cada ejecución se registran en el [informe de M0](m0-report.md).
