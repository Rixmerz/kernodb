# kerno-storage: árbol LSM nivelado (motor de Kerno, Fase 1)

> **Procedencia.** Derivado de `spike-lsm@0cba925`; formato v1 en borrador, nunca para datos de usuario (SAD §20.6).
>
> Este crate nació como copia de `spikes/storage/spike-lsm` con solo renombres ([diseño de la Fase 1](../../docs/architecture/phase1-design.md) §5.1). Desde WP07 escribe y lee **solo** el formato v1 ([`docs/file-format/v1.md`](../../docs/file-format/v1.md)), todavía en borrador: todo archivo lleva el bit `DRAFT`, de modo que ninguna base creada ahora pueda leerse con el formato congelado (ADR-0003). Los archivos del prototipo M0 se rechazan con `Incompatible`. `spike-lsm` queda congelado como línea base del M0 (ADR-0012); este documento se actualiza con cada paquete de la Fase 1 que cambia el motor. Los códecs puros (sin E/S) están en `src/format/` (WP06); `wal.rs`, `sst.rs`, `block.rs` y `manifest.rs` los conectan con el `Vfs`.

Un árbol LSM nivelado al estilo de LevelDB/RocksDB que implementa directamente el contrato del harness `kerno_core::contract::Engine` (`KernoDb`, `KernoSnapshot`). Pretende ser correcto ante caídas y daños del medio y razonablemente rápido, no estar afinado. Toda la E/S de archivos pasa por `Arc<dyn Vfs>`.

## Configuración

`KernoDb::open(vfs, dir, &Config)`; `Engine::open` delega con `Config::from(&Options)`, que toma durabilidad, caché y *write buffer* de `Options` y deja el resto por defecto. La `Config` por defecto conserva el ajuste del M0.

