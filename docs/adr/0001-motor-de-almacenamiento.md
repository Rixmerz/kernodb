# ADR-0001 — Motor de almacenamiento físico

| Campo | Valor |
|---|---|
| Estado | **Aceptado** (2026-10-28, por el propietario del proyecto) — con la evidencia de M0 (410 ejecuciones de benchmark, suites de fallos con 400 y 1000 semillas) y las condiciones de la decisión como trabajo obligatorio de Fase 1 |
| Fecha | 2026-10-27 (propuesta) · 2026-10-28 (aceptación) |
| Relacionado con | SAD §7.1, §15, §17; ADR-0002, ADR-0003, ADR-0004, ADR-0009, ADR-0012; [informe M0](../testing/m0-report.md); [matriz de fallos](../architecture/failure-matrix.md) |

## Contexto

Kerno necesita un núcleo key-value transaccional, embebido y local-first sobre el que construir documentos, revisiones, índices y recuperación para IA. La arquitectura física condiciona el formato persistente, el protocolo de commit, la recuperación, el coste del historial y la supresión de datos, y cambiarla después sería una migración, no una refactorización. Por eso SAD §7.1 y D-11 exigen decidir entre **B+ tree copy-on-write** y **LSM tree** con una prueba reproducible, no por popularidad ni por cifras de terceros.

## Opciones consideradas

1. **A — B+ tree copy-on-write** (shadow paging, estilo LMDB/redb): un archivo de páginas, meta doble, MVCC por retención de páginas, sin hilos de fondo.
2. **B — LSM nivelado** (estilo LevelDB/RocksDB): WAL + memtable + SST inmutables + compactación en segundo plano, MVCC por versiones de archivos.
3. **C — Tercera vía** (adoptar un motor existente como capa de almacenamiento, o un híbrido con dos motores primarios): solo si la evidencia lo justificara. SAD §15 plantea ADR-0001 como implementación propia de uno de los dos enfoques y admite una tercera opción «solo si la evidencia la justifica»; los principios de núcleo pequeño y de medir antes de especializar (SAD §3, principios 2 y 8) y el plan maestro de Kerno descartan dos motores primarios en la primera versión.

## Criterios de decisión (fijados antes de medir)

De SAD §15 y §17.5: correctitud; latencias p50/p95/p99; throughput; tiempo de recuperación; RAM máxima; bytes escritos y tamaño en disco frente a datos; complejidad operacional; comportamiento en recuperación, compactación y rangos; compatibilidad futura con el Revision Engine. Metodología en ADR-0012. Los casos de referencia (LiveSpec, Cauce, memoria de agentes) pesan más que los micro-benchmarks.

## Evidencia

Todo el detalle está en el [informe de M0](../testing/m0-report.md); aquí, lo que decide.

### 1. Correctitud: requisito cumplido por ambos

Ambos prototipos pasan, con 400 semillas, la misma batería compartida (el B+ tree, sin la variante `Buffered` + corte de energía, cuya garantía no ofrece; §2): modelo aleatorio, aislamiento, concurrencia, caídas en puntos aleatorios y en **cada** operación de E/S (muerte de proceso y corte de energía, con desgarro a 512 B y a nivel de byte), un EIO aislado en cada operación, bit flips dirigidos y truncamientos, y una prueba diferencial contra redb, fjall y SQLite. Ninguno devolvió nunca datos erróneos ni retrocedió en silencio.

Llegar ahí exigió corregir defectos reales en los dos, hallados por el harness y por una revisión adversarial con reproducción independiente: en el B+ tree, uno de severidad **alta** (la apertura confiaba en un meta no durable y podía destruir el único árbol durable) y tres medios; en el LSM, uno **medio-alto** (daño a mitad del WAL tomado por cola rota, con pérdida silenciosa de commits confirmados) y uno medio. **La correctitud no desempata**: ambas arquitecturas tienen protocolos con sutilezas, aunque en sitios distintos (invariantes de meta y páginas libres en el B+ tree; ciclo de vida de archivos y concurrencia con hilos de fondo en el LSM).

