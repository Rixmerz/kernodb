# ADR-0008 — Serialización documental y hashes

| Campo | Valor |
|---|---|
| Estado | **Propuesto** (versión mínima para la Etapa 3, 2026-10-31; el propietario aprobó la dirección en la decisión 7 del [acta E3-00](../architecture/etapa3-acta-e3-00.md), pero el ADR aún no está aceptado) |
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

Propuesta mínima, limitada a lo que la Etapa 3 necesita (opción 3 de arriba); la canonicalización estructural (JCS, CBOR) y el diff quedan fuera y se decidirán en un ADR posterior:

1. **Bytes opacos.** La carga útil de una revisión son bytes que la capa no interpreta; `payload_codec = 0`. El hash se calcula sobre `payload_codec ‖ payload` tal como se guarda.
2. **Hash BLAKE3-256** (32 bytes), identificado por un byte `alg` en el campo `content_hash` (`alg = 1`), para no fijar el algoritmo en el formato de la capa. Añadir la dependencia exige justificar que ni la biblioteca estándar ni el manifiesto actual la cubren, y entra con la aceptación de este ADR, no antes.
3. **CRC-32C provisional** (`alg = 0`), ya disponible en el proyecto y **sin garantía criptográfica**, hasta que esta decisión esté completa; sirve solo para detectar discordancia extremo a extremo.
4. El hash se verifica en cada lectura de carga útil; una discordancia es `Corruption` (I-08).

**Nota (2026-10-31, E3-02).** El propietario autorizó E3-02 pidiendo explícitamente calcular y verificar `content_hash` con BLAKE3-256. Por eso la dependencia `blake3` (sin funciones por defecto, solo `std`) entró en `kerno-units` antes de aceptar este ADR. Justificación: ni la biblioteca estándar ni el manifiesto tenían un hash criptográfico. El estado sigue **Propuesto** hasta que el propietario lo acepte.

Codificación del campo: [`units-v1.md`](../file-format/units-v1.md) §4.
