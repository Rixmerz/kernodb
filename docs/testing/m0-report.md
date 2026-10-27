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

Los dos candidatos implementan el mismo contrato (`spike_core::Engine`) y pasan la misma batería compartida, con una excepción declarada: el B+ tree no promete un prefijo consistente en `Buffered` ante corte de energía y no ejecuta esa variante (§2.3). Las pruebas unitarias y de regresión son propias de cada motor. Los motores de referencia solo participan en los benchmarks y en la prueba diferencial.

## 2. Correctitud

### 2.1 Resultado final

Ejecución con `SPIKE_SEEDS=400 cargo test --workspace --all-features --release` sobre el commit `a7d40d7` (log completo en [`tests-seeds400.log`](../../spikes/storage/results/2026-10-27/tests-seeds400.log)):

| Suite (compartida salvo indicación) | B+ tree | LSM |
|---|---|---|
| Funcionales (14 pruebas: modelo aleatorio, reaperturas limpias y sucias, aislamiento bajo churn, 4 lectores concurrentes, límites, espacio, FS real) | ✅ | ✅ |
| Caídas en puntos aleatorios (`crash::run`: 400 semillas × 5 rondas, incluidas caídas durante la recuperación y desgarro a nivel de byte en la mitad de las semillas) | ✅ 3 variantes (sin `Buffered` + corte de energía) | ✅ 4 variantes |
| Caída antes de **cada** operación de E/S del escenario fijo (corte de energía y muerte de proceso en `Sync`, muerte de proceso en `Buffered`; tras recuperar, un commit más, cierre y corte de energía) | ✅ | ✅ |
| Un EIO aislado en **cada** operación de E/S (reinicio y corte inmediato) | ✅ | ✅ |
| `Buffered` + corte de energía = prefijo consistente | ⚠️ no lo garantiza (ver §2.3) | ✅ |
| Bit flips tras cierre limpio (400; la mitad dirigidos a cabeceras/metas/pies) | ✅ 28 detectados al abrir · 191 al leer · 181 inocuos · **0 datos erróneos · 0 retrocesos** | ✅ 36 al abrir · 364 al leer · 0 inocuos · **0 datos erróneos · 0 retrocesos** |
| Truncamientos tras cierre limpio (400; la mitad dirigidos a cabeceras/metas/pies, ¼ a 0 bytes) | ✅ 400/400 detectados al abrir | ✅ 400/400 detectados al abrir |
| Pruebas unitarias propias | ✅ 32 | ✅ 24 unitarias + 7 de integración |
| Regresiones de la revisión adversarial y de la auditoría previa al merge | ✅ 9 | ✅ 7 |
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
| Máquina | VM, Intel Xeon 2,80 GHz, 4 núcleos, 15,7 GiB, ext4, kernel 6.18 |
| `fdatasync` de 4 KiB | p50 113 µs · p95 400 µs · p99 568 µs (el suelo de cualquier commit durable aquí) |
| Compilación | `rustc 1.97.0`, perfil `release`, commit `df45ddb` |
| Método | [ADR-0012](../adr/0012-metodologia-de-benchmarks.md): un proceso y un directorio nuevos por ejecución, 3 repeticiones intercaladas (1 en `large`), mediana (rango entre paréntesis si supera el 15 %), caché de 64 MiB, sin compresión, durabilidad equivalente; con caché caliente, un recorrido completo de la base antes de medir (calentamiento fuera de la medición) |
| Escalas | `small` = 20 000 registros, `medium` = 200 000, `large` = 1 000 000 (carga masiva, lecturas puntuales, escaneos y commits pequeños); las lecturas con caché fría vacían la caché de páginas del SO antes de reabrir (aplicado en todas esas ejecuciones) |
| Datos | 410 ejecuciones, 0 fallos: [`results.jsonl`](../../spikes/storage/results/2026-10-27/results.jsonl), [tablas completas](../../spikes/storage/results/2026-10-27/report.md), [entorno](../../spikes/storage/results/2026-10-27/env.json) |
| Reproducción | `spikes/storage/run-benchmarks.sh` |

† En las tablas, redb en modo `Buffered` usa `Durability::None`, que no sobrevive a la muerte del proceso: sus cifras `Buffered` **no son comparables** con las de los demás motores (ADR-0012).

