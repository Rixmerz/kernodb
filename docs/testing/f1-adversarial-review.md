# Revisión adversarial de la Fase 1 (núcleo de almacenamiento persistente)

Estado: revisión de solo lectura, previa a la fusión de `claude/quirky-allen-d65xsg` en `main` (HEAD `474b509`) · Alcance: `crates/kerno-storage`, `crates/kerno-core`, [matriz de fallos](../architecture/failure-matrix.md), [informe F1](f1-report.md) · No se modificó ningún archivo salvo este documento.

## 1. Resumen: bloqueantes primero

| Id | Severidad | Dónde | Qué |
|---|---|---|---|
| **B1** | **Bloqueante** | `crates/kerno-storage/src/sst.rs:169-185` (sobre todo `:182`) | El error de `wait_writeback` se descarta con `let _ =`. En Linux ese error se **consume** y el `sync_data` final de la SST devuelve `Ok`: un fallo de escritura diferida se toma por durable (viola el invariante de ADR-0002 que `db.rs:1699` cita) y se pierden commits confirmados. Está activo **por defecto** (`config.rs:512`). |
| G1 | Mayor (brecha de prueba) | `crates/kerno-testkit/src/vfs/sim.rs:1086-1089`, `docs/architecture/failure-matrix.md:124` | El simulador modela un fallo de `WaitWriteback` como «sin efecto», que es lo contrario de Linux. Por eso ningún barrido puede detectar B1 y la fila L6 de la matriz afirma algo que solo es cierto de `start_writeback`. |
| M1 | Menor | `crates/kerno-storage/src/format/wal.rs:380-384, 449-463` | Daño en la cabecera de un WAL con una sola trama (o, en `Buffered`, sin `sync()` posterior) se toma por «cabecera nunca sincronizada» y descarta commits confirmados sin error. Está en la especificación (WAL-R6), pero no figura en las limitaciones de la matriz (§4/§5). |
| M2 | Menor | `crates/kerno-core/src/vfs/std_fs.rs:135-142`, `crates/kerno-storage/src/db.rs:1766` | `AccessMode::ReadOnly` abre `LOCK` con `write(true).create(true)`: no puede abrir una base en un medio o directorio de solo lectura. |
| G2 | Menor (brecha de prueba) | `crates/kerno-storage/src/config.rs:522`, `docs/architecture/failure-matrix.md:144` | `close_subcompactions: 8` es el valor **por defecto** en modo `Threads`, pero ningún barrido de caídas o de fallos ejecuta el cierre con subcompactaciones paralelas; la matriz lo presenta como una limitación de una opción no predeterminada. |
| G3 | Menor (brecha de prueba) | `crates/kerno-storage/tests/process_kill.rs:76-85` | El `kill -9` real solo usa un escritor secuencial: sin group commit concurrente, sin transacciones, sin `compact_range`, sin `wal_preallocate`. |
| D1 | Menor (documentación) | `docs/testing/f1-report.md:20, 108`, `docs/architecture/failure-matrix.md:41-43` | Procedencia inexacta del registro de 1000 semillas, evidencia no archivada y tabla Markdown rota. |

No se encontró ningún bloqueante en WAL, MANIFEST, group commit, orden de `fsync` de la recuperación, aislamiento de snapshots, ciclo de vida de archivos ni decodificación del formato v1 con entrada corrupta (ver §4, «Superficies revisadas sin hallazgo»). B1 es el único hallazgo que pierde datos confirmados por un camino realista y por defecto.

## 2. Método y límites

- Lectura línea a línea de `wal.rs`, `format/wal.rs`, `recovery.rs`, `manifest.rs`, `format/manifest.rs`, `commit_queue.rs`, `db.rs` (commit, rotación, flush, compactación, cierre, apertura), `compaction.rs`, `flush.rs`, `sst.rs`, `deleter.rs`, `version.rs`, `rangedel.rs`, `txn.rs`, `snapshots.rs`, `format/{block,coding,header,filter,rangedel,mod}.rs`, `kerno-core/src/{vfs/std_fs.rs,batch.rs,error.rs,probe.rs}`, y de las pruebas `process_kill.rs`, `lazyfs.rs`, `damage.rs`, `common/batch_stream.rs`.
- Se contrastaron las afirmaciones de `failure-matrix.md` y de `f1-report.md` con el código, con el registro `spikes/storage/results/2026-10-29-f1/tests-seeds1000.log` y con `git diff 50583e2 474b509`.
- **No se ejecutó ningún experimento de inyección de fallos** (no hay `dm-flakey`/`dm-error` en este entorno): las clases «inferencial» de abajo se apoyan en la lectura del código y en la semántica documentada del núcleo de Linux, no en una reproducción. Cada una indica cómo reproducirla.
- Se descartó lo que no se pudo trazar hasta un camino realista.

