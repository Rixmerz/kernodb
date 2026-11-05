# Etapa 5 · E5-00 — Re-medición de U6 con índices y relaciones activos

Puerta de entrada de la Etapa 5 ([acta](etapa5-acta.md) §3, fila E5-00; riesgo K-E3-9 / M-9). Los umbrales no cambian: U6 pasa si el p99 de escritura durante `run_retention` es ≤ el peor de las dos ventanas «antes», el rendimiento es ≥ el menor de ellas y el p99 de lectura es ≤ el peor «antes» ([ADR-0012](../adr/0012-metodologia-de-benchmarks.md)).

## Cómo se midió

```
cargo run --release -p kerno-units --example unit_bench -- --only U6 --indexes --relations
```

- Modo completo (no `--quick`), semilla `0xE309`, 5 rondas, dos escritores de 1 KiB y un lector.
- `--relations`: cada actualización de un escritor crea una relación en el mismo conjunto de cambios (K-E3-9).
- `--indexes` (opción nueva del ejemplo, solo de medición): `rebuild_index` tras cargar los datos y al inicio de cada ronda, de modo que la retención corre sobre una base con entradas de índice (M-9).
- Se ejecutaron los dos presupuestos de retención que mide el ejemplo.

## Resumen

| Presupuesto de retención | Escritura p99 durante / antes | Rendimiento durante / antes | Lectura p99 durante / antes | Retención | U6 |
|---|---|---|---|---|---|
| 1000 unidades / 10000 ops | 2,104 ms / 1,671 ms (1,26×) | 3055,9 / 3348,7 ops/s (0,91×) | 0,0052 / 0,0048 ms (1,07×) | 31 940 rev/s | **FAIL** |
| 25 unidades / 100 ops | 1,877 ms / 1,795 ms (1,05×) | 3336,9 / 3155,7 ops/s (1,06×) | 0,0045 / 0,0209 ms (0,21×) | 23 711 rev/s | **FAIL** |

U6 sigue incumpliendo, como en la Etapa 3 y 4 (K-E3-9: p99 1,05–1,18×, rendimiento 0,89–0,94×). Con índices activos el presupuesto grande queda algo peor (1,26× en p99), y el pequeño solo incumple el p99 por un margen estrecho (1,05×) con el rendimiento ya dentro del umbral. Las observaciones de la Etapa 5 no escriben en el motor, así que no deberían agravar esto. El incumplimiento sigue documentado como riesgo y no bloquea (acta §4, política de merge). No se rebajó ningún umbral. Es una sola pasada en una máquina compartida: la varianza entre pasadas es del orden del margen del presupuesto pequeño.

## Salida cruda

