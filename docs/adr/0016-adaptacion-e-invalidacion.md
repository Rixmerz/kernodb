# ADR-0016 — Adaptación operacional e invalidación

| Campo | Valor |
|---|---|
| Estado | Propuesto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
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

Ninguna todavía. Requiere el planificador determinista de la Etapa 5.

## Decisión

Pendiente. Propuesta: opción 2. El estado aprendido es un dato derivado: se guarda separado de los datos de usuario, no participa en transacciones de usuario, se reconstruye desde observaciones o se descarta. Invalidación por número de versión: cada optimización registra las versiones de esquema, índices y estadísticas de las que depende, y se comprueba antes de usarla; una caché de resultados además registra la secuencia de commit de la instantánea y no sirve lecturas de otra secuencia salvo prueba de que los datos leídos no cambiaron.

## Consecuencias

- El planificador necesita versiones monotónicas para esquema, índices y estadísticas desde la Etapa 5.
- Las lecturas no mutan datos de usuario (spec §2); registrar observaciones o actualizar estadísticas desde una lectura escribe solo en el almacén derivado y nunca bloquea al escritor.
- La equivalencia exige que la ruta de referencia se mantenga y se pruebe en CI aunque no se use en producción.

## Condiciones para reabrir

Evidencia de que una segunda optimización aporta más utilidad que la elegida, o de que la invalidación por versión produce demasiados fallos de caché para ser útil.

## Seguimiento

- Elegir el mecanismo concreto al iniciar la Etapa 6, con su benchmark.
- Prueba diferencial planificador/referencia desde la Etapa 5.
