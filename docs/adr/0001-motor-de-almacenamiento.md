# ADR-0001 — Motor de almacenamiento físico

| Campo | Valor |
|---|---|
| Estado | **Propuesto** — recomendación con evidencia de M0 (360 ejecuciones de benchmark, suites de fallos con 400 semillas); pendiente de aprobación |
| Fecha | 2026-10-27 |
| Relacionado con | SAD §7.1, §15, §17; ADR-0002, ADR-0003, ADR-0004, ADR-0009, ADR-0012; [informe M0](../testing/m0-report.md); [matriz de fallos](../architecture/failure-matrix.md) |

## Contexto

Kerno necesita un núcleo key-value transaccional, embebido y local-first sobre el que construir documentos, revisiones, índices y recuperación para IA. La arquitectura física condiciona el formato persistente, el protocolo de commit, la recuperación, el coste del historial y la supresión de datos, y cambiarla después sería una migración, no una refactorización. Por eso SAD §7.1 y D-11 exigen decidir entre **B+ tree copy-on-write** y **LSM tree** con una prueba reproducible, no por popularidad ni por cifras de terceros.

## Opciones consideradas

1. **A — B+ tree copy-on-write** (shadow paging, estilo LMDB/redb): un archivo de páginas, meta doble, MVCC por retención de páginas, sin hilos de fondo.
2. **B — LSM nivelado** (estilo LevelDB/RocksDB): WAL + memtable + SST inmutables + compactación en segundo plano, MVCC por versiones de archivos.
3. **C — Tercera vía** (adoptar un motor existente como capa de almacenamiento, o un híbrido con dos motores primarios): solo si la evidencia lo justificara. El plan descarta dos motores primarios en la primera versión y prefiere un núcleo propio (SAD §13, decisión 1 del plan).

## Criterios de decisión (fijados antes de medir)

De SAD §15 y §17.5: correctitud; latencias p50/p95/p99; throughput; tiempo de recuperación; RAM máxima; bytes escritos y tamaño en disco frente a datos; complejidad operacional; comportamiento en recuperación, compactación y rangos; compatibilidad futura con el Revision Engine. Metodología en ADR-0012. Los casos de referencia (LiveSpec, Cauce, memoria de agentes) pesan más que los micro-benchmarks.

## Evidencia

Todo el detalle está en el [informe de M0](../testing/m0-report.md); aquí, lo que decide.

### 1. Correctitud: requisito cumplido por ambos

Ambos prototipos pasan, con 400 semillas, las mismas suites: modelo aleatorio, aislamiento, concurrencia, caídas en puntos aleatorios y en **cada** operación de E/S (muerte de proceso y corte de energía, con desgarro a 512 B y a nivel de byte), un EIO aislado en cada operación, bit flips dirigidos y truncamientos, y una prueba diferencial contra redb, fjall y SQLite. Ninguno devolvió nunca datos erróneos ni retrocedió en silencio.

Llegar ahí exigió corregir defectos reales en los dos, hallados por el harness y por una revisión adversarial con reproducción independiente: en el B+ tree, uno de severidad **alta** (la apertura confiaba en un meta no durable y podía destruir el único árbol durable) y tres medios; en el LSM, uno **medio-alto** (daño a mitad del WAL tomado por cola rota, con pérdida silenciosa de commits confirmados) y uno medio. **La correctitud no desempata**: ambas arquitecturas tienen protocolos con sutilezas, aunque en sitios distintos (invariantes de meta y páginas libres en el B+ tree; ciclo de vida de archivos y concurrencia con hilos de fondo en el LSM).

### 2. Durabilidad sin `fsync` por commit

| | B+ tree CoW | LSM |
|---|---|---|
| Corte de energía en modo `Buffered` | estado exacto de algún commit **o** corrupción detectada; ~85 % de los cortes simulados dejaron la base **sin poder abrirse** | **prefijo consistente** de commits, siempre |

Ofrecer en un CoW la misma garantía requiere puntos de sincronización y reutilización diferida de páginas (diseño esbozado, no implementado). En el LSM sale del propio diseño del WAL. Kerno quiere ofrecer un modo de durabilidad relajada sin comprometer la consistencia (ADR-0002), así que esto pesa.

### 3. Rendimiento (escala `medium`, 200 000 registros, `Sync` salvo indicación, mediana de 3 repeticiones)

