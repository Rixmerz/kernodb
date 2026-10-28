# kerno-storage: árbol LSM nivelado (motor de Kerno, Fase 1)

> **Procedencia.** Derivado de `spike-lsm@0cba925`; formato de desarrollo del M0, nunca para datos de usuario (SAD §20.6).
>
> Este crate nace como copia de `spikes/storage/spike-lsm` con solo renombres ([diseño de la Fase 1](../../docs/architecture/phase1-design.md) §5.1) y la configuración `Config`. Los archivos que escribe siguen el formato de desarrollo del M0 descrito aquí, **no** el formato v1 ([`docs/file-format/v1.md`](../../docs/file-format/v1.md)): no tiene promesa de compatibilidad y ninguna base de datos de usuario puede crearse con él (ADR-0003). `spike-lsm` queda congelado como línea base del M0 (ADR-0012); este documento describe el mismo comportamiento y se actualizará con cada paquete de la Fase 1 que lo cambie.

Un árbol LSM nivelado al estilo de LevelDB/RocksDB que implementa directamente el contrato del harness `kerno_core::contract::Engine` (`KernoDb`, `KernoSnapshot`). Pretende ser correcto ante caídas y daños del medio y razonablemente rápido, no estar afinado. Toda la E/S de archivos pasa por `Arc<dyn Vfs>`.

## Configuración

`KernoDb::open(vfs, dir, &Config)`; `Engine::open` delega con `Config::from(&Options)`, que toma durabilidad, caché y *write buffer* de `Options` y deja el resto por defecto. La `Config` por defecto reproduce exactamente el M0.

