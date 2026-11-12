# Formato de exportación lógica v0 (borrador)

> **BORRADOR, NO CONGELADO.** Este formato está versionado y sin congelar
> (acta de la Etapa 9, D10 y R-E9-2). Ningún byte es una promesa: puede
> cambiar sin migración hasta que el propietario decida congelarlo (D12). Una
> exportación v0 solo sirve para verificar I-30 (independencia del formato
> físico y reimportación en un motor nuevo), no para conservar datos.

Implementación: `crates/kerno-units/src/export.rs` (`export`, `export_snapshot`).
La importación es E9-05; la prueba de independencia física es E9-06.

## 1. Qué contiene y qué no

**Contiene**, tal como lo responden las lecturas públicas de la capa en una
instantánea: unidades, sus revisiones retenidas, relaciones y políticas de
historial.

**No contiene:**

- **Derivados** (vectores, resúmenes, descriptores, índices): se reconstruyen
  (ADR-0018).
- **Tokens ni identificadores públicos**: esperan a la decisión D1 (ADR-0005).
  Los ids son los internos de 16 bytes.
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
| `format_version` | u32 | `0` |
| `flags` | u8 | bit 0 = borrador (siempre `1` en v0) |
| `reserved` | 3 bytes | ceros |

## 3. Codificación

Enteros en big-endian. `bytes` = `u32` longitud + contenido. `opt_u64` = `u8`
(0 ausente, 1 presente) y, si está presente, `u64`. `opt_bytes` igual con
`bytes`. Las secciones van en este orden y sin relleno:

1. **Políticas.** `default_policy` u8 (`0xFF` si la base aún no tiene marca de
   capa; si no, el byte de `HistoryPolicy`), `n` u32 y `n` registros
   `{unit_type: bytes, policy: u8}` en orden de `unit_type`.
2. **Unidades.** `n` u32 y `n` registros en orden de id:
   `id` (16 bytes), `unit_type` bytes, `created_seq` u64, `created_at` i64
   (µs), `oldest` u64, `head` u64, `r` u32 y `r` revisiones en orden de número:
   `rev` u64, `kind` u8, `parent` opt_u64, `commit_seq` u64, `created_at` i64,
   `schema_version` u32, `content_hash` (`alg` u8 y 32 bytes), `payload_len`
   u32, `restored_from` opt_u64, `metadata` bytes, `actor` opt_bytes, `source`
   opt_bytes, `payload` opt_bytes (ausente en una lápida).
3. **Relaciones.** `n` u32 y `n` registros en orden de id: `id`, `from`, `to`
   (16 bytes cada uno), `relation_type` bytes, `created_seq` u64,
   `provenance` u8, `origin` bytes (vacío si es `Declared`), `confidence`
   u16 en milésimas (0 salvo `Inferred`).
4. **Resumen.** BLAKE3 de 32 bytes de todos los bytes anteriores. No se admite
   nada después.

## 4. Versionado

Un lector de v0 rechaza otra `format_version`. Mientras el formato sea
borrador, un cambio incrementa `format_version` y no hay migración.
