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

Pasada adicional de estrés con `SPIKE_SEEDS=1000` sobre las suites compartidas de ambos candidatos: todo en verde. Bit flips: B+ tree 79 detectados al abrir · 451 al leer · 470 inocuos; LSM 115 · 885 · 0. Truncamientos: 1000/1000 detectados al abrir en ambos. Ningún dato erróneo ni retroceso.

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

### 3.1 Entorno y método

| | |
|---|---|
| Máquina | VM, Intel Xeon 2,10 GHz, 4 núcleos, 15,7 GiB, ext4, kernel 6.18 |
| `fdatasync` de 4 KiB | p50 117 µs · p95 355 µs · p99 459 µs (el suelo de cualquier commit durable aquí) |
| Compilación | `rustc 1.97.0`, perfil `release`, commit `18dbb2b` (mismo código de motores que este informe) |
| Método | [ADR-0012](../adr/0012-metodologia-de-benchmarks.md): un proceso y un directorio nuevos por ejecución, 3 repeticiones intercaladas, mediana (rango entre paréntesis si supera el 15 %), caché de 64 MiB, sin compresión, durabilidad equivalente |
| Escalas | `small` = 20 000 registros, `medium` = 200 000; las lecturas con caché fría vacían la caché de páginas del SO antes de medir |
| Datos | 360 ejecuciones, 0 fallos: [`results.jsonl`](../../spikes/storage/results/2026-10-27/results.jsonl), [tablas completas](../../spikes/storage/results/2026-10-27/report.md), [entorno](../../spikes/storage/results/2026-10-27/env.json) |
| Reproducción | `spikes/storage/run-benchmarks.sh` |

### 3.2 Casos de referencia (escala `medium` salvo indicación)

| Caso | Métrica | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| **Cauce** (`Sync`) | pasos/s (commit durable + lecturas) | 1 237 | **3 462** | 2,8× | 1 865 | 3 341 | 2 121 |
| | p99 commit (µs) | 1 969 | **700** | | 1 266 | 791 | 1 196 |
| | recuperación tras cierre sucio (ms) | **0,7** | 11,2 | | 1,0 | 354 | 0,5 |
| **Cauce** (`Buffered`, `small`) | pasos/s | 17 794 | **71 423** | 4,0× | 20 460 | 58 950 | 9 714 |
| **LiveSpec** | change sets/s | 398 | **637** | 1,6× | 525 | 631 | 445 |
| | p99 commit (µs) | **3 719** | 4 968 | | 2 841 | 2 277 | 12 911 |
| | p99 listado de módulo (µs) | **379** | 574 | | 310 | 634 | 284 |
| **Memoria de agentes** | pasos/s | 1 016 | 1 074 | 1,06× | 1 035 | 1 224 | 662 |
| | p99 escritura de recuerdo (µs) | **1 818** | 5 255 | | 2 284 | 1 194 | 1 595 |
| | p99 recuperación de recuerdo (µs) | **49** | 64 | | 43 | 79 | 113 |
| | espacio / datos vivos | 3,93 | **2,61** | | 5,04 | 3,14 | 4,42 |
| **Historial simulado** | revisiones/s (miles) | 9,7 | **66,9** | 6,9× | 12,3 | 52,0 | 11,0 |
| | p50 historial completo de un registro (µs) | **5,9** | 15,8 | | 6,6 | 19,0 | 9,4 |
| | p99 reconstruir una revisión (µs) | **4,3** | 10,2 | | 4,9 | 29,5 | 7,4 |

### 3.3 Micro-benchmarks (escala `medium`)

