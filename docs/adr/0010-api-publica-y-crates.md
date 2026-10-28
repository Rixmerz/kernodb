# ADR-0010 — API pública y límites de crates

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
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

El trait `Engine` del spike (`spikes/storage/spike-core/src/engine.rs`) es un contrato **interno** de comparación, no la API pública: carece de transacciones read-modify-write, cursores, cancelación y errores por categoría. Sí aporta una taxonomía inicial de errores (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`) que conviene conservar.

## Dependencias

Depende de ADR-0002 (forma de las transacciones).

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
