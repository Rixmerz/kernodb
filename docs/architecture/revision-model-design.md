# Prototipo de versionado a nivel de diseño (M0)

Estado: borrador para revisión · Relacionado con: [SAD §8](SAD.md#8-revision-engine-historial-diferencias-y-procedencia), [SAD §17.2](SAD.md#172-alcance), [ADR-0004](../adr/0004-historial-de-revisiones.md), [ADR-0001](../adr/0001-motor-de-almacenamiento.md)

M0 no construye el Revision Engine. Este documento fija **cómo se representaría** sobre el núcleo key-value ordenado, para que (a) el benchmark `history_sim` mida el coste de esa representación en cada candidato de almacenamiento y (b) ADR-0001 tenga en cuenta la compatibilidad futura con el versionado (criterio 5 de SAD §17.5).

## 1. Principios que el diseño respeta

1. **Revisiones = datos, no journal (D-06).** Las revisiones se escriben como claves normales en el mismo commit que el estado actual. El WAL/journal del motor solo sirve para recuperación y puede truncarse sin afectar al historial (I-20).
2. **Una frontera atómica (I-21).** Estado actual, entrada de revisión y registro del `ChangeSet` van en el mismo `WriteBatch`. El motor ya garantiza atomicidad del batch (I-01); el Revision Engine no necesita un segundo protocolo de commit.
3. **Orden por secuencia de commit (I-26).** El orden histórico lo da `commit_sequence`, no el reloj ni UUIDv7.
4. **Restaurar crea una revisión nueva (I-22).**
5. **Sin contexto inventado.** Actor, fuente y razón son opcionales.

## 2. Disposición de claves

Las colecciones se codifican como prefijos de clave (el núcleo no conoce colecciones). Los enteros se codifican big-endian de ancho fijo para que el orden lexicográfico coincida con el numérico.

| Clave | Valor | Para qué |
|---|---|---|
| `c/{col}/{record_id}` | sobre del estado actual: `rev_no`, `commit_seq`, `content_hash`, `schema_version`, flags, payload | lectura del estado vigente en una sola búsqueda |
| `r/{col}/{record_id}/{rev_no:u64be}` | entrada de revisión: `kind` (`snapshot` \| `delta` \| `tombstone` \| `restore`), `parent_rev`, `commit_seq`, `content_hash`, `changeset_id?`, payload completo o delta | historial de un registro = **un único rango contiguo** |
| `x/{changeset_id}` | `ChangeSet`: `commit_seq`, razón/actor/fuente/tarea opcionales, lista de `(col, record_id, rev_no)` | responder «qué operación produjo este cambio» |
| `xs/{commit_seq:u64be}` | `changeset_id` | recorrer cambios por orden de commit (auditoría, sincronización de derivados) |

`record_id` es el UUIDv7 interno (D-05). `rev_no` es un contador por registro (1, 2, …); la identidad de una revisión es `(record_id, rev_no)` y su posición global es `commit_seq`.

Elegir `record_id` antes que `rev_no` hace que:

- *historial de un registro* sea un `scan` de prefijo;
- *reconstruir la revisión r* sea un `scan` desde el último snapshot ≤ r hasta r (acotado por la política de deltas);
- *diff entre r1 y r2* reconstruya ambas con, como mucho, dos rangos.

La alternativa (`commit_seq` primero) favorece «qué cambió en el commit N», que ya cubre `xs/`.

## 3. Snapshots y deltas

- La primera revisión de un registro es siempre `snapshot`.
- Las siguientes son `delta` estructural respecto de la anterior, salvo que (a) se alcance la profundidad máxima `K` desde el último snapshot, o (b) el tamaño acumulado de deltas supere una fracción `F` del último snapshot. En ambos casos se escribe un `snapshot`.
- `K` y `F` son parámetros a fijar con benchmarks (Fase 3). `history_sim` usa `K = 8` y deltas de ~60 B sobre documentos de 500 B como punto de partida.
- Borrado lógico: revisión `tombstone` (el estado actual desaparece o queda marcado según la política).
- Restauración: revisión `restore` con el contenido histórico completo (snapshot) y `restored_from = rev_no`.
- El formato del delta (p. ej. JSON Patch RFC 6902 o un diff estructural propio) y el hash canónico (RFC 8785) se deciden en ADR-0004/ADR-0008. El diseño solo exige que el delta sea determinista y aplicable sin el WAL.

## 4. Políticas de historial (SAD §8.4)

| Política | Qué escribe cada cambio |
|---|---|
| `none` | solo `c/…` (y `x/` si la aplicación declara ChangeSets) |
| `audit` | `c/…` + entrada `r/…` sin payload (metadatos: kind, hash, commit_seq, changeset) |
| `versioned` | `c/…` + entrada `r/…` con snapshot/delta según §3 |

## 5. Retención, supresión y lo que depende del almacenamiento físico

- **Retención por antigüedad o número de revisiones**: borrar un rango `r/{col}/{id}/[0, n)`. Si se elimina el snapshot base de una cadena, la primera revisión retenida debe reescribirse como `snapshot` en el mismo commit.
- **Supresión de datos personales** (ADR-0009): borrar `c/`, todas las `r/` del registro, referencias en `x/` y derivados (índices, embeddings, colas), en una sola transacción lógica.
- **Borrado físico**: el borrado lógico no borra los bytes. Lo que queda depende del candidato:
  - *CoW B+ tree*: las páginas liberadas conservan el contenido antiguo hasta que se reutilizan. Para garantías de supresión hace falta sobrescribir (scrub) las páginas al liberarlas o usar cifrado por registro con destrucción de claves.
  - *LSM*: las versiones antiguas siguen en SST y en el WAL hasta que la compactación las reescribe y el WAL se recicla; los tombstones deben llegar al último nivel. Hace falta una compactación forzada del rango afectado o, igualmente, borrado criptográfico.
  - En ambos casos, backups y copias externas siguen sus propios plazos (SAD §11.3, NIST SP 800-88).
- **Borrado de rangos**: retención y supresión borran rangos contiguos. Un LSM se beneficia de *range tombstones* (no implementados en el prototipo); un B+ tree libera hojas completas.

## 6. Cómo lo mide M0

El workload `history_sim` (ver [`workloads.rs`](../../spikes/storage/spike-bench/src/workloads.rs)) aplica esta disposición con `K = 8`: cada actualización escribe `cur/{id}` y `rev/{id}/{rev}` en el mismo batch, con revisiones de registros distintos intercaladas en el tiempo (como ediciones reales). Mide:

- coste de escritura del historial (revisiones/s, latencia de commit p99);
- espacio en disco frente a bytes lógicos (estado + historial);
- latencia de leer todo el historial de un registro (`history_scan`);
- latencia de reconstruir una revisión arbitraria (`reconstruct`: snapshot base + deltas en un rango).

Los resultados y su lectura están en el [informe de M0](../testing/m0-report.md) y en ADR-0001.

## 7. Preguntas abiertas (para ADR-0004)

1. ¿`rev_no` por registro o `revision_id` UUIDv7 propio? (El diseño usa contador por registro: claves más cortas y orden trivial.)
2. ¿Deltas estructurales sobre el documento lógico o deltas binarios sobre la serialización? (Estructurales: legibles y deterministas, pero dependen de ADR-0008.)
3. ¿Se versionan también los metadatos (clasificación, retención) del registro?
4. ¿Cómo se representa un `ChangeSet` que abarca miles de registros (lista inline frente a claves `x/{id}/{n}`)?
5. Interacción con snapshots de lectura largos: un lector que recorre historial mientras la retención borra revisiones antiguas debe ver un estado coherente (lo garantiza I-05 del motor).
