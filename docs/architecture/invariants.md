# Invariantes y riesgos (Fase 0)

Estado: borrador para revisión · Relacionado con: [SAD §3, §17, §18](SAD.md), [ADR-0002](../adr/0002-transacciones-y-durabilidad.md), [matriz de fallos](failure-matrix.md)

Este documento lista las propiedades que **ninguna funcionalidad posterior puede romper** y cómo se verifican hoy. Una invariante sin prueba automatizada se marca como *pendiente*: no basta con que aparezca en la documentación (SAD §18.2).

Los identificadores son estables: los ADR, las pruebas y los PR deben citarlos (`I-03`, `R-S2`).

## 1. Invariantes del motor de almacenamiento

Las pruebas citadas viven en [`crates/kerno-testkit/src/harness/`](../../crates/kerno-testkit/src/harness) y `engine_test_suite!` genera la misma batería para cada candidato. Solo cambia lo que cada motor declara de sí mismo: `buffered_power_loss_prefix` (si se exige un prefijo consistente en `Buffered` ante corte de energía; el B+ tree no lo declara y no ejecuta esa variante) y `detects_truncation`. Las pruebas de regresión de cada motor (`tests/regressions.rs`) son propias. La columna «M0» indica el estado al cerrar el spike.

| ID | Invariante | Verificación automatizada | M0 |
|---|---|---|---|
| I-01 | **Atomicidad.** Un `WriteBatch` se aplica completo o no se aplica, también tras un fallo en cualquier punto de E/S. Un batch rechazado por validación no deja efectos. | `crash::run` (todas las variantes), `crash::every_point`, `functional::size_limits`, `functional::concurrent_readers` | verificada |
| I-02 | **Durabilidad `Sync`.** Si `commit` devolvió `Ok` en modo `Sync`, el batch sobrevive a un corte de energía. | `crash::run(PowerLoss, Sync)`, `crash::every_point(PowerLoss)` (exigen `s ≥ último confirmado`; tras recuperar, `every_point` confirma un commit más, cierra y vuelve a cortar la energía para comprobar que lo recuperado es durable) | verificada |
| I-03 | **Durabilidad `Buffered`.** Sin `fsync` por commit: sobrevive a la muerte del proceso. Ante corte de energía, el resultado es un prefijo consistente de commits *o* una limitación documentada por el motor. | `crash::run(ProcessKill, Buffered)`, `crash::every_point(ProcessKill, Buffered)`; `crash::run(PowerLoss, Buffered)` solo para motores que declaran `buffered_power_loss_prefix = true` | verificada (ver ADR-0002) |
| I-04 | **Secuencia de commit.** Cada commit no vacío recibe `seq + 1`; la secuencia sobrevive a reaperturas y caídas; el estado recuperado es exactamente el del commit `seq`. | `functional::commit_sequence`, todas las pruebas de fallo (comparan estado con el modelo en `seq`) | verificada (candidatos y `LogEngine`) |
| I-05 | **Aislamiento por snapshot.** Un snapshot observa exactamente el estado de su `seq` mientras viva, a pesar de commits, reutilización de páginas, flushes o compactaciones posteriores. | `functional::snapshot_isolation_under_churn`, `functional::put_get_overwrite_delete` | verificada |
| I-06 | **Un escritor, lectores concurrentes.** Los commits se serializan; los lectores nunca ven un batch a medias y no bloquean al escritor más allá de una sección crítica corta. | `functional::concurrent_readers` (invariante de suma constante con 4 lectores) | verificada |
| I-07 | **Apertura exclusiva.** Abrir un directorio ya abierto (mismo u otro proceso) falla con `Error::Locked`. | `functional::exclusive_open` (SimVfs), `functional::real_filesystem_smoke` (lock real del SO), `process_lock::second_process_gets_locked`, `process_lock::lock_released_when_holder_killed` (`kerno-storage`: dos procesos, también con `verify()` e `inspect()`) | verificada |
| I-08 | **Integridad.** El daño detectable produce `Error::Corruption` o error de E/S; nunca se devuelven datos distintos de los confirmados. | `corruption::bit_flips`, `corruption::truncations` (todo error debe ser `Corruption`, `Io` o `Poisoned`; cualquier otro tipo, un pánico o un dato distinto del confirmado hace fallar la prueba) | verificada |
| I-09 | **Sin retroceso silencioso.** Tras un cierre limpio, ningún daño de un solo punto puede hacer que la base «vuelva» a un commit anterior o se reinicialice vacía sin error. | `corruption::*` (comparan `seq`), creación atómica de archivos | verificada |
| I-10 | **Fallo de E/S con el dispositivo vivo** (p. ej. `fsync` con EIO). El commit afectado devuelve error y nunca se confirma; el handle queda envenenado (`Error::Poisoned`) hasta reabrir; la recuperación posterior —en el mismo dispositivo o tras un corte de energía inmediato— cumple I-01/I-02. | `faults::single_failure_at_every_point` (un único EIO en **cada** operación de E/S del escenario; si un commit devuelve error, el siguiente commit en ese handle debe devolver `Error::Poisoned`); `shared_ext_failure::linux_fsync_drop_faults_sync` y `linux_fsync_drop_faults_buffered_restart` repiten el barrido con la semántica de Linux (`FsyncFailure::LinuxDropPages`: las páginas de un `fsync` fallido se descartan aunque sigan visibles, y el siguiente `fsync` devuelve `Ok`), `single_io_failure_at_every_point_buffered` en modo `Buffered` y `enospc_at_every_point` con el disco lleno (hoy sobre `LogEngine`) | verificada |
| I-11 | **Recuperación reentrante.** Un fallo *durante* la recuperación deja una base que se puede recuperar de nuevo con las mismas garantías. | `crash::run` (en una de cada cinco rondas, al azar, arma una caída entre las operaciones de E/S 1 y 39 de `open`; si la recuperación hace menos operaciones, la caída no llega a ocurrir), `crash::every_point` (el escenario reabre la base, así que también cae antes de cada operación de E/S de la recuperación) | verificada |
| I-12 | **Espacio acotado.** Borrar y reescribir no hace crecer los archivos sin límite; el espacio liberado se reutiliza o compacta. | `functional::space_is_reclaimed` | verificada |
| I-13 | **Límites explícitos.** Claves ≤ 512 B y valores ≤ 1 MiB en el spike; fuera de rango → `InvalidArgument` antes de escribir. | `functional::size_limits` | verificada |
| I-14 | **Equivalencia lógica entre candidatos.** La misma secuencia de operaciones produce el mismo estado lógico y la misma secuencia en todos los motores. | `spike-bench/tests/differential.rs` (contenido y escaneos acotados en los seis motores; la secuencia, solo en los que la persisten: candidatos y `LogEngine`) | verificada |
| I-15 | **Formato versionado.** Cada archivo persistente lleva magic y versión; una versión desconocida se rechaza sin modificar el archivo. | `regressions.rs::format_version` de cada candidato (meta del B+ tree; MANIFEST del LSM, que fija la versión de todo el directorio) | parcial: las SST del LSM solo llevan una versión implícita en el magic (`KRNOSST1`) y su WAL no lleva ni magic ni versión (ADR-0003) |
| I-18 | **Sin mutación en aperturas que no escriben** *(propuesta, Fase 1)*. La apertura de solo lectura, `verify()`, `inspect()` y toda apertura rechazada no mutan el directorio (salvo crear `LOCK`). | `read_only_open_writes_nothing_*`, `verify_is_read_only`, `inspect_reports_versions_and_bits`, `*_rejected_untouched` (`kerno-storage`, WP08–WP09) | propuesta |

