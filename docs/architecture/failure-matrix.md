# Matriz de fallos (M0)

Estado: borrador para revisión · Anexo de [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) · Criterio 3 de [SAD §17.5](SAD.md#175-criterios-de-aceptación)

Para cada punto de interrupción del protocolo de cada candidato, este documento indica **qué estado debe observarse tras reabrir**. La notación es:

- `N`: último commit confirmado (`commit` devolvió `Ok(N)`).
- `N+1`: commit en curso cuando ocurrió el fallo (no confirmado).
- «`N` o `N+1`»: ambos son correctos; nunca un estado intermedio, nunca `< N`.

Tipos de fallo (modelados por [`SimVfs`](../../spikes/storage/spike-core/src/vfs/sim.rs)):

| Fallo | Qué sobrevive |
|---|---|
| **Muerte del proceso** | Todo lo que el proceso escribió (la caché del SO sobrevive). |
| **Corte de energía** | Solo lo sincronizado; cada escritura pendiente se pierde, se aplica o se rompe a 512 B, en cualquier orden; las operaciones de directorio no sincronizadas sobreviven como prefijo. |
| **EIO aislado** | Una operación falla (sin efecto; un `fsync` fallido deja sus escrituras sin garantía) y el dispositivo sigue funcionando. |

## 1. Cómo se verifica

La matriz no es solo documentación: cada fila se ejercita automáticamente.

| Prueba | Qué recorre | Resultado esperado comprobado |
|---|---|---|
| `crash::every_point` | Caída antes de **cada** operación de E/S mutante de un escenario fijo (creación, 25 commits, cierre limpio, reapertura, 20 commits, cierre sucio, recuperación, 10 commits, cierre), para muerte de proceso y corte de energía | estado = commit `s` con `confirmados ≤ s ≤ intentados`; la base sigue aceptando commits y es durable |
| `faults::single_failure_at_every_point` | Un EIO en **cada** operación de E/S del mismo escenario, seguido de reinicio o de corte de energía inmediato | error devuelto, handle envenenado, ningún commit confirmado con `fsync` fallido, recuperación correcta |
| `crash::run` | Caídas en puntos aleatorios de historias largas (5 rondas por semilla, incluidas caídas **durante** la recuperación y durante trabajo en segundo plano) | igual que arriba, encadenando recuperaciones |
| `corruption::*` | Bit flips y truncamientos tras cierre limpio | dato correcto o error; nunca otro dato ni retroceso |

Los puntos concretos del escenario varían con el motor (cada uno hace un número distinto de operaciones); las filas siguientes agrupan esos puntos por fase del protocolo.

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

**Modo `Buffered`** (sin `fsync`): ante muerte del proceso, `N` o `N+1`. Ante corte de energía **no hay garantía de prefijo**: un meta nuevo puede llegar al disco antes que sus páginas y las páginas liberadas se reutilizan antes de ser durables. Los checksums Merkle garantizan que el resultado es el estado exacto de algún commit **o** `Error::Corruption` detectado, nunca datos erróneos; en simulación, ~85 % de los cortes dejaron la base sin poder abrirse hasta repararla. Ver ADR-0002.

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

## 5. Fuera del modelo (pendiente para Fase 1)

- Semántica de Linux tras un `fsync` fallido (páginas sucias marcadas como limpias): el modelo deja las escrituras pendientes, no perdidas (R-S2).
- Daño del medio **durante** el uso o combinado con caídas.
- Pruebas con dispositivos reales (kill -9, LazyFS / `dm-log-writes`).
- Fallos durante migraciones de formato (no existen todavía).
