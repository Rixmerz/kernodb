# Acta E3-00: puertas de entrada de la Etapa 3

| Campo | Valor |
|---|---|
| Fecha | 2026-10-31 |
| Acto | Aprobación del paquete E3-00 por el propietario del proyecto |
| Documento | [Diseño de la Etapa 3](etapa3-design.md), §11 (E3-00) y §12 |
| Resultado | **Aprobado** |

## Condición de inicio

El roadmap exige, antes de la Etapa 3, la Fase 1 cerrada, el formato v1 congelado y ADR-0013 aceptado. Estado comprobado en esta fecha:

- **Fase 1 cerrada** (PR #2, integrado).
- **Formato v1 congelado** y **ADR-0013 aceptado** (PR #3): el bit `incompat` `DRAFT` está retirado y `docs/file-format/v1.md` ya no lo lista como activo.

## Decisiones de §12

| # | Decisión | Resultado | Condición del propietario (literal) |
|---|---|---|---|
| 1 | Condición de inicio | **Satisfecha** (congelación hecha) | — |
| 2 | Crate y API (ADR-0010) | Aceptada como recomienda el diseño | Nuevo crate `kerno-units`, separación estricta de `kerno-storage`, sin regresiones en T1–T10. |
| 3 | Bit de característica | Aceptada como recomienda el diseño | Marca de capa en disco; sin bit `incompat` para UNITS mientras `kerno-storage` sea interno. |
| 4 | Snapshots frente a deltas | Aceptada como recomienda el diseño | Revisiones de copia completa; sin deltas hasta que los benchmarks los justifiquen. |
| 5 | Carga útil vigente en la cabeza | Aceptada como recomienda el diseño | Una sola copia; se acepta el coste de 2 `get`. |
| 6 | Metadatos versionados | Aceptada como recomienda el diseño | Metadatos y clasificación versionados y auditados en la revisión. |
| 7 | Hash | Aceptada como recomienda el diseño | BLAKE3-256 con su ADR (ADR-0008); CRC-32C provisional hasta completar esa decisión. |
| 8 | `AsOf::Time` | Aceptada como recomienda el diseño | Fuera del alcance de la Etapa 3. |

Las decisiones 9–13 de §12 **no** forman parte de esta aprobación y siguen abiertas.

## Alcance de la aprobación

La aprobación **no autoriza** funcionalidades de etapas posteriores ni cambios innecesarios en el motor de almacenamiento. El único cambio permitido fuera de `kerno-units` es `Txn::read_seq` (E3-01).

## Efectos

- ADR-0004 y ADR-0010 pasan a **Aceptado** (2026-10-31, esta acta).
- ADR-0008 se propone en su versión mínima (hash BLAKE3-256 y bytes opacos) como **Propuesto**.
- E3-01 puede comenzar; E3-02 y siguientes dependen de ella.