### 2. Durabilidad sin `fsync` por commit

| | B+ tree CoW | LSM |
|---|---|---|
| Corte de energía en modo `Buffered` | estado exacto de algún commit **o** corrupción detectada; ~87 % de los cortes simulados dejaron la base **sin poder abrirse** (y otro 12 %, con daño detectado al leer) | **prefijo consistente** de commits, siempre |

Ofrecer en un CoW la misma garantía requiere puntos de sincronización y reutilización diferida de páginas (diseño esbozado, no implementado). En el LSM sale del propio diseño del WAL. Kerno quiere ofrecer un modo de durabilidad relajada sin comprometer la consistencia (ADR-0002), así que esto pesa.

### 3. Rendimiento (escala `medium`, 200 000 registros, `Sync` salvo indicación, mediana de 3 repeticiones; las cifras `large` son de una sola ejecución)

| Workload | Métrica | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| **Cauce** (commit durable + lecturas) | pasos/s | 1 111 | **3 087** | 2,8× | 1 748 | 2 941 | 2 187 |
| | p99 commit · p99 lectura (µs) | 1 966 · **76** | **713** · 98 | | 1 213 · 59 | 658 · 118 | 1 078 · 151 |
| | recuperación tras cierre sucio (ms) | **0,9** | 9,7 | | 1,1 | 363 | 0,7 |
| **Cauce** `Buffered` (`small`) | pasos/s | 16 250 | **57 110** | 3,5× | 15 680 † | 61 490 | 7 413 |
| **LiveSpec** | change sets/s | 408 | **667** | 1,6× | 524 | 522 | 468 |
| | p99 commit · p99 listado de módulo (µs) | **3 762 · 440** | 4 764 · 657 | | 3 148 · 377 | 2 425 · 1 296 | 12 530 · 346 |
| **Memoria de agentes** | pasos/s (`medium` · `small`) | **1 073** · 1 377 | 1 037 · **1 642** | 0,97× · 1,19× | 1 098 · 1 595 | 1 279 · 2 166 | 704 · 832 |
| | p99 escritura (µs) | **1 573** | 5 837 | | 2 128 | 1 090 | 1 613 |
| **Historial simulado** (escritura) | revisiones/s (miles) | 8,3 | **66,6** | 8,0× | 11,4 | 61,6 | 11,4 |
| | p99 commit de 100 revisiones (µs) | 17 870 | **9 118** | | 13 480 | 3 168 | 26 800 |
| **Historial simulado** (lectura) | p50 historial de un registro (µs) | **7,6** | 14,4 | | 5,9 | 20,3 | 9,8 |
| Commits de un registro | commits/s | 1 542 | **4 044** | 2,6× | 2 499 | 4 054 | 4 113 |
| | p99 commit (µs) | 1 331 | **584** | | 951 | 616 | 630 |
| | `fsync` por commit | 2 | **1** | | — | — | — |
| | amplificación de escritura | 112× | **1,1×** | | 132× | 1,2× | 37× |
| Commits de un registro `Buffered` (`small`) | commits/s | 31 080 | **200 800** | 6,5× | 18 770 † | 311 600 | 27 760 |
| Borrados puntuales | commits/s | 1 365 | **4 343** | 3,2× | 2 480 | 3 991 | 3 202 |
| Carga masiva | kops/s (`medium` · `large`) | 20,1 · 10,4 | **162 · 110** | 8,0× · 10,6× | 26,2 · 18,5 | 133 · 131 | 12,6 · 10,3 |
| | espacio en disco / datos | 1,65 | **1,03** | | 2,36 | 1,58 | 1,48 |
| | RAM máxima (MB) | 89 | **25** | | 72 | 136 | 40 |
| Mixto 90/10 (4 hilos) | kops/s | 11,6 | **28,0** | 2,4× | 15,3 | 30,3 | 25,3 |
| | p99 lectura · p99 escritura (µs) | 84 · 7 274 | **62 · 3 817** | | 128 · 6 488 | 45 · 3 504 | 237 · 3 916 |
| Lecturas puntuales, caché caliente | kops/s 1 hilo · p99 (µs) | 188 · 12,5 | **230** · 12,5 | 1,2× | 273 · 9,4 | 235 · 9,0 | 138 · 21,3 |
| Lecturas puntuales, caché fría | kops/s | 57 | **117** | 2,1× | 65 | 143 | 52 |
| Escaneo de 100, caché caliente / fría | p99 (µs) | 171 / 1 758 | **146 / 521** | | 182 / 1 652 | 140 / 310 | 167 / 1 745 |
| Inserción secuencial + actualizaciones | kops/s actualización | 93 | **115** | 1,2× | 111 | 86 | 121 |
| | p99 commit de 100 actualizaciones (µs) | **2 306** | 6 840 | | 1 732 | 2 634 | 7 999 |
| | amplificación de escritura · espacio / datos | **2,3× · 1,62** | 3,4× · 2,08 | | 2,9× · 2,11 | 1,7× · 3,24 | 3,2× · 1,34 |
| Carga masiva: reapertura limpia · cierre limpio | ms | **1,0 · 12** | 3,4 · 759 | | 1,2 · 8,7 | 392 · 96 | 0,5 · 13 |