### 1.1 Invariantes del motor `kerno-storage` (Fase 1, propuestas)

Propuestas de [phase1-design.md §8](phase1-design.md); se promueven a la tabla anterior en WP14. Las de group commit y `sync()` las introduce WP12b.

| ID | Invariante | Verificación automatizada |
|---|---|---|
| I-16 | **Visibilidad tras durabilidad.** En `Sync`, ningún lector observa datos que un corte de energía pueda perder: el líder publica `last_seq` una vez por grupo, después del `fsync` de la trama que lo contiene (W-12). | `shared_concurrency::no_reader_sees_undurable_data`, `group_commit.rs::sync_mode_readers_never_see_undurable_seq` |
| I-17 | **`sync()`.** Tras `sync()` → `Ok(s)`, todo commit con secuencia `≤ s` sobrevive a un corte de energía (`s` es al menos la secuencia de todo commit confirmado antes de la llamada); un `fsync` fallido envenena el handle. | `shared_sync::*`, `sync.rs::sync_returns_durable_seq` |
| I-19 | **Semántica de rangos.** `delete_range(start, end)` borra `[start, end)`: dentro de un lote, un rango oculta las claves puestas antes y no las puestas después (con la misma secuencia gana el punto, SEM-3); un punto `p@s` está oculto en la instantánea `S` solo si un fragmento que lo cubre cumple `s < u ≤ S` (SEM-4), en cualquier fuente: memtables, L0, niveles profundos, archivos de solo fragmentos y archivos que una compactación cortó o redujo. `compact_range` no cambia el estado lógico de ninguna instantánea. | `shared_range::*` (en `LogEngine`, `Emulated<LogEngine>`, `KernoDb` y sus perfiles), `prop_mvcc_model_*`, `differential_range_deletes`, `range_delete.rs`, `compact_range.rs` |

