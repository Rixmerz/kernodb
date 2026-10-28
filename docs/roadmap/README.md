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
2. Crear los primeros crates del producto (`kerno-core`, `kerno-storage`) a partir de `spike-lsm`, conservando el harness de `spike-core` como suite obligatoria.
3. Condiciones de ADR-0001: colas de latencia acotadas (compactación y cierre con presupuesto), *range tombstones* y compactación dirigida, MANIFEST como log de ediciones, group commit.
4. Transacciones read-modify-write, `sync()` explícito y métricas de snapshots (ADR-0002).
5. Repetir los benchmarks de M0 en el hardware objetivo.
