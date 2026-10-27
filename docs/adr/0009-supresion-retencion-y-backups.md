# ADR-0009 — Supresión, retención y backups

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §8.4, §11.5, §12, NIST SP 800-88 Rev. 2 |

## Contexto

Borrar un dato actual no borra sus revisiones, índices, embeddings, páginas liberadas, segmentos antiguos ni backups.

**Pregunta:** Cómo coordinar borrado lógico/físico, revisiones, índices, embeddings y backups.

## Opciones consideradas

1. **Borrado lógico + compactación/scrub forzado** del rango afectado.
2. **Borrado criptográfico** (claves por registro o por colección destruidas).
3. **Combinación** según la clase de dato y la política.

## Propuesta inicial (SAD §15)

Políticas explícitas por colección; operaciones de supresión conscientes de datos derivados; documentación de ventanas de expiración de backups.

## Criterios de decisión

Una prueba de extremo a extremo demuestra qué se elimina inmediatamente, qué queda sujeto a compactación y qué plazo siguen los backups.

## Lo que aportó M0

M0 hizo visible que el borrado físico depende del motor: en CoW B+ tree las páginas liberadas conservan bytes antiguos hasta reutilizarse; en LSM las versiones antiguas persisten en SST y WAL hasta compactación y reciclaje. Ver §5 del [diseño de revisiones](../architecture/revision-model-design.md).

## Dependencias

Depende de ADR-0001, ADR-0004 y ADR-0006.

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
