# ADR-0012 — Metodología de benchmarks y umbrales de salida

| Campo | Valor |
|---|---|
| Estado | Propuesto (aplicado en M0) |
| Fecha | 2026-10-27 |
| Relacionado con | SAD §17.3–17.6, §18, ADR-0001, riesgos R-S1, R-S3, R-S4 |

## Contexto

ADR-0001 debe decidirse con evidencia comparable. Un benchmark mal planteado (durabilidad distinta, datos distintos, una sola ejecución, solo medianas) puede producir la decisión equivocada con apariencia de rigor.

## Decisión

Toda medición que respalde una decisión de arquitectura sigue estas reglas, implementadas en [`spikes/storage/spike-bench`](../../spikes/storage/spike-bench):

### Igualdad de condiciones

| Aspecto | Regla |
|---|---|
| Máquina | Todas las ejecuciones de una comparación en la misma máquina, secuencialmente (nunca en paralelo). |
| Compilación | Mismo `rustc`, perfil `release`, mismas dependencias (`Cargo.lock` versionado). |
| Datos | Mismos generadores deterministas y semillas para todos los motores. |
| Durabilidad | Equivalente y explícita (tabla siguiente). Nunca se compara durabilidad fuerte con relajada como si fueran iguales. |
| Caché | Presupuesto de 64 MiB por motor (en SQLite repartido entre conexiones). La caché de páginas del SO es compartida y se declara (`warm`), o se vacía antes de las fases de lectura (`--cold true`). |
| Compresión | Desactivada en todos (fjall la activa por defecto; se desactiva). |
| Aislamiento | Cada combinación (motor, workload, escala, repetición) corre en un **proceso nuevo** y un **directorio nuevo**, para que RAM máxima y contadores de E/S no se contaminen. |

### Equivalencia de durabilidad

| Modo del spike | Prototipos | redb | fjall | SQLite |
|---|---|---|---|---|
| `sync` (sobrevive a corte de energía al confirmar) | `Durability::Sync` | `Durability::Immediate` | `PersistMode::SyncData` por commit | `journal_mode=WAL`, `synchronous=FULL` |
| `buffered` (sin `fsync` por commit) | `Durability::Buffered` (sobrevive a muerte del proceso) | `Durability::None` — **no equivalente**: no sobrevive a la muerte del proceso | `PersistMode::Buffer` | `synchronous=NORMAL` |

Las cifras de `buffered` de redb se publican marcadas como no comparables.

### Repeticiones y calentamiento

- Cada combinación se ejecuta **3 veces** (configurable), intercalando los motores en cada repetición para que las derivas lentas de la máquina no favorezcan a uno.
- La carga de datos previa (*preload*) no se mide; la medición empieza tras reabrir la base.
- El informe muestra la **mediana**; si el rango entre mínimo y máximo supera el 15 % de la mediana, se muestra el rango junto a ella.

### Métricas obligatorias

Throughput; latencias **p50/p95/p99** (y p999/máximo donde hay colas relevantes); RAM máxima (`VmHWM`); bytes escritos / bytes lógicos (`wchar` de `/proc/self/io`, igual para todos los motores); espacio en disco / bytes lógicos; tiempo de reapertura limpia y de recuperación tras cierre sucio; `fsync` por commit en los prototipos (contados por `CountingVfs`).

### Entorno publicado

Con cada conjunto de resultados se guarda `env.json`: CPU, núcleos, memoria, kernel, sistema de archivos, versión de `rustc`, commit de git y **latencia medida de `fdatasync`** (p50/p95/p99) del disco usado, que es el suelo de cualquier commit durable.

### Escalas

`small` (20 000 registros, cabe en caché), `medium` (200 000), `large` (1 000 000, si el entorno lo permite). El tamaño es un parámetro registrado, no una condición universal.

### Reproducción

Un único comando: [`spikes/storage/run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh). Los datos crudos (`results.jsonl`) se versionan junto al informe.

## Umbrales de salida

Siguiendo SAD §17.4, **no** se fijó ningún umbral de velocidad antes de medir. Con el baseline de M0 se proponen, para la Fase 1:

1. **Correctitud (bloqueante):** cero fallos en las suites compartidas con ≥ 1000 semillas y en el barrido de todos los puntos de E/S, para ambos tipos de fallo.
2. **Regresión (bloqueante en CI de rendimiento):** ninguna métrica de throughput empeora > 10 % y ningún p99 > 20 % respecto del último baseline de la misma máquina, salvo justificación en el PR.
3. **Objetivo relativo (orientativo):** el motor elegido debe quedar dentro de 2× del motor maduro de su misma familia (redb para CoW B+ tree, fjall para LSM) en los workloads de M0 a escala `medium`, y documentar dónde no lo consigue.

## Limitaciones conocidas

- `wchar` cuenta bytes entregados a llamadas de escritura, no bytes que llegan al dispositivo (no incluye amplificación del sistema de archivos ni del FTL).
- Una VM con almacenamiento virtualizado puede tener latencias de `fsync` muy distintas a las de un SSD local; las conclusiones sobre commits pequeños se apoyan también en `fsync`/commit, que es independiente del hardware.
- Los motores de referencia no persisten la secuencia de commit del contrato del spike (no la necesitan los benchmarks).
- Los prototipos tienen días de desarrollo; sus diferencias frente a redb/fjall miden también madurez, no solo arquitectura (R-S1).
