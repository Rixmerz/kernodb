# ADR-0016 — Adaptación operacional e invalidación

| Campo | Valor |
|---|---|
| Estado | Aceptado (2026-11-09); **pendiente de confirmación** del propietario (D16 de la [acta de la Etapa 9](../architecture/etapa9-acta.md), 2026-11-13): G1 a G7 por revisar antes de confirmar |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §2, §9, §10, §15; [ejecución adaptativa](../architecture/adaptive-execution.md); ADR-0007, ADR-0012, ADR-0015, ADR-0018; invariantes I-32…I-36 |

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
| G4 | La invalidación es explícita | Cada optimización declara sus dependencias (esquema, definición de índice, estadísticas, revisiones y datos que lee, precondiciones) con número de versión; se comprueban **antes de cada uso** y una que no coincide invalida la optimización. Una caché de resultados queda ligada a la secuencia de commit de su instantánea y no sirve otra secuencia; se elimina la excepción «salvo prueba de que los datos leídos no cambiaron» hasta que un ADR posterior defina esa prueba | I-35: un caso por dependencia declarada |
| G5 | El aprendizaje no se salta validaciones | Una ruta aprendida pasa por las mismas comprobaciones que la planificada: lectura verificada, filtro, estado activo y orden (`check_units`); ninguna caché devuelve datos sin esas comprobaciones ni evita las invariantes del motor | Mutaciones que quitan una comprobación en la ruta aprendida mueren en la suite |
| G6 | La adaptación es reversible | Desactivarla (global o por consulta), invalidarla o borrar su estado no cambia datos confirmados, revisiones, relaciones, índices transaccionales ni la secuencia de commit, y la semántica de las consultas sigue igual | I-34, I-36: suite completa con la adaptación desactivada; estado lógico y `commit_seq` iguales antes y después |
| G7 | Trazabilidad | Cada decisión sobre una optimización (creación con su evidencia, activación, uso, fallback con su causa, invalidación con la dependencia que cambió, desactivación, borrado) queda registrada con la versión de la optimización y del planificador, y la explicación del plan dice si usó estado aprendido y cuál. El registro sigue las reglas de privacidad de ADR-0015 (sin literales ni contenido) | Prueba que recorre cada transición y encuentra su registro; centinelas (I-37) |

## Decisión

Aceptada por el propietario el 2026-11-09: opción 2, sujeta a las garantías G1–G7. El estado aprendido es un dato derivado: se guarda separado de los datos de usuario, no participa en transacciones de usuario, se reconstruye desde observaciones o se descarta. Invalidación por número de versión: cada optimización registra las versiones de esquema, índices y estadísticas de las que depende, y se comprueba antes de cada uso; una caché de resultados además registra la secuencia de commit de su instantánea y no sirve lecturas de otra secuencia (G4).

## Consecuencias

- El planificador necesita versiones monotónicas para esquema, índices y estadísticas desde la Etapa 5.
- Las lecturas no mutan datos de usuario (spec §2); registrar observaciones o actualizar estadísticas desde una lectura escribe solo en el almacén derivado y nunca bloquea al escritor.
- La equivalencia exige que la ruta de referencia se mantenga y se pruebe en CI aunque no se use en producción.

## Condiciones para reabrir

Evidencia de que una segunda optimización aporta más utilidad que la elegida, o de que la invalidación por versión produce demasiados fallos de caché para ser útil.

## Seguimiento

- Elegir el mecanismo concreto al iniciar la Etapa 6, con su benchmark.
- Prueba diferencial planificador/referencia desde la Etapa 5.
