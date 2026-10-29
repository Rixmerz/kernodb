# Threat model inicial (biblioteca embebida)

Estado: revisado para la Fase 1 (R-07, 2026-10-29; §6) · Relacionado con: [SAD §11](../architecture/SAD.md#11-seguridad-privacidad-y-ciclo-de-vida), ADR-0005, ADR-0006, ADR-0009, invariantes I-08, I-27…I-29

Este documento delimita **qué protege y qué no protege** Kerno en su forma inicial (biblioteca en el mismo proceso + CLI local). Debe revisarse antes de congelar el formato físico (ADR-0003; la revisión del formato v1 está en §6) y de nuevo antes de cualquier servidor de red.

## 1. Alcance y supuestos

- Kerno se ejecuta **dentro** del proceso de la aplicación anfitriona, con sus mismos privilegios. Todo código de ese proceso puede leer la memoria de Kerno, incluidos datos descifrados y claves cargadas.
- La aplicación es responsable de autenticar a sus usuarios y de autorizar cada operación de negocio (SAD §11.1). Kerno ofrece mecanismos (límites, validación, identificadores opacos, auditoría), no decide quién puede hacer qué.
- El sistema operativo y el sistema de archivos son de confianza para el aislamiento entre usuarios del SO; no lo son para la integridad de los datos en reposo (pueden corromperse, truncarse o perder escrituras no sincronizadas).

## 2. Activos

| Activo | Por qué importa |
|---|---|
| Datos confirmados (estado actual) | Pérdida o alteración silenciosa es el fallo más grave del motor. |
| Historial de revisiones y ChangeSets | Contiene versiones antiguas, posiblemente datos personales ya «borrados» del estado actual. |
| Journal/WAL, páginas liberadas, segmentos obsoletos, temporales | Contienen copias de datos que el usuario cree borrados. |
| Backups y exportaciones | Viven fuera del control del motor y con otro ciclo de vida. |
| Claves de cifrado y de tokens | Su pérdida implica pérdida de datos; su robo, exposición de todo lo cifrado. |
| Identificadores internos (UUIDv7) | Revelan instante de creación y facilitan enumeración. |
| Logs, métricas y auditoría | Pueden filtrar contenido si no se minimizan. |

## 3. Adversarios y fallos considerados

| ID | Fuente | Ejemplos | Respuesta prevista |
|---|---|---|---|
| T-01 | **Fallos no maliciosos del medio** | bit rot, sectores dañados, escrituras rotas, truncamientos, `fsync` que falla | Checksums en todo lo persistido, creación atómica de archivos, detección y error explícito (I-08, I-09); envenenamiento tras `fsync` fallido (I-10). **Verificado en M0** con `SimVfs`. |
| T-02 | **Caída del proceso o del sistema** | kill -9, OOM, corte de energía durante commit, compactación o migración | Protocolo de commit y recuperación con matriz de fallos (ADR-0002); recuperación reentrante (I-11). **Verificado en M0.** |
| T-03 | **Acceso de lectura al disco o a backups** | portátil robado, backup en la nube, disco desechado | Cifrado en reposo con AEAD y claves fuera del directorio de datos (ADR-0006, I-28). *Pendiente (Fase 4).* |
| T-04 | **Manipulación deliberada de archivos** | un atacante edita páginas y recalcula CRC32C | CRC32C **no** protege frente a esto (R-S5), tampoco el `db_id` ni el contexto del CRC (§6.1). Requiere AEAD por página/bloque (ADR-0006); el formato v1 reserva la ranura (§6.3). *Pendiente.* |
| T-05 | **Entradas hostiles vía API** | claves/valores enormes, rangos patológicos, documentos maliciosos | Límites explícitos y validación previa (I-13), límites de consulta y paginación (SAD §9.3), fuzzing (Fase 6). |
| T-06 | **Archivos de base hostiles** | abrir una base fabricada para provocar pánicos, bucles o lecturas fuera de rango | Validación de todas las longitudes, punteros y cadenas con límites; en `kerno-storage`, la suite de archivos fabricados con CRC válido (§6.2); fuzzing del formato (Fase 6). |
| T-07 | **Usuarios finales de la aplicación** | adivinar o enumerar IDs, manipular tokens | Tokens opacos y validados (ADR-0005); un token válido no autoriza (I-27); la autorización es de la aplicación. |
| T-08 | **Fugas por observabilidad** | contenido en logs, errores o auditoría | Auditoría minimizada y sin payloads (I-29); mensajes de error sin contenido de documentos. |
| T-09 | **Retención involuntaria** | revisiones, páginas liberadas, segmentos o backups que conservan datos suprimidos | Políticas de retención y supresión que abarcan derivados (ADR-0009); borrado criptográfico donde aplique. |
| T-10 | **Cadena de suministro** | dependencia comprometida | Pocas dependencias en el núcleo, `Cargo.lock` versionado, auditoría de dependencias (`cargo audit`/`cargo deny`) en Fase 6. |

## 4. Fuera de alcance (explícitamente)

- Atacantes con ejecución de código en el proceso anfitrión o con privilegios de administrador en la máquina mientras la base está abierta.
- Canales laterales (tiempos, caché de CPU) en las operaciones del motor.
- Garantizar el borrado físico en medios con wear-leveling o en copias que Kerno no controla (backups externos, snapshots del SO o de la nube); se documentan las ventanas, no se prometen.
- Cumplimiento normativo automático (SAD §11.5): los perfiles mapean controles, no certifican.

## 5. Consecuencias para M0 y siguientes fases

- La elección de almacenamiento (ADR-0001) afecta a T-03, T-04 y T-09: dónde quedan copias antiguas de los datos y qué unidad se cifra y autentica. Esto se recoge en ADR-0001 y ADR-0009.
- El formato (ADR-0003) debe reservar desde la primera versión espacio y versión para cifrado autenticado, aunque la Fase 1 no lo active.
- Ningún componente del núcleo requiere red ni proveedor externo (I-31).

## 6. Revisión de la Fase 1 (R-07)

Alcance: el formato v1 ([`v1.md`](../file-format/v1.md)) y el motor `kerno-storage` tal como quedan al cerrar la Fase 1. No cambia ninguna decisión: fija qué defiende cada mecanismo y qué no, para que el propietario pueda congelar el formato sabiendo qué promete (ADR-0013, ADR-0003).

### 6.1 `db_id` y CRC con contexto: detectan errores, no ataques

- Toda cabecera lleva un `db_id` aleatorio de 16 bytes y el número de archivo (HDR-10), y todo CRC se calcula con un contexto que el lector aporta y que no se almacena (v1.md §1.2: número de archivo, desplazamiento del bloque o generación del MANIFEST). Así una SST copiada de otra base, un archivo renombrado, un bloque movido a otro desplazamiento o un registro del MANIFEST de otra generación **no verifican** aunque cada uno sea íntegro por separado.
- Lo que esto defiende es el **error no malicioso**: restauraciones mezcladas, copias manuales, herramientas que renombran. T-01 queda cubierto.
- Lo que **no** defiende: un atacante con escritura sobre el directorio puede construir una base entera coherente (mismo `db_id`, números y CRC recalculados). El `db_id` no es un secreto ni una clave, y CRC-32C es lineal. T-04 sigue sin respuesta hasta que se active AEAD.

### 6.2 Posición ante archivos fabricados

- Todo valor leído del disco (longitudes, contadores, punteros, varints, secuencias, números de archivo) se valida con aritmética comprobada antes de usarse; los números de archivo están acotados a 2^48−1 (DIR-1) y la secuencia al agotarse devuelve `InvalidArgument`, no vuelve a 0 (I-04).
- `crates/kerno-storage/tests/crafted.rs` fabrica archivos con **CRC válido** (longitudes enormes, índices que se solapan, desorden entre bloques, `smallest`/`largest` falsos, secuencias por encima de `last_seq`, niveles solapados, varints sobrelargos…) y exige que `verify()` informe la regla rota y que el motor devuelva `Corruption` sin pánico ni asignaciones sin acotar. La memoria de la recuperación está acotada (S-2 escribe una SST de L0 cada vez que la memtable reproducida alcanza `write_buffer_bytes`) y la reproducción de un WAL es en flujo.
- Un archivo fabricado puede, como mucho, ocupar CPU y disco acotados por su tamaño. No hay garantía formal: el fuzzing con `cargo fuzz` de los lectores es trabajo de la Fase 6 y sigue pendiente. T-06 queda **mitigado, no cerrado**.

### 6.3 Reserva de AEAD

- Cada bloque de SST y cada registro del MANIFEST llevan una ranura de 40 bytes (`nonce[24] ‖ tag[16]`) justo antes del CRC; la de las tramas del WAL existe solo si la cabecera de ese WAL tiene el bit `AEAD` (v1.md §5; decisión pendiente de firma en ADR-0013).
- La Fase 1 **no cifra ni autentica**: con el bit `AEAD` desactivado la ranura debe ser cero (AEAD-1, cubierta por el CRC) y un archivo con el bit activado se rechaza con `Incompatible("encryption not supported")` antes de cualquier escritura. La reserva evita una migración del formato al activar el cifrado; no protege nada por sí sola.
- Los datos asociados (número de archivo, desplazamiento, tipo de bloque) y la gestión de claves se deciden en ADR-0006 (Fase 4). Hasta entonces T-03 y T-04 siguen **abiertos**.

### 6.4 Garantías de solo lectura

- `AccessMode::ReadOnly`, `verify()` e `inspect()` no mutan el directorio salvo la creación de `LOCK` (I-18): sin hilos, sin `create_dir_all`, sin `sync_dir`, sin borrados, sin checkpoints, y `close` no escribe. Toda apertura rechazada (`Locked`, `Incompatible`, directorio ajeno) deja el directorio idéntico byte a byte. Lo comprueban `read_only_open_writes_nothing_*`, `verify_is_read_only` y `*_rejected_untouched`.
- Un bit `ro_compat` desconocido abre la base entera en solo lectura (HDR-7), y un bit `incompat` desconocido la rechaza (HDR-6): una versión antigua nunca escribe sobre estructuras que no entiende.
- Los tres caminos toman el `LOCK` exclusivo (`flock`), también entre procesos (`process_lock.rs`): un lector externo no puede observar una base mientras un escritor la modifica, y a la inversa.
- Los mensajes de error citan archivo, desplazamiento y regla, nunca bytes de claves ni valores (T-08).

### 6.5 No hay afirmación de borrado físico

- Kerno borra los archivos obsoletos (WAL, SST compactadas) con `remove_file`, de forma diferida y con reintentos. **Eso no borra los datos del medio**: el sistema de archivos puede conservar los bloques, un SSD con wear-leveling o un snapshot del sistema pueden conservar copias, y los bloques de caché o la memoria del proceso pueden contener valores ya suprimidos.
- Una instantánea viva retiene archivos obsoletos mientras exista (`snapshot_retained_bytes` lo expone, `compact_range` informa `pinned_files`). `delete_range` y `compact_range` son **mecanismo**: hacen desaparecer las lápidas y los puntos cubiertos de los archivos vivos, sin política de retención ni garantía de supresión (ADR-0009 pendiente; `crates/kerno-storage/DESIGN.md` enumera lo que puede sobrevivir). T-09 sigue **abierto**.
- Ninguna documentación ni mensaje del producto debe describir `delete` o `compact_range` como borrado seguro.

### 6.6 Resumen del estado

| Amenaza | Estado al cerrar la Fase 1 |
|---|---|
| T-01, T-02 | Cubiertas y probadas (matriz de fallos, `kill -9` real, barridos de `SimVfs`); el registro con un dispositivo con pérdida de páginas (LazyFS) está pendiente ([informe F1](../testing/f1-report.md)) |
| T-03, T-04 | Abiertas: sin cifrado ni autenticación; ranura reservada |
| T-05 | Límites de clave, valor y lote comprobados en `kerno-storage` (`Config::limits`) |
| T-06 | Mitigada con validación y suite de archivos fabricados; fuzzing pendiente (Fase 6) |
| T-08 | Mensajes de error sin contenido |
| T-09 | Abierta: sin garantía de borrado físico |
