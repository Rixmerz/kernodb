# ADR-0015 — Privacidad de la observabilidad

| Campo | Valor |
|---|---|
| Estado | Propuesto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.5, §7, §18; SAD §11.4; [ejecución adaptativa](../architecture/adaptive-execution.md); [threat model](../security/threat-model.md); ADR-0009, ADR-0016; invariantes I-29, I-37 |

## Contexto

La Fase 1 ya expone métricas de almacenamiento (WP04: contadores e histogramas sin contenido de usuario). Las Etapas 5 y 6 añaden observaciones de ejecución, explicaciones de plan y estadísticas, que sí pueden filtrar datos: literales de consultas, claves, valores de filtros o tamaños que revelan contenido. La especificación exige no guardar valores sensibles por defecto; el SAD §11.4 prohíbe registrar contenido completo, secretos y tokens.

## Opciones consideradas

1. **Sin observaciones persistentes**: solo métricas agregadas en memoria. Seguro, pero impide la adaptación basada en evidencia (Etapa 6).
2. **Niveles de detalle con redacción por defecto** (propuesta): `off` (por defecto), `aggregate` (contadores e histogramas por huella, sin texto), `fingerprint` (huella normalizada sin literales) y `diagnostic` (texto de huella y explicación de plan, activable solo explícitamente, con retención corta).
3. **Registro completo con cifrado**: guarda todo cifrado. Complejidad de claves (ADR-0006) y el riesgo sigue presente al descifrar para diagnóstico.

## Criterios de decisión

1. Con el nivel por defecto, ninguna salida de observabilidad contiene literales ni contenido (prueba con centinelas, I-37).
2. La adaptación de la Etapa 6 funciona con el nivel `fingerprint`.
3. La retención y supresión de observaciones cumplen ADR-0009 (una supresión de datos no deja copias en observaciones).
4. Coste de registrar una observación: medido; no puede aumentar la p99 de la consulta observada por encima del umbral que fije la Etapa 5.

## Evidencia

Ninguna todavía. Las métricas de la Fase 1 (WP04) ya cumplen el criterio 1 para el almacenamiento; falta extenderlo a consultas.

## Decisión

Pendiente. Propuesta: opción 2. Los literales se sustituyen por marcadores tipados antes de calcular la huella; la huella usa un hash con algoritmo y versión registrados. Una huella de consultas con pocos valores posibles podría permitir ataques de diccionario: en ese caso se usa un hash con clave local, no exportada.

## Consecuencias

- El planificador debe trabajar con huellas, no con texto de consultas.
- El nivel `diagnostic` necesita documentación operativa y aviso explícito al activarlo.
- Las observaciones son datos derivados: se excluyen de la exportación lógica por defecto y se borran sin afectar a los datos (I-34).

## Condiciones para reabrir

Un caso de uso que exija observaciones con valores (por ejemplo, depuración de una consulta concreta en producción) o una regulación aplicable que imponga requisitos distintos.

## Seguimiento

- Prueba de centinelas en la Etapa 5.
- Revisión del threat model para incluir observaciones y explicaciones de plan.