Cambio en una existente: **I-01** (atomicidad) cubre también a cada miembro de un grupo: una trama es un grupo con un solo CRC, así que tras cualquier caída el grupo está entero o ausente (`group_frame_torn_by_power_loss_recovers_prefix`, escenario C).

## 2. Invariantes de capas superiores (registradas ahora, verificables más adelante)

| ID | Invariante | Origen | Se verificará en |
|---|---|---|---|
| I-20 | El historial de revisiones no depende del journal/WAL de recuperación; compactar o truncar el WAL nunca borra revisiones. | D-06, SAD §7.3 | Fase 3 |
| I-21 | Contenido, revisión e índices transaccionales de un mismo cambio se confirman en la misma frontera atómica. | SAD §6.2, §9.2 | Fases 2–3 |
| I-22 | Restaurar una revisión crea una revisión nueva; nunca reescribe ni borra revisiones posteriores. | SAD §8.3 | Fase 3 |
| I-23 | Un índice derivado expone su estado (`ready`/`building`/`stale`/`failed`) y su watermark; una consulta no declara frescura que no tiene. | SAD §9.2 | Fase 5 |
| I-24 | Un índice reconstruido produce los mismos resultados que uno nuevo sobre los mismos datos. | ADR-0007 | Fase 5 |
| I-25 | Todo embedding referencia la revisión fuente exacta y el perfil de modelo; cambiar la fuente lo marca obsoleto. | SAD §10.2–10.3 | Fase 5 |
| I-26 | El orden de commits lo define la secuencia de la base, nunca el timestamp de UUIDv7. | SAD §5.2 | Fase 2 |
| I-27 | Un identificador público no concede autorización; un token manipulado se rechaza. | SAD §5.2, §11.2 | Fase 4 |
| I-28 | Las claves de cifrado no se almacenan en el directorio de datos por defecto. | SAD §11.3 | Fase 4 |
| I-29 | La auditoría y los logs no registran contenido completo de documentos, secretos ni tokens. | SAD §11.4 | Fase 4 |
| I-30 | La exportación lógica es independiente del formato físico y puede reimportarse en un motor nuevo. | D-10, SAD §13 | Fase 4 |
| I-31 | Ninguna funcionalidad del núcleo requiere red, proveedor de IA o modelo. | SAD §17.5.7 | todas (revisión de dependencias) |
| I-32 | El plan elegido (planificado o aprendido) devuelve los mismos resultados que la ruta de referencia sobre la misma instantánea. | spec v0.2.0 §9, §15.4; [ejecución adaptativa](adaptive-execution.md) | Etapa 5 |
| I-33 | Si una precondición falla o la optimización está invalidada o desactivada, se usa un plan válido y el fallback queda registrado. | spec v0.2.0 §9; ADR-0016 | Etapa 6 |
| I-34 | Activar, desactivar, invalidar o borrar optimizaciones y observaciones no altera datos confirmados ni la secuencia de commit. | spec v0.2.0 §15.5; ADR-0016 | Etapa 6 |
| I-35 | Un cambio de esquema, índice o estadísticas relevantes invalida o revalida las optimizaciones dependientes antes de su siguiente uso. | spec v0.2.0 §9; ADR-0016 | Etapa 6 |
| I-36 | Con la adaptación desactivada (global o por consulta), ninguna optimización aprendida participa. | spec v0.2.0 §9; ADR-0016 | Etapa 6 |
| I-37 | Observaciones, métricas y explicaciones de plan no contienen literales de consulta ni contenido fuera del nivel `diagnostic`. | spec v0.2.0 §6.5; ADR-0015 | Etapa 5 |
| I-38 | Todo artefacto derivado referencia sus fuentes exactas (unidad y revisión), su método y versión; cambiar una fuente lo marca obsoleto o crea una versión nueva. | spec v0.2.0 §10, §15.7; ADR-0018 | Etapa 8 |
| I-39 | Una relación inferida nunca se presenta como declarada y conserva origen y confianza. | spec v0.2.0 §10, §12; ADR-0014, ADR-0017 | Etapa 4 (declaradas), Etapa 7 (inferidas) |

