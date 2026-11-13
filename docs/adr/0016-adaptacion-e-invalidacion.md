# ADR-0016 — Adaptación operacional e invalidación

| Campo | Valor |
|---|---|
| Estado | **Aceptado y confirmado el 2026-11-13**, con las enmiendas de «Revisión de 2026-11-13» (D16 de la [acta de la Etapa 9](../architecture/etapa9-acta.md), por delegación expresa del propietario). Historial: aceptado el 2026-11-09; pendiente de confirmación el 2026-11-13 (D16, opción (b)) hasta la revisión de G1 a G7, que es la de este texto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §2, §9, §10, §15; [ejecución adaptativa](../architecture/adaptive-execution.md); ADR-0007, ADR-0009, ADR-0012, ADR-0015, ADR-0018; invariantes I-32…I-36 |

## Contexto

La especificación propone que el motor aprenda a ejecutar mejor las consultas frecuentes (Etapa 6) sin cambiar su significado, y que toda adaptación pueda inspeccionarse, medirse, desactivarse y revertirse. Hace falta decidir qué puede aprender el motor, dónde vive ese estado, cuándo se invalida y cómo se prueba que no altera resultados.

## Opciones consideradas

1. **Sin adaptación**: planificador determinista basado solo en estadísticas. Línea base obligatoria en cualquier caso.
2. **Adaptación limitada a una optimización reversible** (propuesta para la Etapa 6): un único mecanismo, por ejemplo plan reutilizable por huella o caché de resultados invalidada por secuencia de commit, con metadatos completos (spec §9) y comparación contra el modo desactivado.
3. **Adaptación general** (pesos aprendidos para varias rutas, índices automáticos). Mayor superficie de errores; la especificación prohíbe además cambios de esquema autónomos sin política explícita.

## Criterios de decisión

1. Equivalencia con la ruta de referencia (I-32) en la prueba diferencial, con la optimización activada, desactivada e invalidada a mitad de ejecución.
2. Mejora neta medida frente al modo desactivado, con umbrales fijados antes de medir (ADR-0012), incluyendo coste de mantenimiento, memoria y almacenamiento.
3. Invalidación probada para cada dependencia declarada (I-35).
4. Desactivación global y por consulta (I-36) y borrado sin efecto en los datos (I-34).

## Evidencia

La Etapa 5 dejó el planificador determinista, la ruta de referencia y la prueba diferencial planificador/referencia en CI (I-32 verificada para índices, [etapa5-cierre.md](../architecture/etapa5-cierre.md)), y `ExecutionObservation` con niveles (E5-03, E5-04). No hay todavía evidencia de ninguna optimización aprendida: la aporta el benchmark de la Etapa 6.

## Garantías (condiciones del propietario, 2026-11-07)

Una optimización aprendida solo puede activarse si cumple todas estas garantías; cada una tiene su prueba en la Etapa 6.

| # | Garantía | Regla | Prueba |
|---|---|---|---|
| G1 | La adaptación nunca compromete la corrección | La ruta optimizada devuelve el mismo multiconjunto (y el mismo orden, si la consulta lo fija) que la ruta de referencia sobre la misma instantánea | I-32: prueba diferencial con la optimización activada, desactivada e invalidada a mitad de ejecución |
| G2 | La ruta de referencia siempre existe | Se mantiene, se compila y se prueba en CI aunque no se use; ante cualquier precondición incumplida, dependencia desconocida, error, discrepancia o duda, el planificador ejecuta la ruta de referencia (u otro plan válido sin estado aprendido) | I-33: inyección de precondiciones falsas, invalidaciones y fallos del estado aprendido |
| G3 | Las observaciones no son autoridad | `ExecutionObservation` solo propone candidatos y estima costes; nunca demuestra que una ruta sea válida ni sustituye una comprobación. La validez sale de las precondiciones comprobadas en cada uso y de la prueba diferencial, no de cuántas veces funcionó. Las observaciones históricas pueden utilizarse como señales estadísticas para proponer optimizaciones, pero nunca como evidencia de que las precondiciones actuales siguen siendo válidas. Una observación antigua no autoriza ejecutar un plan: antes de utilizarlo deben verificarse las precondiciones actuales y mantenerse las garantías de equivalencia con la ruta de referencia. | Prueba con observaciones falsificadas o corruptas: el resultado sigue cumpliendo I-32; un plan propuesto por observaciones anteriores a un cambio de dependencia no se ejecuta sin volver a verificar sus precondiciones |
| G4 | La invalidación es explícita | Cada optimización declara sus dependencias con número de versión; se comprueban **antes de cada uso** y una que no coincide invalida la optimización. Lo que el código declara hoy es `SchemaVersion`, `PlannerVersion`, `IndexState`, `IndexWatermark` y `CommitSeq` (`Dependencies` en `crates/kerno-units/src/cache.rs`); no hay dependencias versionadas aparte de estadísticas, de definición de índice ni de precondiciones (ver «Revisión de 2026-11-13», punto 3) Una caché de resultados queda ligada a la secuencia de commit de su instantánea y no sirve otra secuencia; se elimina la excepción «salvo prueba de que los datos leídos no cambiaron» hasta que un ADR posterior defina esa prueba | I-35: un caso por dependencia declarada |
| G5 | El aprendizaje no se salta validaciones | Una ruta aprendida pasa por las mismas comprobaciones que la planificada: filtro, estado activo y orden (`check_units` / `check_relations`, `planner.rs`) sobre las filas que se van a devolver; ninguna caché devuelve filas sin esas comprobaciones ni evita las invariantes del motor. Un acierto de caché **no relee nada**: las filas salen de la memoria de la caché y la comprobación es sobre ellas, no una lectura verificada del almacén (ver «Revisión de 2026-11-13», punto 2) | Mutaciones que quitan una comprobación en la ruta aprendida mueren en la suite |
| G6 | La adaptación es reversible | Desactivarla (global o por consulta), invalidarla o borrar su estado no cambia datos confirmados, revisiones, relaciones, índices transaccionales ni la secuencia de commit, y la semántica de las consultas sigue igual | I-34, I-36: suite completa con la adaptación desactivada; estado lógico y `commit_seq` iguales antes y después |
| G7 | Trazabilidad | Cada decisión sobre una optimización (creación con su evidencia, activación, uso, fallback con su causa, invalidación con la dependencia que cambió, desactivación, borrado) queda registrada en el registro de transiciones de la caché con la versión de la optimización y del planificador; ese registro es en memoria y acotado, no un registro de auditoría persistido (ver «Revisión de 2026-11-13», punto 4). Que una lectura usó la caché lo dicen el `path` y el `source` de `QueryOutcome`. El registro sigue las reglas de privacidad de ADR-0015 (sin literales ni contenido) | Prueba que recorre cada transición y encuentra su registro; centinelas (I-37) |