## 3. Hallazgos

### B1 · Bloqueante · Un fallo de escritura diferida de una SST se toma por durable

**Dónde.** `crates/kerno-storage/src/sst.rs:169-185` (`Sink::write_pending`):

```rust
let _ = self.file.start_writeback(offset, len);
let _ = self.file.wait_writeback(offset, len);   // sst.rs:182
```

`wait_writeback` es `sync_file_range(WAIT_BEFORE | WAIT_AFTER)` (`kerno-core/src/vfs/std_fs.rs:49-51`) y está activo por defecto: `CompactionConfig::default()` pone `bytes_per_writeback: Some(1 << 20)` (`config.rs:512`), así que toda SST de más de 1 MiB (cada flush de 8 MiB y cada compactación) pasa por aquí.

**Por qué es un fallo.** El comentario de `sst.rs:177-179`, la documentación de `kerno-core/src/vfs/mod.rs:58-60` y la fila L6 de la matriz (`failure-matrix.md:124`) afirman que el error «no dice nada sobre los datos, que el `fsync` final informa». Eso es cierto de `start_writeback` (`SYNC_FILE_RANGE_WRITE` devuelve solo errores de envío y deja el error registrado en el mapping), pero no de `wait_writeback`: en Linux `sync_file_range` con `WAIT_*` termina en `file_fdatawait_range()`, que llama a `file_check_and_advance_wb_err()`, es decir, devuelve el error de escritura diferida y **adelanta el cursor `f_wb_err` de esa descripción de archivo abierto**. El `sync_data()` de `finish_with` (`sst.rs:270`) usa el mismo descriptor, así que ya no ve el error, y las páginas que fallaron quedan marcadas como limpias. Es el mismo problema del «segundo `fsync` devuelve `Ok`» que el proyecto modela y trata como fatal en otros sitios (`FsyncFailure::LinuxDropPages`; «un `fsync` fallido no se reintenta», `db.rs:1699-1700`).

**Escenario concreto.**

1. `write_buffer_bytes` de 8 MiB (valor por defecto), `Durability::Sync`, `Background::Threads`. Se confirman commits hasta llenar la memtable.
2. El hilo `kerno-flush` escribe la SST: cada 1 MiB hace `write_at`, `start_writeback` y `wait_writeback`. El dispositivo devuelve EIO para un rango (sector malo, `dm-thin` sin espacio, disco de red); el error queda registrado y `wait_writeback` lo devuelve y lo consume.
3. El resultado se descarta (`sst.rs:182`). `finish_with` hace `sync_data()` → `Ok`. `Table::open_file` (`sst.rs:273`) «verifica» la SST leyendo de la caché de páginas, donde las páginas fallidas siguen intactas, así que pasa SST-O1..O7.
4. `db.rs:1064` hace `sync_dir`, `log_edit` añade y sincroniza el registro `FLUSH`, y `retire_wal` (`db.rs:975-986`) borra o recicla el WAL que tenía esos commits.
5. Tras un corte de energía (o al desalojar la caché y reabrir), los bloques de la SST no están en el disco: `Table::open` o la lectura devuelven `Corruption` y los commits confirmados ya no están en ningún otro sitio. Con LSM y compactaciones el mismo camino afecta a los datos antiguos que la salida de una compactación reescribe.

Lo mismo vale para la compactación (`compaction.rs:816-820` usa el mismo `TableBuilder`). Con `Background::Inline` y las pruebas actuales no se ve porque `SimVfs` no tiene esta semántica (G1).

**Arreglo mínimo.**

