# ADR-0004 — Historial de revisiones

| Campo | Valor |
|---|---|
| Estado | **Aceptado** (2026-10-31, por el propietario; [acta E3-00](../architecture/etapa3-acta-e3-00.md)) |
| Fecha | 2026-10-27 (apertura) · 2026-10-31 (aceptación) |
| Relacionado con | SAD §8, D-06, invariantes I-20…I-22 |

## Contexto

Kerno debe reconstruir y comparar versiones anteriores de los registros sin convertir el journal de recuperación en historial permanente (D-06). Hay que definir qué representa una revisión y cuándo se almacena como snapshot o como delta.

**Pregunta:** ¿Qué representa una revisión y cuándo usar snapshot/delta?

## Opciones consideradas

1. **Snapshot por revisión** — reconstrucción O(1), coste de espacio proporcional al tamaño del documento por cambio.
2. **Solo deltas** — mínimo espacio, reconstrucción O(profundidad); inaceptable sin límite.
3. **Híbrido snapshot + delta con límite de profundidad/tamaño** — propuesta del SAD.

## Propuesta inicial (SAD §15)

Revision Engine separado del journal; `ChangeSet` opcional; snapshot + delta con límite de profundidad; restauración que crea una nueva revisión.

## Criterios de decisión

Reconstrucción exacta, diff determinista y retención configurable.

## Lo que aportó M0

M0 dejó un [diseño preliminar](../architecture/revision-model-design.md) (disposición de claves `c/`, `r/`, `x/`, políticas `none`/`audit`/`versioned`) y midió su coste en ambos candidatos con el workload `history_sim` ([informe M0](../testing/m0-report.md)). El diseño no requiere nada del motor más allá de batches atómicos y scans por rango.

## Dependencias

Depende de ADR-0001 (coste de escritura/lectura de rangos y borrado físico) y de ADR-0008 (formato de delta y hash canónico).

## Decisión

Se acepta el modelo de revisiones del [diseño de la Etapa 3](../architecture/etapa3-design.md) (§2.4, §3), con estas condiciones del propietario:

1. **Copia completa por revisión** (opción 1): cada revisión guarda su carga útil entera; no hay deltas hasta que los benchmarks los justifiquen. `Delta` queda reservado en el byte `kind` de `RevisionMeta`.
2. **Una sola copia de la carga útil vigente**: vive en `REV_PAYLOAD(unit, head_rev)`, no en la cabeza; leerla cuesta 2 `get`, coste aceptado.
3. **Metadatos y clasificación versionados y auditados** en la revisión (`REV_META`).
4. El historial vive en claves propias de la capa, no en el WAL (D-06, I-20); restaurar crea una revisión nueva (I-22).
5. El hash de contenido y el formato de la carga útil como bytes opacos dependen de ADR-0008 (aceptado el 2026-10-31: BLAKE3-256).

La opción 3 (híbrido con límite de profundidad) del SAD queda como evolución futura, sujeta a medición y a un ADR nuevo.
