# ADR-0005 — Identificadores públicos

| Campo | Valor |
|---|---|
| Estado | Propuesto (borrador para revisión; no bloquea M0) |
| Fecha | 2026-10-27 |
| Relacionado con | SAD §5.2, §11.2, D-05, invariantes I-26, I-27, [threat model](../security/threat-model.md), ADR-0006 |

## Contexto

Cada registro tiene un `internal_id` UUIDv7 (D-05). UUIDv7 expone su instante de creación con precisión de milisegundos y, según RFC 9562, no debe tratarse como secreto ni como capacidad de seguridad. Exponerlo directamente revela cuándo se creó cada registro y facilita enumerar registros contiguos en el tiempo.

Kerno necesita una representación pública **opaca, estable y validable**, detrás de una abstracción `IdentifierCodec` intercambiable, sin que la opacidad se confunda con autorización.

## Opciones consideradas

1. **Token aleatorio opaco con mapeo persistido.** Se genera una vez por registro con un CSPRNG del sistema (≥ 128 bits), se guarda junto al registro y en un índice `token → internal_id` dentro de la misma transacción.
   - ✔ No hay claves que gestionar, rotar o perder; un token filtrado no revela nada del ID interno.
   - ✔ Se puede revocar o reemitir el token de un registro sin tocar su identidad interna.
   - ✘ Una entrada de índice extra por registro (≈ 40–60 B) y una búsqueda puntual para resolver cada token.
   - ✘ Exportar/importar exige conservar el mapeo (forma parte de los datos, no de una clave externa).
2. **Token cifrado autenticado y reversible.** `versión ‖ id_de_clave ‖ AEAD(internal_id)` con un algoritmo y biblioteca consolidados (cifrado determinista resistente a mal uso de nonce, p. ej. AES-SIV, RFC 5297), generado y almacenado una sola vez por registro.
   - ✔ Resolución sin índice; tokens válidos verificables sin acceso a la base.
   - ✘ Requiere gestión de claves, rotación y política ante pérdida de clave (ADR-0006) antes de publicar el formato.
   - ✘ Tokens más largos (≥ 40 caracteres en base32) y acoplados a la versión del esquema de cifrado.
   - ✘ Una clave comprometida desanonimiza todos los tokens emitidos con ella.
3. **Exponer el UUIDv7 o una transformación propia de sus bits.** Descartada por el SAD: revela el timestamp y la «ofuscación» casera no es seguridad.

## Criterios de decisión (SAD §15)

Estabilidad del token, rechazo de manipulación, soporte de versionado y claridad de la separación entre identificación y autorización.

## Evidencia de M0

- Resolver un token con la opción 1 cuesta una lectura puntual adicional. En M0, una lectura puntual con caché caliente cuesta entre 0,7 y 8,7 µs (p50, según motor y tamaño, de 20 000 a 1 M de registros; [informe M0](../testing/m0-report.md), `point_reads`), despreciable frente a un commit durable (cientos de µs).
- La entrada de índice extra se escribe en el mismo batch atómico que el registro (I-01), por lo que token y registro no pueden desincronizarse.

## Propuesta

- **Codec por defecto: opción 1** (token aleatorio + mapeo), con formato `k{versión}_{base32 de 128 bits aleatorios}` y, opcionalmente, un prefijo de tipo definido por la aplicación. La validación comprueba prefijo, versión, alfabeto y longitud antes de buscar.
- **Opción 2 como codec alternativo**, implementable cuando ADR-0006 defina proveedor de claves y rotación. El token se genera y almacena una vez por registro; la rotación de claves no cambia tokens ya emitidos (se conservan las claves antiguas para descifrar o se usa el token almacenado).
- En ambos casos, **un token válido no autoriza nada** (I-27): la capa que expone Kerno verifica permisos en cada operación.

## Consecuencias

- El índice `token → internal_id` es transaccional (ADR-0007) y forma parte de la exportación lógica (I-30).
- No hay material criptográfico en el camino crítico por defecto; ADR-0006 puede cerrarse después sin bloquear las fases 1–3.
- El formato del token lleva versión desde el primer día, de modo que cambiar de codec no rompe tokens existentes.

## Seguimiento

- Fase 4: implementar `IdentifierCodec`, pruebas de tokens manipulados (formato, versión, longitud, alfabeto, tokens inexistentes) y de estabilidad tras exportar/importar.
- Revisar esta propuesta cuando ADR-0006 esté aceptado.