### 3.2 Casos de referencia (escala `medium` salvo indicación)

| Caso | Métrica | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| **Cauce** (`Sync`) | pasos/s (commit durable + lecturas) | 1 111 | **3 087** | 2,8× | 1 748 | 2 941 | 2 187 |
| | p99 commit (µs) | 1 966 | **713** | | 1 213 | 658 | 1 078 |
| | p99 lectura (µs) | **76** | 98 | | 59 | 118 | 151 |
| | recuperación tras cierre sucio (ms) | **0,9** | 9,7 | | 1,1 | 364 | 0,7 |
| **Cauce** (`Buffered`, `small`) | pasos/s | 16 250 | **57 110** | 3,5× | 15 680 † | 61 490 | 7 413 |
| **LiveSpec** | change sets/s | 408 | **667** | 1,6× | 524 | 522 | 468 |
| | p99 commit (µs) | **3 762** | 4 764 | | 3 148 | 2 425 | 12 530 |
| | p99 listado de módulo (µs) | **440** | 657 | | 377 | 1 296 | 346 |
| **Memoria de agentes** | pasos/s | **1 073** | 1 037 | 0,97× | 1 098 | 1 279 | 704 |
| | pasos/s (`small`) | 1 377 | **1 642** | 1,19× | 1 595 | 2 166 | 832 |
| | p99 escritura de recuerdo (µs) | **1 573** | 5 837 | | 2 128 | 1 090 | 1 613 |
| | p99 recuperación de recuerdo (µs) | **49** | 101 | | 46 | 73 | 121 |
| | espacio / datos vivos | 3,93 | **2,64** | | 5,04 | 3,14 | 4,43 |
| **Historial simulado** | revisiones/s (miles) | 8,3 | **66,6** | 8,0× | 11,4 | 61,6 | 11,4 |
| | p99 commit de 100 revisiones (µs) | 17 870 | **9 118** | | 13 480 | 3 168 | 26 800 |
| | p50 historial completo de un registro (µs) | **7,6** | 14,4 | | 5,9 | 20,3 | 9,8 |
| | p99 reconstruir una revisión (µs) | **6,4** | 8,7 | | 4,5 | 32,6 | 7,7 |
| | espacio / datos (`medium` · `small`) | 2,04 · **2,32** | **1,88** · 2,98 | | 3,42 · 4,28 | 3,64 · 22,9 | 1,74 · 3,32 |

### 3.3 Micro-benchmarks (escala `medium` salvo indicación)

