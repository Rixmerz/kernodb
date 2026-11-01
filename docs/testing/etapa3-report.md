# Informe de la Etapa 3 — métricas `U` (WP E3-09)

Estado: resultados del 2026-11-01 · Relacionado con: [diseño de la Etapa 3](../architecture/etapa3-design.md) §9.4 y decisión 12, [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [objetivos T1–T10](f1-objetivos-rendimiento.md) (sin tocar), [informe de la Fase 1](f1-report.md)

Este informe mide U1–U6 con los umbrales que el propietario aprobó **antes** de medir (decisión 12, 2026-11-01). Los umbrales no se han ajustado a los resultados: lo que no cumple figura como **incumple**. La única referencia es interna (claves simples en `KernoDb`); la comparación con SQLite queda para la Etapa 9.

## 1. Resumen

| Id | Umbral (decisión 12) | Capa | Referencia | Razón | Resultado |
|---|---|---|---|---|---|
| U1 | p99 ≤ 1,5× | `update` 1 KiB `Sync`: p99 **0,557 ms** (p50 0,221) | put 1 KiB `Sync`: p99 **0,428 ms** (p50 0,175) | **1,30×** | **Cumple** (pasada completa); en la pasada rápida 1,82× (100 muestras, sin valor estadístico) |
| U2 | p99 ≤ 1,5× | `history(id, 100 rev)` solo metadatos: p99 **0,1061 ms** (p50 0,048) | snapshot + `scan` de 100 claves de 100 B: p99 **0,0580 ms** (p50 0,017) | **1,83×** | **Incumple** |
| U3 | se informa; aviso si > 5× un `get` | `unit_as_of` con 1 000 revisiones: p99 **0,0326 ms** | snapshot + `get` 1 KiB: p99 **0,0063 ms** | **5,13×** | **Aviso** (> 5×). En una pasada anterior idéntica salió 4,37×: está en el límite |
| U4 | se informa | ver §4 | | | Informativo |
| U5 | se informa | 1 257,7 B por revisión de 1 KiB | 1 068,0 B por clave de 1 KiB | 1,178× | Informativo |
| U6 | p99 y rendimiento de escritura durante la poda no peores que antes, con la misma carga | p99 escritura durante: **5 693 ms** (presupuesto por defecto) / **217,7 ms** (25 unidades) | antes: **2,38 ms** / **2,19 ms**; baja carga 0,58 / 0,79 ms | 2 393× / 99× (p99); rendimiento 0,00× / 0,01× | **Incumple** (ambos presupuestos) |

Coste en bytes de WAL de un `update` (pendiente de E3-08): **1 416 B** por `update` de 1 KiB frente a **1 077 B** por put de 1 KiB (+339 B, 1,31×); con 64 KiB, 65 929 B frente a 65 590 B (+339 B, 1,01×). El sobrecoste es fijo por `update` (claves `10`, `20`, `21`, `31`, `30` y sus cabeceras).

## 2. Entorno, configuración y reproducción

| | |
|---|---|
| Máquina | VM KVM, Intel Xeon @ 2,10 GHz, 4 vCPU (1 hilo por núcleo), 15 GiB RAM, disco virtio `/dev/vda` ext4 (90 % ocupado). Misma clase de VM que M0/F1; no es la máquina objetivo (C-09) |
| Software | Linux 6.18.44, rustc 1.97.0, perfil `release` del espacio de trabajo |
| Commit | base `155ccf9` + este WP |
| Motor | `KernoDb` con `Options::default()`: `Sync`, caché 64 MiB, búfer de escritura 8 MiB; `StdVfs` sobre `/tmp`; hilos de fondo por defecto |
| Capa | `UnitConfig::with_sources(FixedClock::stepping(.., 1), CounterIds::new(1))`, política `Versioned`, resto por defecto |
| Semilla | `0xE309` (cada métrica la deriva con un XOR fijo) |
| Método | capa y referencia en bases distintas con las mismas opciones, en el mismo proceso; en U1–U3 se alternan por rondas (5 rondas, el orden se invierte en cada una) y cada ronda descarta un calentamiento (U1: 200 operaciones; U2/U3: 10 % de las consultas). Percentiles por rango más cercano. U2/U3 miden tras `flush` + `wait_idle` de ambas bases |

Comandos:

```
cargo run --release -p kerno-units --example unit_bench              # completa, ~7,5 min (U6 ~7,4 min)
cargo run --release -p kerno-units --example unit_bench -- --quick   # rápida, ~3 s (CI: trabajo bench-smoke)
# opciones: --dir <ruta> --seed <u64> --only U1,U3
```

Salida bruta: [`unit_bench-full.txt`](../../spikes/storage/results/2026-11-01-e3/unit_bench-full.txt), [`unit_bench-quick.txt`](../../spikes/storage/results/2026-11-01-e3/unit_bench-quick.txt), entorno en [`env.txt`](../../spikes/storage/results/2026-11-01-e3/env.txt). La pasada rápida es una prueba de humo: sus cifras no sirven para decidir umbrales.

Tamaños de la pasada completa: U1 1 000 unidades/claves, 5 × 1 000 operaciones medidas por lado; U2 100 unidades × 100 revisiones de 100 B frente a 10 000 claves de 100 B, 25 000 consultas por lado; U3 10 unidades × 1 000 revisiones de 1 KiB frente a 10 000 claves de 1 KiB, 25 000 consultas; U4 100 unidades, 2 000 `update` de 1 KiB y 500 de 64 KiB; U5 100 unidades × 100 revisiones de 1 KiB; U6 10 000 unidades frías con 20 revisiones nuevas de 100 B por ronda (5 rondas) y 100 unidades calientes.

## 3. U1–U3: latencias

U1 por ronda (p99 en ms, `update` / put): 0,448/0,504 · 0,548/0,466 · 0,529/0,466 · 1,232/0,393 · 0,449/0,393. El p99 agregado cumple (1,30×), pero una ronda (3) supera 1,5× por sí sola: el margen no es amplio y depende de la varianza del `fdatasync` de la VM. La media es 0,245 ms frente a 0,195 ms (1,25×).

U2 incumple: `history` hace snapshot, lee la cabeza y la política (`get`), escanea 100 `REV_META` y decodifica y valida cada uno (orden de `commit_seq`, IU-3); la referencia solo escanea. p50 2,8×, p99 1,83×. No se baja el umbral; las causas probables (lecturas previas al `scan`, decodificación con comprobaciones) son candidatas a optimizar en un WP posterior.

U3: 5,13× un `get` en esta pasada (4,37× en una pasada previa igual), 0,0326 ms absolutos. Se marca el aviso. **No se añade el índice invertido** (§3.5): antes hay que justificar su coste (escritura y espacio extra por revisión, formato bajo `layer_version`) frente a una latencia absoluta de decenas de µs. Como contexto, `get_unit` (cabeza) p99 0,0075 ms.

## 4. U4: amplificación de escritura

| Tamaño | Lado | Bytes WAL por op. | WAL total | Flush | Compactación | (WAL+flush+compactación)/carga útil |
|---|---|---|---|---|---|---|
| 1 KiB (2 000 ops) | `update` | 1 416 | 2 832 000 | 2 632 819 | 0 | 2,67 |
| 1 KiB | put | 1 077 | 2 154 000 | 107 258 | 0 | 1,10 |
| 64 KiB (500 ops) | `update` | 65 929 | 32 964 500 | 32 982 172 | 39 539 895 | 3,22 |
| 64 KiB | put | 65 590 | 32 795 000 | 19 101 564 | 6 563 980 | 1,78 |

La diferencia en flush/compactación es estructural: los puts sobrescriben 100 claves (el memtable se queda con la última versión), mientras que cada `update` conserva una revisión nueva (decisión 4: copia completa). Es el coste esperado del historial, no un defecto. Los bytes de WAL confirman la hipótesis de §9.3: el sobrecoste es de bytes fijos (+339 B por `update`), no de `fsync`.

## 5. U5: espacio por revisión

Tras 10 000 revisiones de 1 KiB (100 unidades × 100) y compactación completa (`compact_range` de todo el rango): 12 577 291 B en tablas vivas, **1 257,7 B por revisión**, frente a 10 679 854 B para 10 000 claves distintas de 1 KiB (**1 068,0 B**). Sobrecoste **+189,7 B por revisión (1,178×)**: `REV_META`, clave de `REV_PAYLOAD` y la parte de `CHANGESET`/miembro.

## 6. U6: retención con escritura concurrente

Carga idéntica en las tres ventanas: 2 hilos escritores en bucle cerrado (`update` 1 KiB `Sync` sobre 100 unidades calientes) y 1 lector (`get_unit`). Dos ventanas «antes» de 3 s (A y B, para ver el ruido) y la ventana «durante» = `run_retention(KeepLast(1))` repetido hasta `complete`. 5 rondas. Baja carga = 1 escritor solo.

| Presupuesto | Ventana | p99 escritura (ms) | p50 escritura (ms) | Rendimiento (ops/s) | p99 lectura (ms) |
|---|---|---|---|---|---|
| por defecto (1 000 u./10 000 ops) | baja carga | 0,581 | 0,221 | 4 018 | — |
| | antes A / B | 1,761 / 2,379 | 0,624 / 0,687 | 2 959 / 2 584 | 0,0171 / 0,0224 |
| | **durante** | **5 693** | 1 420 | **1,1** | 0,0762 |
| 25 u./100 ops | baja carga | 0,790 | 0,221 | 3 745 | — |
| | antes A / B | 1,940 / 2,193 | 0,663 / 0,601 | 2 748 / 2 963 | 0,0234 / 0,0072 |
| | **durante** | **217,7** | 74,4 | **24,6** | 0,1255 |

Rendimiento de la retención: 7 083 revisiones/s (327 unidades/s) con el presupuesto por defecto y 6 681 revisiones/s con 25 unidades. **Incumple** con ambos presupuestos: la escritura concurrente queda casi bloqueada durante la poda.

Causa (de la lectura de `retention.rs`, no medida aparte): `run_retention` toma la ranura de escritura única al empezar y, dentro de ella, recorre **todas** las cabezas `UNIT_HEAD` desde el principio en cada llamada, incluidas las ya podadas; el presupuesto solo limita las unidades que se escriben, no el recorrido. Con 10 100 unidades cada llamada retiene la ranura segundos, y reducir el presupuesto multiplica las llamadas sin acortar el recorrido. Mitigaciones posibles, a decidir en otro WP: reanudar desde un cursor devuelto por la llamada anterior, limitar también las unidades examinadas, o planificar fuera de la ranura y validar dentro. Nada de esto se ha cambiado aquí. T1–T3 no se han vuelto a medir en este WP; este resultado indica que, mientras se ejecute la retención con carga, los commits concurrentes empeoran mucho más allá de lo medido en baja carga.

## 7. Pendiente e incertidumbres

- U2 y U6 incumplen; U3 está en el límite de 5× con varianza entre pasadas; U1 cumple con margen moderado (una ronda de cinco > 1,5×).
- Una sola máquina (VM con disco virtual); la máquina objetivo sigue sin nombrar (C-09).
- `docs/testing/f1-objetivos-rendimiento.md` (T1–T10) no se ha modificado.
