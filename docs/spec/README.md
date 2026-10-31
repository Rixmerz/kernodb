# Especificaciones de producto

| Documento | Versión | Estado |
|---|---|---|
| [Especificación arquitectónica](especificacion-arquitectonica-v0.2.0.md) | 0.2.0 | Propuesta de diseño, recibida el 2026-10-28; se conserva **sin editar** |

La especificación v0.2.0 no reemplaza al [SAD v0.1.0](../architecture/SAD.md), que sigue siendo el baseline técnico, ni a los ADR aceptados. La especificación describe visión, principios y etapas. Donde contradiga una decisión aceptada, la decisión aceptada se mantiene hasta que un ADR nuevo la reemplace, según las reglas de [`docs/adr/`](../adr/README.md). Un cambio de la especificación se guarda como versión nueva en esta carpeta, sin sobrescribir la anterior.

## Correspondencia con el repositorio

### Etapas (§16) y fases del SAD (§16)

La especificación numera el trabajo en etapas 0–9; el SAD, en fases 0–6. La [hoja de ruta](../roadmap/README.md) usa las etapas para el trabajo posterior a la Fase 1 y conserva las fases ya en curso.

| Etapa (spec) | Fase (SAD) | Estado |
|---|---|---|
| 0 Diagnóstico | Fase 0 / M0 | Cerrada |
| 1 Contratos mínimos | Fase 0 (ADR-0001…0013) | Parcial: contratos de almacenamiento y transacción definidos; unidad, revisión y relación pendientes (pasan a la Etapa 3) |
| 2 Persistencia y transacciones | Fase 1 | En curso |
| 3 Unidades, revisiones y procedencia | Fases 2 y 3 | — |
| 4 Relaciones e índices | Fase 2 (índice primario, filtros) y Fase 5 (índices secundarios) | — |
| 5 Planificador y observabilidad | sin equivalente en el SAD | — |
| 6 Primera plasticidad operacional | sin equivalente en el SAD | — |
| 7 Memoria asociativa | Fase 5 (vectores exactos, ranking híbrido) | — |
| 8 Consolidación y embeddings | Fase 5 (adaptador de embeddings, invalidez de derivados) | — |
| 9 Endurecimiento | Fases 4 y 6 | — |

### ADR requeridos (§17) y ADR del repositorio

| # spec | Tema | ADR |
|---|---|---|
| 1 | Lenguaje y toolchain | Decisión confirmada del SAD §2.1 (Rust); sin ADR propio |
| 2 | Motor físico | [ADR-0001](../adr/0001-motor-de-almacenamiento.md) |
| 3 | Transacciones y concurrencia | [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) |
| 4 | Formato persistente y migraciones | [ADR-0003](../adr/0003-formato-fisico-y-versionado.md), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) |
| 5 | Snapshots, deltas y retención | [ADR-0004](../adr/0004-historial-de-revisiones.md), [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) |
| 6 | Representación física de relaciones | [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) |
| 7 | API pública y lenguaje de consultas | [ADR-0010](../adr/0010-api-publica-y-crates.md) (el lenguaje de consultas queda fuera hasta validar casos de uso, spec §13) |
| 8 | IDs internos y públicos | [ADR-0005](../adr/0005-identificadores-publicos.md) |
| 9 | Seguridad y cifrado | [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md) |
| 10 | Índices textuales y vectoriales | [ADR-0007](../adr/0007-consistencia-de-indices.md), [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) |
| 11 | Privacidad de la observabilidad | [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) |
| 12 | Adaptación e invalidación | [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) |
| 13 | Semántica de la unidad neuronal | [ADR-0017](../adr/0017-semantica-de-la-unidad-neuronal.md) |
| 14 | Versionado y reconstrucción de derivados | [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) |

## Diferencias con el SAD que el propietario debe resolver

1. **Orden de seguridad frente a búsqueda.** El SAD pone seguridad y ciclo de vida (Fase 4) antes de búsqueda e IA (Fase 5); la especificación deja la seguridad en el endurecimiento (Etapa 9), después de memoria asociativa y embeddings. La hoja de ruta propone exigir lo mínimo de la Etapa 9 que afecta a datos derivados (supresión, redacción de logs) antes de empezar la Etapa 8.
2. **Planificador y plasticidad.** Las Etapas 5 y 6 no existen en el SAD v0.1.0; se documentan en [ejecución adaptativa](../architecture/adaptive-execution.md) y ADR-0015/0016.
3. **Comparación con SQLite.** La especificación (§15) pide comparar con SQLite. El M0 ya lo incluyó como motor de referencia ([informe M0](../testing/m0-report.md)), y la tarea de benchmarks de la Fase 1 lo vuelve a medir; falta presentarlo como *baseline* explícito con durabilidad equivalente (ver la hoja de ruta).
