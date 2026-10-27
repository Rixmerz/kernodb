# ADR-0006 — Cifrado y gestión de claves

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §11.3, D-09, invariante I-28, [threat model](../security/threat-model.md) |

## Contexto

El cifrado en reposo y la gestión de claves afectan al formato físico (cabeceras, tamaño de página/bloque, nonces), a la recuperación y a los backups. Debe decidirse antes de congelar el formato (ADR-0003).

**Pregunta:** Alcance del cifrado de archivos y forma de administrar claves.

## Opciones consideradas

1. **Cifrado por página/bloque con AEAD** (p. ej. AES-256-GCM-SIV o XChaCha20-Poly1305 de una biblioteca auditada) — integridad criptográfica incluida; requiere espacio para nonce y tag por unidad.
2. **Cifrado a nivel de sistema de archivos/dispositivo** delegado al SO — sin cambios de formato, pero sin protección de backups exportados ni control de claves por Kerno.
3. **Cifrado por registro/campo** — permite borrado criptográfico selectivo; más metadatos y complejidad en índices.

## Propuesta inicial (SAD §15)

Algoritmos estándar y biblioteca auditada; claves fuera del directorio de datos; interfaz de proveedor de claves (`KeyProvider`) y estrategia de rotación definida.

## Criterios de decisión

Datos, journals, temporales y backups cubiertos; pérdida de clave documentada; pruebas de integridad y rotación.

## Lo que aportó M0

M0 usa CRC32C, que detecta corrupción accidental pero **no** manipulación (riesgo R-S5). Ambos candidatos escriben unidades de tamaño conocido (páginas de 4 KiB en el B+ tree; bloques de ~4 KiB más metadatos en LSM), por lo que reservar espacio para nonce + tag es viable en cualquiera. En CoW B+ tree cada página reescrita recibe una ubicación nueva, lo que facilita nonces únicos; en LSM los archivos son inmutables y se cifran una sola vez al escribirse.

## Dependencias

Condiciona ADR-0003 (cabeceras y tamaños), ADR-0009 (borrado criptográfico) y ADR-0005 (claves para tokens cifrados).

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
