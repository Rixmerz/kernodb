# Informe de la Etapa 3 — métricas `U` (WP E3-09)

Estado: resultados del 2026-11-01 · Relacionado con: [diseño de la Etapa 3](../architecture/etapa3-design.md) §9.4 y decisión 12, [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [objetivos T1–T10](f1-objetivos-rendimiento.md) (sin tocar), [informe de la Fase 1](f1-report.md)

Este informe mide U1–U6 con los umbrales que el propietario aprobó **antes** de medir (decisión 12, 2026-11-01). Los umbrales no se han ajustado a los resultados: lo que no cumple figura como **incumple**. La única referencia es interna (claves simples en `KernoDb`); la comparación con SQLite queda para la Etapa 9.

## 1. Resumen

| Id | Umbral (decisión 12) | Capa | Referencia | Razón | Resultado |
|---|---|---|---|---|---|
| U1 | p99 ≤ 1,5× | `update` 1 KiB `Sync`: p99 **0,557 ms** (p50 0,221) | put 1 KiB `Sync`: p99 **0,428 ms** (p50 0,175) | **1,30×** | **Cumple** (pasada completa); en la pasada rápida 1,82× (100 muestras, sin valor estadístico) |
| U2 | p99 ≤ 1,5× | `history(id, 100 rev)` solo metadatos: antes p99 0,1061 ms (p50 0,048); **después p99 0,0541 ms** (p50 0,028) | snapshot + `scan` de 100 claves de 100 B: antes 0,0580 ms; después **0,0367 ms** (p50 0,017) | antes 1,83×; **después 1,47×** (repetición: 1,53×) | **En el límite**: cumple en la pasada completa, incumple por poco en la repetición (ver §3) |
| U3 | se informa; aviso si > 5× un `get` | `unit_as_of` con 1 000 revisiones: p99 **0,0326 ms** | snapshot + `get` 1 KiB: p99 **0,0063 ms** | **5,13×** | **Aviso** (> 5×). En una pasada anterior idéntica salió 4,37×: está en el límite |
| U4 | se informa | ver §4 | | | Informativo |
| U5 | se informa | 1 257,7 B por revisión de 1 KiB | 1 068,0 B por clave de 1 KiB | 1,178× | Informativo |
| U6 | p99 y rendimiento de escritura durante la poda no peores que antes, con la misma carga | p99 escritura durante: antes del arreglo 5 693 ms / 217,7 ms; **después 2,373 ms** (presupuesto por defecto) / **3,324 ms** (25 unidades) | ventanas «antes»: 2,069 ms / 2,096 ms | después: 1,15× / 1,59× (p99 escritura); rendimiento 1,14× / 0,76×; p99 lectura 1,87× / 6,07× | **Incumple** (ambos presupuestos), aunque el p99 de escritura baja ~2 400× |

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

U2 (antes del arreglo): `history` hacía snapshot, leía la cabeza y la política (`get`), escaneaba 100 `REV_META` (ya era un único `scan`, sin lecturas puntuales por revisión) y decodificaba y validaba cada uno; p50 2,8×, p99 1,83×.

Perfil (callgrind y cronometraje por fases, 100 revisiones): `scan` del motor ≈ 15–17 µs (igual que la referencia), decodificación de `REV_META` ≈ 10 µs (≈ 100 ns por registro con el lector campo a campo), `Key::decode` por entrada ≈ 1 µs, cabeza + política ≈ 1,6 µs. Cambios (sin tocar el formato en disco):

- `RevisionMeta::decode` tiene una vía rápida de desplazamientos fijos que acepta solo lo que acepta el decodificador comprobado, con el mismo resultado; ante cualquier anomalía devuelve al decodificador comprobado, que nombra la regla exacta (prueba `rev_meta_fast_path_agrees_with_checked` con 20 000 valores válidos y mutados). Decodificar 100 registros pasa de ≈ 10 µs a ≈ 3,6 µs.
- `history` compara los bytes de la clave esperada en lugar de decodificar cada clave.

Después: p50 0,028 ms frente a 0,017 ms (1,65×), p99 **0,0541 frente a 0,0367 ms = 1,47×** en la pasada completa; en una repetición inmediata de U1/U2, 0,0597 frente a 0,0389 ms = **1,53×**; en cinco pasadas sueltas de U2 previas, 1,28×–1,54× (cuatro de cinco cumplen). Es decir, U2 queda **en el límite**, no cumple con margen. Lo que queda por encima de la referencia es estructural: decodificar y validar 100 `REV_META`, construir 100 `Revision` (224 B cada una) y dos lecturas previas al `scan`. El umbral no se baja.

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

Rendimiento de la retención (antes del arreglo): 7 083 revisiones/s con el presupuesto por defecto y 6 681 con 25 unidades; la escritura concurrente quedaba casi bloqueada. Causa: `run_retention` tomaba la ranura de escritura al empezar y, dentro, recorría **todas** las cabezas desde el principio en cada llamada (las búsquedas dentro de una transacción con miles de borrados de rango pendientes son además cuadráticas).

### 6.1 Arreglo (en este WP)

- **Planificación fuera de la ranura**: cada llamada lee cabezas y `REV_META` en un snapshot, sin ranura. Bajo la ranura solo comprueba que la cabeza de cada unidad planificada (bytes idénticos) y la política de su tipo siguen igual, vuelve a planificar en la transacción las que cambiaron, y escribe. Si la cabeza no cambió, sus revisiones retenidas tampoco (toda escritura de la capa que las toca reescribe la cabeza); `set_policy` incrementa una época que invalida el cursor.
- **Cursor reanudable en memoria**: una pasada incompleta continúa en la primera unidad no terminada (misma política, sin `KeepFor`, sin cambio de política). Una pasada completa, un error o una reapertura vuelven a empezar desde la primera unidad. Tras una caída no hay nada que recuperar (la poda de cada unidad sigue siendo una sola confirmación del motor, I-21/IU-7).
- **Trabajo de recorrido acotado**: una llamada lee como mucho 4 096 cabezas sin trabajo; puede devolver `complete = false` sin conjunto de cambios.
- **Rangos primero**: el conjunto de cambios escribe todos los borrados de rango antes que los puntos (cabezas, miembros, `REV_META` reescritos), que nunca caen en esos rangos; el resultado es el mismo y el motor no reexamina los puntos ya escritos con cada rango nuevo.

Las pruebas existentes (incluidas las comparaciones con el modelo de `policies.rs`, `retention.rs` y los barridos de caídas de `tests/harness/units.rs`) pasan sin cambios.

### 6.2 Después del arreglo (pasada completa, `unit_bench-full-after.txt`)

| Presupuesto | Ventana | p99 escritura (ms) | p50 escritura (ms) | Rendimiento (ops/s) | p99 lectura (ms) |
|---|---|---|---|---|---|
| por defecto | baja carga | 0,647 | — | — | — |
| | antes A / B | 2,069 / 1,725 | 0,622 / 0,571 | 2 839 / 3 128 | 0,0222 / 0,0135 |
| | **durante** | **2,373** (antes 5 693) | 0,510 | **3 232** (antes 1,1) | 0,0415 |
| 25 u./100 ops | baja carga | 0,493 | — | — | — |
| | antes A / B | 1,606 / 2,096 | 0,575 / 0,581 | 3 123 / 3 026 | 0,0140 / 0,0118 |
| | **durante** | **3,324** (antes 217,7) | 0,741 | **2 290** (antes 24,6) | 0,0852 |

Retención: 18 163 revisiones/s (por defecto) y 19 294 (25 unidades), frente a 7 083 / 6 681. **U6 sigue incumpliendo** con ambos presupuestos: por defecto, el rendimiento ya cumple (1,14×) pero el p99 de escritura es 1,15× el peor «antes» y el p99 de lectura 1,87×; con 25 unidades, p99 1,59×, rendimiento 0,76× y lectura 6,07× (2 020 llamadas, cada una con su `fsync`).

Causa restante (medida): la lectura y el recorrido sufren el coste del motor con muchos rangos en el memtable (`Memtable::covering_seq` lineal en hasta 4 096 rangos por `get`, y `Memtable::fragments` recalculado en cada `scan`; ambos marcados `ponytail` en `kerno-storage`). Experimento **no incluido** en el commit (§9.5 del diseño exige un WP propio con comparación de T1–T10 para tocar `kerno-storage`): con una caché de fragmentos en el memtable, el p99 de lectura durante la poda quedó en 1,12× y la retención subió a ≈ 235 000–388 000 revisiones/s, pero entonces la retención, al ir tan rápido en bucle cerrado, ocupa la ranura buena parte del tiempo (≈ 50 ms por conjunto de 1 000 unidades) y el p99 de escritura empeoró a 30–76 ms (`unit_bench-u6-experiment-engine-fragcache.txt`). Cumplir U6 tal como está definido (retención en bucle cerrado sin efecto en p99) exige además ritmo de la retención o conjuntos de cambios más cortos en tiempo de ranura; no se ha hecho porque cambiaría el contrato de la API (un conjunto de cambios por llamada) o la metodología del banco.

## 7. Pendiente e incertidumbres

- Tras E3-09b (§8): U2 cumple en 2 de 3 pasadas; U6 sigue incumpliendo en las 3. Antes de E3-09b: U6 incumple (mucho menos que antes); U2 está en el límite (1,47× / 1,53× en dos pasadas); U3 está en el límite de 5× con varianza entre pasadas (4,85× en la pasada posterior); U1 cumple (1,25× / 1,43×).
- Salida bruta posterior al arreglo: `unit_bench-full-after.txt`, `unit_bench-u1u2-after-rerun.txt`; la anterior se conserva en `unit_bench-full.txt`.
- Una sola máquina (VM con disco virtual); la máquina objetivo sigue sin nombrar (C-09).
- `docs/testing/f1-objetivos-rendimiento.md` (T1–T10) no se ha modificado.

## 8. WP E3-09b: índice de rangos del memtable, ritmo de la retención y coste fijo de `history`

Aprobado por el propietario dentro del PR #13. Objetivo: U2 y U6 cumplen de forma estable sin bajar umbrales, sin tocar T1–T10 ni la metodología de `unit_bench`. **Resultado: no se alcanza.** U2 cumple en 2 de 3 pasadas completas; U6 incumple en las 3 (mucho más cerca que antes). Detalle abajo.

### 8.1 Cambios

- **`kerno-storage` (WP propio según §9.5 del diseño):** el memtable mantiene sus borrados de rango fragmentados de forma incremental (`RangeIndex`: `BTreeMap` de trozos disjuntos, cada uno con las secuencias que lo cubren). Añadir un rango cuesta `O(log n + k)` bajo el cerrojo de rangos (sin reconstrucción); `covering_seq` pasa de lineal a `O(log n)`; `fragments(seq)` se recorre del índice y, para lectores en o después del último rango, se comparte una lista (`Arc`) construida en la primera lectura tras un cambio, no en la escritura. Sin rangos no hay coste ni asignación. Sin cambios de formato; los dos `ponytail` desaparecen. Prueba nueva `range_index_matches_the_reference` (300 semillas, contra `rangedel::fragment` y una pasada lineal, en cada snapshot), vista fallar por mutación.
- **`kerno-units`, ritmo de la retención (decisión 12b del diseño):** `UnitConfig::retention_pacing` (`RetentionPacing { units_per_change_set: 16, pause_ratio: 9 }` por defecto). Una llamada a `run_retention` escribe su presupuesto en varios conjuntos de cambios de como mucho 16 unidades y, tras cada uno, pausa 9× el trabajo desde la pausa anterior (retención ocupada ~10 % del tiempo). `RetentionReport` añade `change_sets` (todos, en orden); `change_set` es el último. La poda de cada unidad sigue en un solo conjunto de cambios (I-21/IU-7).
- **`kerno-units`, `history`:** caché de políticas por época compartida por los snapshots (la época sube antes y después de todo commit que puede cambiar una política; un snapshot tomado con la época impar o cambiando no usa la caché); página construida directamente en el orden pedido (sin `reverse`) y decodificación de `REV_META` en línea. Mismo formato en disco. Prueba nueva `policy_cache_follows_snapshots`; quitar la comprobación de época hace fallar esa prueba y los barridos de caídas.
- **Barridos de caídas:** `retention_every_point` se amplía con `paced_power_loss_sync` y `paced_process_kill_buffered` (1 unidad por conjunto de cambios): tras cada caída el almacén es exactamente un prefijo de los conjuntos de cambios de la llamada, `verify_units` limpio, cabezas intactas y la retención termina el trabajo. El arnés de `model_crash`/`changeset_every_point` (`tests/harness/units.rs`), cuyo modelo es «un paso = un commit con secuencia predicha», fija `units_per_change_set = usize::MAX`.

### 8.2 T1–T10 antes y después (`spike-bench`, misma sesión, binarios intercalados)

Método: binario «antes» (HEAD `0cdd10f`) y «después» (este WP) de `spike-bench`, cada carga×repetición ejecutada alternando antes → después → `lsm`/`btree`/`fjall` (referencias de la misma sesión), escala `medium`, `Sync`, 3 repeticiones (semillas 42–44), `concurrent_commits` con 8 hilos y aparte con 1; `bulk_load` `large` 1 repetición. Datos y script en `spikes/storage/results/2026-11-01-e3/t1-t10-e3-09b/`. Medianas de `kerno`:

| Id | Métrica | Antes | Después | Después/antes | Referencias (misma sesión) |
|---|---|---|---|---|---|
| T1 | `agent_memory` p99 escritura | 2 257 µs | 2 314 µs | 1,03 | fjall 760 · btree 922 · lsm 5 620 |
| T2 | LiveSpec commit p99 | 2 605 µs | 2 265 µs | 0,87 | lsm 4 112 |
| | Cauce step commit p99 | 885 µs | 930 µs | 1,05 | lsm 464 |
| | `append_seq` inserción / actualización p99 | 1 756 / 3 097 µs | 1 798 / 2 927 µs | 1,02 / 0,95 | lsm 3 830 / 5 599 |
| | `history_sim` p99 | 5 022 µs | 4 870 µs | 0,97 | lsm 8 368 |
| T3 | module-scan / symbol-get p99 | 575 / 29,6 µs | 587 / 30,2 µs | 1,02 / 1,02 | lsm 518 / 28,0 |
| | step-read / recall / recent-scan p99 | 81,4 / 53,9 / 63,2 µs | 81,6 / 51,5 / 58,8 µs | 1,00 / 0,95 / 0,93 | lsm 80,3 / 51,0 / 57,4 |
| T4 | cierre `bulk_load` medium / large | 185,5 / 375,7 ms | 194,8 / 336,3 ms | 1,05 / 0,90 | |
| T5 | `bulk_load` / `point_reads` 1h / 4h / `range_scans` (kops) | 162 / 250 / 841 / 11,7 | 181 / 244 / 800 / 11,8 | 1,11 / 0,98 / 0,95 / 1,01 | lsm 187 / 273 / 866 / 15,3 |
| | `small_commits` / `point_deletes` / `cauce` / `agent_memory` (ops/s) | 5 661 / 5 327 / 3 938 / 1 438 | 5 695 / 5 649 / 3 852 / 1 420 | 1,01 / 1,06 / 0,98 / 0,99 | |
| | `mixed` kops / `rmw_counters` txn/s | 67,0 / 3 790 | 65,9 / 4 133 | 0,98 / 1,09 | |
| T6 | `fsync`/commit 1 y 8 escritores; `cc` 8 hilos | 1,00 · 0,218 · 15 160/s | 1,00 · 0,218 · 15 300/s | 1,00 · 1,01 | |
| | `concurrent_commits` 1 hilo | 5 952/s | 5 509/s | 0,93 | repetición ×6: 5 228 → 5 459 (1,04) |
| T7 | amplificación `small_commits` / `bulk_load` | 4,05 / 3,33 | 1,17 / 3,31 | — / 1,00 | (T7 de `small_commits` es inestable entre repeticiones, ya documentado en F1) |
| T8 | reapertura / recuperación (`bulk_load` medium) | 3,09 / 7,34 ms | 4,72 / 8,22 ms | 1,53 / 1,12 | ver repetición |
| T9 | `rmw_counters` p99 | 4 375 µs | 3 453 µs | 0,79 | |

Las diferencias de reapertura/recuperación y de `concurrent_commits` con 1 hilo se repitieron porque superaban el ruido esperado (ninguna ruta de puntos cambió): 9 repeticiones intercaladas por lado de `bulk_load` con el binario final dan reapertura 3,46 → 3,52 ms (1,02), recuperación 8,64 → 7,25 ms (0,84), cierre 183,6 → 194,2 ms (1,06; rangos 171–282 / 178–207); `concurrent_commits` 1 hilo ×6: 1,04. **Conclusión: ninguna regresión de T1–T10 por encima del ruido entre repeticiones.** El binario final solo difiere del medido en la tabla en no asignar un `Arc` vacío por `scan` cuando no hay rangos.

### 8.3 `unit_bench` completo, 3 pasadas (`unit_bench-full-e3-09b-run{1,2,3}.txt`)

| Id | Pasada 1 | Pasada 2 | Pasada 3 |
|---|---|---|---|
| U1 (≤ 1,5×) | 0,595 / 0,349 ms = 1,71× **incumple** | 0,634 / 0,553 = 1,15× | 0,600 / 0,674 = 0,89× |
| U2 (≤ 1,5×) | 0,0555 / 0,0332 ms = **1,67× incumple** | 0,0613 / 0,0425 = 1,44× | 0,0606 / 0,0461 = 1,32× |
| U3 (aviso > 5×) | 6,79× aviso | 5,53× aviso | 4,71× |
| U4 / U5 | sin cambios (+339 B; 1,178×) | igual | igual |
| U6 por defecto: p99 escr. / rend. / p99 lect. | 1,26× / 0,91× / 0,87× | 1,09× / 0,95× / 1,02× | 1,08× / 0,98× / 0,69× |
| U6 25 u.: p99 escr. / rend. / p99 lect. | 1,11× / 0,96× / 1,90× | 1,17× / 0,95× / 0,73× | 0,98× / 0,95× / 2,04× |
| U6 veredicto | incumple ×2 | incumple ×2 | incumple ×2 |
| Retención (rev/s, por defecto / 25 u.) | 16 112 / 13 997 | 17 886 / 13 226 | 18 898 / 13 794 |

U1 no se tocó en este WP; su incumplimiento en la pasada 1 es variación del p99 de la referencia (0,349 frente a 0,553 y 0,674 ms en las otras), no de la capa (0,595–0,634 ms en las tres).

### 8.4 Por qué U2 y U6 siguen sin cumplir de forma estable

- **U2.** Desglose medido (instrumentación temporal, no confirmada): cabeza + política ≈ 1,0–1,3 µs (antes ~3 lecturas del motor, ahora 1); `scan` de 100 `REV_META` ≈ 15,5 µs (igual que todo el `scan` de referencia); decodificar y construir 100 `Revision` ≈ 9–10 µs (≈ 46 ns por decodificación ya por la vía rápida, el resto construcción). El coste fijo que pedía el WP ya es ~5 % del total; lo que queda es por revisión. La razón en p50 es ~1,6× y el p99 de ambos lados varía ±25 % entre pasadas, así que el veredicto oscila alrededor de 1,5×. Bajar más exigiría no materializar `Revision` (cambio de API) o un `scan` sin copias en el motor.
- **U6.** Con el índice de rangos la lectura durante la poda dejó de degradarse sistemáticamente (0,69–1,02× por defecto) y con el ritmo el p99 de escritura queda en 1,08–1,26× (antes de E3-09: 2 400×; tras E3-09: 1,15×). Pero el criterio exige no empeorar *nada* frente a la peor ventana «antes», y la poda de 1,1 M de revisiones por ronda genera trabajo de fondo inevitable (cada conjunto de cambios con su `fsync`, rotaciones de memtable por 4 096 rangos, vaciados y compactaciones) que cuesta un 2–9 % de rendimiento del escritor aunque la retención solo esté activa ~10 % del tiempo. Barrido de ritmo (una pasada U6 cada uno): 32 u./1× → rendimiento 0,70–0,76×; 8 u./4× (pausa por tiempo bajo ranura) → 0,88–0,95×; 4 u./9× → 0,90–0,94×; 16 u./9× (pausa por trabajo total, elegido) → 0,95–0,98×. El rendimiento no llega a 1,0× al bajar más el ciclo de trabajo, y el p99 de lectura del lector (decenas de µs) varía 2–3× entre ventanas «antes» de la misma pasada, por lo que también oscila. Cumplir U6 tal como está definido requeriría, como mínimo, commits de retención sin `fsync` propio (durabilidad por commit en el motor, que no existe en el contrato) o que la poda no genere vaciados/compactaciones durante la ventana; ninguna de las dos cabe en este WP sin cambiar el contrato del motor.