## 3. Riesgos

Los riesgos de producto están en [SAD §19](SAD.md#19-riesgos-y-mitigaciones). Aquí se añaden los riesgos propios del spike y del método de verificación.

| ID | Riesgo | Consecuencia | Mitigación |
|---|---|---|---|
| R-S1 | Los prototipos no son motores de producción; sus cifras reflejan también la calidad de una implementación de días. | Elegir por un detalle de implementación y no por la arquitectura. | Comparar cada prototipo con un motor maduro de la misma familia (redb para CoW B+ tree, fjall para LSM) y razonar sobre la diferencia; ver ADR-0012. |
| R-S2 | El modelo de fallos de `SimVfs` no coincide exactamente con un sistema de archivos real (ext4 `data=ordered`, semántica de `rename`, errores de `fsync`, caché del controlador). | Confianza excesiva en la recuperación. | Modelo deliberadamente pesimista (reordenación y escrituras rotas a 512 B o a nivel de byte). En Fase 1, `SimVfs` añade como opciones la semántica de Linux tras un `fsync` fallido (`FsyncFailure::LinuxDropPages`), la durabilidad por directorio (`DirDurability::PerDir`), el disco lleno (`set_space_limit`, `fill_disk_after`) y la muerte del proceso que deja pendiente lo no sincronizado (`CrashKind::ProcessKillKeepPending`), recorridas por `extended_failure_suite!`; quedan pendientes las pruebas con dispositivo real (p. ej. LazyFS o `dm-log-writes`) y kill -9 de procesos (el harness ya puede relanzar el binario de pruebas como proceso hijo: `kerno_testkit::process`). |
| R-S3 | La latencia de `fdatasync` de la VM del spike (p50 ~0,1 ms, p99 ~0,6 ms) no representa SSD de consumo, discos de red o portátiles. | Conclusiones sobre commits pequeños que no se trasladan. | El informe publica la latencia medida de `fdatasync`; las conclusiones se expresan en `fsync` por commit y amplificación, no solo en commits/s. Repetir en hardware objetivo antes de Fase 1. |
| R-S4 | Picos de latencia por compactación (LSM) o por crecimiento del árbol (B+ tree) que una mediana oculta. | Latencias de cola imprevistas en Cauce. | Reportar p95/p99/p999/máx y repetir ejecuciones. |
| R-S5 | CRC32C detecta corrupción accidental, no manipulación deliberada. | Falsa sensación de integridad frente a un atacante con acceso al disco. | Integridad criptográfica (AEAD) se decide en ADR-0006; el threat model lo deja explícito. |
| R-S6 | El contrato del spike usa batches sin lectura-dentro-de-transacción. | Subestimar el coste de transacciones read-modify-write. | ADR-0002 define la API transaccional; el prototipo elegido debe ampliarse antes de cerrar Fase 1. |
| R-S7 | Licencias: copiar diseño o código de proyectos de referencia. | Riesgo legal. | Los prototipos son implementaciones propias; los motores de referencia solo se usan como dependencias en el benchmark. Ver `CONTRIBUTING.md`. |