| Workload | Métrica | B+ tree | LSM | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|
| **Cauce** (commit durable + lecturas) | pasos/s | 1 237 | **3 462** | 1 865 | 3 341 | 2 121 |
| **Cauce** `Buffered` (`small`) | pasos/s | 17 794 | **71 423** | 20 460 | 58 950 | 9 714 |
| **LiveSpec** | change sets/s | 398 | **637** | 525 | 631 | 445 |
| | p99 listado de módulo (µs) | **379** | 574 | 310 | 634 | 284 |
| **Memoria de agentes** | pasos/s | 1 016 | 1 074 | 1 035 | 1 224 | 662 |
| | p99 escritura (µs) | **1 818** | 5 255 | 2 284 | 1 194 | 1 595 |
| **Historial simulado** (escritura) | revisiones/s (miles) | 9,7 | **66,9** | 12,3 | 52,0 | 11,0 |
| **Historial simulado** (lectura) | p50 historial de un registro (µs) | **5,9** | 15,8 | 6,6 | 19,0 | 9,4 |
| Commits pequeños | commits/s | 1 718 | **4 943** | 2 930 | 5 248 | 4 800 |
| | `fsync` por commit | 2 | **1** | — | — | — |
| | amplificación de escritura | 112× | **1,1×** | 132× | 1,2× | 37× |
| Commits pequeños `Buffered` (`small`) | commits/s | 37 562 | **297 402** | 22 604 | 355 996 | 28 755 |
| Carga masiva | kops/s | 20,1 | **173** | 28,4 | 150 | 12,3 |
| | espacio en disco / datos | 1,65 | **1,03** | 2,36 | 1,58 | 1,48 |
| Mixto 90/10 (4 hilos) | kops/s | 10,3 | **29,1** | 14,6 | 25,8 | 24,0 |
| Lecturas puntuales, caché caliente | p99 (µs) | 12,9 | 12,2 | 11,8 | 10,4 | 21,1 |
| Lecturas puntuales, caché fría | kops/s | 83 | **157** | 93 | 189 | 74 |
| Escaneo de 100, caché caliente / fría | p99 (µs) | 180 / 1 182 | **123 / 297** | 157 / 1 125 | 148 / 293 | 157 / 1 099 |
| Actualizaciones sostenidas (append_seq) | p99 commit de 100 (µs) | **2 074** | 6 283 | 1 720 | 2 037 | 7 893 |
| Reapertura limpia · recuperación tras cierre sucio | ms | **1,2 · 0,7** | 3,3 · 11 | 0,8 · 1,0 | 336 · 354 | 0,5 · 0,5 |
| Cierre limpio | ms | **15** | 718 | 7,8 | 90 | 17 |
| RAM máxima (carga masiva) | MB | 89 | **25** | 72 | 136 | 73 |

