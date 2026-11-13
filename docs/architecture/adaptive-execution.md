# Ejecución adaptativa: `ExecutionObservation` y equivalencia con la ruta de referencia

Estado: **parcialmente implementado**. La Etapa 5 (planificador y observabilidad, `kerno-units`) está cerrada en [etapa5-cierre.md](etapa5-cierre.md) (aprobación del propietario pendiente): el planificador de §2 de la especificación con los caminos existentes, la explicación del plan y `ExecutionObservation` con niveles. La Etapa 6 (primera plasticidad operacional) está **cerrada pendiente de aprobación del propietario** ([cierre](etapa6-cierre.md), 2026-11-11): la caché de resultados (`cache`) está implementada y **desactivada por defecto**; su mejora neta frente al modo desactivado no está afirmada (el p99 de W1 no se certifica). **Sin implementar:** los planes aprendidos (`learned`), que no se eligieron como optimización de la Etapa 6 ([acta](etapa6-acta.md) §7, punto 1). Hoja de ruta: [roadmap](../roadmap/README.md). Relacionado con: [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.5, §9, §10 y §15; [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md); [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md); invariantes I-32…I-37 en [invariantes](invariants.md).

Este documento fija **qué** se observa y **qué** se debe poder probar antes de que exista una sola optimización aprendida. No elige estructuras de datos ni fórmulas de coste: eso lo decidió la Etapa 5 con evidencia, en [etapa5-planificador.md](etapa5-planificador.md) (costes y caminos) y [etapa5-observacion.md](etapa5-observacion.md) (niveles y retención).

## 1. Términos

- **Ruta de referencia.** Ejecución simple y correcta de una consulta, sin estado aprendido: recorrido por índice primario o barrido completo, filtrado y ordenación explícitos. Es el oráculo de corrección y siempre está disponible.
- **Plan candidato.** Estrategia válida para una consulta: un índice concreto, un orden de recorrido o un plan reutilizado.
- **Optimización aprendida.** Estado derivado de observaciones (plan reutilizable, caché de resultados, prioridad de una ruta) que cambia **cómo** se ejecuta una consulta, nunca **qué** devuelve.
- **Huella de consulta.** Forma normalizada de una consulta con los literales sustituidos por marcadores tipados (`unit_type = ?str`). Identifica la forma, no los valores.

## 2. `ExecutionObservation`

Registro opcional, desactivado por defecto, de una ejecución. Campos conceptuales:

| Campo | Contenido | Datos sensibles |
|---|---|---|
| `observation_id` | identificador local y creciente del observador | no |
| `fingerprint` | huella con clave de la forma de la consulta, `units/blake3-keyed/v1/<hex>` o `relations/…` (E5-01) | no contiene literales |
| `fingerprint_text` | la forma con marcadores tipados, p. ej. `units[unit_type=?text]` | solo con nivel `diagnostic` (ADR-0015) |
| `path` | camino que respondió (`UnitsReference`, `IndexPrimary`, `IndexType`, `RelationsReference` o `Adjacency`); vacío si la consulta falló antes de elegir | no |
| `plan_version` | `PLANNER_VERSION` (E5-03). Sin versión de optimización aprendida: es de la Etapa 6 | no |
| `commit_seq` | secuencia de la instantánea leída (I-04) | no |
| `duration_ns` | duración en nanosegundos, medida con el reloj del observador | no |
| `rows_returned` | filas devueltas | no |
| `outcome` | `Ok` o `Error(clase)`, con la clase `Corruption`, `Storage`, `Invalid` u `Other`; nunca el mensaje | solo la clase |
| `fallback` | `None`, o el camino abandonado y la causa: `Corrupt`, o `Invalid` con `NotMatching`, `NotActive` u `OutOfOrder` | no |
| `recorded_at` | marca de tiempo del registro (no ordena commits, I-26) | no |

Implementado en E5-03 y E5-04 (`crates/kerno-units/src/observation.rs`). **No implementado**, con el motivo: `plan_id` y `plan_kind` como campo (el camino basta: `ExecutionObservation::plan_kind()` deriva `reference` o `planned`; `learned` es de la Etapa 6); `rows_examined` y `bytes_read` (ningún camino los cuenta, y contarlos obliga a instrumentar los bucles, [etapa5-observacion.md](etapa5-observacion.md) §2); `cache` (Etapa 6); `Outcome` no tiene la variante `cancelled`. Las causas `precondition_failed`, `invalidated`, `disabled` y `discrepancy` de la caché (Etapa 6, I-33) se registran en el **registro de transiciones de la caché** (G7), no en el campo `fallback` de la observación, que solo lleva `Corrupt` o `Invalid`; un `discrepancy` aparece en la observación como `Invalid` con el camino `ResultCache` abandonado. Hallazgo H-6 del [cierre de la Etapa 6](etapa6-cierre.md).

Reglas:

1. **Nunca** se guardan valores de los parámetros, contenido de unidades, claves completas ni tokens (I-29, ADR-0015).
2. Las observaciones son datos derivados de diagnóstico: no participan en transacciones, no se replican en exportaciones lógicas por defecto y pueden borrarse sin efecto en los datos.
3. Registrar una observación no puede hacer fallar ni bloquear la consulta observada; si el registro falla, la consulta devuelve su resultado y la pérdida se cuenta (`ObservationStats::dropped`, `evicted`).
4. Retención acotada por tamaño y por edad, configurable; por defecto 1024 registros y 1 hora (`ObservationConfig`, E5-03).

## 3. Metadatos obligatorios de una optimización aprendida

Cada optimización registra (spec §9): huella a la que aplica, precondiciones, dependencias de esquema, índices y estadísticas, estimación de coste, evidencia observada (número de observaciones y ventana), fecha y versión, invalidadores y ruta de fallback. Una optimización sin alguno de estos campos no se puede activar. **H-3 de la Etapa 6, cerrado en P3:** la caché de resultados lleva dependencias de esquema, planificador, índice y secuencia de commit; su evidencia de creación es la que el observador tenía de esa forma de consulta (observaciones y ventana, `Observer::evidence_of`) y la entrada lleva el coste medido, en nanosegundos, de la ejecución que sustituye. Sin una u otra no se crea la entrada, así que no puede activarse; en particular, con el observador por debajo de `Fingerprint` no hay evidencia y la caché no guarda nada. Los dos datos se ven en el registro de transiciones (`Created`) y en la explicación del plan (`Plan::cache()`, que además dice si la caché se consultó y si responde estado aprendido). Las estadísticas siguen cubiertas solo por la coincidencia exacta de la secuencia de commit ([ADR-0016](../adr/0016-adaptacion-e-invalidacion.md), «Revisión de 2026-11-13»). Límite: «estimación de coste» es una medida de una sola ejecución, no un modelo.

## 4. Invariantes de equivalencia

| ID | Invariante | Cómo se probará |
|---|---|---|
| I-32 | **Equivalencia de resultados.** Para toda consulta y toda instantánea, el plan elegido (planificado o aprendido) devuelve el mismo multiconjunto de resultados que la ruta de referencia; si la consulta fija un orden, la misma secuencia. | Prueba diferencial: cada consulta generada se ejecuta por la ruta de referencia y por cada plan candidato sobre la misma instantánea (`commit_seq`); también con pruebas de propiedades sobre datos y consultas aleatorias. **Planificador implementado (E5-02):** `tests/index.rs` (`check`, semillas 1–6 y barridos por páginas) y `tests/relations.rs` (`assert_equivalent`) comparan `query` y `query_relations` con la ruta de referencia, con y sin índice listo, y exigen que el camino ejecutado sea el que `explain_*` anuncia. Los planes `learned` son de la Etapa 6. |
| I-33 | **Fallback correcto.** Si una precondición no se cumple, la optimización está invalidada o desactivada, se ejecuta la ruta de referencia (u otro plan válido) y el resultado cumple I-32; el fallback queda registrado: su causa en el registro de transiciones de la caché (`Fallback { cause }`) y, cuando la caché responde y falla su comprobación, también en el campo `fallback` de la observación como `Invalid` con el camino `ResultCache` abandonado. Las causas `precondition_failed`, `invalidated` y `disabled` solo están en el registro de la caché. | Inyección de precondiciones falsas e invalidaciones durante la prueba diferencial. |
| I-34 | **Invalidar no altera datos.** Activar, desactivar, invalidar o borrar optimizaciones y observaciones no cambia ningún dato confirmado, revisión, relación ni índice transaccional, ni la secuencia de commit. | Comparar el estado lógico y `commit_seq` antes y después de cada operación de administración. |
| I-35 | **Invalidación por dependencias.** Un cambio de esquema, de definición de índice o de estadísticas relevantes invalida o revalida las optimizaciones que dependen de él antes de su siguiente uso. | Pruebas que cambian cada dependencia declarada y comprueban que la siguiente ejecución no usa el plan obsoleto. |
| I-36 | **Desactivación global y por consulta.** Con la adaptación desactivada (globalmente o para una consulta), ninguna optimización aprendida participa y los resultados cumplen I-32. | Ejecutar la suite completa con la adaptación desactivada y comparar con la ruta de referencia. |
| I-37 | **Observaciones sin datos sensibles.** Ninguna observación, métrica o explicación de plan contiene literales de consulta ni contenido de unidades fuera del nivel `diagnostic`. | Prueba con datos centinela: buscar los centinelas en todo lo exportado por observabilidad. **Implementado (E5-04):** `tests/sentinels.rs` busca los centinelas en observaciones, contadores, explicaciones y errores, en los cuatro niveles, y comprueba que `fingerprint_text` solo existe en `diagnostic`. |

Una discrepancia (hoy: las filas de una entrada fallan `check_units` / `check_relations` al servirse; no hay verificación muestreada contra la ruta de referencia) registra la causa `Discrepancy` en el registro de transiciones de la caché, **elimina la entrada** (`DeleteReason::Discrepancy`) y devuelve el resultado de la ruta de referencia, que puede guardarse como entrada nueva. **No desactiva** la optimización para esa consulta ni de forma global. Ver [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md), «Revisión de 2026-11-13».

## 5. Criterio de valor (spec §9 y §10)

Una optimización solo se mantiene si aporta **utilidad neta** medida: mejora de latencia o de trabajo examinado frente a su coste de mantenimiento, memoria y almacenamiento, con el modo adaptativo desactivado como comparación. La frecuencia por sí sola no basta. Los umbrales se fijan antes de medir (ADR-0012).

## 6. Fuera de alcance

Aprendizaje de conocimiento (relaciones inferidas, resúmenes, consolidación): Etapas 7–8 y ADR-0018. Un plan aprendido nunca modifica contenido.