| Campo | Defecto | Efecto |
|---|---|---|
| `access` | `ReadWrite` | `ReadOnly` todavía no está implementado: `open` devuelve `InvalidArgument` sin tocar nada |
| `create_if_missing` | `true` | con `false`, si `dir` no tiene `MANIFEST`, `open` falla antes de crear o modificar nada (ni el directorio ni `LOCK`): `Corruption` si hay archivos WAL o SST canónicos (una base dañada, igual que con `true`) y `InvalidArgument` en otro caso (no hay base) |
| `durability` | `Sync` | modo de durabilidad de todos los commits del handle |
| `cache_bytes` | 64 MiB | capacidad de la caché de bloques |
| `write_buffer_bytes` | 8 MiB | tamaño de memtable que provoca el cambio y el flush (mínimo efectivo 4 KiB); de él derivan el tamaño objetivo de las SST y el objetivo de L1 mientras `compaction` no los fije |
| `max_immutable_memtables` | 4 | los escritores esperan con este número de memtables pendientes de flush (mínimo 1; `0` es `InvalidArgument`) |
| `test` (oculto, solo pruebas) | `TestConfig::default()` | `background`: `Threads` (hilos de segundo plano, como en el M0) o `Inline` (ver [Modo en línea](#modo-en-línea-backgroundinline)); `db_id`: identificador fijo para bases nuevas, ignorado hasta que el motor escriba el formato v1 (el formato del M0 no tiene `db_id`) |
| `compaction` | `CompactionConfig::default()` | umbrales de L0 (`l0_trigger` 4, `l0_slowdown` 8, `l0_stop` 12), `level_multiplier` 10, frenado (`Sleep` de hasta 1 ms) y `bytes_per_sync` (`None`); `level_base_bytes` y `target_file_size` son `None` y se derivan del *write buffer* efectivo (4 × y 1 ×, como en el M0), así que una `Config` construida con `..Config::default()` y otro `write_buffer_bytes` conserva la geometría del M0; `CompactionConfig::m0(wb)` fija esos tamaños de forma explícita |

El módulo oculto `testing` define `Profile` y `Profiled<P>`: un tipo de motor distinto por perfil, que abre `KernoDb` con `Config::from(opts)` ajustada por `P::adjust`, para ejecutar las macros del harness en varias configuraciones sin tocar `Options`. Perfiles: `Inline` (`Engine::name()` = `kerno-inline`, `test.background = Inline`).

## Estructura del directorio

| Archivo        | Función |
|----------------|---------|
| `LOCK`         | `Vfs::lock` (apertura exclusiva, `Error::Locked`) |
| `MANIFEST`     | instantánea con checksum de la forma del LSM, reemplazada de forma atómica |
| `MANIFEST.tmp` | solo existe mientras se escribe un MANIFEST nuevo |
| `NNNNNN.wal`   | log de escritura anticipada (WAL), uno por memtable |
| `NNNNNN.sst`   | tabla ordenada (SST) |

Los números de WAL y de SST salen de un único contador creciente (`next_file`) y nunca se reutilizan. Al abrir se borran los archivos que siguen estos patrones pero no están vivos: los WAL anteriores al `log_number` del MANIFEST, las SST que este no referencia y `MANIFEST.tmp`. Solo cuentan los nombres canónicos (`NNNNNN.wal`, seis o más dígitos exactamente como se escriben); los archivos con otros nombres, como un `1.wal` suelto, no se tocan. Un archivo que ya no existe cuando la recuperación intenta borrarlo (lo borró a la vez una instantánea de un handle anterior) no es un error.

## Formatos

Todos los enteros son little-endian. `varint` es LEB128 sin signo, y `bytes` es una longitud `varint` seguida de los datos. Todos los checksums son crc32c.

**Registro del WAL.** `len u32 | crc u32 | seq u64 | batch`. El crc cubre `len`, `seq` y `batch`. El bit 31 de `len` (`SYNCED`) marca un registro escrito en modo `Sync`; la longitud del cuerpo son los 31 bits bajos, así que un lote que codifica 2 GiB o más se rechaza con `InvalidArgument` antes de escribir nada. El lote es `count varint` seguido, por operación, de `tag u8 (1 put / 2 delete) | key bytes | [value bytes]`. Cada registro sale en un único `write_at`.

**SST.** Guarda una versión por clave de usuario, ordenadas:

```text
data block*   entries | u32 offset per entry | count u32 | crc u32     (~4 KiB target)
              entry = key_len varint | seq varint | kind u8 | [value_len varint] | key | value
filter block  bloom bits (10 bits/key, 7 probes) | k u8 | crc u32
index block   smallest_key bytes | count varint | (last_key bytes | offset varint | len varint)* | crc u32
footer        filter_off u64 | filter_len u64 | index_off u64 | index_len u64 | entries u64 | magic u64 | crc u32
```

No hay compresión de prefijos ni de bloques. La búsqueda binaria dentro de un bloque usa el array de desplazamientos.

**MANIFEST.** `"KRNOMAN1" | body_len u32 | crc u32 | body`, donde el cuerpo es `format u32 | last_seq u64 | log_number u64 | next_file u64 | levels u32`. Por cada nivel guarda el número de archivos y, por cada archivo, `number, size, entries, smallest, largest`. L0 se lista del más nuevo al más viejo; L1 y los siguientes, ordenados y sin solapes. Significado de los campos:

- `last_seq`: el commit más nuevo contenido en las SST.
- `log_number`: los WAL numerados `>= log_number` contienen, en orden, los commits posteriores a `last_seq`.

Cada actualización del MANIFEST es `write MANIFEST.tmp → sync_data → rename → sync_dir`.

## Camino de commit

1. Se ejecuta `WriteBatch::validate` y se toma el mutex del escritor (escritor único). Un handle envenenado devuelve `Poisoned`, y un lote vacío devuelve la secuencia actual.
2. `make_room` comprueba el *write buffer* y aplica contrapresión (ver abajo). Si la memtable supera `write_buffer_bytes`, cambia de memtable:
   - En modo `Buffered`, primero `sync_data` del WAL viejo.
   - Crea el WAL nuevo y después `sync_dir`.
   - Publica una `Version` nueva en la que la memtable vieja es inmutable.
3. Codifica el registro con `seq = last + 1`, lo escribe en el WAL con `write_at` y, en modo `Sync`, hace `sync_data`.
4. Inserta en la memtable: un `crossbeam_skiplist::SkipMap` con clave (clave de usuario ascendente, seq descendente) y lápidas. Las claves repetidas dentro de un lote se reducen a la última operación.
5. Publica `last_seq` bajo el mutex de estado. Una instantánea toma `(Arc<Version>, last_seq)` bajo el mismo mutex, así que capturarla es atómico respecto de la publicación. Los lectores con una seq más antigua ignoran las entradas más nuevas de la skip list.

**fsyncs por commit `Sync`: exactamente un `sync_data` (el del WAL).** Amortizados además:

- Un `sync_dir` por cambio de memtable, es decir, cada `write_buffer_bytes`.
- En segundo plano, por flush: `sync_data` de la SST, `sync_dir` y después `sync_data` + `sync_dir` del MANIFEST.

No hay *group commit*: los escritores concurrentes pagan cada uno su propia sincronización.

**Contrapresión:**

- Con ≥ `l0_slowdown` archivos en L0 (8 por defecto), cada commit espera una vez hasta 1 ms a la compactación (`SlowdownPolicy::Sleep`; `Off` lo desactiva).
- Con ≥ `max_immutable_memtables` memtables inmutables (4 por defecto) o ≥ `l0_stop` archivos en L0 (12 por defecto), el commit se bloquea hasta que un hilo de segundo plano avanza.
- Los umbrales están en `Config::compaction` (`CompactionConfig`); `CompactionConfig::m0(write_buffer)`, el valor por defecto, reproduce las constantes del M0.
- Cada bloqueo es un **episodio**: se cuenta una vez al entrar en la espera, por motivo (memtables o L0), con su duración; los despertares intermedios no cuentan (el M0 contaba despertares).

## Trabajo en segundo plano

Hay dos hilos: el de **flush** (`kerno-flush`) y el de **compactación** (`kerno-compact-0`). Comparten una variable de condición con el escritor. Las ediciones del MANIFEST se serializan con un mutex propio: cada titular calcula los niveles nuevos a partir de los últimos duraderos, escribe el MANIFEST y después instala una `Version` nueva.

**Flush** (primero la memtable inmutable más vieja):

1. Escribe la versión más nueva de cada clave en una SST de L0 nueva, y después `sync_data` y `sync_dir`.
2. Escribe un MANIFEST con la tabla al principio de L0, `last_seq = max(last_seq, mem.max_seq)` y `log_number = mem.wal_number + 1`.
3. Instala la versión nueva y borra el WAL. El borrado es de mejor esfuerzo; la recuperación elimina los restos.

**Compactación** (nivelada, como en LevelDB):

- L0 puntúa `archivos / l0_trigger`; Ln (n = 1..5), `bytes / (level_base_bytes × level_multiplier^(n−1))`; con los valores por defecto, `archivos / 4` y `bytes / (4 × write_buffer × 10^(n−1))`. Gana la puntuación más alta ≥ 1.
- La compactación de L0 fusiona todos los archivos de L0 y los de L1 que se solapan en L1.
- La de Ln toma un archivo, elegido por turno según la clave, más los de Ln+1 que se solapan. Un archivo sin solape se baja de nivel sin reescribirlo.
- Los archivos de salida se cortan en `target_file_size` (por defecto, el *write buffer* efectivo).
- La fusión conserva solo la versión más nueva de cada clave. Las instantáneas más antiguas siguen leyendo los archivos de entrada a través de su propia `Version`.
- Una lápida se descarta cuando ningún archivo de un nivel inferior al de salida tiene un rango de claves que contenga la clave.
- Las salidas se sincronizan, después `sync_dir`, después el MANIFEST y después la instalación de la versión. Solo entonces se marcan obsoletas las entradas.
- Al apagar, la compactación en curso se abandona y sus salidas parciales se eliminan.
- Si la escritura del MANIFEST falla, no se borra nada. Un `rename` cuyo `sync_dir` falló puede sobrevivir, así que las salidas podrían estar referenciadas. La siguiente recuperación borra los archivos que el MANIFEST superviviente no referencia.

### Modo en línea (`Background::Inline`)

Configuración solo de pruebas (`Config::test.background`, perfil `testing::Inline`; [diseño de la Fase 1](../../docs/architecture/phase1-design.md) §1 punto 6 y §5.1). No se lanza ningún hilo. `make_room` no aplica ralentización ni bloqueo (no hay nadie a quien esperar): si la memtable activa está llena, el mismo commit, con el mutex del escritor tomado, la cambia (paso 2 del camino de commit), escribe en L0 la inmutable más vieja con el flush de siempre y ejecuta todas las compactaciones que elija `pick` hasta que no quede ninguna pendiente; solo entonces escribe su propio registro en el WAL nuevo. El orden de bloqueo sigue siendo `escritor → manifest → estado`, y las posiciones por turno de estas compactaciones viven en el estado del escritor. El cierre es el de siempre, también en el hilo que llama. Si uno de esos pasos falla, el propio commit devuelve el error de ese paso (`Io`, o `Corruption` si la compactación encuentra una SST dañada) y el handle queda envenenado: desde ahí, `Poisoned`.

Sin hilos, la secuencia de operaciones del sistema de archivos depende solo de las llamadas: los barridos `every_point` numeran igual en cada ejecución, las trazas de `SimVfs` se repiten operación por operación (`inline_io_trace_is_deterministic`) y la prueba de modelo `proptest` reduce y reproduce cualquier fallo.

### Ganchos de prueba (`TestHooks`) y sonda de fase

`KernoDb` implementa `kerno_core::contract::TestHooks` (también por `hooks()`, oculto):

| Gancho | `Threads` | `Inline` |
|---|---|---|
| `wait_idle` | espera a que no haya flush pendiente o en curso ni compactación pendiente o en curso (incluidos los borrados que siguen a cada una) | vuelve enseguida |
| `flush` | cambia la memtable activa si tiene commits y espera a que el hilo de flush escriba todas las inmutables | lo mismo, en el hilo que llama; no compacta |
| `compact_once` | las compactaciones corren solas: espera como `wait_idle` y devuelve `true` si terminó al menos una mientras esperaba (entre ellas, cualquiera pendiente o en curso al llamarlo; puede haber más de una) | ejecuta como mucho una compactación; `true` si había alguna pendiente |

Todos devuelven `Poisoned` si el handle está envenenado. El código del motor etiqueta su E/S con `kerno_core::probe::phase`, que `SimVfs::trace` registra con cada operación: `create` (apertura de una base nueva: sincronización de los directorios que la contienen y recuperación inicial), `session` (apertura de una base existente: recuperación), `commit`, `rotate`, `flush`, `compact`, `close`, `checkpoint` (el MANIFEST del cierre, que no necesita ningún WAL) y `delete` (borrado de WAL y SST obsoletos, también en `Table::drop`).

## Versiones, instantáneas y vida de los archivos

`Version = { memtable activa, memtables inmutables (de la más nueva a la más vieja), niveles de Arc<Table> }`. Cada cambio de memtable, flush y compactación instala una versión nueva. Una instantánea guarda un `Arc<Version>` y su seq.

- **Get:** memtable activa, después las inmutables, después L0 de la más nueva a la más vieja, después un archivo candidato por cada nivel más profundo (búsqueda binaria sobre `largest`). En cada archivo comprueba el rango de claves, el filtro bloom y después el índice.
- **Scan:** una fusión de k vías (mínimo lineal sobre un número pequeño de fuentes) que entrega la versión visible más nueva de cada clave. Se saltan las lápidas, se tratan todas las combinaciones de `Bound` y los rangos vacíos o invertidos no devuelven nada.

Una SST se borra en `Table::drop`, pero solo si la tabla está marcada como obsoleta. Una tabla se marca obsoleta solo después de que un MANIFEST duradero deje de referenciarla, así que desaparece cuando se suelta la última `Version` o instantánea que la usa. Ese `drop` puede ocurrir en cualquier hilo.

## Recuperación (en cada apertura, también la de una base nueva)

0. Antes de tocar nada, `open` rechaza con `InvalidArgument` una `Config` inválida y el acceso `ReadOnly` (todavía no implementado). Con `create_if_missing = false` y un directorio sin `MANIFEST`, solo lo lista: si hay archivos WAL o SST canónicos es `Corruption`, el mismo error que en el paso 1; si no (directorio inexistente, vacío o con otros archivos), `InvalidArgument`.
1. Sincroniza el directorio que contiene el de la base (en cada apertura, como en el M0, para que un directorio de base recién creado sea duradero). Una base nueva (sin `MANIFEST`) sincroniza además el que contiene cada directorio que `create_dir_all` acaba de crear (antes mira cuáles faltan): sin un único journal de metadatos, la entrada de un directorio solo es duradera cuando se sincroniza el que la contiene (lo encontró el modelo de durabilidad por directorio de WP03b: con `/data/apps/db` sobre un disco vacío, un corte de energía podía borrar la base entera tras commits `Sync` confirmados). En una ruta relativa de un solo componente, ese directorio es el actual, `.`. Estas sincronizaciones son obligatorias: si una falla, la apertura falla. Los ancestros de más arriba no tienen por qué haber cambiado, y sincronizarlos puede fallar por motivos ajenos a la base (`sync_dir` abre el directorio y necesita permiso de lectura, que falta en un ancestro que solo se puede atravesar, como un `home` en modo 0711 o los datos de una app en Android; un sistema de archivos de solo lectura puede no admitir `fsync` de directorios). Pero una apertura anterior de una base nueva pudo crear parte de la cadena (`create_dir_all` crea un nivel cada vez: un corte, un `ENOSPC` o un `EIO` a mitad deja ancestros nuevos sin el directorio de la base, o con él) y fallar o morir antes de sincronizarlos. Como no se sabe cuáles, en toda base nueva los ancestros por encima del más alto sincronizado de forma obligatoria se sincronizan además sin garantía, de abajo arriba. La subida se detiene en silencio solo cuando el error muestra que el proceso no puede sincronizar ese directorio (`PermissionDenied`, `ReadOnlyFilesystem`, `Unsupported`): uno que no puede leer no lo creó él, ni nada por encima. Cualquier otro error, como un `EIO`, hace fallar la apertura: tras un `fsync` fallido no se reintenta suponiendo que los datos llegaron al disco (ADR-0002, I-10); antes una reapertura se lo tragaba y confirmaba commits sobre un directorio no duradero (revisión de WP03b). La primera corrección sincronizaba todos los ancestros hasta `/`: con un ancestro de solo travesía, la base no se podía crear nunca, y en una ruta relativa saltaba `.` (revisión de WP03b). DIR-3 ([v1.md](../../docs/file-format/v1.md) §2) pide hoy solo `sync_dir(padre)`; WP07 debe conservar esta regla para rutas anidadas y relativas. Después toma el bloqueo y lista el directorio. Si el MANIFEST existe, lo lee y lo verifica; la corrupción se informa, nunca se trata como «empezar vacío». Si no existe pero hay archivos WAL o SST, es `Corruption`. Si el directorio está vacío, es una base nueva.
2. Abre cada SST referenciada. Comprueba que existe y después su tamaño, pie, índice (disposición y orden), checksum del filtro, número de entradas y rango de claves contra el MANIFEST.
3. Reproduce los WAL `>= log_number` en orden de número. Los registros deben llevar seqs consecutivas a partir de `last_seq + 1`:
   - Un registro que verifica pero tiene una seq incorrecta o un lote mal formado es `Corruption`.
   - La reproducción de un log se detiene en el primer registro incompleto o con crc incorrecto. Eso solo se acepta en el log más nuevo; en uno más viejo es `Corruption`.
   - **¿Cola rota o daño del medio?** Un log lo escribe un único handle en un único modo de durabilidad, y en modo `Sync` un registro solo se escribe después de que el anterior sea duradero. Si a un registro inválido le sigue en cualquier parte del archivo un registro `SYNCED` válido, el inválido era duradero y se ha dañado después: `Corruption`, y no se borra nada. (Lo encontró la revisión adversarial del M0: antes, ese daño se truncaba como una cola rota, perdiendo en silencio commits confirmados y borrando el log.) El daño en el *último* registro de un log sigue siendo indistinguible de un commit roto en vuelo, y en modo `Buffered` las escrituras desordenadas hacen que un registro válido después de uno inválido sea un resultado normal de un corte de energía.
4. Escribe los commits reproducidos en una SST de L0 nueva (sincronizada, y después `sync_dir`).
5. Escribe un MANIFEST cuyo `log_number` es un número de WAL nuevo, de modo que todos los WAL existentes quedan obsoletos. Borra los WAL obsoletos y las SST huérfanas, crea el WAL nuevo y después `sync_dir`.

Consecuencias:

- A una cola rota nunca le siguen anexados, porque el log viejo no se vuelve a escribir.
- El estado que informa `open` es duradero antes de que `open` vuelva.
- Una caída en cualquier paso deja o bien el MANIFEST viejo con todos sus WAL, o bien el MANIFEST nuevo. Ambos se recuperan al mismo estado.
- Si los niveles superan su objetivo, la compactación se programa justo después de abrir (en modo `Inline`, se ejecuta en el siguiente cambio de memtable o en el cierre).

## Cierre limpio

1. Detiene y espera a los hilos de segundo plano. Si el handle está envenenado, devuelve `Poisoned`.
2. Hace flush de todas las memtables a L0.
3. Escribe un MANIFEST con `last_seq` igual al commit más nuevo y un `log_number` por encima de todos los WAL.
4. Borra todos los WAL y después `sync_dir`.
5. Ejecuta compactaciones en el hilo que llama hasta que ningún nivel supere su objetivo. (Lo encontró la revisión adversarial del M0: antes, el cierre abandonaba la compactación en curso y el paso 2 añadía un archivo más a L0, así que muchas sesiones cortas hacían crecer L0 sin límite y cada reapertura empezaba con compactaciones atrasadas.)

Tras un cierre limpio el directorio contiene solo `LOCK`, `MANIFEST` y SST, y todos sus bytes tienen checksum. Por eso un bit invertido o un truncado producen un error en lugar de un commit más antiguo.

## Cobertura de integridad

| Dato | Protección | Se comprueba |
|------|------------|--------------|
| Registro del WAL | crc sobre len, seq y lote; continuidad de seq | reproducción |
| Bloque de datos | crc; estructura (entradas contiguas, claves crecientes); última clave = clave del índice | cada lectura desde disco |
| Filtro | crc + comprobación de parámetros (un filtro dañado ocultaría claves) | apertura |
| Índice | crc; los bloques cubren `[0, filter_off)`, claves crecientes | apertura |
| Pie | crc + número mágico; coherencia de la disposición | apertura |
| SST frente a MANIFEST | tamaño, número de entradas, clave menor y mayor | apertura |
| MANIFEST | número mágico, longitud exacta, crc, invariantes estructurales | apertura |

Cualquier discrepancia devuelve `Error::Corruption`. La corrupción encontrada por una compactación en segundo plano envenena el handle.

## Modos de durabilidad

- **Sync:** un commit es duradero cuando vuelve.
- **Buffered:** sin sincronización por commit, pero los registros se escriben con `write_at` antes de que `commit` vuelva, así que una caída del proceso no pierde nada confirmado. Ante un corte de energía, el estado recuperado es un **prefijo de commits**, nunca más antiguo que el estado que informó la última `open`. Se cumple porque:
  - el WAL viejo se sincroniza antes de crear uno nuevo, así que solo el log más nuevo puede tener huecos, y la reproducción se detiene en el primer hueco;
  - la SST y el MANIFEST son duraderos antes de borrar cualquier WAL al que sustituyen;
  - la recuperación hace duradero su resultado antes de volver.

## Errores y envenenamiento

Cualquier error de E/S en el camino de commit (anexado, sincronización, cambio de memtable) o en un hilo de segundo plano (flush, compactación, escritura del MANIFEST) envenena el handle. A partir de ahí, `commit`, `snapshot` y `close` devuelven `Poisoned` hasta que se reabre la base. Las instantáneas tomadas antes siguen funcionando sobre lo que siga siendo legible.

`Drop` activa la marca de apagado, despierta a todos los que esperan y espera a los dos hilos, que terminan al apagar, al envenenarse o en su primera E/S fallida. Un pánico en un hilo de segundo plano se captura y envenena el handle.

## Estadísticas (`Engine::stats`)

`KernoDb::metrics()` devuelve una instantánea tipada (`Metrics`, en `src/metrics.rs`); `stats()` la aplana con nombres estables y conserva todos los del M0 (`write_stalls` cuenta ahora episodios).

- **Forma:** `l{0..6}_files` / `l{0..6}_bytes`, `l0_depth` (máximo de archivos de L0 que se solapan en una clave), `l0_files_max`, `memtable_bytes`, `immutable_memtables`, `last_seq`, `block_cache_bytes`.
- **Escritura:** commits, bytes y sincronizaciones del WAL, cambios de memtable y su duración dentro del commit (`memtable_rotation_micros`, `_max`).
- **Fases del commit:** histogramas log2 (cada potencia de dos partida en cuatro: percentiles con error < 25 %) de `make_room`, `wal_write`, `wal_sync`, `apply` y el total, con `commit_<fase>_{count,micros,max_micros,p50_micros,p99_micros}`. `wal_sync` se separa además en `wal_sync_busy` (con un flush o una compactación escribiendo) y `wal_sync_quiet`. `queue` es 0 hasta que exista group commit (WP12). Cada fase cuesta un `Instant::now()` y tres sumas atómicas relajadas.
- **Bloqueos:** `stall_episodes_{memtable,l0}`, `stall_micros_{memtable,l0}`, `stall_micros_max`, `slowdown_commits`, `slowdown_micros`; los nombres del M0 `write_stalls`, `write_stall_micros` y `write_slowdowns` se derivan de ellos.
- **Segundo plano:** flushes y bytes de flush (incluidos los del cierre y de la recuperación), `flush_micros_max`, compactaciones, `compaction_micros_max`, movimientos triviales, bytes leídos y escritos por la compactación, lápidas descartadas, compactaciones de L0 con sus bytes de entrada y archivos de L1 reescritos (totales y máximos), `compactions_while_l0_slowdown` (compactaciones de niveles profundos elegidas con L0 en frenado) y `pending_compaction_bytes` (bytes por encima del objetivo de L1 a L5, más todo L0 cuando alcanza `l0_trigger`).
- **Lecturas:** comprobaciones del bloom, negativos y falsos positivos; aciertos y fallos de la caché de bloques. Cada contador se reparte en 16 ranuras alineadas a 64 bytes (una por hilo, por turno) que se suman al leer, para que los lectores concurrentes no compitan por la misma línea de caché.
- **Mantenimiento:** escrituras del MANIFEST, archivos obsoletos borrados.
- **Apertura:** `last_open_clean` (1 si no se reprodujo ningún registro del WAL) y `last_open_replayed_bytes`.

## Limitaciones conocidas / qué cambiaría para producción

Los paquetes de la Fase 1 atacan varias de ellas ([diseño de la Fase 1](../../docs/architecture/phase1-design.md) §5).

- **Formato del MANIFEST.** Es una instantánea completa que se reescribe en cada flush y compactación, con coste O(archivos). En producción sería un log de ediciones de solo anexado con checkpoints periódicos. Eso necesita una marca de cierre limpio o similar para que una última edición dañada no pase por una cola rota.
- **Escrituras.** No hay *group commit*, así que los escritores `Sync` concurrentes se serializan con un fsync cada uno. Tampoco se reciclan los WAL, así que cada cambio de memtable paga una creación más `sync_dir`.
- **Compactación.** Un único hilo de compactación, sin subcompactaciones ni limitación de ritmo. L0→L1 siempre toma todo L0. No hay límites de solape con el nivel abuelo en los archivos de salida ni tamaños de nivel dinámicos.
- **Formato de SST.** Sin compresión de prefijos, sin compresión, sin índice o filtro particionados y sin versionado del formato más allá de los números mágicos.
- **Memoria y archivos.** El índice y el filtro de cada tabla quedan residentes y todas las tablas quedan abiertas, así que la memoria crece con el número de archivos. La caché de bloques solo guarda bloques de datos.
- **Coste de arranque.** La recuperación siempre reescribe la cola reproducida del WAL como una SST de L0 y reescribe el MANIFEST. Es sencillo y hace duradero el resultado de `open`, pero cada apertura tras un cierre sucio paga un flush.
- **Lecturas.** Los recorridos fusionan con una búsqueda lineal del mínimo en lugar de un montículo, y copian claves y valores. Los `get` reservan memoria para una clave de sondeo.
- **Efectos secundarios del borrado.** El borrado de archivos ocurre en el hilo que suelta la última referencia (quizá un lector que suelta una instantánea), y sus errores se ignoran; los restos se limpian en la siguiente apertura.
- **Archivos desconocidos** en el directorio se ignoran en lugar de borrarse.
