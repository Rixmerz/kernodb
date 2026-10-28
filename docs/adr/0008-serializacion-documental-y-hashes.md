# ADR-0008 — Serialización documental y hashes

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §5.3, §13, RFC 8785 |

## Contexto

Los hashes de contenido de las revisiones deben ser reproducibles entre ejecuciones y versiones; eso exige una serialización canónica.

**Pregunta:** JSON inicial, soporte binario y canonicalización para hashing.

## Opciones consideradas

1. **JSON + RFC 8785 (JCS)** para hashing; almacenamiento en una codificación binaria propia o estándar (CBOR determinista, RFC 8949 §4.2).
2. **Codificación binaria canónica única** para almacenamiento y hash.
3. **Bytes opacos** (la aplicación decide) con hash sobre los bytes tal cual.

## Propuesta inicial (SAD §15)

Separar formato lógico y físico; evaluar RFC 8785 para JSON cuando sus restricciones sean aceptables; versionar el algoritmo de hash.

## Criterios de decisión

Resultados reproducibles en los casos soportados y pruebas de compatibilidad.

## Lo que aportó M0

Sin trabajo en M0 más allá de reservar en el diseño de revisiones el campo `content_hash` y el identificador de algoritmo.

## Dependencias

Condiciona ADR-0004 (deltas y hashes) y ADR-0003 (versionado del algoritmo).

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