† redb `Durability::None` no sobrevive a la muerte del proceso: cifra no comparable (ADR-0012).

Entorno: VM con 4 núcleos y `fdatasync` p50 de 113 µs (p99 568 µs), 410 ejecuciones sin fallos; detalle, rangos y escalas `small` y `large` en el [informe de M0 §3](../testing/m0-report.md#3-rendimiento).

Lectura de los datos:

- **El LSM reproduce el perfil de fjall; el B+ tree queda por debajo de redb salvo en inserción secuencial.** En `medium` y modo `Sync`, el prototipo LSM rinde entre 0,8× y 1,3× fjall; el B+ tree, entre 0,55× y 0,8× redb en Cauce, LiveSpec, commits de un registro, borrados, mixto, historial y carga masiva (en parte por sus dos `fsync` por commit frente a uno), a la par en memoria de agentes y por encima en inserción secuencial (1,3×). Por eso las razones LSM / B+ incluyen también diferencias de madurez (riesgo R-S1). **Entre los motores maduros (fjall frente a redb) la ventaja de la arquitectura LSM se mantiene y en general es menor**: empate en LiveSpec (1,0×), 1,6× en commits de un registro y en borrados, 1,7× en Cauce, 2,0× en el mixto y 5,1–5,4× en carga masiva y escritura de historial; solo en memoria de agentes es mayor que entre los prototipos (1,2× frente a 0,97×); redb gana en actualizaciones secuenciales (fjall 0,78×) y en lecturas puntuales con un hilo (0,86×).
- **Rendimiento de escritura: el LSM gana en casi todo, con márgenes muy distintos.** En los casos de referencia va de un empate (memoria de agentes: 0,97× en `medium`, 1,19× en `small`, con valores de 1–4 KiB y embeddings de 3 KiB) a 1,6× (LiveSpec), 2,8× (Cauce) y 8,0× (escritura de historial). En micro-benchmarks, de 1,2–1,3× (inserción secuencial) a 2,6× (commits durables de un registro), 6,5× (commits sin `fsync`) y 8–10,6× (carga masiva aleatoria). En un CoW cada commit reescribe la ruta raíz-hoja de cada hoja tocada más metadatos; con dos `fsync` (prototipo) o uno (redb), el techo de commits durables queda por debajo del LSM.
- **Bytes escritos y espacio: a favor del LSM salvo en escritura secuencial y a escala `small`.** Escribe ~100× menos bytes por commit pequeño y ocupa menos tras una carga y en memoria de agentes en `medium` (2,64 frente a 3,93); con inserciones secuenciales y actualizaciones ocupa más (2,08 frente a 1,62) y, en `medium`, también escribe más (3,4× frente a 2,3×); a escala `small` ocupa más en el historial (2,98 frente a 2,32) y en memoria de agentes (2,03 frente a 1,64).
- **Latencia de cola: depende del workload.** El LSM tiene mejor p99 en los commits durables pequeños (Cauce 2,8×, commits de un registro 2,3×, escritura en el mixto 1,9×, historial 2,0×, borrados 3,2×). El B+ tree lo tiene en `medium` cuando la compactación del LSM coincide con escritura sostenida (memoria de agentes 3,7×, actualizaciones secuenciales 3,0×, inserción secuencial 2,1×, commit de LiveSpec 1,3×; en `small`, con poca compactación, esas cuatro colas las gana el LSM) y en lecturas cortas de rangos pequeños (listado de módulo 1,5×, recuperación de recuerdos 2,0×, lectura de Cauce 1,3×).
- **Lecturas con caché caliente: parecidas**; el B+ tree gana en el historial de un registro (1,9×, un rango contiguo dentro de una hoja) y en escaneos a escala `small`, y el LSM en lecturas puntuales y en escaneos desde `medium`. **Con caché fría gana el LSM** (2,1× en lecturas puntuales, p99 de escaneo 3,4× menor): la asignación copy-on-write dispersa las páginas y las SST son secuenciales.
- **Apertura y cierre: gana el B+ tree** (reapertura de 1–2 ms frente a 3–18 ms en `medium` y `large`; cierre de 12 ms frente a 0,76 s en `medium` y de 22 ms frente a 3,6 s en `large`, porque el LSM compacta antes de cerrar).
- **Memoria:** el LSM usa mucha menos RAM al cargar (25 frente a 89 MB), pero algo más en memoria de agentes y LiveSpec, por sus memtables.

### 4. Criterios cualitativos

| Criterio | B+ tree CoW | LSM |
|---|---|---|
| Complejidad operacional | **Menor**: sin hilos de fondo, sin compactación ni ajustes | Mayor: compactación, backpressure, ciclo de vida de archivos, ajustes por nivel |
| Recuperación | **Trivial**: elegir meta + `fsync`; sin reproducción | Reproducción del WAL + reescritura a L0 + MANIFEST; reentrante y probada |
| Compactación | No hay; el archivo no se encoge sin compactación offline | Continua; amplificación de escritura en segundo plano; tombstones |
| Rangos | Recorrido directo de hojas | Fusión de varias fuentes; necesita bloom + índices |
| Compatibilidad con Revision Engine | Cada revisión es una escritura aleatoria más (caro: 8,0× menos revisiones/s que el LSM en el prototipo, 5,4× redb frente a fjall; 112× bytes en commits de un registro) | Las revisiones son escrituras secuenciales en WAL/memtable (barato); lectura de historial algo más lenta |
| Retención y supresión (ADR-0009) | Páginas liberadas conservan bytes hasta reutilizarse; borrar rangos libera hojas | Versiones antiguas en SST/WAL hasta compactar; necesita *range tombstones* y compactación forzada |
| Cifrado (ADR-0006) | Nonce nuevo por página reescrita | Archivos inmutables: se cifran una vez |
| Snapshots largos | Retienen páginas (el archivo crece) | Retienen archivos y versiones (el directorio crece) |

## Decisión

**Adoptar un LSM nivelado como arquitectura de almacenamiento del núcleo de Kerno**, partiendo del diseño y las pruebas del prototipo `spike-lsm`, con estas condiciones antes de congelar el formato (ADR-0003):

1. **Colas de lectura y escritura acotadas**: ajustar compactación (subcompactaciones o límites por ronda, límites de solapamiento con el nivel abuelo), presupuesto de compactación en `close()` y métricas de stalls; criterio de aceptación con los p99 de LiveSpec, Cauce y memoria de agentes frente al baseline de M0 (el p99 de escritura en memoria de agentes, 3,7× el del B+ tree, es el caso a resolver).
2. **Range tombstones y compactación dirigida** de rangos, requisito de la retención y supresión de revisiones (ADR-0004, ADR-0009).
3. **MANIFEST como log de ediciones** con marca de cierre limpio, y **group commit** para escritores concurrentes.
4. Mantener **el prototipo B+ tree y todo el harness** como oráculo diferencial y como referencia de rendimiento de lectura.
5. **Revisar esta decisión** si un caso de uso real resulta dominado por lecturas de cola baja con pocas escrituras (disparador: incumplir los objetivos de p99 de lectura de LiveSpec tras las optimizaciones del punto 1).

### Por qué no las otras opciones

- **B+ tree CoW**: más simple de operar, con apertura y cierre inmediatos y mejores colas cuando la compactación del LSM compite con escrituras sostenidas, pero los casos de referencia de Kerno son intensivos en escritura (Cauce: commits pequeños durables; memoria de agentes: ingesta continua; Revision Engine: cada cambio escribe además una revisión y un `ChangeSet`). Ahí el prototipo CoW rinde entre lo mismo (memoria de agentes) y 8× menos (escritura de historial), con 2,8× menos en Cauce y 1,6× menos en LiveSpec; entre motores maduros (redb frente a fjall) la distancia va en el mismo sentido y en general es menor (de un empate en LiveSpec a 1,7× en Cauce y 5,4× en historial; solo en memoria de agentes es mayor, 1,2×); escribe ~100× más bytes por commit pequeño, tiene peores colas en commits durables pequeños, lee peor con caché fría y no ofrece un modo de durabilidad relajada consistente sin trabajo adicional sustancial.
- **Tercera vía**: en días de desarrollo, los prototipos quedaron cerca de los motores maduros de su familia —el LSM, a la par de fjall (0,8–1,3× en `medium`); el B+ tree, dentro del objetivo de 2× de redb de ADR-0012 (0,55–0,8× en la mayoría de workloads, a la par en memoria de agentes y por encima en inserción secuencial)— y superaron una batería de fallos más exigente que la de una prueba funcional, así que la evidencia **no** obliga a renunciar al núcleo propio. Un híbrido con dos motores primarios contradice el principio de núcleo pequeño (SAD §3, principio 2). Adoptar un motor externo sigue siendo un plan de contingencia si la Fase 1 no cumple sus criterios de salida.

## Consecuencias

- **Decisiones que ahora dependen del almacenamiento**: protocolo de commit (WAL + MANIFEST, ADR-0002), formato (WAL, SST, MANIFEST con cabeceras versionadas, ADR-0003), coste y diseño de retención/supresión (compactación dirigida, ADR-0009), unidad de cifrado (archivo/bloque inmutable, ADR-0006), métricas operativas (niveles, compactación, stalls).
- **Decisiones que siguen desacopladas**: semántica de transacciones y errores (ADR-0002), modelo de revisiones sobre claves (ADR-0004; el diseño solo exige batches atómicos y scans), identificadores (ADR-0005), índices derivados con watermark de secuencia (ADR-0007), serialización (ADR-0008), API (ADR-0010), vectores (ADR-0011).
- La Fase 1 empieza con un motor con hilos de fondo: las pruebas de concurrencia y de fallos durante compactación son obligatorias desde el primer día (el harness de M0 ya las incluye).
- El cierre limpio cuesta más (compacta hasta estabilizar); se acotará con un presupuesto.

## Seguimiento

- Aceptado el 2026-10-28 junto con ADR-0002 y ADR-0003; M0 queda cerrado.
- Repetir los benchmarks de M0 en el hardware objetivo (riesgo R-S3) antes de cerrar la Fase 1.
- Implementar los puntos 1–3 de la decisión en Fase 1, con criterios de salida de ADR-0012.
- **Seguimiento de la Fase 1 (WP14, 2026-10-29; sin cambios de decisión).** El motor del producto (`kerno-storage`) implementa las condiciones de esta decisión sobre el formato v1 ([`docs/file-format/v1.md`](../file-format/v1.md), interpretaciones en [ADR-0013](0013-formato-v1-y-protocolo-del-motor.md), aún «Propuesto»). La medición de salida en la clase de VM de M0 **no cumple T1** (condición 1) y deja otros incumplimientos documentados en el [informe de la Fase 1](../testing/f1-report.md); la revisión de contingencia (R-10/R-11) la decide el propietario. La repetición en el hardware objetivo (R-S3) sigue abierta.