1. En `sst.rs:182` propagar el error: `self.file.wait_writeback(offset, len)?`, salvo que el error indique que la plataforma no soporta la llamada (`ErrorKind::Unsupported`/`InvalidInput`, ya que `sync_file_range` puede dar `ENOSYS`/`EINVAL`); cualquier otro error envenena el handle por la ruta ya existente (`or_poison("flush failed" | "compaction failed")`). `start_writeback` puede seguir ignorándose.
2. Alternativa sin riesgo: no usar `WAIT_*` y acotar la cola con `bytes_per_sync` (que ya propaga `sync_data()?`, `sst.rs:166`).
3. Corregir los tres textos (`sst.rs:177-179`, `vfs/mod.rs:58-73`, `failure-matrix.md:124`) para que digan que solo `start_writeback` es consultivo.

**Clase de evidencia.** Inferencial (código y semántica del núcleo de Linux). **Reproducción:** una SST de varios MiB sobre `dm-flakey`/`dm-dust` con error de escritura en un rango, y comprobar que, con el código actual, `flush` termina bien y la base queda sin datos tras `echo 3 > drop_caches`/reinicio. **Causa:** introducida (WP de la palanca de escritura diferida, commit `ce15048`, activada por defecto).

### G1 · Mayor (brecha de prueba) · El simulador modela al revés el fallo que B1 explota

**Dónde.** `crates/kerno-testkit/src/vfs/sim.rs:139-141` y `:1086-1089`: `WaitWriteback` es «una operación sin efecto sobre ningún estado», incluso con un error inyectado; los comentarios de `sim.rs:134-138` dicen lo mismo de `StartWriteback`. Con `FsyncFailure::LinuxDropPages` solo `sync_data` descarta páginas (`sim.rs:1070-1076`).

**Consecuencia.** Todos los barridos de la fila L6 y de FL (`failure_matrix_rows_are_covered`, `single_failure_at_every_point`, `extended_failure_suite!`) pasan con un EIO en `WaitWriteback` porque el simulador deja las escrituras pendientes y el `sync_data` siguiente las hace durables: justo lo que Linux no hace. La matriz (`failure-matrix.md:124`) convierte esa suposición en una garantía. Tampoco LazyFS la cubriría (descarta páginas no sincronizadas, no inyecta EIO en escritura diferida).

**Arreglo mínimo.** En `SimVfs`, un error inyectado en `WaitWriteback` (y en `SyncData`) con `LinuxDropPages` debe descartar para siempre las escrituras que cubre y marcar el error como ya visto; añadir un barrido que exija que el motor envenene el handle (o que la SST no se referencie). Añadir a `spikes/storage/run-lazyfs.sh` (o a un script aparte) un escenario con `dm-flakey`.

**Clase de evidencia.** Determinista (lectura de `sim.rs:1079-1089`). **Causa:** pre-existente en el simulador, expuesta por la palanca de `ce15048`.

### M1 · Menor · Cabecera de WAL dañada con una sola trama descarta commits confirmados

**Dónde.** `format/wal.rs:449-463` (rama `header_is_broken`) y `:380-384`: la búsqueda de una trama posterior exige `p < synced_len ≤ q`, pero la primera trama de un WAL siempre guarda `synced_len = 0` (`wal.rs:119, 131`: `synced_len: 0` al crear) y, en `Buffered`, todas las tramas anteriores al primer `sync()`. La especificación lo asume (`v1.md:212`: «una trama con `synced_len = 0` bajo una cabecera rota pertenece a un commit cuyo `fsync` nunca terminó»), pero eso solo es cierto sin daño posterior: tras confirmar el commit, la cabecera y la trama son durables a la vez.

**Escenario.** Base en `Sync`, escritor muerto con `kill -9` tras el primer commit de un WAL nuevo (cabecera + una trama, ambas sincronizadas); un bit se corrompe en los 96 bytes de la cabecera (magic, `header_len` o CRC). La recuperación trata el WAL como «sin tramas con cabecera rota» (REC-10c lo admite en el WAL más reciente) y abre con `last_seq` menor, **sin error**, perdiendo el commit confirmado. Con dos o más tramas en `Sync` sí se detecta (la segunda tiene `synced_len > 0`).

**Por qué no lo detectan las pruebas.** `damage_after_unclean_crash_never_returns_wrong_data` (`tests/damage.rs:170-249`) elige el byte al azar en todo el archivo y solo tolera secuencias menores que las confirmadas en la última trama (`in_last_frame`, `:209`, `:232`); la cabecera cae antes de `from`, así que el aserto fallaría, pero un WAL de una sola trama es raro en una sesión de 220 commits y el barrido no lo busca (en el registro de 1000 semillas: «873 reported, 101 recovered (36 lost the damaged last frame)», línea 366). La matriz (`failure-matrix.md:108` y §5) documenta la ambigüedad de la **última trama** pero no la de la cabecera.

