# Architecture Decision Records

Cada ADR registra una decisión duradera, su contexto, las opciones evaluadas, la evidencia y sus consecuencias ([plantilla](template.md)). La numeración y las preguntas iniciales de 0001–0012 provienen de [SAD §15](../architecture/SAD.md#15-architecture-decision-records-adr); 0013 del diseño de la Fase 1; 0014–0018 de la [especificación v0.2.0](../spec/README.md).

Reglas:

- Un ADR **Aceptado** no se edita para cambiar la decisión: se crea otro que lo reemplaza y se marca el anterior como *Reemplazado por ADR-XXXX*.
- Los criterios de decisión se escriben antes de mirar los resultados.
- Una decisión que dependa de benchmarks enlaza el código, el comando y los datos crudos.
- Las decisiones confirmadas D-01…D-11 del SAD §2.1 no necesitan ADR propio; si alguna se reabre, se documenta aquí.

| ADR | Título | Estado |
|---|---|---|
| [0001](0001-motor-de-almacenamiento.md) | Motor de almacenamiento físico | **Aceptado**: LSM nivelado (2026-10-28) |
| [0002](0002-transacciones-y-durabilidad.md) | Modelo de transacciones y durabilidad | **Aceptado** (2026-10-28) |
| [0003](0003-formato-fisico-y-versionado.md) | Formato físico y versionado | **Aceptado** (2026-10-28; formato concreto se congela al cerrar Fase 1) |
| [0004](0004-historial-de-revisiones.md) | Historial de revisiones | **Aceptado** (2026-10-31; [acta E3-00](../architecture/etapa3-acta-e3-00.md)) |
| [0005](0005-identificadores-publicos.md) | Identificadores públicos | Propuesto (borrador) |
| [0006](0006-cifrado-y-gestion-de-claves.md) | Cifrado y gestión de claves | Pendiente |
| [0007](0007-consistencia-de-indices.md) | Consistencia y reconstrucción de índices | Pendiente |
| [0008](0008-serializacion-documental-y-hashes.md) | Serialización documental y hashes | Propuesto (mínimo: BLAKE3-256 y bytes opacos) |
| [0009](0009-supresion-retencion-y-backups.md) | Supresión, retención y backups | Pendiente |
| [0010](0010-api-publica-y-crates.md) | API pública y límites de crates | **Aceptado** (2026-10-31; [acta E3-00](../architecture/etapa3-acta-e3-00.md)) |
| [0011](0011-vectores-y-recuperacion-hibrida.md) | Perfil de vector y recuperación híbrida | Pendiente |
| [0012](0012-metodologia-de-benchmarks.md) | Metodología de benchmarks y umbrales de salida | Propuesto (aplicada en M0) |
| [0013](0013-formato-v1-y-protocolo-del-motor.md) | Formato v1 y protocolo del motor | Propuesto (WP06; se acepta antes de congelar v1 en WP14) |
| [0014](0014-representacion-fisica-de-relaciones.md) | Representación física de relaciones | Propuesto (spec v0.2.0 §17.6; se decide en la Etapa 4) |
| [0015](0015-privacidad-de-la-observabilidad.md) | Privacidad de la observabilidad | Propuesto (spec v0.2.0 §17.11; se decide en la Etapa 5) |
| [0016](0016-adaptacion-e-invalidacion.md) | Adaptación operacional e invalidación | Propuesto (spec v0.2.0 §17.12; se decide en la Etapa 6) |
| [0017](0017-semantica-de-la-unidad-neuronal.md) | Semántica de la «unidad neuronal» | Propuesto (spec v0.2.0 §17.13; se decide al iniciar la Etapa 3) |
| [0018](0018-versionado-y-reconstruccion-de-derivados.md) | Versionado y reconstrucción de artefactos derivados | Propuesto (spec v0.2.0 §17.14; se decide antes de la Etapa 8) |

Los ADR 0014–0018 provienen de la lista de la [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §17; la correspondencia completa está en [`docs/spec/`](../spec/README.md).