## Decisión

Aceptada por el propietario el 2026-11-09: opción 2, sujeta a las garantías G1–G7. El estado aprendido es un dato derivado: se guarda separado de los datos de usuario, no participa en transacciones de usuario, se reconstruye desde observaciones o se descarta. Invalidación por número de versión: cada optimización registra las versiones de esquema, planificador y estado del índice de las que depende, y se comprueba antes de cada uso (las estadísticas y los datos leídos quedan cubiertos por la coincidencia exacta de la secuencia de commit, no por una versión propia); una caché de resultados además registra la secuencia de commit de su instantánea y no sirve lecturas de otra secuencia (G4).

## Revisión de 2026-11-13

El 2026-11-13 el propietario, por delegación expresa, confirmó este ADR (D16) tras revisar G1 a G7 contra el código. **G2, G3 y G6 se confirman tal como están escritas.** Donde el texto prometía más de lo que hace el código se corrigió el texto y la diferencia queda como trabajo abierto; ninguna promesa se descarta en silencio.

1. **G1 y G5, estado aprendido solo del planificador (cambio de código).** `ResultCache::create`, `ResultCache::activate`, `CacheEntry::new` y `Dependencies::current` eran públicas y reexportadas: un llamador podía plantar filas que luego se servían como respuesta. Ahora el planificador usa versiones `pub(crate)` (`store`, `arm`, `build`, `in_force`) y las cuatro públicas existen solo con la característica `test-support` de `kerno-units` y en las pruebas unitarias del crate, como en la decisión D23 ([etapa9-usos-de-engine.md](../architecture/etapa9-usos-de-engine.md)). Prueba: cuatro doctests de `ResultCache` (`cache.rs`), uno por función, que sin la característica deben **no compilar** (`compile_fail,E0599`) y con ella compilan. Mutación (restaurar la vía pública, una a la vez, en una copia): quitar el `cfg` de `cache.rs:854` (`create`), `:860` (`activate`), `:305` (`CacheEntry::new`) o `:166` (`Dependencies::current`) hace fallar el doctest correspondiente con `... compile fail ... FAILED` y «Test compiled successfully, but it's marked `compile_fail`». Las pruebas `cache`, `cache_planner` y `cache_reversibility` pasan a requerir `test-support` en `Cargo.toml`.
2. **G5, qué comprueba de verdad `check_units`.** Comprueba, sobre las filas que se van a devolver, que cada unidad está activa, que cumple el filtro y que el orden es el estrictamente ascendente por identificador de la ruta de referencia (`check_relations`, lo análogo para relaciones). Sobre un acierto **no hay lectura verificada**: las filas vienen de la memoria de la caché, y lo que garantiza que son las de la instantánea es la coincidencia exacta de dependencias (punto 3). Si la comprobación falla, el planificador abandona la caché, registra el `fallback` y responde por la ruta de referencia.
3. **G4, dependencias realmente declaradas.** El código declara `SchemaVersion`, `PlannerVersion`, `IndexState`, `IndexWatermark` y `CommitSeq`. **No hay** dependencias versionadas de estadísticas, de definición de índice ni de precondiciones. Las estadísticas y los datos leídos están cubiertos únicamente porque la entrada solo se sirve con la secuencia de commit **exactamente igual** a la de la instantánea que la creó. Esa condición es más fuerte que una versión de estadísticas o de datos: cualquier commit, sea o no relevante para la consulta, invalida la entrada, así que ninguna estadística ni dato puede haber cambiado sin que la entrada deje de servirse. El coste es que se invalida de más (falsos fallos de caché), nunca de menos; la condición de reapertura de este ADR cubre ese coste. Un `IndexState` o una marca de agua del índice distintos invalidan también aunque la secuencia no cambie (la reconstrucción del índice).
4. **G7, límites conocidos y trabajo abierto.**
   - El registro de transiciones es **en memoria y acotado** (1024 registros o una hora por defecto, `CacheConfig`) y se pierde al cerrar; no es un registro de auditoría persistido, en coherencia con la decisión D8 de la acta de la Etapa 9. Límite declarado.
   - `ResultCache::set_key` borra entradas, registro y contadores **sin dejar un registro de transición** (`cache.rs`, `set_key`; `clear` sí deja `Cleared`). Límite conocido.
   - **Trabajo abierto 1:** la explicación del plan (`Plan` / `explain_*`, `plan.rs`) no lista la caché de resultados; solo `QueryOutcome` (`path`, `source`, `index.rs`) la muestra. G7 pedía que la explicación lo dijera.
   - **Trabajo abierto 2:** la evidencia de creación es una constante (`Evidence { observations: 1, window: Duration::ZERO }`, `planner.rs`, `store()`) y las entradas no llevan estimación de coste, contra «sin alguno de estos campos no se puede activar» de [adaptive-execution.md](../architecture/adaptive-execution.md) §3. Es el hallazgo H-3 de la Etapa 6, [sigue abierto](../architecture/etapa6-cierre.md).
   - **Trabajo abierto 3:** el registro tampoco deja huella de `set_key` (arriba); queda como mejora del registro, no de la corrección.
