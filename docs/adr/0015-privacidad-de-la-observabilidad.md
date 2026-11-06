# ADR-0015 — Privacidad de la observabilidad

| Campo | Valor |
|---|---|
| Estado | **Aceptado** el 2026-11-06 con la evidencia de E5-04 (decisión del propietario del 2026-11-05; [acta de la Etapa 5](../architecture/etapa5-acta.md) §8) |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto |
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

- **Criterio 1 (sin literales ni contenido, I-37).** `crates/kerno-units/tests/sentinels.rs` planta centinelas en el tipo, la carga útil, los metadatos, el tipo de relación, el origen y la fuente de procedencia, y los valores de ambos filtros, y los busca (texto, lista decimal de bytes y hex) en los registros y contadores del observador, en las explicaciones de plan, en las huellas y en los errores de consultas y escrituras, en los cuatro niveles y con un registro dañado. Ninguno aparece, tampoco en `diagnostic`: el texto de la forma lleva marcadores tipados. Comprueba además que cada huella informada es la de la misma forma con otros literales, para que un literal metido solo en la entrada del hash no pase. La prueba incluye un control (la respuesta al llamador sí contiene los centinelas) y muere con la mutación «copiar el literal en la huella o en la explicación» en sus tres variantes: en el texto de la forma (filtro y relaciones), solo en la entrada del hash y en el coste de un candidato del plan. Informe: [E5-04](../architecture/etapa5-centinelas.md).
- **Criterio 2 (la Etapa 6 funciona con `fingerprint`).** El registro de nivel `fingerprint` lleva huella con clave, camino, plan, filas, secuencia, duración y fallback ([E5-03](../architecture/etapa5-observacion.md)); no hace falta texto.
- **Criterio 3 (supresión, ADR-0009).** Las observaciones viven solo en memoria, con retención por tamaño y edad, y no guardan contenido de unidades; una supresión de datos no deja copias en ellas. La exportación y la persistencia quedan fuera de alcance hasta una decisión nueva.
- **Criterio 4 (coste).** Umbral fijado antes de medir: ≤ 5 % de la p99 de la consulta observada. En [E5-03](../architecture/etapa5-observacion.md) §7 cumple en 7 de 8 mediciones; `diagnostic` en 20 000 unidades, ejecución 1, queda en +5,9 % (+3,6 % en la ejecución 2). Se registra como riesgo y no bloquea (acta §8, punto 5); `diagnostic` es solo por activación explícita.
- **Clave local.** La huella usa BLAKE3 con clave local no exportada (sin accesor, sin `Debug` de sus bytes): una forma tiene pocos valores y un hash sin clave caería a un diccionario ([E5-01](../architecture/etapa5-huella-y-plan.md)).

## Decisión

Opción 2, aceptada el 2026-11-06 con la evidencia de arriba, por decisión del propietario del 2026-11-05. Los literales se sustituyen por marcadores tipados antes de calcular la huella; la huella usa BLAKE3 con clave local no exportada, con algoritmo y versión registrados. El nivel por defecto es `off`; `diagnostic` solo se activa explícitamente.

## Consecuencias

- El planificador debe trabajar con huellas, no con texto de consultas.
- El nivel `diagnostic` necesita documentación operativa y aviso explícito al activarlo.
- Las observaciones son datos derivados: se excluyen de la exportación lógica por defecto y se borran sin afectar a los datos (I-34).

## Condiciones para reabrir

Un caso de uso que exija observaciones con valores (por ejemplo, depuración de una consulta concreta en producción) o una regulación aplicable que imponga requisitos distintos.

## Seguimiento

- Hecho en E5-04: prueba de centinelas y revisión del [threat model](../security/threat-model.md) (T-08, T-11).
- Si se añade un campo a una observación, una explicación o un error, la prueba de centinelas lo cubre antes de integrarlo.
