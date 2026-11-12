# Roadmap

El plan completo, con objetivos y salidas por fase, está en [SAD §16](../architecture/SAD.md#16-plan-de-implementación). Este archivo registra **dónde estamos**.

| Fase | Contenido | Estado |
|---|---|---|
| 0 / M0 | Spike de almacenamiento, harness, ADR iniciales, invariantes, matriz de fallos | **Cerrada** (2026-10-28): ADR-0001/0002/0003 aceptados — ver checklist |
| 1 | Núcleo persistente | **En curso** |
| 2 | Transacciones y documentos | — |
| 3 | Versionado (Revision Engine) | **Cerrada** (Etapa 3, 2026-11-04): riesgos abiertos K-E3-8 y K-E3-9 ([cierre](../architecture/etapa3-cierre.md)) |
| 4 | Seguridad y ciclo de vida | — |
| 5 | Búsqueda e IA | — |
| 6 | Beta endurecida | — |

## Checklist de M0 (SAD §17.5)

| # | Criterio | Estado | Dónde |
|---|---|---|---|
| 1 | ADR-0001 selecciona un enfoque o documenta por qué ninguno debe implementarse todavía | Hecho: **LSM nivelado**, aceptado el 2026-10-28 | [ADR-0001](../adr/0001-motor-de-almacenamiento.md) |
| 2 | Informe reproducible con código, comandos, configuración y resultados | Hecho | [informe M0](../testing/m0-report.md), [`run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh) |
| 3 | Matriz de puntos de fallo y estado esperado tras recuperación | Hecho | [matriz de fallos](../architecture/failure-matrix.md) |
| 4 | Significado exacto de commit y durabilidad propuestos | Hecho: aceptado el 2026-10-28 | [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) |
| 5 | Comparación por complejidad, recuperación, compactación, rangos y compatibilidad con Revision Engine | Hecho | ADR-0001 §Evidencia (3 y 4), [diseño de revisiones](../architecture/revision-model-design.md) |
| 6 | Decisiones que dependen del almacenamiento frente a las desacopladas | Hecho | ADR-0001 §Consecuencias |
| 7 | Sin dependencia funcional obligatoria de IA, red o proveedor externo | Hecho | los prototipos solo dependen de `crc32c`, `parking_lot`, `quick_cache` y `crossbeam-skiplist`; los motores de referencia solo se usan en el benchmark |

## Tareas inmediatas de SAD §20

| # | Tarea | Estado |
|---|---|---|
| 1 | Repositorio, licencia y convenciones de contribución | Hecho: `MIT OR Apache-2.0` ([LICENSE-MIT](../../LICENSE-MIT), [LICENSE-APACHE](../../LICENSE-APACHE)) y [`CONTRIBUTING.md`](../../CONTRIBUTING.md) |
| 2 | SAD como baseline y carpeta `docs/adr/` | Hecho |
| 3 | ADR-0001 y ADR-0002 como borradores | Hecho (con evidencia) |
| 4 | Harness de pruebas y benchmark de M0 | Hecho (+ CI en `.github/workflows/ci.yml`) |
| 5 | Spike B+ tree frente a LSM con resultados reproducibles | Hecho |
| 6 | Aprobar formato y semántica de commit antes de crear bases de usuarios | Semántica de commit (ADR-0002) y reglas de formato (ADR-0003) aceptadas; el formato v1 concreto se especifica y congela en Fase 1 |
| 7 | Núcleo persistente solo tras cerrar M0 | M0 cerrado; Fase 1 en curso |

## Fase 1: plan de trabajo

1. Especificar el formato v1 del motor LSM según ADR-0003 (cabeceras versionadas, bits de características, espacio reservado para AEAD) con vectores de prueba.
2. Crear los primeros crates del producto (`kerno-core`, `kerno-storage`) a partir de `spike-lsm`, conservando el harness del M0 (ahora en `kerno-testkit`) como suite obligatoria.
3. Condiciones de ADR-0001: colas de latencia acotadas (compactación y cierre con presupuesto), *range tombstones* y compactación dirigida, MANIFEST como log de ediciones, group commit.
4. Transacciones read-modify-write, `sync()` explícito y métricas de snapshots (ADR-0002).
5. Repetir los benchmarks de M0 en el hardware objetivo.

### Estado de los paquetes de trabajo (WP01–WP14)

Estado a 2026-10-29. WP01–WP13b están integrados y probados en `kerno-storage`, `kerno-testkit` y `kerno-core`; el estado de cada decisión pendiente está en el [informe de la Fase 1](../testing/f1-report.md).

| Paquete | Estado |
|---|---|
| WP01–WP13b | Integrados (formato v1, motor LSM, MANIFEST como log de ediciones, *range tombstones*, lectura de solo lectura, group commit, transacciones read-modify-write) |
| WP14 | Integrado **parcialmente**: `kill -9` real con 200 iteraciones y barrido de 1000 semillas en verde; benchmarks medidos en la clase de VM de M0 con **T1 sin cumplir** y otros incumplimientos documentados. **Pendientes**: registro de LazyFS, benchmarks en la máquina objetivo (C-09), escala `large` completa. La **congelación del formato v1** se ejecutó el 2026-10-31 con ADR-0013 aceptado ese día: esta celda decía que seguía pendiente y que ADR-0013 estaba en «Propuesto», texto anterior desactualizado |

La Fase 1 **no estaba cerrada** a 2026-10-29: la condición de inicio de la etapa 3 (formato v1 congelado con ADR-0013 aceptado) no se cumplía entonces, y la revisión de contingencia de ADR-0001 por T1 es una decisión abierta del propietario. Estado a 2026-11-11: el formato v1 se congeló y ADR-0013 quedó aceptado el 2026-10-31, y la Fase 1 quedó cerrada ([acta E3-00](../architecture/etapa3-acta-e3-00.md)).

### Benchmarks de cierre de la Fase 1 y SQLite como baseline

La tarea de cierre de la Fase 1 (revisión adversarial, benchmarks, documentación) mide `kerno` con `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` ([objetivos T1–T10](../testing/f1-objetivos-rendimiento.md)). A petición de la [especificación v0.2.0](../spec/README.md) §15, el informe presentará además **SQLite como baseline explícito**: una tabla `kerno` / SQLite por workload, con durabilidad equivalente declarada (`Sync` frente a `journal_mode=WAL` + `synchronous=FULL`; `Buffered` frente a `synchronous=NORMAL`, cuya garantía ante corte de energía no es idéntica y se anota). Esto **no** añade umbrales nuevos a T1–T10 ni reemplaza los resultados de M0 (B+ tree, LSM, redb, fjall y SQLite del 2026-10-27), que se conservan en [`spikes/storage/results/`](../../spikes/storage/results/) y en el [informe M0](../testing/m0-report.md). Los umbrales frente a SQLite, si se quieren, los fija el propietario antes de medir (ADR-0012).

## Etapas 3–9 (especificación v0.2.0)

Después de la Fase 1 el trabajo se organiza por las etapas de la [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §16; la [correspondencia con las fases del SAD](../spec/README.md) está en `docs/spec/`. La **Etapa 3 está cerrada** (aprobada por el propietario el 2026-11-04). La **Etapa 4 está cerrada** (aprobada por el propietario el 2026-11-05, [cierre](../architecture/etapa4-cierre.md)); la **Etapa 5 está cerrada** (aprobada por delegación del propietario el 2026-11-09, [cierre](../architecture/etapa5-cierre.md)); la **Etapa 6 está cerrada** (aprobada por delegación el 2026-11-11, [cierre](../architecture/etapa6-cierre.md)), con la caché desactivada por defecto; la **Etapa 7 está cerrada y aprobada por el propietario el 2026-11-12** ([cierre](../architecture/etapa7-cierre.md); acta aprobada el 2026-11-11, [acta](../architecture/etapa7-acta.md)), con **E7-00 a E7-06 hechos** (E7-00 a E7-05 el 2026-11-11: conjunto de evaluación congelado y ADR-0011 aceptado; modelo de consulta asociativa y ruta de referencia; ejecución por índice; contexto temporal y procedencia verificables; ranking con modo diagnóstico; medición, en la que el ranking no se activa por defecto). Una etapa ya implementada y probada se salta (spec §16).

| Etapa | Depende de | Condición de inicio | ADR que se deciden |
|---|---|---|---|
| 3 Unidades, revisiones y procedencia | Etapa 2 (Fase 1) | Fase 1 cerrada: WP14 integrado, formato v1 congelado con ADR-0013 aceptado (congelación ejecutada el 2026-10-31; a 2026-10-29 estaba **pendiente**: ver el estado de WP14 arriba) | 0004, 0008, 0010, 0017 |
| 4 Relaciones e índices | 3 | API de unidades estable (ADR-0010 aceptado) | 0007, 0014 |
| 5 Planificador y observabilidad | 4 | Consultas por filtro y relación con ruta de referencia probada | 0015 |
| 6 Primera plasticidad operacional | 5 | Planificador determinista con prueba diferencial en CI | 0016 |
| 7 Memoria asociativa | 4, 5 | Recorridos de relaciones y explicación de planes disponibles | 0011 |
| 8 Consolidación y embeddings | 7, 6 | ADR-0018 aceptado el 2026-11-12; supresión de derivados definida (ADR-0009, **cumplida el 2026-11-12**) | 0009, 0018, 0020 (persistencia de vectores, aceptado el 2026-11-12 por delegación del propietario) |
| 9 Endurecimiento | todas las anteriores (las partes de seguridad pueden adelantarse) | Ver decisión pendiente sobre el orden | 0005, 0006, 0012 |

### Etapa 3 — Unidades, revisiones y procedencia

- **Entregables:** tipos `Unit`, `Revision` y `ChangeSet` (spec §6.1–6.3) sobre `kerno-storage`; CRUD transaccional y archivado; listar, comparar y restaurar revisiones; políticas de historial `none`/`audit`/`versioned` (SAD §8); serialización y hashes (ADR-0008); primera API pública tipada (ADR-0010).
- **Aceptación:** se reconstruye la evolución de una unidad y se restaura sin destruir revisiones posteriores (I-22); el historial no depende del WAL (I-20); contenido y revisión se confirman en la misma frontera atómica (I-21), también en la matriz de fallos.
- **Estado: cerrada** el 2026-11-04 ([cierre de la Etapa 3](../architecture/etapa3-cierre.md)). Los criterios de aceptación se cumplen; IU-1…IU-10 quedan promovidas. Dos métricas incumplen sus umbrales y el propietario lo aceptó como riesgo abierto, sin rebajar ningún umbral: **K-E3-8** (U2: `history` llega a 1,50× de la referencia en una pasada de tres) y **K-E3-9** (U6: con retención concurrente, p99 de escritura 1,05–1,18× y rendimiento 0,89–0,94×, 1,06× en una pasada). **K-E3-7** (las lecturas comprueban menos que `verify_units`) queda aceptado por diseño, con revisión recomendada en la Etapa 9.

### Etapa 4 — Relaciones e índices

- **Estado: cerrada, aprobada por el propietario el 2026-11-05** ([etapa4-cierre.md](../architecture/etapa4-cierre.md), 2026-11-05; [acta](../architecture/etapa4-acta.md) aprobada el 2026-11-04). Relaciones declaradas (ADR-0014, opción 2), ruta de referencia, índice primario con estado y watermark (ADR-0007) e `IDX_TYPE` justificado por el workload W1. Los cinco criterios se cumplen y la revisión adversarial no dejó bloqueantes. I-21 (relaciones), I-23, I-24, I-32 (índices) e I-39 (declaradas) quedan promovidas y aprobadas. Siguen abiertos K-E3-8 y K-E3-9 (U6 no re-medido con índices). ADR-0007: los índices de unidades siguen derivados (solo `rebuild_index`) hasta que un workload con escrituras intercaladas pida mantenerlos en cada `ChangeSet`.
- **Entregables:** relaciones tipadas declaradas (ADR-0014); índice primario y filtros básicos; índices secundarios solo con un workload que los justifique; ruta de referencia para todas las consultas.
- **Aceptación:** los resultados por índice coinciden con la ruta de referencia (I-32 aplicado a índices); índices reconstruidos equivalentes a nuevos (I-24); estado de frescura expuesto (I-23); una relación declarada nunca se confunde con una inferida (I-39).

### Etapa 5 — Planificador y observabilidad

- **Entregables:** planificador determinista (spec §9: validar, generar candidatos, estimar coste, elegir, ejecutar, registrar, validar invariantes); explicación del plan; `ExecutionObservation` con niveles de detalle ([ejecución adaptativa](../architecture/adaptive-execution.md), ADR-0015).
- **Aceptación:** el plan puede inspeccionarse; prueba diferencial planificador frente a ruta de referencia en CI (I-32); prueba de centinelas sin literales ni contenido en observaciones (I-37).
- **Estado: cerrada el 2026-11-06, aprobada por delegación del propietario el 2026-11-09** ([cierre de la Etapa 5](../architecture/etapa5-cierre.md)). Planificador de los caminos existentes con explicación del plan (E5-01, E5-02), `ExecutionObservation` con niveles y retención en memoria (E5-03) y centinelas (E5-04). Los criterios de la acta §4 se cumplen; el de fallback con estado `Building` se cumple solo a nivel de plan (un snapshot no ve ese estado). Riesgos abiertos: K-E3-8, K-E3-9 (U6 sigue incumpliendo con índices y relaciones, E5-00), y sobrecoste del planificador en W1 fuera de umbral en una medición de seis (R-E5-1). I-32 (planificador) e I-37 quedan promovidas.
- **Acta:** [etapa5-acta.md](../architecture/etapa5-acta.md) (aprobada por el propietario el 2026-11-05).

### Etapa 6 — Primera plasticidad operacional

- **Entregables:** una única optimización reversible (plan reutilizable o caché invalidada por secuencia), con los metadatos de spec §9 y desactivación global y por consulta (ADR-0016).
- **Aceptación:** mejora neta medida frente al modo desactivado con umbrales fijados antes de medir; equivalencia de resultados (I-32, I-33); invalidación probada por dependencia (I-35); invalidar no altera datos (I-34); desactivación verificada (I-36).
- **Estado: cerrada el 2026-11-11, aprobada por delegación del propietario el 2026-11-11** ([cierre de la Etapa 6](../architecture/etapa6-cierre.md)). Única optimización: caché de resultados ligada a `commit_seq` (E6-01 a E6-03), usada por el planificador con su fallback registrado (E6-02); medición y centinelas (E6-04, E6-04b). **La caché queda desactivada por defecto.** Criterios: equivalencia, fallback, invalidación por dependencia, reversibilidad, desactivación y trazabilidad sin literales cumplidos; **mejora neta no afirmada**: el p50 de W1 cumple (−97,8 %), el p99 de W1 no se certifica (+0,5 % a +7,8 % en cuatro corridas) y el peor caso sin repeticiones cumple en la medición emparejada (coste real −0,6 % a +0,5 %) pero no en todas las corridas. Umbrales sin cambios. Riesgos abiertos: H-3, H-5, H-6 y H-7 del cierre. I-33 a I-36 quedan promovidas.

### Etapa 7 — Memoria asociativa

- **Entregables:** recuperación por relaciones, contexto temporal y procedencia; búsqueda vectorial **exacta** como baseline si hay un caso de uso (ADR-0011); ranking con modo diagnóstico que explica sus señales.
- **Aceptación:** calidad de recuperación evaluable con un conjunto de consultas y respuestas esperadas fijado antes de medir; toda respuesta rastreable a sus fuentes; sin ANN antes de un benchmark que lo justifique.
- **Acta:** [etapa7-acta.md](../architecture/etapa7-acta.md) (aprobada por delegación del propietario el 2026-11-11). Paquetes E7-00 a E7-06.
- **Estado:** E7-00 **hecho** el 2026-11-11: conjunto de evaluación congelado con SHA-256 ([conjunto](../architecture/etapa7-conjunto-evaluacion.md)), métricas probadas por mutación y líneas base B0 y B1 medidas; ADR-0011 aceptado. E7-01 **hecho** el 2026-11-11: `AssocQuery` con cotas duras (profundidad 4, 1 000 resultados, 100 000 visitas), ruta de referencia en anchura sobre `reference_relations_of` con orden determinista y rastro por resultado ([consulta asociativa](../architecture/etapa7-consulta-asociativa.md)). E7-02 **hecho** el 2026-11-11: el recorrido toma los vecinos del índice de adyacencia como camino del planificador (`assoc_query`, `explain_assoc`), cada respuesta comprobada y con caída registrada a la ruta de referencia; prueba diferencial en los dos motores, inyección de respuestas falsas, centinelas de I-37 y 29 mutaciones detectadas ([ejecución indexada](../architecture/etapa7-ejecucion-indexada.md)). E7-03 **hecho** el 2026-11-11: la ventana de `commit_seq` y los filtros de clase, actor y fuente se comprueban contra un oráculo de fuerza bruta en la ruta de referencia, la de índice y la del planificador, y cada relación y revisión que cita un rastro se encuentra en la instantánea (I-40), con 16 mutaciones detectadas y sin cambios en el código de producción ([contexto y procedencia](../architecture/etapa7-contexto-y-procedencia.md)). E7-04 **hecho** el 2026-11-11: ranking en `assoc_rank` con puntuación entera determinista, modo diagnóstico cuyas aportaciones suman la puntuación (I-41) y señal vectorial exacta opcional (ADR-0011); queda apagado por defecto hasta la medición ([ranking](../architecture/etapa7-ranking.md)). E7-05 **hecho** el 2026-11-11: el ranking no pasa a ser el orden por defecto, porque con sus pesos por defecto queda por debajo del orden simple «saltos, luego identificador» (nDCG@10 0,912 frente a 0,942) y el umbral de 1,10 veces ese orden no es alcanzable; el camino por índice cumple §6.3 (p50 0,15 ms frente a 105 ms a profundidad 2, en las doce corridas) ([medición](../architecture/etapa7-medicion.md)). E7-06 **hecho** el 2026-11-12: revisión adversarial sin bloqueantes, criterios de §4 de la acta cumplidos (I-32 con la limitación H-1 del índice, no afirmada para tiendas de 64 relaciones o más con una entrada ausente en ambos extremos), T4 no cumplido por su redacción, e I-40 e I-41 promovidas y aprobadas por el propietario el 2026-11-12 ([cierre](../architecture/etapa7-cierre.md)). **Estado de la etapa: cerrada y aprobada por el propietario el 2026-11-12.** Decisiones del propietario (cierre §8): la Etapa 8 arranca, con el ranking de resultados asociativos desactivado por defecto y marcado como experimental; T4 no se reescribe ahora, y si el ranking se vuelve a medir será con un conjunto nuevo y un umbral definido antes de medir.

### Etapa 8 — Consolidación y embeddings

- **Entregables:** registro común de derivados (ADR-0018); persistencia de vectores como derivados de la capa de unidades, con su bit de versión de capa 2 y sus criterios de aceptación (ADR-0020); adaptadores o jobs externos para embeddings y resúmenes, fuera del núcleo (I-31); obsolescencia y reconstrucción.
- **Aceptación:** cada derivado se vincula a sus fuentes y se invalida o reconstruye al cambiarlas (I-25, I-38); suprimir una fuente afecta a sus derivados según ADR-0009; un resumen nunca reemplaza el original auditable.
- **Estado: cerrada como trabajo de ingeniería el 2026-11-12** ([cierre de la Etapa 8](../architecture/etapa8-cierre.md)), con decisiones tomadas por delegación del propietario el 2026-11-12 y revocables por él. **Q4 no demostrada** (no cumplida). Puntos abiertos: Q2, Q3 y Q4 dan el valor máximo en los dos brazos (efecto techo); P2 no mide la lectura de vectores persistidos; los resúmenes son solo en memoria, sin medición de centinelas; el orden de bloqueo del commit guardado no tiene prueba; los vectores son deterministas con semilla y no dicen nada de un modelo real; corpus, consultas y juicios los escribió el mismo equipo; la lista de fricción de E8-09 tiene quince puntos a resolver antes de 1.0, y el punto 4 toca una promesa (el olvido es solo lógico y la compactación no es alcanzable por la envoltura pública); la mutación de las pruebas de E8-06 y E8-07 no tiene evidencia (F-10 del cierre). Sin hallazgo bloqueante abierto. **Paquete de seguimiento antes de 1.0: E8-10**, un segundo conjunto de evaluación congelado por separado, en el que cada clase de consulta lleva vectores de consulta, con consultas lo bastante difíciles para no saturar, umbrales fijados antes de medir y una sola medición. Historial: el arranque lo decidió el propietario ([cierre de la Etapa 7, §8.1](../architecture/etapa7-cierre.md)). El propietario aprobó el [acta de la Etapa 8](../architecture/etapa8-acta.md) el 2026-11-12, con las decisiones D1 a D12 (D10 en opción b, §8.1 del acta). **E8-00 hecho** el 2026-11-12: [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) aceptado con la opción 2, [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) aceptado con la opción 3 (E9A-00b) y escrito en su archivo, y cierre de la Etapa 7 aprobado. **E8-S0 (informe SQLite, sin umbrales) hecho** en `078839a` ([informe](../testing/sqlite-report.md)). La [acta de adelanto de la Etapa 9](../architecture/etapa9-adelanto-acta.md) propone además que E9A-02 esté cerrado antes de que la Etapa 8 escriba ningún derivado; esta entrada no decide esa propuesta.

### Etapa 9 — Endurecimiento

- **Entregables:** IDs públicos (ADR-0005), cifrado y claves (ADR-0006), auditoría minimizada, exportación/importación y migraciones con fixtures de versiones previas, fuzzing de entradas externas, pruebas de estrés, benchmarks comparativos con SQLite y motores de referencia (SAD Fases 4 y 6).
- **Aceptación:** límites y garantías documentados; fallos críticos resueltos o marcados como bloqueantes; I-27…I-30 verificados.
- **Adelanto (en paralelo con la Etapa 7):** [acta](../architecture/etapa9-adelanto-acta.md), aprobada por delegación del propietario el 2026-11-11. Paquetes E9A-00a a E9A-06. **Cierre del adelanto (E9A-06) hecho el 2026-11-12:** [cierre](../architecture/etapa9-adelanto-cierre.md). Ningún hallazgo bloqueante abierto; I-29 promovida para `kerno-storage`, aprobada por delegación del propietario el 2026-11-12 y revocable. I-27, I-28 e I-30 siguen pendientes (Fase 4).

### Decisiones pendientes del propietario para estas etapas

1. **Orden de seguridad frente a IA (resuelta el 2026-11-11 por delegación del propietario, en el [acta de adelanto de la Etapa 9](../architecture/etapa9-adelanto-acta.md), §10):** el SAD hace la seguridad (Fase 4) antes que búsqueda e IA (Fase 5); la especificación la deja en la Etapa 9. Propuesta: adelantar a antes de la Etapa 8 lo que afecta a derivados (supresión, ADR-0009; redacción de logs, I-29) y dejar el resto en la Etapa 9.
2. Aceptar o modificar ADR-0014…0018 en las etapas indicadas. ADR-0018 quedó aceptado el 2026-11-12 (opción 2).
3. Umbrales de benchmark frente a SQLite, si se quieren como criterio y no solo como informe.

### Decisiones del propietario de 2026-11-11

1. **Punto de corte en E7-05.** Si el ranking no cumple los umbrales de calidad del [acta de la Etapa 7](../architecture/etapa7-acta.md) (§6.2), la Etapa 8 no empieza hasta que el propietario decida cómo seguir. **Resuelto el 2026-11-12:** la Etapa 8 arranca con el ranking desactivado por defecto y marcado como experimental, y T4 no se reescribe ahora ([cierre de la Etapa 7, §8](../architecture/etapa7-cierre.md)).
2. **Comparación con SQLite antes de la Etapa 8, como informe sin umbrales.** La comparación con SQLite, entregable de la Etapa 9, se adelanta antes de la Etapa 8 como informe sin umbrales, sobre las cargas que ambos motores pueden ejecutar. La pendiente 3 (umbrales frente a SQLite) sigue abierta.
3. **Caso de uso real antes de 1.0.** Antes de un 1.0 se construye un caso de uso pequeño y real (memoria para un agente) que usa solo la API pública, como lo usaría un tercero.