**Arreglo mínimo.** (a) Documentar la limitación en `failure-matrix.md` §4 y en el informe, y añadir un vector de directorio negativo/positivo con un WAL de una trama y cabecera dañada, para que quede fijado; o (b) cerrarla en el escritor: al crear el WAL, sincronizar la cabecera antes de aceptar la primera trama (un `sync_data` por rotación) y exigir en WAL-R6 que una cabecera rota seguida de una trama válida con CRC correcto sea `Corruption` también con `synced_len = 0` cuando la base estaba en `Sync`.

**Clase de evidencia.** Inferencial. **Causa:** pre-existente (diseño de v1, WAL-R6).

### M2 · Menor · La apertura de solo lectura necesita permiso de escritura sobre `LOCK`

**Dónde.** `std_fs.rs:135-142` (`OpenOptions::new().read(true).write(true).create(true)`), usada por `KernoDb::open` también para `AccessMode::ReadOnly` (`db.rs:1766`). `SimVfs::lock` nunca falla por permisos (`sim.rs:1264-1282`), así que `read_only.rs` no lo ve (`assert_dir_unchanged(..., &["LOCK"])`).

**Escenario.** Un directorio de copia de seguridad o un montaje de solo lectura (ADR-0009) sin `LOCK` previo, o con `LOCK` en modo 0444: `open` devuelve `Error::Io(EROFS/EACCES)` aunque la base sea legible, y con permiso de escritura crea un archivo nuevo en un directorio que el llamante pidió abrir en solo lectura (I-18 lo admite: «salvo crear `LOCK`»).

**Arreglo mínimo.** Para solo lectura, abrir `LOCK` con `read(true)` y `flock` compartido/exclusivo sin `create`; si no existe, tomar el bloqueo sobre el directorio. Añadir una prueba con `StdVfs` sobre un directorio 0555.

**Clase de evidencia.** Inferencial. **Causa:** pre-existente.

### G2 · Menor (brecha de prueba) · El cierre por defecto no tiene barrido de caídas

`config.rs:522` fija `close_subcompactions: 8` por defecto y `compaction.rs:121-125` lo aplica en `Background::Threads` (el modo por defecto); el modo `Inline`, que usan todos los barridos deterministas de caída (`coverage.rs`, `crash::every_point`), usa `max_subcompactions = 1`. La matriz (`failure-matrix.md:144`) dice que con `max_subcompactions > 1` «no se ejecuta un barrido propio; es una limitación conocida», como si fuera una opción no predeterminada. Lo que cubre el cierre por defecto es la prueba funcional `close_compaction_uses_close_subcompactions` (`tests/compaction.rs:574-615`), que no inyecta caídas, y `process_kill.rs` solo lo ejecuta en el padre, que nunca muere en el cierre.

El riesgo está acotado por el código: las salidas son huérfanas hasta un único registro `COMPACTION` y `compaction::apply` (`compaction.rs:856-877`) devuelve `Corruption` interna ante salidas solapadas en vez de instalar nada. Un defecto aquí envenenaría el cierre, no corrompería datos. **Arreglo mínimo:** un barrido de caídas con `Background::Threads` y `close_subcompactions > 1` sobre un `SimVfs` (basta con ordenar las operaciones por hilo con el punto de caída por contador global) o, como mínimo, reescribir la frase de la matriz para que diga que es la configuración por defecto del cierre. **Clase:** determinista (configuración y matriz). **Causa:** introducida por `ce15048`.

### G3 · Menor (brecha de prueba) · El `kill -9` real cubre un solo escritor

`process_kill.rs:76-85`: el hijo hace `commit` secuencial de un hilo. No hay grupos de varios escritores (G1..G3 de la matriz solo con `SimVfs`), ni `begin_write`, ni `compact_range`, ni `wal_preallocate`, y los rangos son ~2,5 % de los lotes (`common/batch_stream.rs:36-39`). El informe lo presenta como «`kill -9` real de un escritor» (correcto) y no afirma más, pero conviene no leerlo como cobertura del group commit sobre un disco real. **Arreglo mínimo:** un segundo hijo con 4 hilos escritores y reconocimientos por hilo (la comprobación `s ≥ máx(acked)` ya vale) y una variante con `wal_preallocate: true`. **Clase:** determinista. **Causa:** pre-existente en el alcance de WP14.

