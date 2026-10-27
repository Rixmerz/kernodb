# ADR-0002 — Modelo de transacciones y durabilidad

| Campo | Valor |
|---|---|
| Estado | Propuesto (borrador con evidencia de M0; pendiente de aprobación junto con ADR-0001) |
| Fecha | 2026-10-27 |
| Relacionado con | SAD §4.2, §6.2, §7.2, §7.3; [matriz de fallos](../architecture/failure-matrix.md); invariantes I-01…I-11; ADR-0001 |

## Contexto

SAD §7.2 exige definir explícitamente:

1. qué significa que una transacción está confirmada;
2. cuándo se sincroniza y qué devuelve la API si la sincronización falla;
3. cómo se detectan escrituras parciales;
4. cómo se resuelven caídas antes, durante y después del commit;
5. cómo se recupera una actualización de formato interrumpida;
6. qué herramienta verifica o exporta una base dañada sin sobrescribirla.

El criterio de aprobación (SAD §15) es una **matriz de fallos** con el estado esperado tras reabrir en cada punto de interrupción, y la documentación de las garantías de cada modo de durabilidad.

## Opciones consideradas

1. **WAL + protocolo de commit**, con estructura principal reconstruible (LSM; también un B-tree con WAL estilo SQLite/InnoDB).
2. **Copy-on-write con raíz/meta de commit** (shadow paging estilo LMDB/redb): el commit es la escritura atómica de un meta que apunta a la nueva raíz.
3. **Variante híbrida** según el motor elegido (p. ej. CoW con «1PC + checksum», o CoW con un WAL pequeño para commits pequeños).

Ambas opciones principales se implementaron en M0 y se probaron con la misma matriz de fallos.

## Decisión propuesta

### Semántica de commit (independiente del motor)

| Concepto | Definición |
|---|---|
| **Transacción** | Un conjunto de escrituras (put/delete) sobre una base, aplicado atómicamente. Kerno tendrá un único escritor por base (D-03): la transacción de escritura ve sus propias escrituras y el último estado confirmado (serializable para escrituras); los lectores trabajan sobre snapshots (aislamiento snapshot). |
| **Secuencia de commit** | Entero de 64 bits, monotónico, asignado por la base: cada commit no vacío recibe `anterior + 1`. Se persiste, sobrevive a caídas y es el orden transaccional (nunca el timestamp de UUIDv7, I-26). Sirve de *watermark* para derivados (ADR-0007). |
| **Confirmada** | `commit` devolvió `Ok(seq)`. En modo `Sync` esto implica **durable ante corte de energía**. En modo `Buffered`, implica visible y durable ante muerte del proceso. |
| **Visibilidad** | Un commit es visible para snapshots nuevos cuando `commit` retorna, y nunca antes de cumplir la garantía de su modo de durabilidad: en `Sync`, ningún lector observa datos que una caída podría perder. |
| **Snapshot** | Vista del estado exacto tras el commit `seq`, estable mientras viva, sin bloquear al escritor. Los snapshots de larga duración retienen versiones antiguas (páginas o archivos) y se expondrán como métrica (edad del snapshot más antiguo, bytes retenidos). |
| **Commit vacío** | No crea commit; devuelve la secuencia actual. |

### Errores y resultado de un commit

| Error | Significado para el llamador |
|---|---|
| `InvalidArgument` | Rechazado antes de escribir. **No aplicado.** |
| `Locked` | La base está abierta en otro handle o proceso. |
| `Poisoned` | Un error anterior dejó el handle en estado desconocido; **no aplicado**. Hay que reabrir. |
| `Io` durante el commit | **Resultado indeterminado** hasta reabrir: tras la recuperación el commit está completo o ausente, nunca a medias. El handle queda envenenado. |
| `Corruption` | Se detectó daño; no se devuelven datos dañados. |

Tras un `fsync` fallido **no se reintenta asumiendo que los datos llegaron al disco** (I-10): el handle queda envenenado y la recuperación decide con lo que realmente está en disco.

### Modos de durabilidad

| Modo | Garantía | Coste observado en M0 |
|---|---|---|
| `Sync` (por defecto) | Confirmado ⇒ sobrevive a corte de energía. | 1 `fsync` por commit en LSM, 2 en el B+ tree CoW del prototipo. |
| `Buffered` | Confirmado ⇒ sobrevive a muerte del proceso. Ante corte de energía, **el motor elegido debe garantizar un prefijo consistente** de commits (nunca datos dañados). Se completa con una llamada explícita `sync()` y en `close()`. | 0 `fsync` por commit. |