| Workload | Métrica | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| Commits de un registro (`Sync`) | commits/s | 1 542 | **4 044** | 2,6× | 2 499 | 4 054 | 4 113 |
| | p99 commit (µs) | 1 331 | **584** | | 951 | 616 | 630 |
| | `fsync` por commit | 2 | **1** | | — | — | — |
| | amplificación de escritura | 112× | **1,1×** | | 132× | 1,2× | 37× |
| | commits/s (`large`, 1 M) | 1 554 | **4 069** | 2,6× | 3 138 | 4 326 | 4 460 |
| Commits de un registro (`Buffered`, `small`) | commits/s | 31 080 | **200 800** | 6,5× | 18 770 † | 311 600 | 27 760 |
| Borrados puntuales (`Sync`) | commits/s | 1 365 | **4 343** | 3,2× | 2 480 | 3 991 | 3 202 |
| | p50 lectura de clave borrada · p99 de clave viva (µs) | 3,4 · 21,5 | **0,7 · 15,5** | | 1,9 · 15,0 | 1,8 · 10,6 | 7,6 · 38,9 |
| Carga masiva aleatoria | kops/s | 20,2 | **162** | 8,0× | 26,2 | 133 | 12,6 |
| | kops/s (`large`) | 10,4 | **110** | 10,6× | 18,5 | 131 | 10,3 |
| | amplificación de escritura | 10,1× | **3,4×** | | 10,6× | 1,6× | 36,6× |
| | espacio / datos (cerrada) | 1,65 | **1,03** | | 2,36 | 1,58 | 1,48 |
| | RAM máxima (MB) | 89 | **25** | | 72 | 136 | 40 |
| | cierre limpio (ms; `large`) | **12; 22** | 759; 3 587 | | 8,7; 13 | 96; 919 | 13; 14 |
| | reapertura limpia (ms; `large`) | **1,0; 1,3** | 3,4; 10,1 | | 1,2; 0,9 | 392; 292 | 0,5; 0,6 |
| Lecturas puntuales, caché caliente | kops/s, 1 hilo / 4 hilos | 188 / 370 | **230 / 620** | 1,2× / 1,7× | 273 / 366 | 235 / 773 | 138 / 441 |
| | p50 · p99, 1 hilo (µs) | 4,8 · 12,5 | **3,2 · 12,5** | | 3,7 · 9,4 | 3,5 · 9,0 | 6,3 · 21,3 |
| | kops/s, 1 hilo (`large`) | 135 | **163** | 1,2× | 176 | 117 | 101 |
| Lecturas puntuales, **caché fría** | kops/s, 1 hilo · p99 (µs) | 56,7 · 117 | **117 · 86** | 2,1× | 64,9 · 114 | 143 · 82 | 52,0 · 182 |
| Escaneo de 100, caché caliente | p50 · p99 (µs) | 82 · 171 | **58 · 146** | | 84 · 182 | 73 · 140 | 73 · 167 |
| | p50 · p99 (µs), `small` | **17 · 58** | 23 · 72 | | 25 · 84 | 65 · 132 | 50 · 128 |
| | p50 · p99 (µs), `large` | 91 · 189 | **68 · 128** | | 99 · 176 | 105 · 164 | 77 · 140 |
| Escaneo de 100, **caché fría** | p50 · p99 (µs) | 103 · 1 758 | **84 · 521** | | 91 · 1 652 | 75 · 310 | 74 · 1 745 |
| Mixto 90/10, 4 hilos | kops/s | 11,6 | **28,0** | 2,4× | 15,3 | 30,3 | 25,3 |
| | p99 lectura · p99 escritura (µs) | 84 · 7 274 | **62 · 3 817** | | 128 · 6 488 | 45 · 3 504 | 237 · 3 916 |
| | p99 lectura (µs), `small` | **12** | 31 | | 29 | 24 | 177 |
| Inserción secuencial + actualizaciones (lotes de 100) | kops/s inserción · actualización | 102 · 93 | **136 · 115** | 1,3× · 1,2× | 77 · 111 | 140 · 86 | 118 · 121 |
| | p99 commit de inserción · de actualización (µs) | **1 947 · 2 306** | 4 139 · 6 840 | | 3 103 · 1 732 | 1 504 · 2 634 | 7 999 · 7 999 |
| | amplificación de escritura · espacio / datos | **2,3× · 1,62** | 3,4× · 2,08 | | 2,9× · 2,11 | 1,7× · 3,24 | 3,2× · 1,34 |

### 3.4 Lectura de los resultados

1. **Los prototipos representan a su familia.** El B+ tree se comporta como redb y el LSM como fjall en la mayoría de las filas (redb es algo más rápido que el prototipo en lecturas puntuales; fjall, en commits sin `fsync`). Las diferencias entre candidatos son de arquitectura, no de madurez de la implementación.
2. **Rendimiento de escritura: el LSM gana en casi todo, con márgenes muy distintos.** En los casos de referencia, de un empate en memoria de agentes (0,97× en `medium`, 1,19× en `small`) a 1,6× en LiveSpec, 2,8× en Cauce y 8,0× escribiendo historial. En micro-benchmarks, de 1,2–1,3× en inserción secuencial a 2,6× en commits durables de un registro, 3,2× en borrados, 6,5× en commits sin `fsync` y 8–10,6× en carga masiva aleatoria. El factor que más pesa: el B+ tree hace dos `fsync` por commit (redb, uno) y reescribe la ruta raíz-hoja de cada hoja tocada; el LSM, un `fsync` y una escritura secuencial.
3. **Bytes escritos y espacio: casi siempre a favor del LSM, con excepciones.** Escribe ~100× menos por commit pequeño (1,1× frente a 112×) y ocupa menos tras una carga (1,03× frente a 1,65×) y en memoria de agentes (2,64× frente a 3,93×). Pero en inserción secuencial con actualizaciones escribe y ocupa más (3,4× frente a 2,3× de amplificación; 2,08× frente a 1,62× de espacio), y en el historial a escala `small` ocupa más (2,98× frente a 2,32×), por versiones aún sin compactar.
4. **Latencia de cola: depende del workload; ninguno gana en todo.**
   - Gana el **LSM** en los commits durables pequeños: p99 de Cauce 2,8× menor, de commits de un registro 2,3× menor, de escritura en el mixto 1,9× menor, del historial 2,0× menor y de los borrados 3,2× menor.
   - Gana el **B+ tree** cuando la compactación del LSM coincide con escritura sostenida o lecturas: p99 de escritura en memoria de agentes 3,7× menor, de actualizaciones secuenciales 3,0× menor, de commit en LiveSpec 1,3× menor; y en lecturas cortas de rangos pequeños (recuperación de recuerdos 2,0×, listado de módulo 1,5×, lectura de Cauce 1,3×).
   - Es el punto débil principal del LSM y la primera condición de ADR-0001.
