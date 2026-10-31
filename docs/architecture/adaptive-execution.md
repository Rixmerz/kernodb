# Ejecución adaptativa: `ExecutionObservation` y equivalencia con la ruta de referencia

Estado: borrador de contrato, **sin implementación**. Se implementa en las Etapas 5 (planificador y observabilidad) y 6 (primera plasticidad operacional) de la [hoja de ruta](../roadmap/README.md). Relacionado con: [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.5, §9, §10 y §15; [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md); [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md); invariantes I-32…I-37 en [invariantes](invariants.md).

Este documento fija **qué** se observa y **qué** se debe poder probar antes de que exista una sola optimización aprendida. No elige estructuras de datos ni fórmulas de coste: eso se decide al empezar la Etapa 5, con evidencia.

## 1. Términos

- **Ruta de referencia.** Ejecución simple y correcta de una consulta, sin estado aprendido: recorrido por índice primario o barrido completo, filtrado y ordenación explícitos. Es el oráculo de corrección y siempre está disponible.
- **Plan candidato.** Estrategia válida para una consulta: un índice concreto, un orden de recorrido o un plan reutilizado.
- **Optimización aprendida.** Estado derivado de observaciones (plan reutilizable, caché de resultados, prioridad de una ruta) que cambia **cómo** se ejecuta una consulta, nunca **qué** devuelve.
- **Huella de consulta.** Forma normalizada de una consulta con los literales sustituidos por marcadores tipados (`unit_type = ?str`). Identifica la forma, no los valores.

## 2. `ExecutionObservation`

Registro opcional, desactivado por defecto, de una ejecución. Campos conceptuales:

| Campo | Contenido | Datos sensibles |
|---|---|---|
| `observation_id` | identificador local de la observación | no |
| `fingerprint` | hash versionado de la huella normalizada (algoritmo y versión registrados) | no contiene literales |
| `fingerprint_text` | texto de la huella | solo con nivel `diagnostic` (ADR-0015) |
| `plan_id` / `plan_kind` | plan ejecutado: `reference`, `planned` o `learned` | no |
| `plan_version` | versión del planificador y, si aplica, de la optimización aprendida | no |
| `commit_seq` | secuencia de la instantánea leída (I-04) | no |
| `duration_ns` | tiempo de ejecución medido con reloj monotónico | no |
| `rows_examined` / `rows_returned` | contadores | no |
| `bytes_read` | bytes leídos del almacenamiento | no |
| `outcome` | `ok`, `error(kind)`, `cancelled` | solo el tipo de error, sin mensaje |
| `cache` | `hit`, `miss`, `bypass` o `none` | no |
| `fallback` | `none`, `precondition_failed`, `invalidated`, `discrepancy`, `disabled` | no |
| `recorded_at` | marca de tiempo del registro (no ordena commits, I-26) | no |

Reglas:

1. **Nunca** se guardan valores de los parámetros, contenido de unidades, claves completas ni tokens (I-29, ADR-0015).
2. Las observaciones son datos derivados de diagnóstico: no participan en transacciones, no se replican en exportaciones lógicas por defecto y pueden borrarse sin efecto en los datos.
3. Registrar una observación no puede hacer fallar ni bloquear la consulta observada; si el registro falla, la consulta devuelve su resultado y la pérdida se cuenta en una métrica.
4. Retención acotada por tamaño y por edad, configurable; el valor por defecto se fija en la Etapa 5.

## 3. Metadatos obligatorios de una optimización aprendida

Cada optimización registra (spec §9): huella a la que aplica, precondiciones, dependencias de esquema, índices y estadísticas, estimación de coste, evidencia observada (número de observaciones y ventana), fecha y versión, invalidadores y ruta de fallback. Una optimización sin alguno de estos campos no se puede activar.

## 4. Invariantes de equivalencia

| ID | Invariante | Cómo se probará |
|---|---|---|
| I-32 | **Equivalencia de resultados.** Para toda consulta y toda instantánea, el plan elegido (planificado o aprendido) devuelve el mismo multiconjunto de resultados que la ruta de referencia; si la consulta fija un orden, la misma secuencia. | Prueba diferencial: cada consulta generada se ejecuta por la ruta de referencia y por cada plan candidato sobre la misma instantánea (`commit_seq`); también con pruebas de propiedades sobre datos y consultas aleatorias. |
| I-33 | **Fallback correcto.** Si una precondición no se cumple, la optimización está invalidada o desactivada, se ejecuta la ruta de referencia (u otro plan válido) y el resultado cumple I-32; el fallback queda registrado en la observación. | Inyección de precondiciones falsas e invalidaciones durante la prueba diferencial. |
| I-34 | **Invalidar no altera datos.** Activar, desactivar, invalidar o borrar optimizaciones y observaciones no cambia ningún dato confirmado, revisión, relación ni índice transaccional, ni la secuencia de commit. | Comparar el estado lógico y `commit_seq` antes y después de cada operación de administración. |
| I-35 | **Invalidación por dependencias.** Un cambio de esquema, de definición de índice o de estadísticas relevantes invalida o revalida las optimizaciones que dependen de él antes de su siguiente uso. | Pruebas que cambian cada dependencia declarada y comprueban que la siguiente ejecución no usa el plan obsoleto. |
| I-36 | **Desactivación global y por consulta.** Con la adaptación desactivada (globalmente o para una consulta), ninguna optimización aprendida participa y los resultados cumplen I-32. | Ejecutar la suite completa con la adaptación desactivada y comparar con la ruta de referencia. |
| I-37 | **Observaciones sin datos sensibles.** Ninguna observación, métrica o explicación de plan contiene literales de consulta ni contenido de unidades fuera del nivel `diagnostic`. | Prueba con datos centinela: buscar los centinelas en todo lo exportado por observabilidad. |

Una discrepancia detectada en producción (por ejemplo, por verificación muestreada contra la ruta de referencia) desactiva la optimización afectada, registra `fallback = discrepancy` y devuelve el resultado de la ruta de referencia.

## 5. Criterio de valor (spec §9 y §10)

Una optimización solo se mantiene si aporta **utilidad neta** medida: mejora de latencia o de trabajo examinado frente a su coste de mantenimiento, memoria y almacenamiento, con el modo adaptativo desactivado como comparación. La frecuencia por sí sola no basta. Los umbrales se fijan antes de medir (ADR-0012).

## 6. Fuera de alcance

Aprendizaje de conocimiento (relaciones inferidas, resúmenes, consolidación): Etapas 7–8 y ADR-0018. Un plan aprendido nunca modifica contenido.