Entorno: VM con 4 núcleos y `fdatasync` p50 de 117 µs; detalle, rangos y escala `small` en el [informe de M0 §3](../testing/m0-report.md#3-rendimiento).

Lectura de los datos:

- **Cada prototipo reproduce el perfil del motor maduro de su familia** (B+ tree ≈ redb, LSM ≈ fjall), a veces mejorándolo. Las diferencias medidas son, por tanto, de **arquitectura**, no de madurez de la implementación (riesgo R-S1 acotado).
- **Escrituras: el LSM gana en todo lo que escribe**, de 1,6× (LiveSpec) a 8× (commits sin `fsync`), y escribe ~100× menos bytes por commit pequeño. En memoria de agentes, con valores de 1–4 KiB y embeddings de 3 KiB, quedan empatados. En un CoW cada commit reescribe la ruta raíz-hoja de cada hoja tocada más metadatos; con dos `fsync` (prototipo) o uno (redb) el techo de commits durables queda por debajo del LSM.
- **Lecturas con caché caliente: empate**; el B+ tree gana en el historial de un registro (un rango contiguo dentro de una hoja) y el LSM en escaneos largos. **Con caché fría gana el LSM** (1,9× en lecturas puntuales, p99 de escaneo 4× menor): la asignación copy-on-write dispersa las páginas y las SST son secuenciales.
- **Latencia de cola bajo escritura sostenida, apertura y cierre: gana el B+ tree.** El LSM paga la compactación (p99 de actualizaciones sostenidas 3× peor, p99 de escritura en memoria de agentes 2,9× peor; cierre de ~0,7 s porque ahora compacta antes de cerrar).
- **Espacio y memoria: gana el LSM** (páginas medio llenas y caché de nodos en el B+ tree).

### 4. Criterios cualitativos

| Criterio | B+ tree CoW | LSM |
|---|---|---|
| Complejidad operacional | **Menor**: sin hilos de fondo, sin compactación ni ajustes | Mayor: compactación, backpressure, ciclo de vida de archivos, ajustes por nivel |
| Recuperación | **Trivial**: elegir meta + `fsync`; sin reproducción | Reproducción del WAL + reescritura a L0 + MANIFEST; reentrante y probada |
| Compactación | No hay; el archivo no se encoge sin compactación offline | Continua; amplificación de escritura en segundo plano; tombstones |
| Rangos | Recorrido directo de hojas | Fusión de varias fuentes; necesita bloom + índices |
| Compatibilidad con Revision Engine | Cada revisión es una escritura aleatoria más (caro: 112× bytes) | Las revisiones son escrituras secuenciales en WAL/memtable (barato); lectura de historial algo más lenta |
| Retención y supresión (ADR-0009) | Páginas liberadas conservan bytes hasta reutilizarse; borrar rangos libera hojas | Versiones antiguas en SST/WAL hasta compactar; necesita *range tombstones* y compactación forzada |
| Cifrado (ADR-0006) | Nonce nuevo por página reescrita | Archivos inmutables: se cifran una vez |
| Snapshots largos | Retienen páginas (el archivo crece) | Retienen archivos y versiones (el directorio crece) |

## Decisión propuesta

**Adoptar un LSM nivelado como arquitectura de almacenamiento del núcleo de Kerno**, partiendo del diseño y las pruebas del prototipo `spike-lsm`, con estas condiciones antes de congelar el formato (ADR-0003):

1. **Colas de lectura y escritura acotadas**: ajustar compactación (subcompactaciones o límites por ronda, límites de solapamiento con el nivel abuelo), presupuesto de compactación en `close()` y métricas de stalls; criterio de aceptación con los p99 de LiveSpec y Cauce frente al baseline de M0.
2. **Range tombstones y compactación dirigida** de rangos, requisito de la retención y supresión de revisiones (ADR-0004, ADR-0009).
3. **MANIFEST como log de ediciones** con marca de cierre limpio, y **group commit** para escritores concurrentes.
4. Mantener **el prototipo B+ tree y todo el harness** como oráculo diferencial y como referencia de rendimiento de lectura.
5. **Revisar esta decisión** si un caso de uso real resulta dominado por lecturas de cola baja con pocas escrituras (disparador: incumplir los objetivos de p99 de lectura de LiveSpec tras las optimizaciones del punto 1).

### Por qué no las otras opciones

- **B+ tree CoW**: más simple de operar y con mejores colas de latencia, pero los casos de referencia de Kerno son intensivos en escritura (Cauce: commits pequeños durables; memoria de agentes: ingesta continua; Revision Engine: cada cambio escribe además una revisión y un `ChangeSet`). Ahí el CoW es entre 1,6× y 8× más lento (2,8× en Cauce, 6,9× escribiendo historial), escribe ~100× más bytes por commit pequeño, lee peor con caché fría y no ofrece un modo de durabilidad relajada consistente sin trabajo adicional sustancial.
- **Tercera vía**: en días de desarrollo, los prototipos alcanzaron el rendimiento de los motores maduros de su familia y superaron una batería de fallos más exigente que la de una prueba funcional, así que la evidencia **no** obliga a renunciar al núcleo propio. Un híbrido con dos motores primarios contradice el alcance del SAD. Adoptar un motor externo sigue siendo un plan de contingencia si la Fase 1 no cumple sus criterios de salida.

## Consecuencias

- **Decisiones que ahora dependen del almacenamiento**: protocolo de commit (WAL + MANIFEST, ADR-0002), formato (WAL, SST, MANIFEST con cabeceras versionadas, ADR-0003), coste y diseño de retención/supresión (compactación dirigida, ADR-0009), unidad de cifrado (archivo/bloque inmutable, ADR-0006), métricas operativas (niveles, compactación, stalls).
- **Decisiones que siguen desacopladas**: semántica de transacciones y errores (ADR-0002), modelo de revisiones sobre claves (ADR-0004; el diseño solo exige batches atómicos y scans), identificadores (ADR-0005), índices derivados con watermark de secuencia (ADR-0007), serialización (ADR-0008), API (ADR-0010), vectores (ADR-0011).
- La Fase 1 empieza con un motor con hilos de fondo: las pruebas de concurrencia y de fallos durante compactación son obligatorias desde el primer día (el harness de M0 ya las incluye).
- El cierre limpio cuesta más (compacta hasta estabilizar); se acotará con un presupuesto.

## Seguimiento

- Aprobar o rechazar este ADR junto con ADR-0002 y ADR-0003.
- Repetir los benchmarks de M0 en el hardware objetivo (riesgo R-S3) antes de cerrar la Fase 1.
- Implementar los puntos 1–3 de la decisión en Fase 1, con criterios de salida de ADR-0012.
