# Matriz de fallos

> **Histórico:** las secciones 1 a 5 describen el M0 (spikes de Fase 0) y se conservan como referencia. La matriz vigente del formato LSM v1 está en la sección 6.

Estado: vigente; la sección 6 (LSM v1) queda **finalizada** al cierre de la Fase 1 (2026-10-29) · Anexo de [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) · Criterio 3 de [SAD §17.5](SAD.md#175-criterios-de-aceptación)

Para cada punto de interrupción del protocolo de cada candidato, este documento indica **qué estado debe observarse tras reabrir**. La notación es:

- `N`: último commit confirmado (`commit` devolvió `Ok(N)`).
- `N+1`: commit en curso cuando ocurrió el fallo (no confirmado).
- «`N` o `N+1`»: ambos son correctos; nunca un estado intermedio, nunca `< N`.

Tipos de fallo (modelados por [`SimVfs`](../../crates/kerno-testkit/src/vfs/sim.rs)):

| Fallo | Qué sobrevive |
|---|---|
| **Muerte del proceso** | Todo lo que el proceso escribió (la caché del SO sobrevive). |
| **Corte de energía** | Solo lo sincronizado; cada escritura pendiente se pierde, se aplica o se rompe (a 512 B o, en la mitad de las semillas y puntos, en cualquier byte), en cualquier orden; las operaciones de directorio no sincronizadas sobreviven como prefijo. |
| **EIO aislado** | Una operación falla (sin efecto; un `fsync` fallido deja sus escrituras sin garantía) y el dispositivo sigue funcionando. |

Desde la Fase 1, `SimVfs` ofrece además modelos opcionales, desactivados por defecto (el comportamiento por defecto, incluidas las extracciones del generador aleatorio, está fijado por la prueba `default_mode_crash_images_unchanged`):

| Fallo (opción de `SimVfs`) | Qué sobrevive |
|---|---|
| **Muerte del proceso sin pérdida de lo pendiente** (`CrashKind::ProcessKillKeepPending`) | Lo mismo que en la muerte del proceso, pero lo no sincronizado sigue pendiente: un corte de energía posterior aún puede perderlo. |
| **`fsync` fallido con semántica de Linux** (`FsyncFailure::LinuxDropPages`) | Las escrituras cubiertas por el `fsync` fallido se descartan para siempre aunque los lectores las sigan viendo, y el siguiente `fsync` devuelve `Ok` sin persistirlas; los cambios de longitud (`set_len`) son metadatos y siguen pendientes hasta el siguiente `fsync` correcto. Por defecto (`FsyncFailure::KeepPending`) quedan pendientes, no perdidas. |
| **Durabilidad por directorio** (`DirDurability::PerDir`) | `sync_dir(d)` solo persiste las operaciones sobre entradas de `d` (un `rename` cuenta para el directorio de destino), junto con las anteriores de las que dependen (mismo inodo o mismo nombre); un directorio nuevo solo es durable cuando se sincroniza su padre. |
| **Disco lleno** (`set_space_limit`, `fill_disk_after`) | La escritura o el `set_len` que haría crecer el espacio usado por encima del límite falla con `StorageFull` (ENOSPC) y no tiene efecto; un archivo borrado o reemplazado libera su espacio cuando se cierra su último handle, como en Linux. |

## 1. Cómo se verifica

La matriz no es solo documentación: cada fila se ejercita automáticamente.

| Prueba | Qué recorre | Resultado esperado comprobado |
|---|---|---|
| `crash::every_point` | Caída antes de **cada** operación de E/S mutante de un escenario fijo (creación, 25 commits, cierre limpio, reapertura, 20 commits, cierre sucio, recuperación, 10 commits, cierre): muerte de proceso y corte de energía en `Sync`, y muerte de proceso en `Buffered` | estado = commit `s` con `confirmados ≤ s ≤ intentados`; la base recuperada acepta un commit más, cierra, y ese estado sobrevive a un corte de energía posterior |
| `faults::single_failure_at_every_point` | Un EIO en **cada** operación de E/S del mismo escenario, seguido de reinicio o de corte de energía inmediato | error devuelto, handle envenenado, ningún commit confirmado con `fsync` fallido, recuperación correcta |
| `crash::run` | Caídas en puntos aleatorios de historias largas (5 rondas por semilla, incluidas caídas **durante** la recuperación y durante trabajo en segundo plano), en `Sync` y `Buffered`; el corte de energía en `Buffered` solo para motores que declaran prefijo consistente | igual que arriba, encadenando recuperaciones; en `Buffered` + corte de energía, cualquier prefijo desde el inicio de la ronda |
| `corruption::*` | Bit flips y truncamientos tras cierre limpio | dato correcto o error; nunca otro dato ni retroceso |
| `shared_ext_failure::*` (`extended_failure_suite!`, hoy sobre `LogEngine`) | Los modelos opcionales en **cada** operación de E/S del escenario anterior: muerte del proceso sin pérdida de lo pendiente seguida de un corte de energía inmediato (`Sync` y `Buffered`), un EIO con semántica de Linux (`Sync`; `Buffered` con reinicio), un EIO en `Buffered`, corte de energía con durabilidad por directorio (`Sync`) y disco lleno | `confirmados ≤ s ≤ intentados`; tras la muerte del proceso y el corte inmediato, exactamente el `s` recuperado (la recuperación hizo durable su estado); con disco lleno, solo errores `Io` o `Poisoned` y recuperación correcta al liberar espacio; al final, un commit más, cierre, corte de energía y comparación con el modelo |
| `process_kill.rs` (`kerno-storage`, WP14) | `kill -9` (SIGKILL) real de un proceso escritor (y, en dos pruebas más, de uno con 4 hilos escritores en group commit, con y sin `wal_preallocate`) con `StdVfs` sobre un disco real, en `Sync` y en `Buffered`, en momentos aleatorios (en pleno commit, flush o compactación, o durante el arranque y la recuperación del propio hijo); cada iteración continúa desde el estado que dejó la anterior, y se alterna un cierre limpio con un manejador abandonado | `s ≥` último commit que el hijo informó como confirmado, contenido exactamente igual al modelo tras los lotes `1..=s`, `verify()` sin incidencias antes de abrir. 20 iteraciones por modo en la CI, 200 en la ejecución semanal y de salida (`KERNO_KILL_ITERS`) |
| `lazyfs.rs` + `spikes/storage/run-lazyfs.sh` (WP14) | Escenario A sobre un montaje LazyFS con `StdVfs`; tras el escritor se descartan las páginas no sincronizadas (`lazyfs::clear-cache`), con una prueba de control que muestra que el montaje sí pierde datos sin `fsync` | `durable ≤ s ≤ último` y contenido igual al modelo, con `durable` = todo lo confirmado en `Sync` y el resultado de `sync()` en `Buffered`. **Su registro es criterio de salida y a 2026-10-29 está pendiente** (ver [informe F1](../testing/f1-report.md)) |

Los puntos concretos del escenario varían con el motor (cada uno hace un número distinto de operaciones); las filas siguientes agrupan esos puntos por fase del protocolo. Las pruebas comprueban la regla común a todas las filas (`N ≤ s ≤ N+1` respecto del commit en curso), no el valor concreto que predice cada fila: las filas explican por qué la regla se cumple en cada punto. Las filas de modo `Buffered` se ejercitan con caídas aleatorias (`crash::run`) y con el barrido de muerte de proceso en `Buffered`; no hay barrido de corte de energía en `Buffered`, porque ahí la única garantía es el prefijo (LSM) o ninguna (B+ tree).

## 2. Candidato A: copy-on-write B+ tree ([diseño](../../spikes/storage/spike-btree/DESIGN.md))

Protocolo de commit `Sync`: escribir páginas nuevas (nunca sobre páginas alcanzables desde el meta vigente o desde un snapshot vivo) → `fsync` → escribir meta en el slot que **no** contiene el meta vigente → `fsync`.

| # | Punto de fallo | Muerte del proceso | Corte de energía | Por qué |
|---|---|---|---|---|
| B1 | Antes de escribir nada (validación, aplicación en memoria) | `N` | `N` | Nada tocó el disco. |
| B2 | Durante la escritura de páginas nuevas | `N` | `N` | Las páginas van a ids libres o más allá del high-water mark; el meta de `N` no las referencia. |
| B3 | Antes o durante el primer `fsync` | `N` | `N` | Ídem; las páginas pueden estar rotas, pero nada las referencia. |
| B4 | Durante la escritura del meta | `N` o `N+1` | `N` o `N+1` | El slot escrito puede quedar roto (crc inválido → se usa el otro slot, `N`) o completo (`N+1`, cuyas páginas ya son durables). |
| B5 | Antes o durante el segundo `fsync` | `N` o `N+1` | `N` o `N+1` | Ídem. |
| B6 | Tras el segundo `fsync`, antes de devolver | `N+1` | `N+1` | Commit durable; aún no confirmado al llamador. |
| B7 | Creación: antes del `rename` de `data.kbt.tmp` | base vacía | base vacía | El temporal se borra al abrir. |
| B8 | Creación: tras el `rename`, antes del `fsync` del directorio | base vacía | base vacía (con o sin archivo) | Toda apertura sincroniza el directorio antes de aceptar commits. |
| B9 | Apertura | estado previo | estado previo | La recuperación no escribe datos: `fsync` del directorio padre y del directorio de la base, y `fsync` del archivo **antes** de confiar en el meta elegido. |
| B9b | Caída entre escribir el meta de `N+1` y sincronizarlo; reinicio; corte de energía durante el siguiente commit | — | `N` o `N+1` | El `fsync` al abrir hace durable el meta visible antes de reutilizar las páginas que liberó. Sin él (defecto de la revisión de M0) el único árbol durable podía sobrescribirse. |
| B10 | Cierre limpio (copia del meta al segundo slot) | `N` | `N` | Ambos slots válidos, con `N` o `N−1`/`N`; el más nuevo es `N`. |

**Modo `Buffered`** (sin `fsync`): ante muerte del proceso, `N` o `N+1`. Ante corte de energía **no hay garantía de prefijo**: un meta nuevo puede llegar al disco antes que sus páginas y las páginas liberadas se reutilizan antes de ser durables. Los checksums Merkle garantizan que el resultado es el estado exacto de algún commit **o** `Error::Corruption` detectado, nunca datos erróneos; en simulación, ~87 % de los cortes (130 de 150) dejaron la base sin poder abrirse hasta repararla y otro 12 % se abrió con daño detectado al leer. Ver ADR-0002.

**EIO aislado**: cualquier error tras validar envenena el handle; tras reabrir, `N` o `N+1`.

**`fsync` por commit `Sync`**: 2 (`sync_data` de páginas + `sync_data` del meta). Ningún `sync_dir` por commit.

## 3. Candidato B: LSM tree ([diseño](../../spikes/storage/spike-lsm/DESIGN.md))

Protocolo de commit `Sync`: registro `[len|crc|seq|batch]` en el WAL activo (un `write_at`) → `fsync` del WAL → inserción en memtable → publicación de la secuencia.

| # | Punto de fallo | Muerte del proceso | Corte de energía | Por qué |
|---|---|---|---|---|
| L1 | Antes de escribir el registro | `N` | `N` | |
| L2 | Durante la escritura del registro | `N` o `N+1` | `N` | Un registro roto falla su crc y es la cola del WAL más nuevo → se ignora. |
| L3 | Antes o durante el `fsync` del WAL | `N` o `N+1` | `N` o `N+1` | El registro puede haber llegado completo. |
| L4 | Tras el `fsync`, antes de publicar | `N+1` | `N+1` | Durable; se reproduce del WAL. |
| L5 | Cambio de memtable: `fsync` del WAL viejo (solo `Buffered`), creación del WAL nuevo, `fsync` del directorio | `N` | `N` | El WAL nuevo todavía está vacío; si su entrada de directorio se pierde, no contenía nada. |
| L6 | Flush: escritura/`fsync` de la SST nueva | `N` | `N` | SST huérfana (no está en el MANIFEST) → se borra al abrir; los datos se reproducen del WAL. |
| L7 | Flush: escritura de `MANIFEST.tmp`, `fsync`, `rename`, `fsync` del directorio | `N` | `N` | Sobrevive el MANIFEST viejo (+ WAL) o el nuevo (+ SST): mismo estado lógico. |
| L8 | Flush: borrado del WAL ya volcado | `N` | `N` | WAL con número `< log_number` → se borra al abrir. |
| L9 | Compactación: escritura de SST de salida | `N` | `N` | Salidas huérfanas → se borran al abrir. |
| L10 | Compactación: cambio de MANIFEST | `N` | `N` | Atómico (temporal + `rename`); entradas y salidas contienen el mismo estado lógico. |
| L11 | Compactación: borrado de entradas obsoletas | `N` | `N` | Solo se borran cuando un MANIFEST durable ya no las referencia y ningún snapshot las usa. |
| L12 | Recuperación: `fsync` del directorio padre, reproducción del WAL → SST L0 → MANIFEST con `log_number` nuevo → borrado de WAL viejos → WAL nuevo | estado previo | estado previo | Hasta que el MANIFEST nuevo es durable, el viejo y sus WAL siguen vigentes; después, la SST contiene lo reproducido. La recuperación puede interrumpirse y repetirse. |
| L13 | Cierre limpio: flush de memtables, MANIFEST, borrado de WAL, compactaciones hasta que ningún nivel exceda su objetivo | `N` | `N` | Combinación de L6–L11. |

**Modo `Buffered`**: ante muerte del proceso, `N` o `N+1`. Ante corte de energía, **un prefijo consistente** de commits no anterior al estado que informó la última apertura: el WAL viejo se sincroniza antes de crear uno nuevo (solo el más reciente puede tener huecos y la reproducción se detiene en el primero), y SST + MANIFEST son durables antes de borrar el WAL que reemplazan.

**EIO aislado**: cualquier error en el commit o en un hilo de segundo plano envenena el handle; si el `rename` del MANIFEST tuvo éxito pero su `fsync` de directorio falló, no se borra nada (la siguiente recuperación limpia lo que el MANIFEST superviviente no referencia).

**`fsync` por commit `Sync`**: 1 (`sync_data` del WAL). Amortizados: 1 `sync_dir` por cambio de memtable; 4 por flush en segundo plano (SST, directorio, MANIFEST, directorio).

## 4. Daño del medio tras cierre limpio

| Daño | B+ tree | LSM |
|---|---|---|
| Bit flip en un slot de meta | El otro slot tiene el mismo `N` → correcto | — |
| Bit flip en MANIFEST | — | `Corruption` al abrir |
| Bit flip en página/bloque de datos referenciado | `Corruption` al leer (crc en el padre) | `Corruption` al leer (crc del bloque) |
| Bit flip en índice / bloom / footer | — (no existen) | `Corruption` al abrir (crc verificado al cargar) |
| Bit flip en página libre o bytes no usados | inocuo | — (no hay bytes no cubiertos) |
| Truncamiento de cualquier archivo (incluso a 0 B) | `Corruption` al abrir (longitud < `page_count × 4096`; archivo sin meta válido nunca se reinicializa) | `Corruption` al abrir (tamaño contrastado con el MANIFEST; MANIFEST vacío o ausente con datos presentes) |

**Daño tras un cierre sucio.** B+ tree: cada slot guarda dos copias del meta en sectores distintos; dañar una copia del meta más reciente no cambia nada (antes de la revisión de M0, un solo bit hacía volver al commit anterior en silencio). Solo si se dañan **ambas** copias vuelve al commit anterior. LSM: el daño de un registro del WAL seguido de un registro válido escrito en modo `Sync` se informa como `Corruption` en vez de truncarse (antes de la revisión, se perdían en silencio commits confirmados); el daño del **último** registro sigue siendo indistinguible de una escritura rota del commit en curso.

## 5. Fuera del modelo y límites de la evidencia

- Daño del medio **durante** el uso o combinado con caídas en `kerno-storage`: cubierto desde WP09 por `damage_during_use_is_detected_or_harmless` y `damage_after_unclean_crash_never_returns_wrong_data` (error o datos exactos; tras una caída, nunca una secuencia menor que la confirmada salvo que el daño esté en la última trama del WAL más nuevo, indistinguible de una cola rota) y por `verify_detects_every_bit_flip`. Los motores M0 no lo aplican.
- Pruebas con procesos y dispositivos reales: el bloqueo entre procesos lo prueba `process_lock.rs` (WP09) y la muerte de un escritor real, `process_kill.rs` (WP14). Una muerte de proceso **no** pierde la caché del SO, así que esas pruebas demuestran el prefijo confirmado y la recuperación del WAL, no la durabilidad ante un corte de energía: esa solo la demuestran `SimVfs` (modelo) y, cuando se registre, LazyFS. **El registro de LazyFS está pendiente** a 2026-10-29 (`/dev/fuse` existía en el entorno de WP14, pero LazyFS no pudo construirse allí); `dm-log-writes` no se ha intentado.
- Fallos durante migraciones de formato (no existen todavía).

## 6. LSM v1 (`kerno-storage`)

Matriz del motor de Fase 1 ([diseño](phase1-design.md), §6.3). Las filas se asignan a los puntos de E/S por `(fase, tipo de archivo, operación)` en `crates/kerno-storage/tests/coverage.rs`, que ejecuta el escenario B en modo `Inline` y falla si una fila no recibe ningún punto. Las filas con prima (L5′, L7′, L10′, L12′, L13′) sustituyen a las del M0 por el protocolo del log de ediciones del MANIFEST.

| Fila | Punto de fallo | Muerte del proceso | Corte de energía | Por qué |
|---|---|---|---|---|
| L1–L4 | commit: antes de escribir el registro, durante su escritura, antes o durante el `fsync` del WAL, tras el `fsync` | `N` o `N+1` | `N` o `N+1` (`N` si el registro se rompió o no llegó) | Un registro roto falla su crc y es la cola del WAL más nuevo → se ignora; tras el `fsync` es durable y se reproduce. |
| L5′ | rotación: creación del WAL nuevo (o `rename` de `WAL.spare`) y escritura de su cabecera, `sync_dir`; con `wal_preallocate`, también la preparación de `WAL.spare` desde cero (fase `wal_spare`) | `N` | `N` | La cabecera se sincroniza (`sync_data`, W-1) antes de aceptar la primera trama: un WAL sin tramas es huérfano y se ignora; el WAL viejo sigue vigente. Como ninguna trama precede a una cabecera durable, una cabecera rota con una trama válida detrás es `Corruption` aunque su `synced_len` sea 0 (WAL-R6; vectores `wal-one-frame-broken-header` y `dirty-wal-one-frame`). Un `WAL.spare` a medias no es un nombre canónico: nunca se reproduce y se borra al abrir. |
| L6 | flush: escritura y `fsync` de la SST (con `bytes_per_writeback`, también las peticiones `start_writeback` entre trozos) | `N` | `N` | SST huérfana (ningún registro `FLUSH` la referencia) → se borra al abrir; los datos se reproducen del WAL. Una petición de volcado (`sync_file_range` en Linux) no garantiza nada: lo que cubre sigue pendiente hasta el `fsync`. Solo `start_writeback` es consultivo (su error queda registrado para el `fsync` final y se ignora); el error de `wait_writeback` **no** lo es, porque en Linux consume el error de escritura diferida y el `fsync` final devolvería `Ok`: se propaga y envenena el handle (solo `Unsupported`/`InvalidInput` se toleran). `SimVfs` con `FsyncFailure::LinuxDropPages` descarta para siempre las escrituras que cubre un `WaitWriteback` fallido (`write_amp_levers::a_failed_wait_for_writeback_never_loses_acknowledged_commits`). |
| L7′ | flush: registro `FLUSH` en el MANIFEST | `N` | `N` | Una cola rota del MANIFEST se acepta tras una caída; si el registro es completo, la SST y `log_number` nuevo están vigentes: mismo estado lógico. |
| L8 | flush: borrado del WAL ya volcado o, con `wal_preallocate`, su reciclado (`rename` a `WAL.spare`, ceros, `sync_data`, `sync_dir`) | `N` | `N` | WAL con número `< log_number` o `WAL.spare` → se borran al abrir; el reciclado solo empieza cuando el MANIFEST ya no necesita ese WAL. |
| L9 | compactación: escritura de las SST de salida (también peticiones `start_writeback`; con `dynamic_base_level` la salida puede ir a L2 con L1 vacío, y el registro `COMPACTION` lleva ese nivel) | `N` | `N` | Salidas huérfanas → se borran al abrir. |
| L10′ | compactación: registro `COMPACTION` | `N` | `N` | Un único registro atómico; entradas y salidas contienen el mismo estado lógico. |
| L11 | compactación: borrado diferido de entradas obsoletas | `N` | `N` | Solo se borran cuando el MANIFEST ya no las referencia y ningún snapshot las usa; las no borradas son huérfanas y se borran al abrir. |
| L12′ | arranque de sesión (S-2..S-5): reproducción, flush de recuperación, `SESSION_OPEN`, borrado de WAL viejos, WAL nuevo | estado previo | estado previo | Hasta que el registro nuevo es durable, el estado previo y sus WAL siguen vigentes; la recuperación es reentrante (REC-10c). |
| L13′ | cierre limpio (W-9): flush, compactación obligatoria y con presupuesto, checkpoint `CLEAN`, borrado de WAL | `N` | `N` | Combinación de L6–L11 y CK. |
| CK | checkpoint: `MANIFEST.tmp`, `fsync`, `rename`, `sync_dir` | `N` | `N` | Sobrevive el MANIFEST viejo o el nuevo, ambos con el mismo estado lógico. |
| CR | creación (DIR-3) | base vacía | base vacía (con o sin archivos) | Toda apertura sincroniza el directorio antes de aceptar commits; un directorio a medio crear se completa o se rechaza sin reinicializar datos. |
| R0 | reapertura tras `CLEAN`: registro `SESSION_OPEN` | `N` | `N` | Un `SESSION_OPEN` roto es una cola rota del MANIFEST; el estado `CLEAN` anterior sigue siendo el de `N`. |
| RT | flush o compactación que escribe SST con fragmentos de rango (también SST sin puntos), y recuperación con rangos en el WAL | `N` | `N` | Las mismas reglas que L6, L7′ y L9–L11: una SST con fragmentos es huérfana hasta que un registro del MANIFEST la referencia, y un registro es atómico. Los rangos están en el WAL (forma canónica, WAL-C1) hasta el `FLUSH`: tras cualquier caída vuelven a la memtable y ocultan lo que ocultaban. Una compactación que descarta puntos cubiertos o fragmentos sin nada debajo lo hace en las salidas, que son huérfanas hasta su registro `COMPACTION`. |
| DC | `compact_range`: las compactaciones dirigidas (fase `compact_range`: SST de salida, registros `COMPACTION` con motivo dirigido, borrado de entradas) y el checkpoint que le sigue (`checkpoint`, fila CK) | `N` | `N` | Cada paso es una compactación o un checkpoint ordinario: el estado lógico no cambia entre un paso y el siguiente, así que una caída en medio deja una base correcta, solo menos compactada. Lo que se retiene (instantáneas, medio, memoria) no es una garantía de borrado: `crates/kerno-storage/DESIGN.md`. |
| RO | apertura de solo lectura, `verify()` e `inspect()` | `N` | `N` | **Cero** operaciones mutantes (I-18); no hay punto de fallo propio. Pruebas: `read_only_open_writes_nothing_*`, `verify_is_read_only` (también sobre bases dañadas), `inspect_reports_versions_and_bits`. |
| FL | `fsync` fallido con semántica de Linux | error; sin commit confirmado | `N` | El handle queda envenenado; las escrituras descartadas nunca se confirman. |
| NS | disco lleno (ENOSPC) | error `Io` o `Poisoned` | `N` | Sin efecto sobre lo confirmado; la base se recupera al liberar espacio. |
| KP | muerte del proceso que deja pendiente lo no sincronizado, seguida de corte de energía | `N` o `N+1` | `N` o `N+1` | La recuperación hace durable su estado antes de aceptar commits. |

Las filas de grupos y `sync()` (G1–G3, S1–S2, FLb) están en la subsección siguiente (WP12b). RT y DC (rangos, `compact_range`) los implementó WP10b. `failure_matrix_rows_are_covered` cubre las filas de WP08 y `range_rows_are_covered` las de rangos (DC se deriva de la fase `compact_range` de la traza; RT se registra al observar una SST con fragmentos durante el escenario); los barridos de caídas son `range_delete_crash_at_every_point` y `compact_range_crash_at_every_point`.

**SC (subcompactaciones).** Las construye WP05 (`max_subcompactions`, y `close_subcompactions` para el cierre) y **no añaden un punto de fallo propio**: una compactación dividida en subintervalos escribe sus salidas como SST huérfanas y las instala con **un único** registro `COMPACTION` (`subcompactions_install_single_manifest_write`), así que se rigen por las filas L9–L11. No hay fila SC en `coverage.rs`: la cubren los barridos de las filas L9–L11, que usan `Background::Inline` (una subcompactación), y la prueba anterior, que fija la única edición. **El cierre por defecto** (`Background::Threads` con `close_subcompactions: 8`) **no tiene un barrido de caídas propio**: solo lo cubren la prueba funcional `close_compaction_uses_close_subcompactions` (sin inyectar caídas) y la propiedad de instalación de arriba; es una limitación conocida de la configuración por defecto, no de una opción rara. El riesgo es acotado: las salidas son huérfanas hasta un único registro `COMPACTION`, y `compaction::apply` rechaza salidas solapadas con `Corruption` sin instalar nada.

**Estado final de las filas.** Todas las filas de este apartado tienen una prueba que las ejercita, salvo las ya indicadas: SC (arriba) y los dispositivos con pérdida de páginas (LazyFS, pendiente de registro). La suite se ejecutó con 1000 semillas en la salida de la Fase 1 sin fallos ([informe F1](../testing/f1-report.md)).

### Presupuesto de `fsync` (LSM v1)

| Operación | `fsync` |
|---|---|
| Commit en `Sync` | 1 (< 1 con concurrencia, por group commit) |
| Rotación de WAL | 1 `sync_dir` (+1 `sync_data` del WAL viejo en `Buffered`) |
| Flush | 3 |
| Compactación | salidas + directorio + 1 |
| Checkpoint | 2 + `rename` |
| Reapertura limpia | 2 |

### Group commit y `sync()`

WP12b aporta las filas de grupo (G1–G3), de `sync()` (S1–S2) y de `fsync` fallido en `Buffered` (FLb). Notación: `N` = último commit confirmado antes del grupo o de la llamada; `N+k` = el grupo de `k` commits en curso, **completo**.

Protocolo de commit (`Sync`, un grupo): el líder toma `writer` y el grupo FIFO → `make_room` una vez → asigna secuencias consecutivas → **una trama** (`count = k`, un CRC) en **un** `write_at` → `sync_data` del WAL → aplica a la memtable en orden → publica `last_seq` y `durable_seq` **una vez**, tras el `fsync` → completa a los miembros y cede el liderazgo.

| # | Punto de fallo | Muerte del proceso | Corte de energía | Por qué |
|---|---|---|---|---|
| G1 | Con el grupo tomado, antes de escribir la trama (incluido `make_room`: cambio de memtable, creación del WAL) | `N` | `N` | Nada del grupo tocó el WAL. Los miembros no fueron confirmados. |
| G2 | Durante el `write_at` de la trama del grupo | `N` o `N+k` | `N` o `N+k` | Una trama es un grupo con un solo CRC: una escritura rota falla el CRC y es la cola rota del WAL más nuevo (se ignora entera). **Nunca hay un grupo a medias**; en `Sync` el `synced_len` de la trama (WAL-R5) distingue la cola rota del daño. |
| G3 | Antes o durante el `fsync` del grupo, o tras él y antes de publicar | `N` o `N+k` | `N` o `N+k` | La trama puede haber llegado entera. Si el `fsync` falla, **todos** los miembros reciben `Io`, los encolados detrás del grupo reciben `Poisoned` sin que se escriba nada más y el handle queda envenenado; tras reabrir, el grupo está entero o ausente. Los lectores no vieron ninguna secuencia del grupo (publicación después del `fsync`, I-16). |
| S1 | `sync()` con tramas sin sincronizar (`Buffered`): antes o durante su `fsync` | prefijo ≥ `N` | prefijo ≥ último `sync()` correcto | `sync()` encola una petición sin lote; el líder hace `sync_data` solo si hay tramas sin sincronizar y devuelve la secuencia duradera. Hasta que vuelve, solo vale lo anterior. Con la base en `Sync`, o sin tramas pendientes, `sync()` no hace E/S. |
| S2 | `sync()` cuyo `fsync` falla | `N` (todo lo confirmado) | prefijo ≥ último `sync()` correcto | `sync()` devuelve `Io` y envenena; `commit` y `sync` posteriores devuelven `Poisoned` sin E/S. |
| FLb | `Buffered` tras un `fsync` fallido con semántica de Linux (`FsyncFailure::LinuxDropPages`) | todo lo confirmado | prefijo ≥ último `sync()` correcto | Linux descarta las páginas del `fsync` fallido y el siguiente `fsync` devuelve `Ok` sin persistirlas: los commits `Buffered` posteriores al último `sync()` correcto pueden perderse aunque se confirmaran. El motor no reintenta (CONTRIBUTING): envenena, y el estado recuperado sigue siendo un prefijo consistente no anterior a ese `sync()`. |

**Presupuesto de `fsync` de los grupos** (WP12b): un commit `Sync` cuesta **1 `sync_data` por grupo** del WAL (exactamente 1 con un escritor; menos de 1 por commit con escritores concurrentes: `fsyncs_per_commit_below_half_with_8_writers`); `sync()` cuesta 0 en `Sync` y 0 o 1 en `Buffered` (1 solo si hay tramas sin sincronizar). La rotación, el flush y la compactación no cambian respecto del M0.

**Cómo se verifica.**

| Prueba | Filas |
|---|---|
| `shared_concurrency::scenario_c_*` (escenario C: 4 escritores × 10 commits con `set_sync_latency`; muerte de proceso, corte de energía, muerte con pendientes y barridos EIO con ambos modelos de `fsync`, en `Sync` y `Buffered`), sobre `KernoDb`, `Profiled<Inline>` y `Profiled<Serial>` | G1–G3 |
| `shared_concurrency::multi_writer_power_loss_prefix_no_holes`, `no_reader_sees_undurable_data` | G2, G3 (I-01, I-16) |
| `shared_sync::*` (`sync_makes_buffered_commits_durable`, `sync_failure_poisons`, `sync_on_poisoned_returns_poisoned`, `power_loss_at_every_io_point_buffered_with_sync`, `scenario_c_*_with_sync`) | S1, S2, FLb (I-17) |
| `group_commit.rs::failed_group_fsync_fails_all_members_and_poisons`, `queued_requests_get_poisoned_after_failure`, `leader_panic_poisons_and_wakes_followers`, `group_frame_torn_by_power_loss_recovers_prefix`, `sync_mode_readers_never_see_undurable_seq` | G2, G3 |
| `loom_leader_handoff`, `loom_poison_fanout`, `loom_txn_gate_drain`, `loom_backpressure` (`--cfg kerno_loom`) | el protocolo de la cola |

Los vectores de directorio `dirty-group-frame` y `dirty-buffered-synced-len` pasan a WP11 (que genera los vectores de directorio).
