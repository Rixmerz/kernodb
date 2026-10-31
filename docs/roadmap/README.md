# Roadmap

El plan completo, con objetivos y salidas por fase, está en [SAD §16](../architecture/SAD.md#16-plan-de-implementación). Este archivo registra **dónde estamos**.

| Fase | Contenido | Estado |
|---|---|---|
| 0 / M0 | Spike de almacenamiento, harness, ADR iniciales, invariantes, matriz de fallos | **Cerrada** (2026-10-28): ADR-0001/0002/0003 aceptados — ver checklist |
| 1 | Núcleo persistente | **En curso** |
| 2 | Transacciones y documentos | — |
| 3 | Versionado (Revision Engine) | — |
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
| WP14 | Integrado **parcialmente**: `kill -9` real con 200 iteraciones y barrido de 1000 semillas en verde; benchmarks medidos en la clase de VM de M0 con **T1 sin cumplir** y otros incumplimientos documentados. **Pendientes**: registro de LazyFS, benchmarks en la máquina objetivo (C-09), escala `large` completa y la **congelación del formato v1** (ADR-0013 sigue «Propuesto») |

La Fase 1 **no está cerrada**: la condición de inicio de la etapa 3 (formato v1 congelado con ADR-0013 aceptado) no se cumple todavía, y la revisión de contingencia de ADR-0001 por T1 es una decisión abierta del propietario.

### Benchmarks de cierre de la Fase 1 y SQLite como baseline

La tarea de cierre de la Fase 1 (revisión adversarial, benchmarks, documentación) mide `kerno` con `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` ([objetivos T1–T10](../testing/f1-objetivos-rendimiento.md)). A petición de la [especificación v0.2.0](../spec/README.md) §15, el informe presentará además **SQLite como baseline explícito**: una tabla `kerno` / SQLite por workload, con durabilidad equivalente declarada (`Sync` frente a `journal_mode=WAL` + `synchronous=FULL`; `Buffered` frente a `synchronous=NORMAL`, cuya garantía ante corte de energía no es idéntica y se anota). Esto **no** añade umbrales nuevos a T1–T10 ni reemplaza los resultados de M0 (B+ tree, LSM, redb, fjall y SQLite del 2026-10-27), que se conservan en [`spikes/storage/results/`](../../spikes/storage/results/) y en el [informe M0](../testing/m0-report.md). Los umbrales frente a SQLite, si se quieren, los fija el propietario antes de medir (ADR-0012).

## Etapas 3–9 (especificación v0.2.0)

Después de la Fase 1 el trabajo se organiza por las etapas de la [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §16; la [correspondencia con las fases del SAD](../spec/README.md) está en `docs/spec/`. Ninguna etapa ha empezado. Una etapa ya implementada y probada se salta (spec §16).

| Etapa | Depende de | Condición de inicio | ADR que se deciden |
|---|---|---|---|
| 3 Unidades, revisiones y procedencia | Etapa 2 (Fase 1) | Fase 1 cerrada: WP14 integrado, formato v1 congelado con ADR-0013 aceptado (a 2026-10-29 **pendiente**: ver el estado de WP14 arriba) | 0004, 0008, 0010, 0017 |
| 4 Relaciones e índices | 3 | API de unidades estable (ADR-0010 aceptado) | 0007, 0014 |
| 5 Planificador y observabilidad | 4 | Consultas por filtro y relación con ruta de referencia probada | 0015 |
| 6 Primera plasticidad operacional | 5 | Planificador determinista con prueba diferencial en CI | 0016 |
| 7 Memoria asociativa | 4, 5 | Recorridos de relaciones y explicación de planes disponibles | 0011 |
| 8 Consolidación y embeddings | 7, 6 | ADR-0018 aceptado; supresión de derivados definida (ADR-0009) | 0009, 0018 |
| 9 Endurecimiento | todas las anteriores (las partes de seguridad pueden adelantarse) | Ver decisión pendiente sobre el orden | 0005, 0006, 0012 |

### Etapa 3 — Unidades, revisiones y procedencia

- **Entregables:** tipos `Unit`, `Revision` y `ChangeSet` (spec §6.1–6.3) sobre `kerno-storage`; CRUD transaccional y archivado; listar, comparar y restaurar revisiones; políticas de historial `none`/`audit`/`versioned` (SAD §8); serialización y hashes (ADR-0008); primera API pública tipada (ADR-0010).
- **Aceptación:** se reconstruye la evolución de una unidad y se restaura sin destruir revisiones posteriores (I-22); el historial no depende del WAL (I-20); contenido y revisión se confirman en la misma frontera atómica (I-21), también en la matriz de fallos.

### Etapa 4 — Relaciones e índices

- **Entregables:** relaciones tipadas declaradas (ADR-0014); índice primario y filtros básicos; índices secundarios solo con un workload que los justifique; ruta de referencia para todas las consultas.
- **Aceptación:** los resultados por índice coinciden con la ruta de referencia (I-32 aplicado a índices); índices reconstruidos equivalentes a nuevos (I-24); estado de frescura expuesto (I-23); una relación declarada nunca se confunde con una inferida (I-39).

### Etapa 5 — Planificador y observabilidad

- **Entregables:** planificador determinista (spec §9: validar, generar candidatos, estimar coste, elegir, ejecutar, registrar, validar invariantes); explicación del plan; `ExecutionObservation` con niveles de detalle ([ejecución adaptativa](../architecture/adaptive-execution.md), ADR-0015).
- **Aceptación:** el plan puede inspeccionarse; prueba diferencial planificador frente a ruta de referencia en CI (I-32); prueba de centinelas sin literales ni contenido en observaciones (I-37).

### Etapa 6 — Primera plasticidad operacional

- **Entregables:** una única optimización reversible (plan reutilizable o caché invalidada por secuencia), con los metadatos de spec §9 y desactivación global y por consulta (ADR-0016).
- **Aceptación:** mejora neta medida frente al modo desactivado con umbrales fijados antes de medir; equivalencia de resultados (I-32, I-33); invalidación probada por dependencia (I-35); invalidar no altera datos (I-34); desactivación verificada (I-36).

### Etapa 7 — Memoria asociativa

- **Entregables:** recuperación por relaciones, contexto temporal y procedencia; búsqueda vectorial **exacta** como baseline si hay un caso de uso (ADR-0011); ranking con modo diagnóstico que explica sus señales.
- **Aceptación:** calidad de recuperación evaluable con un conjunto de consultas y respuestas esperadas fijado antes de medir; toda respuesta rastreable a sus fuentes; sin ANN antes de un benchmark que lo justifique.

### Etapa 8 — Consolidación y embeddings

- **Entregables:** registro común de derivados (ADR-0018); adaptadores o jobs externos para embeddings y resúmenes, fuera del núcleo (I-31); obsolescencia y reconstrucción.
- **Aceptación:** cada derivado se vincula a sus fuentes y se invalida o reconstruye al cambiarlas (I-25, I-38); suprimir una fuente afecta a sus derivados según ADR-0009; un resumen nunca reemplaza el original auditable.

### Etapa 9 — Endurecimiento

- **Entregables:** IDs públicos (ADR-0005), cifrado y claves (ADR-0006), auditoría minimizada, exportación/importación y migraciones con fixtures de versiones previas, fuzzing de entradas externas, pruebas de estrés, benchmarks comparativos con SQLite y motores de referencia (SAD Fases 4 y 6).
- **Aceptación:** límites y garantías documentados; fallos críticos resueltos o marcados como bloqueantes; I-27…I-30 verificados.

### Decisiones pendientes del propietario para estas etapas

1. **Orden de seguridad frente a IA:** el SAD hace la seguridad (Fase 4) antes que búsqueda e IA (Fase 5); la especificación la deja en la Etapa 9. Propuesta: adelantar a antes de la Etapa 8 lo que afecta a derivados (supresión, ADR-0009; redacción de logs, I-29) y dejar el resto en la Etapa 9.
2. Aceptar o modificar ADR-0014…0018 en las etapas indicadas.
3. Umbrales de benchmark frente a SQLite, si se quieren como criterio y no solo como informe.