5. **Lecturas con caché caliente: parecidas.** El LSM es algo más rápido en lecturas puntuales (1,2× con 1 hilo, 1,7× con 4) y en escaneos a partir de `medium`; el B+ tree, en escaneos a escala `small` y en el historial de un registro (1,9×, un rango contiguo dentro de una hoja).
6. **Lecturas con caché fría: gana el LSM** (2,1× en lecturas puntuales; p99 de escaneos 3,4× menor). La asignación copy-on-write dispersa las páginas por el archivo; las SST del LSM son secuenciales.
7. **Apertura, recuperación y cierre: gana el B+ tree** (≈1 ms frente a 3–10 ms al abrir; 12 ms frente a 759 ms al cerrar en `medium` y 22 ms frente a 3,6 s en `large`, porque el LSM compacta antes de cerrar). Abrir en milisegundos es irrelevante para una base embebida; el cierre del LSM se acotará.
8. **Memoria:** el LSM usa mucha menos RAM al cargar (25 MB frente a 89 MB) y en inserciones secuenciales, pero algo más en memoria de agentes (94 frente a 77 MB) y LiveSpec (86 frente a 74 MB) por sus memtables. En lecturas puntuales con caché caliente la RAM máxima es parecida en los cuatro motores en Rust (139–163 MB) y menor en SQLite (66 MB).

### 3.5 Limitaciones de la medición

- Una sola máquina virtual con almacenamiento virtualizado: `fdatasync` es barato (p50 ~0,1 ms, p99 ~0,6 ms). En un disco con `fsync` más caro, la ventaja del LSM en commits durables (1 frente a 2 `fsync`) crece; en uno más barato, se reduce. Repetir en el hardware objetivo (R-S3).
- `wchar` mide bytes entregados a llamadas de escritura, no bytes en el dispositivo.
- La escala `large` se ejecutó una sola vez y solo en cuatro workloads; sus cifras indican tendencia, no mediana.
- La CPU de la VM figura como 2,80 GHz en esta ejecución y como 2,10 GHz en una ejecución anterior del mismo día: la VM puede cambiar de host entre sesiones, pero dentro de una ejecución todas las combinaciones comparten máquina e intercalan motores.
- redb en modo `Buffered` usa `Durability::None`, que no sobrevive a la muerte del proceso: sus cifras `Buffered` no son equivalentes (†).

## 4. Conclusión

Ambas arquitecturas son implementables con las garantías exigidas; ninguna gana por correctitud. En los casos de referencia de Kerno —intensivos en escritura, con historial y con durabilidad por commit— el **LSM** rinde entre un empate (memoria de agentes) y 8× más (escritura de historial), con 2,8× en Cauce y 1,6× en LiveSpec; escribe ~100× menos bytes por commit pequeño, ocupa menos en la mayoría de los casos, lee mejor con caché fría y ofrece un modo `Buffered` consistente. El **B+ tree** tiene mejores colas cuando la compactación del LSM compite con escrituras sostenidas o lecturas cortas, lecturas de historial más rápidas, apertura y cierre inmediatos y una operación más simple. La recomendación, sus condiciones y las decisiones que dependen de ella están en [ADR-0001](../adr/0001-motor-de-almacenamiento.md).
