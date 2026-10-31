# ADR-0010 — API pública y límites de crates

| Campo | Valor |
|---|---|
| Estado | **Aceptado** (2026-10-31, por el propietario; [acta E3-00](../architecture/etapa3-acta-e3-00.md)) |
| Fecha | 2026-10-27 (apertura) · 2026-10-31 (aceptación) |
| Relacionado con | SAD §6, §9.3, §14 |

## Contexto

La API pública debe ser pequeña, tipada y estable; los detalles del motor físico no deben filtrarse.

**Pregunta:** Qué tipos y errores son públicos y cuáles módulos son internos.

## Opciones consideradas

1. **Un crate público `kerno`** que reexporta una fachada; crates internos no publicados.
2. **Varios crates públicos** por capa.
3. **Un único crate** con módulos privados hasta Fase 2.

## Propuesta inicial (SAD §15)

API pequeña, tipada y estable; no hacer públicos los detalles de páginas o del motor físico; módulos separados solo cuando sus contratos estén claros.

## Criterios de decisión

Ejemplo de aplicación externa que compila; guía de errores; política de deprecación y semver.

## Lo que aportó M0

El trait `Engine` del spike (hoy en `crates/kerno-core/src/contract.rs`) es un contrato **interno** de comparación, no la API pública: carece de transacciones read-modify-write, cursores, cancelación y errores por categoría. Sí aporta una taxonomía inicial de errores (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`) que conviene conservar.

## Dependencias

Depende de ADR-0002 (forma de las transacciones).

## Decisión

Se acepta, para la Etapa 3, la separación por crates del [diseño de la Etapa 3](../architecture/etapa3-design.md) (§6), con estas condiciones del propietario:

1. **Crate nuevo `kerno-units`**, genérico sobre `TxnEngine` de `kerno-core`, con separación estricta de `kerno-storage`: no depende de sus módulos internos y no introduce regresiones en los objetivos T1–T10.
2. **Marca de capa en disco** (`LAYER_META`), sin bit `incompat` para UNITS mientras `kerno-storage` sea interno.
3. El único cambio permitido fuera de `kerno-units` es `Txn::read_seq` en `kerno-core`, con sus `impl` en `WriteTxn` y `EmulatedTxn`.
4. `AsOf::Time` queda fuera de la Etapa 3.

La fachada pública única (`kerno`, opción 1) y la política de semver se deciden cuando exista una API pública estable; esta aceptación no las fija.