```text
# unit_bench mode=full relations=true seed=0xe309 dir=/tmp engine=KernoDb options=Options { durability: Sync, cache_bytes: 67108864, write_buffer_bytes: 8388608 } vfs=StdVfs rounds=5
## U6: run_retention (KeepLast(1), RetentionBudget { max_units: 1000, max_ops: 10000 }) with 2 writer(s) of 1 KiB updates and 1 reader
raw U6 low load (1 writer, no reader) write: n=12845 p50_ms=0.1921 p99_ms=0.5649 max_ms=134.5764 mean_ms=0.2313
raw U6 low load throughput_ops_s=4281.4
raw U6 round 0: before_a 3.00s 10844 writes, before_b 3.00s 10369 writes, during 6.42s 22118 writes
raw U6 round 1: before_a 3.00s 10138 writes, before_b 3.00s 10713 writes, during 10.43s 26282 writes
raw U6 round 2: before_a 3.00s 8555 writes, before_b 3.00s 8065 writes, during 6.79s 22394 writes
raw U6 round 3: before_a 3.00s 10342 writes, before_b 3.00s 10590 writes, during 7.29s 22960 writes
raw U6 round 4: before_a 3.00s 11534 writes, before_b 3.00s 10498 writes, during 7.62s 24083 writes
raw U6 before_a write: n=51413 p50_ms=0.5297 p99_ms=1.6039 max_ms=21.8421 mean_ms=0.5813
raw U6 before_a read: n=6167794 p50_ms=0.0021 p99_ms=0.0048 max_ms=2.2397 mean_ms=0.0024
raw U6 before_a write_throughput_ops_s=3427.4 secs=15.00
raw U6 before_b write: n=50235 p50_ms=0.5346 p99_ms=1.6714 max_ms=305.4555 mean_ms=0.5986
raw U6 before_b read: n=6154640 p50_ms=0.0021 p99_ms=0.0044 max_ms=4.8780 mean_ms=0.0024
raw U6 before_b write_throughput_ops_s=3348.7 secs=15.00
raw U6 during write: n=117837 p50_ms=0.5478 p99_ms=2.1043 max_ms=866.4685 mean_ms=0.6524
raw U6 during read: n=14431398 p50_ms=0.0023 p99_ms=0.0052 max_ms=8.7954 mean_ms=0.0026
raw U6 during write_throughput_ops_s=3055.9 secs=38.56
raw U6 retention: calls=55 revisions_pruned=1231634 units_retained=50500 secs=38.561 revisions_per_s=31940 units_per_s=1310
verdict U6: FAIL budget 1000 units/10000 ops: write p99 during 2.104 ms vs before 1.671 ms (1.26x; low load 0.565 ms); throughput during 3055.9 ops/s vs before 3348.7 ops/s (0.91x); read p99 during 0.0052 ms vs before 0.0048 ms (1.07x); retention 31940 revisions/s
## U6: run_retention (KeepLast(1), RetentionBudget { max_units: 25, max_ops: 100 }) with 2 writer(s) of 1 KiB updates and 1 reader
raw U6 low load (1 writer, no reader) write: n=14278 p50_ms=0.1932 p99_ms=0.4617 max_ms=7.4767 mean_ms=0.2074
raw U6 low load throughput_ops_s=4758.9
raw U6 round 0: before_a 3.00s 10784 writes, before_b 3.00s 7940 writes, during 11.36s 40108 writes
raw U6 round 1: before_a 3.00s 10988 writes, before_b 3.00s 9891 writes, during 10.37s 34952 writes
raw U6 round 2: before_a 3.00s 10077 writes, before_b 3.00s 10494 writes, during 10.62s 34215 writes
raw U6 round 3: before_a 3.00s 8956 writes, before_b 3.00s 8172 writes, during 10.91s 36164 writes
raw U6 round 4: before_a 3.00s 9499 writes, before_b 3.00s 10840 writes, during 11.26s 36501 writes
raw U6 before_a write: n=50304 p50_ms=0.5359 p99_ms=1.6220 max_ms=26.9959 mean_ms=0.5939
raw U6 before_a read: n=5964102 p50_ms=0.0021 p99_ms=0.0067 max_ms=7.5707 mean_ms=0.0025
raw U6 before_a write_throughput_ops_s=3353.5 secs=15.00
raw U6 before_b write: n=47337 p50_ms=0.5676 p99_ms=1.7946 max_ms=37.7999 mean_ms=0.6315
raw U6 before_b read: n=5732162 p50_ms=0.0021 p99_ms=0.0209 max_ms=2.3089 mean_ms=0.0026
raw U6 before_b write_throughput_ops_s=3155.7 secs=15.00
raw U6 during write: n=181940 p50_ms=0.5267 p99_ms=1.8770 max_ms=197.9254 mean_ms=0.5973
raw U6 during read: n=21237913 p50_ms=0.0023 p99_ms=0.0045 max_ms=7.1740 mean_ms=0.0025
raw U6 during write_throughput_ops_s=3336.9 secs=54.52
raw U6 retention: calls=2020 revisions_pruned=1292811 units_retained=50500 secs=54.523 revisions_per_s=23711 units_per_s=926
verdict U6: FAIL budget 25 units/100 ops: write p99 during 1.877 ms vs before 1.795 ms (1.05x; low load 0.462 ms); throughput during 3336.9 ops/s vs before 3155.7 ops/s (1.06x); read p99 during 0.0045 ms vs before 0.0209 ms (0.21x); retention 23711 revisions/s
time U6: 195.6 s
# summary
U6 FAIL budget 1000 units/10000 ops: write p99 during 2.104 ms vs before 1.671 ms (1.26x; low load 0.565 ms); throughput during 3055.9 ops/s vs before 3348.7 ops/s (0.91x); read p99 during 0.0052 ms vs before 0.0048 ms (1.07x); retention 31940 revisions/s
U6 FAIL budget 25 units/100 ops: write p99 during 1.877 ms vs before 1.795 ms (1.05x; low load 0.462 ms); throughput during 3336.9 ops/s vs before 3155.7 ops/s (1.06x); read p99 during 0.0045 ms vs before 0.0209 ms (0.21x); retention 23711 revisions/s
```