5. **ADR-0018, derivados y supresión.** ADR-0018 cuenta las cachés y las optimizaciones aprendidas como artefactos derivados, con descriptor y con invalidación cuando se suprime una fuente (criterio 3, ADR-0009). Este ADR no lo decía. Revisado el código: **un olvido o supresión no puede dejar una respuesta de caché servible**, porque borrar una unidad, y la purga de una retención, son commits (cambian la secuencia de commit) y un acierto exige la secuencia exacta. Prueba: `crates/kerno-units/tests/cache_forget.rs`, en `emulated_log` y `kerno_db`: la respuesta se cachea y se sirve (la prueba no es vacía), se borra la unidad o se ejecuta la retención, y la lectura siguiente de una instantánea nueva no sale de la caché y no devuelve la unidad olvidada. Una instantánea anterior al olvido sigue viendo su propio estado, como toda instantánea. Relación entre los dos ADR: ADR-0016 rige la invalidación de la caché por secuencia de commit y no usa el registro de derivados de ADR-0018; la caché no tiene descriptor ni fuentes por unidad, y no los necesita para esta garantía porque su invalidación es más gruesa (todo commit). Si una caché futura sirviera datos de otra secuencia (la excepción que G4 eliminó), tendría que registrar sus fuentes como pide ADR-0018. Sigue vigente el límite de ADR-0009: nada promete borrado físico, y las filas cacheadas viven solo en memoria hasta cerrar.
6. **Discrepancia.** El comportamiento real es: la causa (`Discrepancy`) queda en el registro de transiciones de la caché, la entrada se elimina (`DeleteReason::Discrepancy`) y la respuesta es la de la ruta de referencia, que puede volver a guardarse como entrada nueva. No se desactiva la optimización para esa consulta. Ver [adaptive-execution.md](../architecture/adaptive-execution.md) §2 y §4.

## Consecuencias

- El planificador necesita versiones monotónicas para esquema, índices y estadísticas desde la Etapa 5.
- Las lecturas no mutan datos de usuario (spec §2); registrar observaciones o actualizar estadísticas desde una lectura escribe solo en el almacén derivado y nunca bloquea al escritor.
- La equivalencia exige que la ruta de referencia se mantenga y se pruebe en CI aunque no se use en producción.

## Condiciones para reabrir

Evidencia de que una segunda optimización aporta más utilidad que la elegida, o de que la invalidación por versión produce demasiados fallos de caché para ser útil.

## Seguimiento

- Elegir el mecanismo concreto al iniciar la Etapa 6, con su benchmark.
- Prueba diferencial planificador/referencia desde la Etapa 5.
