# Formato de exportación lógica (borrador, `format_version = 1`)

> **BORRADOR, NO CONGELADO.** Este formato está versionado y sin congelar
> (acta de la Etapa 9, D10 y R-E9-2). Ningún byte es una promesa: puede
> cambiar sin migración hasta que se congele. D12 (tomada el 2026-11-13) deja la congelación para después de D13 y de un uso real fuera del equipo. Una
> exportación v0 solo sirve para verificar I-30 (independencia del formato
> físico y reimportación en un motor nuevo), no para conservar datos.

Implementación: `crates/kerno-units/src/export.rs` (`export`, `export_snapshot`).
La importación es E9-05; la prueba de independencia física es E9-06. Desde E9-02 (fusionado el 2026-11-13) el borrador lleva los tokens de los identificadores públicos y su `format_version` es **1** (la versión 0 no los llevaba; ver §4). El nombre del fichero conserva el «v0» de su primera versión.

## 1. Qué contiene y qué no

**Contiene**, tal como lo responden las lecturas públicas de la capa en una
instantánea: unidades, sus revisiones retenidas, relaciones, políticas de
historial y el **token de identificador público** de cada unidad que lo tiene
([ADR-0005](../adr/0005-identificadores-publicos.md): el mapeo forma parte de
la exportación lógica, de modo que un token es estable tras exportar e
importar; [units-v3.md](units-v3.md)). Una unidad borrada no tiene token: borrar
quita el mapeo.

**No contiene:**

- **Derivados** (vectores, resúmenes, descriptores, índices): se reconstruyen
  (ADR-0018).
- **Los internos no se sustituyen:** los ids de unidad son los internos de 16
  bytes; el token viaja aparte, en su sección (§3, punto 2, antes de las unidades).
- **Conjuntos de cambios** (`CHANGESET`, `CS_MEMBER`): la exportación guarda,
  en cada revisión, su `commit_seq`, actor y fuente; no guarda la razón ni la
  correlación del conjunto. Es un límite declarado del borrador.
- **Nada físico**: ni la secuencia de la instantánea, ni ficheros, niveles,
  desplazamientos, estado de compactación o de reclamación. El mismo estado
  lógico da los mismos bytes (I-30, T-EX2).
- **Revisiones cuyo contenido la política no conserva**: se omiten, igual que
  para cualquier lector (`get_revision` devuelve `HistoryUnavailable`).

## 2. Cabecera propia

La magia es `KUNITEXD` (8 bytes). **`KRNO_EXP` queda reservada sin uso**: este
formato no la escribe y nada la lee (`MAGIC_EXPORT` en `format/header.rs`
sigue sin escritor).

| Campo | Tipo | Valor |
|---|---|---|
| `magic` | 8 bytes | `KUNITEXD` |
| `format_version` | u32 | `1` (la `0` no llevaba tokens y se rechaza) |
| `flags` | u8 | bit 0 = borrador (siempre `1` mientras sea borrador) |
| `reserved` | 3 bytes | ceros |

## 3. Codificación

Enteros en big-endian. `bytes` = `u32` longitud + contenido. `opt_u64` = `u8`
(0 ausente, 1 presente) y, si está presente, `u64`. `opt_bytes` igual con
`bytes`. Las secciones van en este orden y sin relleno (los tokens van antes de las unidades, de modo que el final del fichero conserva la forma de la versión 0):

1. **Políticas.** `default_policy` u8 (`0xFF` si la base aún no tiene marca de
   capa; si no, el byte de `HistoryPolicy`), `n` u32 y `n` registros
   `{unit_type: bytes, policy: u8}` en orden de `unit_type`.
2. **Tokens.** `n` u32 y `n` registros en orden estricto de id de unidad:
   `unit` (16 bytes) y `token` (16 bytes). Cada `unit` es una unidad del fichero
   que no está borrada (su última revisión no es `Delete`); un `token` no puede
   repetirse ni tener sus 16 bytes iguales. Una unidad sin token no aparece.
3. **Unidades.** `n` u32 y `n` registros en orden de id:
   `id` (16 bytes), `unit_type` bytes, `created_seq` u64, `created_at` i64
   (µs), `oldest` u64, `head` u64, `r` u32 y `r` revisiones en orden de número:
   `rev` u64, `kind` u8, `parent` opt_u64, `commit_seq` u64, `created_at` i64,
   `schema_version` u32, `content_hash` (`alg` u8 y 32 bytes), `payload_len`
   u32, `restored_from` opt_u64, `metadata` bytes, `actor` opt_bytes, `source`
   opt_bytes, `payload` opt_bytes (ausente en una lápida).
4. **Relaciones.** `n` u32 y `n` registros en orden de id: `id`, `from`, `to`
   (16 bytes cada uno), `relation_type` bytes, `created_seq` u64,
   `provenance` u8, `origin` bytes (vacío si es `Declared`), `confidence`
   u16 en milésimas (0 salvo `Inferred`).
5. **Resumen.** BLAKE3 de 32 bytes de todos los bytes anteriores. No se admite
   nada después.

`import` restaura cada token en la transacción de la primera revisión
conservada de su unidad. Un fichero con un token repetido (un token para dos
unidades) se rechaza antes de la primera escritura con `UnitError::PublicIdTaken`
(el texto del error no nombra ni el token ni ninguna unidad); el resto de
incumplimientos de esta sección son `Corruption`, también antes de escribir.

## 4. Versionado

Un lector rechaza otra `format_version` (`Incompatible`). Mientras el formato
sea borrador, un cambio incrementa `format_version` y no hay migración: la
sección de tokens es ese cambio, y por eso la versión pasó de 0 a 1. Una
exportación de versión 0 ya no se importa; volver a exportar desde la base
de origen la regenera.