| Workload | Métrica | B+ tree | LSM | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|
| Commits de un registro (`Sync`) | commits/s | 1 718 | **4 943** | 2 930 | 5 248 | 4 800 |
| | `fsync` por commit | 2 | **1** | — | — | — |
| | amplificación de escritura | 112× | **1,09×** | 132× | 1,21× | 37,5× |
| Commits de un registro (`Buffered`, `small`) | commits/s | 37 562 | **297 402** | 22 604 | 355 996 | 28 755 |
| Carga masiva aleatoria | kops/s | 20,1 | **173** | 28,4 | 150 | 12,3 |
| | amplificación de escritura | 10,1× | **3,4×** | 10,6× | 1,6× | 36,6× |
| | espacio / datos (cerrada) | 1,65 | **1,03** | 2,36 | 1,58 | 1,48 |
| | RAM máxima (MB) | 89 | **25** | 72 | 136 | 73 |
| | cierre limpio (ms) | **15** | 718 | 7,8 | 90 | 17 |
| | reapertura limpia (ms) | **1,2** | 3,3 | 0,8 | 336 | 0,5 |
| Lecturas puntuales, caché caliente | kops/s (1 hilo / 4 hilos) | 202 / 497 | **240 / 721** | 256 / 468 | 268 / 806 | 163 / 482 |
| Lecturas puntuales, **caché fría** | kops/s (1 hilo) · p99 µs | 83 · 78 | **157 · 57** | 93 · 74 | 189 · 55 | 74 · 127 |
| Escaneo de 100, caché caliente | p50 · p99 (µs) | 80 · 180 | **55 · 123** | 85 · 157 | 83 · 148 | 69 · 157 |
| Escaneo de 100, **caché fría** | p50 · p99 (µs) | 83 · 1 182 | **60 · 297** | 79 · 1 125 | 79 · 293 | 58 · 1 099 |
| Mixto 90/10, 4 hilos | kops/s | 10,3 | **29,1** | 14,6 | 25,8 | 24,0 |
| | p99 lectura · p99 escritura (µs) | **41** · 8 274 | 50 · **4 073** | 97 · 6 742 | 126 · 4 157 | 155 · 4 172 |
| Inserción secuencial + actualizaciones | kops/s actualizaciones | 101 | **142** | 103 | 107 | 128 |
| | p99 commit de 100 (µs) | **2 074** | 6 283 | 1 720 | 2 037 | 7 893 |

### 3.4 Lectura de los resultados

1. **Los prototipos representan a su familia.** El B+ tree se comporta como redb y el LSM como fjall en casi todas las filas (a veces mejor). Las diferencias entre candidatos son de arquitectura, no de madurez de la implementación.
2. **Escrituras: el LSM gana en todo lo que escribe**, de 1,6× (LiveSpec) a 8× (commits sin `fsync`), con ~100× menos bytes escritos por commit pequeño. En el B+ tree cada commit reescribe la ruta raíz-hoja de cada hoja tocada más el meta; con dos `fsync` por commit (prototipo) o uno (redb), su techo queda por debajo.
3. **Lecturas con caché caliente: empate.** El LSM es algo más rápido en lecturas puntuales y escaneos largos; el B+ tree, en colas (p99) cuando hay escrituras concurrentes y en el historial de un registro (2,7×, un rango contiguo dentro de una hoja).
4. **Lecturas con caché fría: el LSM gana claramente** (1,9× en lecturas puntuales; p99 de escaneos 4× menor). La asignación copy-on-write dispersa las páginas por el archivo; las SST del LSM son secuenciales.
5. **Latencia de cola bajo escritura sostenida: gana el B+ tree.** El LSM paga la compactación: p99 de actualizaciones sostenidas 3× peor y p99 de escritura en memoria de agentes 2,9× peor. Es el punto débil principal del LSM y la primera condición de ADR-0001.
6. **Apertura, recuperación y cierre: gana el B+ tree** (≈1 ms frente a 3–11 ms al abrir; 15 ms frente a 718 ms al cerrar, porque el LSM ahora compacta antes de cerrar). Todos los valores salvo el cierre del LSM son irrelevantes para una base embebida; el cierre se acotará.
7. **Espacio y memoria: gana el LSM** (1,03× frente a 1,65× tras cargar; 25 MB frente a 89 MB de RAM máxima). En memoria de agentes, con expiración continua, el B+ tree llega a 3,9× por páginas medio llenas que no se compactan.

### 3.5 Limitaciones de la medición

- Una sola máquina virtual con almacenamiento virtualizado: `fdatasync` es barato (~0,1–0,5 ms). En un disco con `fsync` más caro, la ventaja del LSM en commits durables (1 frente a 2 `fsync`) crece; en uno más barato, se reduce. Repetir en el hardware objetivo (R-S3).
- `wchar` mide bytes entregados a llamadas de escritura, no bytes en el dispositivo.
- Escala `large` (1 M registros) no incluida en esta ejecución por tiempo; `run-benchmarks.sh` la admite (`SCALES=small,medium,large`).
- redb en modo `Buffered` usa `Durability::None`, que no sobrevive a la muerte del proceso: sus cifras `Buffered` no son equivalentes.

## 4. Conclusión

Ambas arquitecturas son implementables con las garantías exigidas; ninguna gana por correctitud. En los casos de referencia de Kerno —intensivos en escritura, con historial y con durabilidad por commit— el **LSM** rinde entre 1,6× y 8× más, escribe mucho menos, ocupa menos, lee mejor con caché fría y ofrece un modo `Buffered` consistente. El **B+ tree** tiene mejores colas de latencia bajo escritura, lecturas de historial más rápidas y una operación más simple. La recomendación, sus condiciones y las decisiones que dependen de ella están en [ADR-0001](../adr/0001-motor-de-almacenamiento.md).
