# Acta de entrada de la Etapa 5: planificador y observabilidad

| Campo | Valor |
|---|---|
| Fecha | 2026-11-05 (borrador) |
| Estado | Propuesta, pendiente de aprobación del propietario |
| Acto | Aprobación del alcance y de los paquetes E5-00 a E5-05 por el propietario del proyecto |
| Documentos | [Roadmap, Etapa 5](../roadmap/README.md#etapa-5--planificador-y-observabilidad) · [cierre de la Etapa 4](etapa4-cierre.md) · [ejecución adaptativa](adaptive-execution.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) · [ADR-0007](../adr/0007-consistencia-de-indices.md) · [invariantes](invariants.md) · spec v0.2.0 §9, §16 |

## 0. Estado

Esta acta es una **propuesta**. Ningún WP arranca antes de que el propietario la apruebe (§8). Los WP son secuenciales, como en las Etapas 3 y 4.

## 1. Condición de inicio

| Condición | Estado |
|---|---|
| Etapa 4 cerrada | Cumplida: [cierre de la Etapa 4](etapa4-cierre.md), aprobado el 2026-11-05. Se hereda con K-E3-8, K-E3-9 y la recomendación de M-9 |
| Decisión abierta de ADR-0007 (§6 del cierre) | Resuelta el 2026-11-05: los índices de unidades siguen derivados y solo `rebuild_index` los escribe; tras una escritura, `query` usa la ruta de referencia hasta la siguiente reconstrucción |
| ADR-0015 (privacidad de la observabilidad) | *Propuesto*. Ver §7, punto 1 |

## 2. Alcance

### 2.1 Objetivos

1. Planificador determinista con los siete pasos de spec §9: validar la consulta, generar candidatos válidos, estimar coste, elegir, ejecutar, registrar y validar invariantes con fallback.
2. Explicación del plan, inspeccionable por el llamador.
3. `ExecutionObservation` con niveles de detalle, sin literales ni contenido fuera de `diagnostic` (ADR-0015, I-37).

### 2.2 Punto de partida

`kerno-units` ya elige entre dos caminos, sin planificador explícito: `UnitSnapshot::query` usa `IndexPath::Auto` (`IDX_TYPE` si el filtro fija `unit_type`, si no `IDX_PRIMARY`) cuando el índice está `Ready` y, si no, `reference_filter`; `QueryOutcome::source` dice cuál. Las relaciones se consultan por adyacencia (`relations_of`) con su ruta de referencia (`reference_relations_of`). No hay coste estimado, explicación ni observaciones; la capa no emite métricas (IU-10).

Los candidatos de esta etapa son **solo los caminos existentes**: ruta de referencia, `IDX_PRIMARY`, `IDX_TYPE` y adyacencia. La etapa no añade índices, caminos ni consultas nuevas (spec §16: ninguna función sin necesidad).

### 2.3 Fuera de alcance

- Optimizaciones aprendidas, planes reutilizables y cachés: Etapa 6 (ADR-0016, I-33…I-36). El `plan_kind` `learned` no se implementa.
- Índices nuevos, mantenimiento de índices en cada `ChangeSet` o reconstrucción en segundo plano (decisión de ADR-0007 del 2026-11-05).
- Lenguaje de consultas o analizador de texto: la consulta sigue siendo `UnitFilter` y la consulta de relaciones existente.
- Persistencia de observaciones en el motor y exportación: las observaciones viven en memoria con retención acotada salvo que el propietario decida otra cosa (§7, punto 3).
- Comparación con SQLite: Etapa 9.

## 3. Paquetes de trabajo

Todos son secuenciales. «Hecho» incluye `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, la suite del WP y la suite completa en verde, el conteo de pruebas antes y después, y cada prueba nueva vista fallar por mutación.

| WP | Contenido | Depende de | Aceptación |
|---|---|---|---|
| **E5-00** Puertas de entrada | Aprobación de esta acta. Decisión registrada sobre ADR-0015 y sobre los puntos de §7. Re-medición de U6 con índices y relaciones activos, con la salida cruda comprometida (M-9) | Etapa 4 cerrada | Acta aprobada; estado de ADR-0015 actualizado en `docs/adr/`; salida de `unit_bench` comprometida; ningún WP posterior arranca antes |
| **E5-01** Huella y plan explicable | Huella normalizada de `UnitFilter` y de la consulta de relaciones, con literales sustituidos por marcadores tipados y hash con algoritmo y versión registrados (ADR-0015). Tipo de plan y su explicación: candidatos considerados, coste estimado, elegido, motivo y estado de frescura del índice (I-23) | E5-00 | La explicación lista todos los candidatos y el motivo de la elección; la huella es estable para la misma forma y no contiene literales; vectores fijos de huella |
| **E5-02** Planificador determinista | Los siete pasos de spec §9 sobre los candidatos existentes. Coste con las estadísticas que ya hay (estado y watermark del índice, rango de tipo); ninguna estadística nueva sin consumidor. Validación del resultado y fallback a la ruta de referencia, registrado. `query` pasa a usar el planificador sin cambiar su resultado | E5-01 | Misma entrada y misma instantánea dan el mismo plan (determinismo); prueba diferencial planificador frente a ruta de referencia en CI (I-32), con las semillas y filtros de `tests/index.rs` y `tests/relations.rs`; fallback probado con índice `Stale`, `Building` y `Failed` |
| **E5-03** `ExecutionObservation` | Registro opcional con los campos de [ejecución adaptativa §2](adaptive-execution.md) aplicables (sin `cache` ni `learned`) y los niveles `off`, `aggregate`, `fingerprint` y `diagnostic`. Retención acotada por tamaño y edad, en memoria. Registrar nunca hace fallar ni bloquea la consulta | E5-02 | Nivel por defecto según §7; `diagnostic` solo por activación explícita; un fallo de registro se cuenta y la consulta devuelve su resultado; coste de registro medido frente a `off` (ADR-0015, criterio 4) |
| **E5-04** Centinelas | Prueba de centinelas en tipos, cargas útiles, metadatos, procedencia y valores de filtro, buscados en toda salida de observabilidad, explicación y error, en cada nivel. Revisión del threat model para observaciones y explicaciones | E5-03 | Ningún centinela fuera de `diagnostic` (I-37); la prueba muere con la mutación «copiar el literal en la huella o en la explicación» |
| **E5-05** Cierre | Revisión adversarial. Promoción de I-32 (planificador) e I-37. Actualización del roadmap, del SAD §9 y de `adaptive-execution.md` | todos | Criterios de §4 cumplidos; informe de revisión sin bloqueantes abiertos |

Camino crítico: E5-00 → E5-01 → E5-02 → E5-03 → E5-04 → E5-05.

## 4. Criterios de aceptación

Del [roadmap, Etapa 5](../roadmap/README.md#etapa-5--planificador-y-observabilidad):

| Criterio | Invariante | Evidencia prevista | Estado |
|---|---|---|---|
| El plan puede inspeccionarse | — (spec §9, modo diagnóstico) | Explicación con candidatos, costes, elección y motivo (E5-01, E5-02) | pendiente |
| El plan elegido devuelve lo mismo que la ruta de referencia | I-32 (parte del planificador) | Prueba diferencial planificador frente a referencia en CI, unidades y relaciones, con y sin índice listo (E5-02) | pendiente |
| Fallback a la ruta de referencia registrado cuando el índice no es utilizable | I-23, I-32 | Pruebas con cada estado de frescura (E5-02) | pendiente |
| Ninguna observación, métrica o explicación contiene literales ni contenido fuera de `diagnostic` | I-37, I-29, IU-10 | Prueba de centinelas en todos los niveles (E5-04) | pendiente |
| Observar no altera datos confirmados ni la secuencia de commit | I-34 (solo observaciones; la parte de optimizaciones es de la Etapa 6) | Prueba: mismas lecturas y misma secuencia con observación `off` y `diagnostic` (E5-03) | pendiente |

## 5. Riesgos heredados

| ID | Riesgo en la Etapa 5 | Tratamiento propuesto |
|---|---|---|
| **K-E3-8** | U2 incumple en `history`. El planificador no toca `history` | Sin acción salvo decisión del propietario; ningún umbral se rebaja |
| **K-E3-9** | U6 incumple con retención concurrente; no se re-midió con índices (M-9) | Re-medir en E5-00 y comprometer la salida. Las observaciones no escriben en el motor, así que no deberían afectar a U6 |
| **M-11** | Prueba intermitente `retention_every_point_paced_power_loss_sync` | Puede romper la suite completa de forma aleatoria; un fallo de esa prueba no se atribuye a un WP sin reproducirlo |
| **Nuevo** | El planificador añade coste a cada `query` (estimar y explicar) | Medir W1 con planificador frente al camino actual en E5-02; umbral solo si el propietario lo fija antes (§6) |
| **Nuevo** | Una huella de pocos valores admite ataques de diccionario (ADR-0015) | Hash con clave local no exportada si el propietario lo decide (§7, punto 4) |

Política de merge: la [puerta de corrección de la Fase 1](phase1-design.md#71-puerta-de-corrección-adr-0012-umbral-1-c-25-bloqueante) es bloqueante. Los incumplimientos de rendimiento se registran como riesgos y no bloquean.

## 6. Umbrales de rendimiento

Esta acta **no propone umbrales**. T1–T10 y U1–U6 no cambian y no se rebajan. ADR-0015 (criterio 4) pide que la Etapa 5 fije el umbral de coste de registrar una observación sobre la p99 de la consulta observada: si el propietario lo quiere, se fija antes de medir (§7, punto 5).

## 7. Puntos que el propietario debe decidir

1. **ADR-0015.** Está *Propuesto*. ¿Se acepta (opción 2, niveles con redacción) en E5-00, antes de E5-03, o se acepta con la evidencia de E5-04? ADR-0016 y ADR-0018 pueden seguir *Propuestos*: no se usan en esta etapa.
2. **Nivel por defecto de las observaciones.** ADR-0015 propone `off`. Alternativa: `aggregate` (contadores sin texto), útil para la Etapa 6 sin exponer literales.
3. **Dónde viven las observaciones.** Propuesta: en memoria, con retención por tamaño y edad y valores por defecto fijados en E5-03. Persistirlas exige formato y ADR-0009 (supresión), fuera de alcance.
4. **Hash de la huella.** Hash versionado sin clave o hash con clave local no exportada (ADR-0015, ataques de diccionario).
5. **Umbrales.** Si se fija un umbral para el coste de observación (ADR-0015, criterio 4) y para el sobrecoste del planificador sobre W1, antes de medir.
6. **Asignación de I-34.** [invariants.md](invariants.md) la sitúa en la Etapa 6; esta acta propone verificar en E5-03 solo la parte de observaciones.

## 8. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| — | **Pendiente** | El propietario aprueba o ajusta el alcance (§2), los paquetes (§3), los criterios (§4) y los puntos de §7 antes de que E5-00 empiece |
