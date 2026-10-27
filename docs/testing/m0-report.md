# Informe de M0 — spike de almacenamiento

Estado: resultados del 2026-10-27 · Relacionado con: [ADR-0001](../adr/0001-motor-de-almacenamiento.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [plan de pruebas](test-plan.md), [matriz de fallos](../architecture/failure-matrix.md)

Este informe recoge la evidencia con la que se decide ADR-0001: primero la **correctitud** (que es requisito, no criterio de desempate) y después el **rendimiento**. Todo es reproducible desde el commit indicado en `env.json`.

## 1. Qué se comparó

| Motor | Arquitectura | Papel |
|---|---|---|
| `btree` | B+ tree copy-on-write propio ([diseño](../../spikes/storage/spike-btree/DESIGN.md)) | candidato A |
| `lsm` | LSM nivelado propio ([diseño](../../spikes/storage/spike-lsm/DESIGN.md)) | candidato B |
| `redb` 4.1 | B+ tree copy-on-write maduro | referencia de la familia A |
| `fjall` 3.1 | LSM maduro | referencia de la familia B |
| SQLite 3 (rusqlite 0.41, WAL) | B-tree + WAL | referencia general |

Los dos candidatos implementan el mismo contrato (`spike_core::Engine`) y pasan exactamente las mismas pruebas; los motores de referencia solo participan en los benchmarks y en la prueba diferencial.

## 2. Correctitud

### 2.1 Resultado final

Ejecución con `SPIKE_SEEDS=400 cargo test --release` sobre el commit final (ver `results/2026-10-27/`):

| Suite (compartida salvo indicación) | B+ tree | LSM |
|---|---|---|
| Funcionales (14 pruebas: modelo aleatorio, reaperturas limpias y sucias, aislamiento bajo churn, 4 lectores concurrentes, límites, espacio, FS real) | ✅ | ✅ |
| Caídas en puntos aleatorios (4 variantes × 400 semillas × 5 rondas, incluidas caídas durante la recuperación y desgarro a nivel de byte en la mitad de las semillas) | ✅ | ✅ |
| Caída antes de **cada** operación de E/S del escenario fijo (corte de energía y muerte de proceso) | ✅ | ✅ |
| Un EIO aislado en **cada** operación de E/S (reinicio y corte inmediato) | ✅ | ✅ |
| `Buffered` + corte de energía = prefijo consistente | ⚠️ no lo garantiza (ver §2.3) | ✅ |
| Bit flips tras cierre limpio (400; la mitad dirigidos a cabeceras/metas/pies) | ✅ 28 detectados al abrir · 191 al leer · 181 inocuos · **0 datos erróneos · 0 retrocesos** | ✅ 34 al abrir · 366 al leer · 0 inocuos · **0 datos erróneos · 0 retrocesos** |
| Truncamientos tras cierre limpio (400, ¼ a 0 bytes) | ✅ 400/400 detectados al abrir | ✅ 400/400 detectados al abrir |
| Pruebas unitarias propias | ✅ 32 | ✅ 31 |
| Regresiones de la revisión adversarial | ✅ 6 | ✅ 5 |
| Prueba diferencial (misma historia en btree, lsm, LogEngine, redb, fjall, SQLite) | ✅ | ✅ |

Los «inocuos» del B+ tree son bytes que el motor nunca lee (páginas libres, relleno del meta, la segunda copia del meta cuando la primera está intacta). El LSM no tiene bytes sin cubrir: todo daño se detecta.

### 2.2 Defectos encontrados y cómo se encontraron

M0 no solo midió: el proceso de verificación encontró y corrigió defectos reales en los dos candidatos. Esto es evidencia en sí misma sobre dónde está la complejidad de cada arquitectura.

**Durante la implementación (por el harness):**

| Motor | Defecto | Detectado por |
|---|---|---|
| B+ tree | Slot del meta elegido por paridad de la secuencia: tras perder un slot, el siguiente commit sobrescribía el único meta válido | prueba unitaria propia |
| B+ tree | Directorio sincronizado solo si el archivo «acababa de crearse» | `single_io_failure_at_every_point` |
| LSM | Si el `rename` del MANIFEST tenía éxito pero su `fsync` de directorio fallaba, se borraban salidas que el MANIFEST superviviente podía referenciar | revisión propia del implementador |
| referencia | Mismo defecto de directorio; corrupción a mitad del log tratada como cola rota | suites de fallos y de corrupción |

**Por la revisión adversarial** (5 revisores por motor, uno por clase de fallo; cada hallazgo, verificado por 2 escépticos independientes con prueba de reproducción; todos los verificados quedaron **confirmados y reproducidos**):

| Motor | Severidad | Defecto | Estado |
|---|---|---|---|
| B+ tree | **alta** | La apertura trataba como durable un meta solo visible (proceso muerto entre escribirlo y sincronizarlo, o `fsync` fallido); sus páginas liberadas —el árbol anterior, el único durable— se reutilizaban, y un reinicio seguido de un corte de energía durante el siguiente commit podía destruir la base (~30 % de las ejecuciones simuladas) | corregido (`fsync` al abrir) + regresión |
| B+ tree | media | Un bit dañado en el meta más reciente tras un cierre sucio hacía volver silenciosamente al commit anterior | corregido (dos copias del meta por slot) + regresión |
| B+ tree | media | Un pánico dentro del commit no envenenaba el handle; commits posteriores dejaban la base sin poder abrirse | corregido |
| B+ tree | media (rendimiento) | Los valores de más de ~1 KiB nunca se cacheaban: un `pread` por página de 4 KiB en cada lectura | corregido (caché unificada) |
| B+ tree | baja | Pánico ante un archivo fabricado con crc válidos; desbordamiento de la secuencia en `u64::MAX` | corregidos + regresiones |
| LSM | **media-alta** | Daño a mitad del WAL más reciente tras un cierre sucio se trataba como cola rota: commits confirmados en modo `Sync` se perdían en silencio y el WAL se borraba | corregido (bandera `SYNCED` por registro) + regresión |
| LSM | baja (rendimiento: media) | `close()` abandonaba la compactación en curso y añadía otro archivo L0: L0 crecía sin límite con sesiones cortas, y cada reapertura empezaba con trabajo atrasado | corregido (compacta al cerrar) + regresión |
| LSM | baja | Lotes > 4 GiB confirmados y luego ilegibles; carrera con un snapshot de un handle anterior que borra un SST huérfano; nombres no canónicos; aritmética sin comprobar con archivos fabricados; doble lectura de bloque en `seek` | corregidos |
| ambos | media | El directorio de la base nunca se sincronizaba en su directorio padre: un corte de energía podía hacer desaparecer una base nueva con todo lo confirmado | corregido en ambos + regresiones |

Hallazgos de rendimiento de severidad baja que no llegaron a verificarse (cuota de agentes agotada) y quedan como limitaciones conocidas: contención de contadores atómicos globales en lecturas concurrentes (LSM); `fdatasync` de un WAL que crece en cada commit, sin preasignación (LSM); copias extra de páginas en lotes atómicos muy grandes (B+ tree).

**Defectos del propio harness** encontrados por los candidatos y corregidos: dos carreras con trabajo en segundo plano (disparador de caída que sobrevivía a la recuperación; `dir_size` frente a borrados concurrentes) y dos huecos de cobertura (daño uniforme que casi nunca alcanzaba metas y cabeceras; desgarro solo a 512 B, que hacía atómicas las escrituras pequeñas). Cada hueco se validó después reintroduciendo el defecto que dejaba pasar ([plan de pruebas §2.1](test-plan.md#21-validación-del-harness-mutation-testing-manual)).

### 2.3 Diferencia arquitectónica observada en durabilidad

En modo `Buffered` (sin `fsync` por commit) y ante un corte de energía:

- **LSM**: siempre un prefijo consistente de commits. Sale del propio diseño: el WAL viejo se sincroniza antes de crear uno nuevo y la reproducción se detiene en el primer hueco.
- **B+ tree CoW**: nunca datos erróneos (los checksums Merkle lo impiden), pero **no** un prefijo: en simulación, ~85 % de los cortes dejaron la base sin poder abrirse, porque un meta nuevo puede llegar al disco antes que sus páginas y las páginas liberadas se reutilizan antes de ser durables. Ofrecer la garantía exige puntos de sincronización periódicos y reutilización diferida (diseño esbozado en el prototipo).

## 3. Rendimiento

> Sección pendiente de completar con los resultados de `run-benchmarks.sh` (en curso).