### D1 · Menor (documentación) · Procedencia y evidencia del informe

1. `f1-report.md:20` dice que el registro de 1000 semillas se obtuvo «sobre el commit `50583e2`», pero el propio registro ejecuta `tests/process_kill.rs` y `tests/lazyfs.rs` (líneas 564-615), que **no existen** en `50583e2` y se añaden en `474b509` (`git ls-tree 50583e2 crates/kerno-storage/tests/` no los contiene). El registro salió de un árbol de trabajo con esos archivos sin confirmar. `git diff --stat 50583e2 474b509 -- crates` confirma que lo único que cambia son pruebas y un vector, así que el resultado es creíble, pero la frase no es reproducible tal cual.
2. `f1-report.md:11` marca `fmt`/`clippy`/`cargo test --release` como «Verificados en el cierre» y §6 solo enumera los comandos: ninguna salida archivada. Lo mismo vale para los 200 `kill -9` (§3): salida pegada, sin archivo bajo `spikes/storage/results/2026-10-29-f1/`.
3. §2 presenta una sola fila `kerno` de bit flips (256/744) pero el registro tiene otra para el escenario con rangos (`shared_ranges`: 279/721, `kerno-inline` 265/735, líneas 834 y 877): no hay discrepancia, pero la tabla omite el único escenario con fragmentos de rango.
4. `failure-matrix.md:41-43`: hay una línea en blanco entre la fila de `shared_ext_failure::*` y las de `process_kill.rs`/`lazyfs.rs`, así que esas dos filas se renderizan como texto suelto y no como parte de la tabla de §1.

**Arreglo mínimo:** corregir la frase de procedencia (o regenerar el registro sobre `474b509`), archivar las salidas de §3 y §6 y quitar la línea en blanco. **Clase:** determinista.

## 4. Superficies revisadas sin hallazgo

Se anota lo que se intentó romper y por qué no se sostiene, para que el siguiente revisor no repita el trabajo.

