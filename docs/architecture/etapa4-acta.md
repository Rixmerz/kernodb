# Acta de entrada de la Etapa 4: relaciones e índices

| Campo | Valor |
|---|---|
| Fecha | 2026-11-04 (borrador) |
| Estado | **Pendiente de aprobación del propietario** |
| Acto | Aprobación del alcance y de los paquetes E4-00 a E4-05 por el propietario del proyecto |
| Documentos | [Roadmap, Etapa 4](../roadmap/README.md#etapa-4--relaciones-e-índices) · [cierre de la Etapa 3](etapa3-cierre.md) · [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) · [ADR-0007](../adr/0007-consistencia-de-indices.md) · [invariantes](invariants.md) · [objetivos T1–T10](../testing/f1-objetivos-rendimiento.md) |

## 0. Estado

Esta acta fue **aprobada por el propietario el 2026-11-04** (ver §8). Ningún WP posterior arranca antes que el anterior, como en la Etapa 3.

## 1. Condición de inicio

El roadmap exige para la Etapa 4 una API de unidades estable (ADR-0010 aceptado). Estado comprobado el 2026-11-04:

| Condición | Estado |
|---|---|
| API de unidades estable (ADR-0010 aceptado) | Cumplida: ADR-0010 **Aceptado** el 2026-10-31 ([acta E3-00](etapa3-acta-e3-00.md)). La API pública de `kerno-units` se cerró con la Etapa 3 |
| Etapa 3 cerrada | Cumplida: [cierre de la Etapa 3](etapa3-cierre.md), aprobado por el propietario el 2026-11-04. Se hereda con los riesgos K-E3-8 y K-E3-9 |

## 2. Alcance

### 2.1 Objetivos

1. Declarar relaciones tipadas entre unidades ([ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md)).
2. Índice primario y filtros básicos.
3. Índices secundarios solo cuando un workload medido los justifique.
4. Una ruta de referencia para todas las consultas: cada índice se compara contra ella.

### 2.2 Punto de partida

La API pública de `kerno-units` no tiene listado ni consulta por tipo o por campo. Expone `get_unit` por id, `history`, `history_page`, `unit_as_of` y `state_as_of`; el recorrido interno de cabezas existe solo para verificación y retención. Por tanto, la Etapa 4 empieza sin ningún índice y sin una ruta de referencia pública: E4-01 la crea.

### 2.3 Fuera de alcance

- Planificador, explicación de planes y observabilidad: Etapa 5.
- Supresión física, borrado legal y retención legal: ADR-0009 (K-E3-6).
- Relaciones inferidas: Etapa 7. En esta etapa solo se cumple la parte de I-39 que afecta a las declaradas.
- Múltiples escritores: la ranura de escritura única es una propiedad del motor (I-06).
- Comparación con SQLite: reservada a la Etapa 9. En esta etapa la referencia es interna.
- Deltas, troceado de cargas útiles y `AsOf::Time`.

## 3. Paquetes de trabajo

Todos son secuenciales. «Hecho» incluye `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, la suite del WP y la suite completa en verde, el conteo de pruebas antes y después, y cada prueba nueva vista fallar por mutación.

| WP | Contenido | Depende de | Aceptación |
|---|---|---|---|
| **E4-00** Puertas de entrada | Aprobación de esta acta. Decisión registrada sobre ADR-0007 (hoy *Pendiente*) y ADR-0014 (hoy *Propuesto*), y sobre los puntos de la §7 | Etapa 3 cerrada | Acta aprobada por el propietario; estado de ADR-0007 y ADR-0014 actualizado en `docs/adr/`; ningún WP posterior arranca antes |
| **E4-01** Ruta de referencia | Recorrido completo sobre un `UnitSnapshot`, sin índices, que devuelve el resultado correcto por definición para cada consulta de la etapa | E4-00 | Pruebas sobre `Emulated<LogEngine>` y `KernoDb`; ninguna consulta de la etapa se publica sin su ruta de referencia |
| **E4-02** Relaciones declaradas (ADR-0014) | Prototipo de la opción 2 (registro y dos índices de adyacencia en el mismo lote) frente a la opción 1 sobre `kerno-storage`, con los criterios 1–5 de ADR-0014 fijados antes de medir. Semántica de archivar o borrar una unidad con relaciones (criterio 4) | E4-01 | Prototipo y comparación registrados en ADR-0014 («Evidencia»); toda relación lleva procedencia obligatoria; filas nuevas en la [matriz de fallos](failure-matrix.md) para registro e índices (I-21); U2 y U6 re-medidos con los umbrales ya aprobados, sin cambiarlos |
| **E4-03** Índice primario y filtros básicos | Índice primario y filtros sobre la ruta de referencia. Estado de frescura `ready`/`building`/`stale`/`failed` con watermark | E4-01, E4-02 | Resultados iguales a la ruta de referencia sobre la misma instantánea (I-32 aplicado a índices); índice reconstruido igual que uno nuevo (I-24); estado y watermark expuestos (I-23) |
| **E4-04** Índices secundarios | Solo los que un workload medido justifique. Cada uno tiene su workload escrito antes de construirse | E4-03 | Cada índice secundario tiene workload documentado y equivalencia con la ruta de referencia; ningún índice sin workload |
| **E4-05** Cierre | Revisión adversarial de la etapa. Promoción de las invariantes de la etapa. Actualización del roadmap y del SAD §9 | todos | Criterios de §4 cumplidos; informe de revisión sin bloqueantes abiertos |

Camino crítico: E4-00 → E4-01 → E4-02 → E4-03 → E4-04 → E4-05.

## 4. Criterios de aceptación

Del [roadmap, Etapa 4](../roadmap/README.md#etapa-4--relaciones-e-índices):

| Criterio | Evidencia prevista | Estado |
|---|---|---|
| Los resultados por índice coinciden con la ruta de referencia (I-32 aplicado a índices) | Pruebas diferenciales índice frente a E4-01 sobre la misma instantánea | cumplido en E4-03 (`tests/index.rs`: `differential_seeded`, `differential_across_scan_pages`; [invariantes](invariants.md) §2.2) |
| Un índice reconstruido es equivalente a uno nuevo (I-24) | Prueba de reconstrucción frente a construcción nueva | cumplido en E4-03 (`tests/index.rs`: `rebuilt_equals_fresh`) |
| El estado de frescura está expuesto (I-23) | Prueba de estados y watermark, incluido tras una caída | cumplido en E4-03 (`tests/index.rs`: `states_and_watermark`, `index_every_point_*`, [matriz de fallos](failure-matrix.md) §9) |
| Cada índice secundario tiene workload documentado y equivalencia con la ruta de referencia; ningún índice sin workload (E4-04) | Workload escrito antes de medir, medición, pruebas diferenciales por índice | cumplido en E4-04 ([etapa4-workloads.md](etapa4-workloads.md): W1 justifica `IDX_TYPE`; `tests/index.rs`: `check` compara `IndexPath::Primary` y `IndexPath::ByType` con la referencia en `differential_*`, `rebuilt_equals_fresh`, `index_every_point_*`; UNIT-38) |
| Una relación declarada nunca se confunde con una inferida (I-39) | Prueba de procedencia obligatoria y del campo de origen | pendiente |

## 5. Riesgos heredados de la Etapa 3

| ID | Riesgo en la Etapa 4 | Tratamiento propuesto |
|---|---|---|
| **K-E3-8** | U2 incumple en `history`: 1,50× (1,503) de la referencia en 1 de 3 pasadas, sin cambiar el umbral. Las relaciones añaden registros a los `ChangeSet` y no deben empeorar la lectura de historial | Re-medir U2 en E4-02 con el mismo umbral. Juzgar U2 sobre `history_page` solo si el propietario lo decide (decisión 12 de la Etapa 3). Ningún umbral se rebaja |
| **K-E3-9** | U6 incumple con retención concurrente: p99 de escritura 1,05–1,18× y rendimiento 0,89–0,94× (1,06× en una pasada). Cada relación escribe tres registros, dentro de la ranura de escritura única (K-E3-2), y puede empeorar el p99 durante la poda | Re-medir U6 con índices y relaciones activos en E4-02 y E4-03. El incumplimiento sigue documentado mientras no se corrija. Ningún umbral se rebaja |

K-E3-7 (las lecturas comprueban menos que `verify_units`) no se hereda: quedó aceptado por diseño, con revisión recomendada en la Etapa 9.

Política de merge de la etapa: la [puerta de corrección de la Fase 1](phase1-design.md#71-puerta-de-corrección-adr-0012-umbral-1-c-25-bloqueante) es bloqueante. Los incumplimientos de rendimiento se registran como riesgos y no bloquean, como ocurrió con U2 y U6 en la Etapa 3.

## 6. Umbrales de rendimiento

Esta acta **no propone umbrales** para la Etapa 4. Los objetivos T1–T10 de [f1-objetivos-rendimiento.md](../testing/f1-objetivos-rendimiento.md) no cambian y no se rebajan. Si el propietario quiere umbrales para relaciones o índices, los fija antes de medir, como se hizo con U1–U6 en la Etapa 3 (ADR-0012). SQLite no interviene en esta etapa.

## 7. Puntos que el propietario debe decidir

1. **ADR-0014 y ADR-0007.** Hoy están *Propuesto* y *Pendiente*. ADR-0014 pide un prototipo al inicio de la etapa. Su decisión final sale de E4-02 y se registra en el ADR.
2. **Asignación de I-32.** [invariants.md](invariants.md) la sitúa en la Etapa 5, pero el roadmap de la Etapa 4 la exige «aplicada a índices». Hay que decidir si se promueve en E4-03 o si cambia la asignación.
3. **Asignación de I-23 e I-24.** `invariants.md` las sitúa en «Fase 5» del SAD; el roadmap de la Etapa 4 las exige. Mismo tratamiento que el punto 2.
4. **Semántica de unidades archivadas o borradas con relaciones** (ADR-0014, criterio 4): rechazo, cascada o relación colgante marcada. Se decide en E4-02 y no depende de ADR-0009 mientras no haya supresión física.

## 8. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-04 | **Aprobada** (E4-00 cerrado) | El propietario aprueba o ajusta el alcance (§2), los paquetes (§3), los criterios (§4) y los puntos de §7 antes de que E4-00 empiece |

### Decisiones del propietario sobre la §7 (2026-11-04)

1. ADR-0007 **Aceptado**; ADR-0014 sigue *Propuesto* hasta E4-02.
2. I-32 se promueve en **E4-03** (aplicada a índices).
3. I-23 e I-24 se promueven en **E4-03**.
4. Unidades archivadas o borradas con relaciones: **rechazo**.
