# Architecture Decision Records

Cada ADR registra una decisión duradera, su contexto, las opciones evaluadas, la evidencia y sus consecuencias ([plantilla](template.md)). La numeración y las preguntas iniciales provienen de [SAD §15](../architecture/SAD.md#15-architecture-decision-records-adr).

Reglas:

- Un ADR **Aceptado** no se edita para cambiar la decisión: se crea otro que lo reemplaza y se marca el anterior como *Reemplazado por ADR-XXXX*.
- Los criterios de decisión se escriben antes de mirar los resultados.
- Una decisión que dependa de benchmarks enlaza el código, el comando y los datos crudos.
- Las decisiones confirmadas D-01…D-11 del SAD §2.1 no necesitan ADR propio; si alguna se reabre, se documenta aquí.

| ADR | Título | Estado |
|---|---|---|
| [0001](0001-motor-de-almacenamiento.md) | Motor de almacenamiento físico | Propuesto: **LSM nivelado** (recomendación con evidencia de M0) |
| [0002](0002-transacciones-y-durabilidad.md) | Modelo de transacciones y durabilidad | Propuesto (borrador con matriz de fallos) |
| [0003](0003-formato-fisico-y-versionado.md) | Formato físico y versionado | Propuesto (reglas de versionado e integridad; formato no congelado) |
| [0004](0004-historial-de-revisiones.md) | Historial de revisiones | Pendiente (diseño preliminar disponible) |
| [0005](0005-identificadores-publicos.md) | Identificadores públicos | Propuesto (borrador) |
| [0006](0006-cifrado-y-gestion-de-claves.md) | Cifrado y gestión de claves | Pendiente |
| [0007](0007-consistencia-de-indices.md) | Consistencia y reconstrucción de índices | Pendiente |
| [0008](0008-serializacion-documental-y-hashes.md) | Serialización documental y hashes | Pendiente |
| [0009](0009-supresion-retencion-y-backups.md) | Supresión, retención y backups | Pendiente |
| [0010](0010-api-publica-y-crates.md) | API pública y límites de crates | Pendiente |
| [0011](0011-vectores-y-recuperacion-hibrida.md) | Perfil de vector y recuperación híbrida | Pendiente |
| [0012](0012-metodologia-de-benchmarks.md) | Metodología de benchmarks y umbrales de salida | Propuesto (aplicada en M0) |