- **Orden de `fsync` del WAL y de la recuperación.** Commit: `write_at` + `sync_data` antes de aplicar a la memtable y de publicar `last_seq` (`db.rs:703-757`). Rotación en `Buffered`: sincroniza el WAL viejo antes de crear el nuevo (`db.rs:902-908`). Recuperación: SST sincronizadas + `sync_dir` (`recovery.rs:470-474`), `sync_data` del WAL más nuevo con tramas antes de reclamar su secuencia (`:486-488`, S-2b), WAL nuevo creado antes de `SESSION_OPEN` (REC-10c), borrado solo después (`:515-533`). No se encontró una ventana de pérdida.
- **MANIFEST.** `append` es un `write_at` y un `sync_data`; `checkpoint` escribe, sincroniza, renombra y sincroniza el directorio, y ante cualquier error el escritor conserva su posición y el llamante envenena (`manifest.rs:155-212`); una cola rota fuerza checkpoint (W-7). Los `unwrap` de `format/manifest.rs:387-410` están protegidos por la máscara de etiquetas requeridas (`:371-380`).
- **Group commit.** Una trama por grupo, un CRC; el límite de grupo (`config.rs:157-158`) y el de lote (`:114`) caben en `MAX_FRAME_COMMITS`/`MAX_FRAME_PAYLOAD`, así que el motor no puede escribir una trama que él mismo rechace al reabrir. Fallo de grupo: todos reciben `Io`, los encolados `Poisoned` (`commit_queue.rs:213-235`). El orden de bloqueos `txn_gate → queue → writer → manifest → state` se respeta en todas las rutas leídas; `rotate` y `prepare_spare_if_absent` usan `try_lock` del `spare`, que es el único bloqueo que otro hilo mantiene mucho tiempo.
- **Aislamiento de snapshots.** Un snapshot es `Arc<Version>` + `last_seq` leídos bajo el mismo bloqueo (`db.rs:593-602`); la rotación solo ocurre bajo el bloqueo del escritor al empezar un grupo, cuando el anterior ya publicó, así que ninguna SST de una versión contiene secuencias mayores que la del snapshot (por eso `Table::get` y `covering_seq` pueden ignorar `seq`). Las compactaciones solo se instalan en versiones nuevas y las entradas antiguas viven mientras la versión del snapshot las referencie (`Table::drop`, `sst.rs:476-483`). El registro de snapshots solo alimenta métricas.
- **Ciclo de vida de archivos.** `mark_obsolete` se llama después de que `log_edit` volvió con `Ok` (`db.rs:1223-1226`); los números de archivo no se reutilizan (`recovery.rs:318`, `next_file` del CLEAN); una compactación o un flush fallidos dejan huérfanos que borra la apertura. `Busy` impide compactaciones con tablas o rangos de salida solapados (`compaction.rs:272-278`), que es lo que hace seguro descartar una tombstone cuando «ningún nivel inferior tiene la clave».
- **Decodificación de v1 con entrada corrupta.** `Reader`/`decode_varint` no hacen aritmética sin comprobar (`coding.rs:70-170`); `parse_data_block`, `decode_rangedel`, `decode_filter`, `decode_header`, `decode_commit` y `check_frame` validan longitudes antes de indexar y comparan con `u64` las sumas de campos acotados por límites del formato (`block.rs:165-224`, `format/wal.rs:147-282`, `filter.rs:61-84`, `header.rs:149-225`). Los `unwrap` restantes en código no de prueba son conversiones de cortes de longitud fija ya comprobada. No se halló un camino a pánico ni a datos erróneos.
- **Rangos y `max_tomb`.** El atajo de `version.rs:37` (`largest_seq < max_tomb` termina la búsqueda) depende del invariante «una tabla superficial cuyo rango cubre la clave no contiene secuencias menores que las de la clave en niveles inferiores»; se siguió el flujo de L0 más antiguo primero, solapamiento por envolvente en `overlapping` y movimientos triviales y se sostiene. Es una dependencia implícita sin aserto (conviene un `debug_assert` en `compaction::apply` y una prueba de modelo con `dynamic_base_level: true`, que no es el valor por defecto).
- **Transacciones.** El hueco de serializabilidad está cerrado: el `drain` bajo `txn_gate` espera a que el grupo anterior publique y complete (`db.rs:561-570`, `commit_queue.rs:238-243`).

## 5. Qué se afirma y qué demuestran las pruebas

| Afirmación | Qué la sostiene | Brecha |
|---|---|---|
| «Ningún dato erróneo ni retroceso» con 1000 semillas (`f1-report.md` §2) | Registro `tests-seeds1000.log`: 57 líneas `test result:` sin fallos; `kerno` con `harmless: 0` en todos los barridos. Cierto para daño **tras cierre limpio** (no hay WAL) y para el barrido de daño tras caída (`damage.rs`), cuyo único «retroceso» tolerado es la última trama. | M1 (cabecera de un WAL de una trama); no se cubre daño del medio durante un fallo de escritura diferida (B1). |
| «Corte de energía» (matriz §1, §6) | `SimVfs`, con las suposiciones de su modelo. | G1: el modelo difiere de Linux en `WaitWriteback`. LazyFS sigue pendiente y no la cubriría. |
| «`kill -9` real, 200 + 200» (`f1-report.md` §3) | `process_kill.rs`: `s ≥ acked`, contenido igual al modelo, `verify()` limpio; una muerte de proceso no pierde la caché del SO, y el informe lo dice. | G3; salida no archivada (D1). |
| Todas las filas de §6 de la matriz «tienen una prueba» | `coverage.rs` exige que cada fila reciba al menos un punto de E/S. | Solo prueba que la fase se ejecuta, no el resultado concreto de cada fila (la propia matriz lo dice, línea 45); SC y el cierre por defecto, G2. |

## 6. Orden de corrección propuesto

1. **B1** (una línea y tres textos) y **G1** (modelo del simulador y un barrido) en la misma fusión: sin G1 el arreglo de B1 no queda protegido por ninguna prueba.
2. M1: decidir entre documentar y cerrar; añadir el vector.
3. M2, G2, G3 y D1 pueden ir en una fusión posterior sin bloquear la salida de la Fase 1.

Lo que esta revisión **no** pudo comprobar: B1 y M1 sobre un dispositivo real (ver reproducciones), la ejecución de LazyFS (pendiente, ya declarada en el informe), y cualquier comportamiento en plataformas distintas de Linux/ext4.