M0 mostró que esta última garantía **no es gratuita en un CoW B+ tree**: sin barreras, un meta nuevo puede persistir antes que sus páginas y las páginas liberadas se reutilizan antes de ser durables; el prototipo detecta el problema (nunca devuelve datos erróneos) pero ~87 % de los cortes simulados dejaron la base sin poder abrirse (y otro 12 %, con daño detectado al leer). Ofrecerla en un CoW exige puntos de sincronización periódicos y reutilización diferida de páginas (ver [diseño del prototipo](../../spikes/storage/spike-btree/DESIGN.md#durability-modes)). En un LSM sale del propio diseño del WAL.

**Agrupación de commits (group commit)** queda como optimización de Fase 1 para escritores concurrentes; no cambia la semántica.

### Detección de escrituras parciales y recuperación

- Todo registro de journal, página, bloque, manifest o meta lleva longitud explícita y checksum verificado antes de usarse (ADR-0003).
- El último commit válido se identifica sin ambigüedad: meta con secuencia más alta y crc válido (CoW) o MANIFEST + reproducción del WAL con secuencias consecutivas (LSM).
- Una cola rota solo se acepta al final del journal más reciente tras una caída; tras un cierre limpio, o seguida de registros válidos, es corrupción.
- Los archivos que definen el estado se crean y reemplazan de forma atómica (temporal → `fsync` → `rename` → `fsync` del directorio). Un archivo existente vacío o sin estado válido es corrupción, nunca «base nueva».
- La recuperación es automática al abrir, **reentrante** (puede caer y repetirse, I-11) y no destructiva: solo descarta lo que el protocolo define como no confirmado (colas rotas, archivos huérfanos no referenciados por el estado durable). Ante corrupción, `open` falla con `Corruption` y **no repara**.
- La matriz completa, por motor y por punto de fallo, está en [failure-matrix.md](../architecture/failure-matrix.md).

### Actualización de formato interrumpida

Se diseñará en ADR-0003: la migración escribe a archivos nuevos (o a una versión nueva del estado) y el cambio de versión es el mismo paso atómico que un commit (meta o MANIFEST); el estado de la migración se persiste para poder reanudarla o abortarla. M0 no implementa migraciones.

### Herramienta de verificación

La CLI (Fase 2) tendrá `kerno verify` (lee todo, valida checksums y estructura, no escribe) y `kerno export --salvage` (exporta lo legible a formato lógico en otro directorio). Ninguna reparación destructiva se ejecuta automáticamente (SAD §12).

## Evidencia (M0)

- Ambos prototipos pasan, con 400 semillas en `release`: caídas en puntos aleatorios (incluso durante la recuperación), caída antes de **cada** operación de E/S del escenario fijo (muerte de proceso y corte de energía), un EIO aislado en cada operación (reinicio y corte inmediato), bit flips y truncamientos. Ver el [informe de M0](../testing/m0-report.md).
- Las suites se validaron inyectando fallos conocidos en el motor de referencia (`fsync` omitido, `rename` sin `fsync`, error de `fsync` ignorado, sincronización de directorio condicional); todas fueron detectadas ([plan de pruebas](../testing/test-plan.md#21-validación-del-harness-mutation-testing-manual)).

## Consecuencias

- La API pública de Fase 1 (ADR-0010) expondrá: `begin_write()` (transacción de escritura con lectura de sus propias escrituras), `commit()`/`abort()`, `snapshot()`, `sync()`, y el modo de durabilidad por base y, opcionalmente, por commit.
- La secuencia de commit es parte del formato persistente y del contrato público.
- El motor elegido en ADR-0001 hereda la fila correspondiente de la matriz de fallos como **especificación**; cualquier cambio de protocolo debe actualizarla y mantener las suites verdes.

## Seguimiento

- Fase 1: transacciones read-modify-write sobre el motor elegido; `sync()` explícito; métricas de snapshots; group commit si los benchmarks lo justifican.
- Fase 1: semántica Linux de `fsync` fallido y pruebas con dispositivos reales (R-S2).