| Campo | Defecto | Efecto |
|---|---|---|
| `access` | `ReadWrite` | `ReadOnly` todavía no está implementado: `open` devuelve `InvalidArgument` sin tocar nada |
| `create_if_missing` | `true` | con `false`, si `dir` no tiene `MANIFEST`, `open` falla antes de crear o modificar nada (ni el directorio ni `LOCK`): `Corruption` si hay archivos WAL o SST canónicos (una base dañada, igual que con `true`) y `InvalidArgument` en otro caso (no hay base) |
| `durability` | `Sync` | modo de durabilidad de todos los commits del handle |
| `cache_bytes` | 64 MiB | capacidad de la caché de bloques |
| `write_buffer_bytes` | 8 MiB | tamaño de memtable que provoca el cambio y el flush (mínimo efectivo 4 KiB); de él derivan el tamaño objetivo de las SST y el objetivo de L1 mientras `compaction` no los fije |
| `max_immutable_memtables` | 4 | los escritores esperan con este número de memtables pendientes de flush (mínimo 1; `0` es `InvalidArgument`) |
| `limits` | `Limits::default()` | límites de API de un commit, comprobados antes de escribir nada (v1.md §1.4): `max_key_len` 512 B, `max_value_len` 1 MiB, `max_batch_bytes` 64 MiB (bytes que el commit ocupa en el WAL) y `max_batch_ops` 2^20. Deben estar dentro de los máximos del formato (16 KiB, 64 MiB, 2^30 y 2^24) y ser al menos 1: si no, `open` devuelve `InvalidArgument` sin tocar nada |
| `allow_unverified_platform` | `false` | las garantías de `Durability::Sync` solo están verificadas en Linux (ext4, xfs). En otra plataforma, abrir con `Sync` devuelve `InvalidArgument("durability not verified on this platform")` salvo que esto sea `true`; `Buffered` no lo necesita |
| `test` (oculto, solo pruebas) | `TestConfig::default()` | `background`: `Threads` (hilos de segundo plano, como en el M0) o `Inline` (ver [Modo en línea](#modo-en-línea-backgroundinline)); `db_id`: identificador fijo para una base nueva (por defecto, uno aleatorio), para que los vectores y las pruebas sean reproducibles |
| `compaction` | `CompactionConfig::default()` | umbrales de L0 (`l0_trigger` 4, `l0_slowdown` 8, `l0_stop` 12), `level_multiplier` 10, frenado (`Sleep` de hasta 1 ms) y `bytes_per_sync` (`None`); `level_base_bytes` y `target_file_size` son `None` y se derivan del *write buffer* efectivo (4 × y 1 ×, como en el M0), así que una `Config` construida con `..Config::default()` y otro `write_buffer_bytes` conserva la geometría del M0; `CompactionConfig::m0(wb)` fija esos tamaños de forma explícita |

El módulo oculto `testing` define `Profile` y `Profiled<P>`: un tipo de motor distinto por perfil, que abre `KernoDb` con `Config::from(opts)` ajustada por `P::adjust`, para ejecutar las macros del harness en varias configuraciones sin tocar `Options`. Perfiles: `Inline` (`Engine::name()` = `kerno-inline`, `test.background = Inline`).

## Estructura del directorio

| Archivo        | Función |
|----------------|---------|
| `LOCK`         | `Vfs::lock` (apertura exclusiva, `Error::Locked`); sin estado, 0 bytes |
| `MANIFEST`     | forma del LSM con cabecera común de 96 bytes: una generación, con un único registro `SNAPSHOT`, reemplazada de forma atómica |
| `MANIFEST.tmp` | solo existe mientras se escribe un MANIFEST nuevo |
| `NNNNNN.wal`   | log de escritura anticipada (WAL), uno por memtable |
| `NNNNNN.sst`   | tabla ordenada (SST) |

Los números de WAL y de SST salen de un único contador creciente (`next_file`) y nunca se reutilizan. Al abrir se borran los archivos que siguen estos patrones pero no están vivos: los WAL anteriores al `log_number` del MANIFEST, las SST que este no referencia y `MANIFEST.tmp`. Solo cuentan los nombres canónicos (DIR-1: `{n:06}.wal` o `{n:06}.sst` con `1 ≤ n ≤ 2^48−1`, exactamente como se escriben); los archivos con otros nombres, como un `1.wal` suelto o un número fuera de rango, son desconocidos: no se abren, no se cuentan y no se tocan. Un archivo que ya no existe cuando la recuperación intenta borrarlo (lo borró a la vez una instantánea de un handle anterior) no es un error.

**Creación y archivos ajenos (DIR-2, DIR-3, REC-1..3).** Sin `MANIFEST`, solo se crea una base si el directorio no existe o contiene únicamente `LOCK` y/o `MANIFEST.tmp` (lo que deja una caída durante la creación). Si hay archivos WAL o SST canónicos, es `Corruption` (REC-2); si hay cualquier otro archivo (el `data.kbt` del B+ tree, el `log` del motor de referencia, un `notes.txt`), `InvalidArgument("not a Kerno database")` sin tocar nada, ni siquiera `LOCK`. La creación hace `create_dir_all` y `sync_dir` de los directorios que contienen los creados **solo al crear**, toma el `LOCK` y escribe el primer MANIFEST (generación 1, `SNAPSHOT` vacío y `CLEAN`) por `MANIFEST.tmp → sync_data → rename → sync_dir`; después sigue el arranque normal. El directorio se vuelve a listar con el `LOCK` tomado: esa es la lista con la que se recupera.

## Formatos

El formato está especificado byte a byte en [`docs/file-format/v1.md`](../../docs/file-format/v1.md); aquí solo lo que el motor hace con él.

- **Cabecera común** (96 bytes, §3) en los tres tipos de archivo: `magic`, `format_version`, bits `compat` / `ro_compat` / `incompat`, `db_id`, número de archivo (la generación en el MANIFEST) y un campo por tipo (`first_edit_seq`, `base_seq`, tamaño de bloque), con un CRC32C propio. Se valida entera (HDR-1..10) en todo archivo que se lee, antes de interpretar nada más y antes de escribir nada. El motor pone `DRAFT` en `incompat` de todo lo que escribe; un bit `incompat` desconocido, `AEAD` o `format_version > 1` es `Incompatible`; los bits `compat` desconocidos se ignoran (y un archivo reescrito no los conserva); un bit `ro_compat` desconocido da por ahora `Incompatible("read-only open not yet supported")` (WP08 lo convertirá en apertura de solo lectura).
- **WAL** (§6): trama de 32 bytes de cabecera + carga, **una trama por commit** hasta el group commit (WP12). Cada trama lleva `first_seq`, `count = 1` y `synced_len` (W-11): el prefijo del archivo que el escritor sabía duradero al escribirla. El CRC lleva como contexto el número de archivo. La carga es el commit en forma canónica (WAL-C1).
- **SST** (§7): cabecera, bloques de datos, filtro, propiedades, índice y pie de 80 bytes. Cada bloque va encapsulado como `carga ‖ ranura AEAD de 40 bytes (ceros) ‖ crc`, con el desplazamiento del bloque como contexto del CRC. Guarda una versión por clave de usuario y una secuencia por entrada. Sin compresión de prefijos ni de bloques; la búsqueda binaria dentro de un bloque usa el array de desplazamientos.
- **MANIFEST** (§8): un registro `SNAPSHOT` con `next_file`, `last_seq`, `log_number`, `num_levels` (7), `flags` (`CLEAN`), `prev_generation` y un `FileMeta` por archivo (nivel, número, tamaño, entradas, fragmentos de rango —siempre 0—, rango de secuencias, extremos de clave). L0 se lista del más nuevo al más viejo; L1 y los siguientes, ordenados y sin solapes. Significado de los campos:
  - `last_seq`: el commit más nuevo contenido en las SST.
  - `log_number`: los WAL numerados `>= log_number` contienen, en orden, los commits posteriores a `last_seq`.

Cada actualización del MANIFEST es la del M0 sobre registros v1: `write MANIFEST.tmp → sync_data → rename → sync_dir`, con la generación + 1 y el `edit_seq` siguiente (continúa entre generaciones). WP08 pasa a anexar registros al archivo vigente.

## Camino de commit

1. Se comprueba el lote contra `Config::limits` (operaciones, claves, valores y bytes; `InvalidArgument` antes de escribir nada, sin consumir secuencia ni envenenar), se canonicaliza (WAL-C1; la forma canónica es lo que se escribe **y** lo que aplica la memtable) y se toma el mutex del escritor (escritor único). Un handle envenenado devuelve `Poisoned`, un lote vacío devuelve la secuencia actual y con la secuencia 2^63−1 ya usada el commit falla con `InvalidArgument("commit sequence exhausted")`, también sin escribir ni envenenar.
2. `make_room` comprueba el *write buffer* y aplica contrapresión (ver abajo). Si la memtable supera `write_buffer_bytes`, cambia de memtable:
   - En modo `Buffered`, primero `sync_data` del WAL viejo.
   - Crea el WAL nuevo (cabecera con `base_seq = last_seq + 1`, sin `sync_data`: W-1) y después `sync_dir`.
   - Publica una `Version` nueva en la que la memtable vieja es inmutable.
3. Codifica una trama con `seq = last + 1` y el `synced_len` vigente, la escribe en el WAL con un único `write_at` y, en modo `Sync`, hace `sync_data` (que fija `synced_len` en los bytes escritos).
4. Inserta en la memtable: un `crossbeam_skiplist::SkipMap` con clave (clave de usuario ascendente, seq descendente) y lápidas. Aplica la forma canónica, así que las claves repetidas dentro de un lote ya se redujeron a la última operación.
5. Publica `last_seq` bajo el mutex de estado. Una instantánea toma `(Arc<Version>, last_seq)` bajo el mismo mutex, así que capturarla es atómico respecto de la publicación. Los lectores con una seq más antigua ignoran las entradas más nuevas de la skip list.

**fsyncs por commit `Sync`: exactamente un `sync_data` (el del WAL).** Amortizados además:

- Un `sync_dir` por cambio de memtable, es decir, cada `write_buffer_bytes`.
- En segundo plano, por flush: `sync_data` de la SST, `sync_dir` y después `sync_data` + `sync_dir` del MANIFEST.

No hay *group commit*: los escritores concurrentes pagan cada uno su propia sincronización. La cabecera de la trama (32 bytes) añade 16 bytes por commit respecto del M0.

**Contrapresión:**

- Con ≥ `l0_slowdown` archivos en L0 (8 por defecto), cada commit espera una vez hasta 1 ms a la compactación (`SlowdownPolicy::Sleep`; `Off` lo desactiva).
- Con ≥ `max_immutable_memtables` memtables inmutables (4 por defecto) o ≥ `l0_stop` archivos en L0 (12 por defecto), el commit se bloquea hasta que un hilo de segundo plano avanza.
- Los umbrales están en `Config::compaction` (`CompactionConfig`); `CompactionConfig::m0(write_buffer)`, el valor por defecto, reproduce las constantes del M0.
- Cada bloqueo es un **episodio**: se cuenta una vez al entrar en la espera, por motivo (memtables o L0), con su duración; los despertares intermedios no cuentan (el M0 contaba despertares).

## Trabajo en segundo plano

Hay dos hilos: el de **flush** (`kerno-flush`) y el de **compactación** (`kerno-compact-0`). Comparten una variable de condición con el escritor. Las ediciones del MANIFEST se serializan con un mutex propio: cada titular calcula los niveles nuevos a partir de los últimos duraderos, escribe el MANIFEST y después instala una `Version` nueva.

**Flush** (primero la memtable inmutable más vieja):

1. Escribe la versión más nueva de cada clave en una SST de L0 nueva, y después `sync_data` y `sync_dir`.
2. Escribe la generación siguiente del MANIFEST con la tabla al principio de L0, `last_seq = max(last_seq, mem.max_seq)` y `log_number = mem.wal_number + 1` (en WP08 pasará a ser el número exacto del WAL vivo más antiguo).
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

Primero se **clasifica y valida** todo, sin escribir; solo después se escribe.

0. Antes de tocar nada, `open` rechaza con `InvalidArgument` una `Config` inválida (límites fuera del formato, `max_immutable_memtables = 0`, `Sync` en una plataforma sin verificar) y el acceso `ReadOnly` (todavía no implementado). Lista el directorio. Sin `MANIFEST`: `Corruption` si hay archivos WAL o SST canónicos (REC-2), `InvalidArgument("not a Kerno database")` si hay otros archivos (REC-3), `InvalidArgument` si no hay base y `create_if_missing` es `false`; en todos estos casos no se crea ni se modifica nada. Con `create_if_missing` y un directorio vacío (o con solo `LOCK` y `MANIFEST.tmp`) se crea la base (ver Creación arriba).
1. Crear el directorio y sincronizar sus padres ocurre **solo al crear** y antes del `LOCK` (el `LOCK` vive dentro del directorio). Se sincroniza el directorio que contiene el de la base y el que contiene cada directorio que `create_dir_all` acaba de crear (antes mira cuáles faltan): sin un único journal de metadatos, la entrada de un directorio solo es duradera cuando se sincroniza el que la contiene (lo encontró el modelo de durabilidad por directorio de WP03b: con `/data/apps/db` sobre un disco vacío, un corte de energía podía borrar la base entera tras commits `Sync` confirmados). En una ruta relativa de un solo componente, ese directorio es el actual, `.`. Estas sincronizaciones son obligatorias: si una falla, la apertura falla. Los ancestros de más arriba no tienen por qué haber cambiado, y sincronizarlos puede fallar por motivos ajenos a la base (`sync_dir` abre el directorio y necesita permiso de lectura, que falta en un ancestro que solo se puede atravesar, como un `home` en modo 0711 o los datos de una app en Android; un sistema de archivos de solo lectura puede no admitir `fsync` de directorios). Pero una apertura anterior de una base nueva pudo crear parte de la cadena (`create_dir_all` crea un nivel cada vez) y fallar o morir antes de sincronizarlos. Como no se sabe cuáles, en toda base nueva los ancestros por encima del más alto sincronizado de forma obligatoria se sincronizan además sin garantía, de abajo arriba. La subida se detiene en silencio solo cuando el error muestra que el proceso no puede sincronizar ese directorio (`PermissionDenied`, `ReadOnlyFilesystem`, `Unsupported`). Cualquier otro error, como un `EIO`, hace fallar la apertura: tras un `fsync` fallido no se reintenta suponiendo que los datos llegaron al disco (ADR-0002, I-10). Después toma el bloqueo y vuelve a listar el directorio: nadie más puede cambiarlo, y esa lista es con la que se recupera.
2. Lee el MANIFEST (HDR-1..9, MAN-R*, MAN-S*): la corrupción se informa, nunca se trata como «empezar vacío»; un *magic* del M0, una versión o un bit `incompat` desconocidos son `Incompatible`. Abre cada SST referenciada (SST-O1..O7): cabecera (con `db_id` y número de archivo, HDR-10), tamaño, pie, disposición, bloques meta, índice, propiedades, filtro y todos los campos del `FileMeta` (rango de secuencias incluido, que debe caber en `last_seq`).
3. Reproduce los WAL `>= log_number` en orden de número, en flujo (lecturas de 1 MiB; el archivo no se carga entero). Cada log empieza en la secuencia siguiente al anterior (`base_seq`, WAL-R1) y las tramas llevan secuencias consecutivas:
   - Una trama que verifica pero tiene una secuencia, un `count`, un `synced_len` o una carga incorrectos es `Corruption` (WAL-R3, WAL-C4).
   - La lectura de un log se detiene en la primera trama incompleta o con CRC incorrecto en `P`. Si desde `P` todo son ceros, es relleno; si no, se buscan tramas posteriores válidas con `P < synced_len ≤ q`: si existe alguna, el daño era duradero → `Corruption`; si no, es una cola rota (WAL-R5).
   - Una cabecera rota (WAL-R6: HDR-1, HDR-3 o un *magic* desconocido) con una trama posterior que dice haber sido sincronizada es `Corruption`; si no, el log cuenta como vacío (un WAL recién creado cuya cabecera aún no era duradera).
   - Qué logs pueden terminar rotos lo decide REC-10 sobre toda la cadena: los anteriores al último con commits están íntegros (cabecera válida, sin cola rota), ese puede tener cola rota y los posteriores no tienen commits.
4. Escribe los commits reproducidos en una SST de L0 nueva (`creation_reason` 3, sincronizada, y después `sync_dir`).
5. Escribe la generación siguiente del MANIFEST, cuyo `log_number` es un número de WAL nuevo, de modo que todos los WAL existentes quedan obsoletos. Borra los WAL obsoletos y las SST huérfanas, crea el WAL nuevo (cabecera, `sync_dir`).

El orden 4-5 es el del M0: el MANIFEST antes del WAL nuevo. Su `base_seq` es así el `last_seq` que un MANIFEST ya duradero contiene, y una caída entre ambos deja un MANIFEST sin WAL vivo, que se acepta. WP08 lo cambia (WAL antes del `SESSION_OPEN`, con REC-10c); por eso no se aplica aquí ninguna regla de WAL ausente (REC-9) ni del estado limpio (REC-6..8).

Consecuencias:

- A una cola rota nunca le siguen anexados, porque el log viejo no se vuelve a escribir.
- El estado que informa `open` es duradero antes de que `open` vuelva.
- Una caída en cualquier paso deja o bien el MANIFEST viejo con todos sus WAL, o bien el MANIFEST nuevo. Ambos se recuperan al mismo estado.
- Un fallo antes del cambio de MANIFEST (un disco lleno al escribir la SST o el MANIFEST) no cambia ningún archivo que ya existiera; después, el directorio es una base consistente que no necesita WAL.
- Si los niveles superan su objetivo, la compactación se programa justo después de abrir (en modo `Inline`, se ejecuta en el siguiente cambio de memtable o en el cierre).

## Cierre limpio

1. Detiene y espera a los hilos de segundo plano. Si el handle está envenenado, devuelve `Poisoned`.
2. Hace flush de todas las memtables a L0.
3. Escribe un MANIFEST con `last_seq` igual al commit más nuevo y un `log_number` por encima de todos los WAL. Es el MANIFEST **limpio** (`CLEAN`, con `log_number = next_file`, W-9) si no hay ninguna compactación pendiente; si la hay, este es uno sin `CLEAN`.
4. Borra todos los WAL y después `sync_dir`.
5. Ejecuta compactaciones en el hilo que llama hasta que ningún nivel supere su objetivo. (Lo encontró la revisión adversarial del M0: antes, el cierre abandonaba la compactación en curso y el paso 2 añadía un archivo más a L0, así que muchas sesiones cortas hacían crecer L0 sin límite y cada reapertura empezaba con compactaciones atrasadas.) Si se ejecutó alguna, escribe después el MANIFEST `CLEAN`: la marca de limpio va siempre después de la última compactación, no antes.

Tras un cierre limpio el directorio contiene solo `LOCK`, `MANIFEST` y SST, y todos sus bytes tienen checksum. Por eso un bit invertido o un truncado producen un error en lugar de un commit más antiguo. Las reglas del estado limpio (qué archivos sobrantes delatan un cierre que no terminó) llegan con WP08; hasta entonces `CLEAN` solo se escribe.

## Cobertura de integridad

| Dato | Protección | Se comprueba |
|------|------------|--------------|
| Cabecera de cada archivo | crc propio; versión, bits, `db_id` y número de archivo | cada vez que se lee el archivo, antes de cualquier escritura |
| Trama del WAL | crc con el número de archivo como contexto; continuidad de secuencias; `synced_len` | reproducción |
| Bloque de datos | crc con (número de archivo, desplazamiento) como contexto, así que un bloque movido no verifica; estructura; claves crecientes; última clave = clave del índice; secuencias dentro del rango de la tabla | cada lectura desde disco |
| Bloques de filtro, propiedades e índice | crc con contexto; ranura AEAD a cero; análisis exacto | apertura |
| Pie | crc con el número de archivo; *magic*; coherencia de la disposición | apertura |
| SST frente a MANIFEST | tamaño, entradas, rango de claves y de secuencias, `db_id` | apertura |
| Registro del MANIFEST | crc con la generación como contexto; invariantes MAN-S1..S5 | apertura |

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
- **Apertura:** `last_open_clean` (1 si no se reprodujo ningún commit del WAL) y `last_open_replayed_bytes` (bytes de las tramas válidas reproducidas, sin las cabeceras de los logs).

## Limitaciones conocidas / qué cambiaría para producción

Los paquetes de la Fase 1 atacan varias de ellas ([diseño de la Fase 1](../../docs/architecture/phase1-design.md) §5).

- **MANIFEST.** Cada generación es una instantánea completa que se reescribe en cada flush y compactación, con coste O(archivos); WP08 lo convierte en un log de ediciones de solo anexado con checkpoints periódicos.
- **Escrituras.** No hay *group commit*, así que los escritores `Sync` concurrentes se serializan con un fsync cada uno. Tampoco se reciclan los WAL, así que cada cambio de memtable paga una creación más `sync_dir`.
- **Compactación.** Un único hilo de compactación, sin subcompactaciones ni limitación de ritmo. L0→L1 siempre toma todo L0. No hay límites de solape con el nivel abuelo en los archivos de salida ni tamaños de nivel dinámicos.
- **Formato de SST.** Sin compresión de prefijos, sin compresión, sin índice o filtro particionados y sin versionado del formato más allá de los números mágicos.
- **Memoria y archivos.** El índice y el filtro de cada tabla quedan residentes y todas las tablas quedan abiertas, así que la memoria crece con el número de archivos. La caché de bloques solo guarda bloques de datos.
- **Coste de arranque.** La recuperación siempre reescribe la cola reproducida del WAL como una SST de L0 y escribe una generación nueva del MANIFEST, aunque el cierre anterior fuera limpio. Es sencillo y hace duradero el resultado de `open`, pero cada apertura tras un cierre sucio paga un flush.
- **Lecturas.** Los recorridos fusionan con una búsqueda lineal del mínimo en lugar de un montículo, y copian claves y valores. Los `get` reservan memoria para una clave de sondeo.
- **Efectos secundarios del borrado.** El borrado de archivos ocurre en el hilo que suelta la última referencia (quizá un lector que suelta una instantánea), y sus errores se ignoran; los restos se limpian en la siguiente apertura.
- **Archivos desconocidos** en el directorio se ignoran en lugar de borrarse (y, en un directorio sin `MANIFEST`, hacen que no se cree la base).
- **Solo lectura y rangos.** Un bit `ro_compat` desconocido y las lápidas de rango (un WAL con rangos o una SST con fragmentos) se rechazan con `Incompatible` hasta WP08 y WP10; no se ignoran, porque ignorarlos devolvería claves borradas.
