# Diagnóstico de las colas de latencia de escritura

| Campo | Valor |
|---|---|
| Estado | Diagnóstico de WP04 ([diseño de la Fase 1](../architecture/phase1-design.md) §5.4). Mediciones **provisionales**: la máquina se compartía con otros paquetes que compilaban y ejecutaban pruebas (carga de 1 minuto entre 1,4 y 14 durante las ejecuciones). La medición de referencia es la de WP14, con la máquina en reposo |
| Fecha | 2026-10-28 |
| Código medido | `b742393` (`kerno-storage` = `spike-lsm` + instrumentación; el comportamiento por defecto es el del M0) |
| Objetivos | [f1-objetivos-rendimiento.md](f1-objetivos-rendimiento.md), registrados antes de medir (commit `fed9656`) |
| Datos | [`spikes/storage/results/2026-10-28-f1-diag/`](../../spikes/storage/results/2026-10-28-f1-diag): [`report.md`](../../spikes/storage/results/2026-10-28-f1-diag/report.md) (tablas), `results.jsonl`, `ab.jsonl`, `trace.jsonl` y sus repeticiones `*-5reps.jsonl`, `overhead.jsonl` y `overhead-final*.jsonl`, `env.json`, `runs.log` (con `uptime` antes y después de cada ejecución) |
| Relacionado | [ADR-0001](../adr/0001-motor-de-almacenamiento.md) (condición 1), [informe M0](m0-report.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) |

## 1. Resumen

- **La cola de `agent_memory` es casi entera `fdatasync` del WAL.** En el 1 % de commits de escritura de recuerdo más lentos, el **99,4 %** del tiempo (mediana de 3 ejecuciones; 98,6 % en otras 5 con la máquina más cargada) está en la sincronización del WAL; el **100 %** del tiempo queda atribuido a fases con nombre (requisito: ≥ 80 %). Ningún commit de la cola se bloqueó, ninguno cambió de memtable y, con la máquina poco cargada, ninguno se ralentizó.
- **Ese `fdatasync` es lento cuando algo más escribe en el disco.** Con un flush o una compactación del propio motor en curso, el p99 de `wal_sync` es de 12 ms (16 ms con la máquina más cargada); sin ellos, 4 ms; el `fdatasync` aislado de 4 KiB de `env.json`, 0,42 ms. Con la máquina poco cargada, 41 de los 50 commits de la cola sincronizaron con escritura de fondo del motor, frente al 15 % de todos los commits.
- **Veredictos:** H3 (interferencia de escritura) **confirmada**, es el mecanismo; H2 (deuda tras la carga) **confirmada** como origen de esa escritura de fondo (la fase medida empieza con ~78 MB de compactación pendiente y la mitad de la cola cae en su primer quinto), pero sin el efecto «L0 sin prioridad»; H1 (flushes multi-prefijo) **confirmada en lo estructural** (cada compactación de L0 reescribe el 63 % de los archivos de L1; la compactación escribe 2,8× los bytes del flush, frente a 1,2–1,4× en las otras cargas) y **rechazada como causa directa**: las paradas y ralentizaciones que predecía no aparecen en la fase medida; H4 (rotación en primer plano) **rechazada**.
- **A/B:** `bytes_per_sync = 1 MiB` reduce a la mitad el p99 de `wal_sync` con escritura de fondo en todas las sesiones, pero el p99 de escritura de recuerdo solo mejora en la sesión menos cargada (−39 %) y la compactación se vuelve más lenta (el doble de deuda al empezar la fase medida, L0 hasta 12 archivos, alguna parada). Quitar el frenado no cambia nada; subir los límites de L0 empeora.
- **T1, provisional:** `kerno` 5,0 ms frente a fjall 0,45 ms (11×) y spike-btree 0,81 ms (6,2×), con objetivos de ≤ 2× y ≤ 1,5×. Lo que hay que corregir en WP05 es la competencia del WAL con la escritura de fondo, no la contrapresión.

## 2. Método

### 2.1 Instrumentación (WP04)

`KernoDb::metrics()` devuelve métricas tipadas y `stats()` las aplana con nombres estables ([DESIGN.md](../../crates/kerno-storage/DESIGN.md), «Estadísticas»). Las que usa este diagnóstico:

| Pregunta | Métrica |
|---|---|
| ¿Dónde pasa el tiempo un commit? | histogramas log2 (cada potencia de dos partida en cuatro) de `make_room`, `wal_write`, `wal_sync`, `apply` y el total: `commit_<fase>_{count,micros,p50_micros,p99_micros,max_micros}` |
| ¿Compite el `fdatasync` del WAL con la escritura de fondo? (H3) | `wal_sync` separado en `wal_sync_busy` (había un flush o una compactación escribiendo al empezar o al terminar la sincronización) y `wal_sync_quiet` |
| ¿Se bloquean o frenan los escritores? (H1, H2) | episodios de bloqueo contados una vez al entrar en la espera, por motivo (`stall_episodes_{memtable,l0}`, `stall_micros_*`, `stall_micros_max`); `slowdown_commits`, `slowdown_micros` |
| ¿Cuánto reescribe cada compactación de L0? (H1) | `l0_compactions`, `l0_compaction_input_bytes{,_max}`, archivos de L1 reescritos (`l0_compaction_l1_files{,_max}`) y presentes (`l0_compaction_l1_files_present`) |
| ¿Hay deuda de compactación? ¿L0 pierde frente a niveles profundos? (H2) | `pending_compaction_bytes`, `l0_files_max`, `compactions_while_l0_slowdown` |
| ¿Cuesta la rotación en primer plano? (H4) | `memtable_rotation_micros{,_max}` |

`spike-bench` añade al final de cada carga todas las estadísticas del motor como `stat_<nombre>`. Para atribuir la cola, `--trace true` lee las estadísticas antes y después de cada commit medido de `agent_memory` (fuera de la región cronometrada): como los commits se serializan, la diferencia de los contadores acumulados es exactamente el desglose de ese commit. De los 5 000 commits medidos se toma el 1 % más lento (50) y se reparte su latencia, medida por el banco, entre bloqueo por memtables, bloqueo por L0, frenado, rotación, resto de `make_room`, `wal_write`, `wal_sync` y `apply` (fases con nombre), tiempo del motor fuera de ellas y tiempo fuera del motor (`attr_tail_*`; `attr_all_*` para los 5 000). La traza carga algo la máquina (dos `stats()` por paso) y nunca se usa para comparar motores.

**Coste de la instrumentación** (criterio de aceptación: rendimiento de `kerno` ≥ 0,97× spike-lsm en la misma sesión): **no verificable en esta máquina compartida; el veredicto queda para WP14.** La primera ejecución (`small`, 3 repeticiones, `overhead.jsonl`, carga 4,7–5,2) da medianas de commits/s 1,03×, lecturas con 1 hilo 1,00× y con 4 hilos 1,07×, pero no sirve como prueba: en `small_commits` la repetición 0 de los dos motores es atípica (1 144 commits/s `lsm` y 1 254 `kerno`, frente a unos 6 500–6 900 en las repeticiones 1 y 2), así que la dispersión de commits/s es del 89 % (`lsm`) y del 82 % (`kerno`), muy por encima del 15 % de la regla de riesgo, que ya obligaba a repetir con 5. Las repeticiones sobre el commit final del paquete (`e1575d3`, WP04 integrado con WP03a, con el mismo código del motor; `overhead-final.jsonl`, `overhead-final-5reps.jsonl`) se hicieron con carga 9,8–12,2: con 3 repeticiones, 1,30×, 0,92× y 0,86×; con 5, 1,18×, 1,17× y 1,09×, con dispersiones de hasta el 496 %. Una ejecución independiente de la revisión (5 repeticiones, carga 2,6–4,2, no registrada en el repositorio) dio 0,67×, 0,98× y 1,35×, con commits/s de `kerno` entre 916 y 4 918. Con esta dispersión la proporción no demuestra ni refuta el criterio. Después de esas mediciones, los histogramas de fases del commit pasaron a escribirse sin operaciones atómicas de lectura-modificación-escritura (hay un solo escritor, bajo el cerrojo de escritura): cargas y almacenamientos relajados, más seis `Instant::now()` por commit. Los contadores de lectura repartidos por hilo evitan que los 4 lectores escriban en la misma línea de caché (en el M0 todos sumaban en el mismo contador atómico).

### 2.2 Ejecuciones

Escala `medium` (200 000 registros), `Sync`, caché de 64 MiB, un proceso y un directorio nuevos por ejecución, motores intercalados (ADR-0012), ext4 sobre `/dev/vda` (VM de 4 núcleos a 2,10 GHz; el M0 se midió en otra VM a 2,80 GHz).

| Archivo | Orden | Carga de 1 min |
|---|---|---|
| `results.jsonl` | `suite --engines lsm,kerno,btree,fjall --workloads agent_memory,livespec,cauce,append_seq --scales medium --reps 3 --durability sync` | 1,4–3,1 |
| `results-5reps.jsonl` | lo mismo con `--reps 5` (regla de riesgo: dispersión > 15 %) | 2,0–8,2 |
| `ab.jsonl` | por variante, `KERNO_TUNING=<variante> suite --engines kerno,kerno-tuned --workloads agent_memory … --reps 3` | 1,9–5,6 |
| `ab-5reps.jsonl` | las dos variantes con `bytes_per_sync`, `--reps 5` | 3,2–14 |
| `trace.jsonl`, `trace-5reps.jsonl` | `KERNO_TUNING=bytes_per_sync=1048576 suite --engines kerno,kerno-tuned --workloads agent_memory … --trace true`, 3 y 5 repeticiones | 4,9–5,6; 10–13 |
| `overhead.jsonl` | `suite --engines lsm,kerno --workloads small_commits,point_reads --scales small --reps 3` | 4,7–5,2 |
| `overhead-final.jsonl`, `overhead-final-5reps.jsonl` | lo mismo sobre el commit final del paquete, con `--reps 3` y `--reps 5` | 9,8–12,2 |

**Ruido.** Las latencias absolutas dependen mucho de la carga: con la máquina a 8–14, el p99 de escritura de recuerdo de `kerno` pasa de 5 a 11 ms y las de spike-btree y fjall se disparan en algunas repeticiones (rangos de 0,9–9,9 ms y 0,6–7,8 ms en `results-5reps.jsonl`). Por eso el diagnóstico se apoya en métricas estructurales (episodios y motivos de bloqueo, archivos de L0, tamaño de las compactaciones, archivos de L1 por compactación, reparto por fases), que no dependen de la CPU disponible, y toma las proporciones entre motores de la sesión menos cargada (`results.jsonl`) como provisionales.

## 3. Resultados

### 3.1 Comparación en la misma sesión (`results.jsonl`, carga 1,4–3,1)

p99 en µs, mediana de 3 (rango si supera el 15 %). `kerno` y spike-lsm ejecutan el mismo algoritmo: sus estadísticas estructurales coinciden (37 flushes, 24–25 compactaciones en `agent_memory`; idénticas en las otras tres cargas).

| Carga | Métrica | spike-btree | spike-lsm | `kerno` | fjall | `kerno` / fjall | `kerno` / spike-btree |
|---|---|---:|---:|---:|---:|---:|---:|
| `agent_memory` | escritura de recuerdo p99 | 807 (782–3 314) | 5 276 (5 194–7 770) | **4 997** (4 502–7 471) | 454 | 11,0× | 6,2× |
| | escritura de recuerdo p50 | 370 | 255 | 245 | 205 | | |
| LiveSpec | commit p99 | 7 795 (2 161–12 394) | 4 168 | 4 452 | 780 | 5,7× | |
| Cauce | commit p99 | 1 148 | 409 | 412 | 360 | 1,1× | |
| `append_seq` | commit de 100, inserción · actualización | 1 114 · 1 556 | 3 328 · 5 231 | 3 170 · 5 018 | 699 · 728 | 4,5× · 6,9× | |

### 3.2 Atribución de la cola de `agent_memory` (`--trace true`, `kerno`)

Reparto del tiempo total de los 50 commits de escritura de recuerdo más lentos (de 5 000), mediana por ejecución:

| | 3 repeticiones (carga ~5) | 5 repeticiones (carga 10–13) |
|---|---:|---:|
| Latencia media de la cola · mínima (≈ p99) | 9,2 ms · 5,0 ms | 15,3 ms · 10,4 ms |
| `wal_sync` | **99,4 %** (99,2–99,5) | **98,6 %** (96,5–99,8) |
| `wal_write` · `apply` | 0,2 % · 0,3 % | 0,2 % · 0,5 % |
| bloqueos (memtables, L0) | 0 % | 0 % |
| frenado (`slowdown`) | 0 % | 0 % (hasta 2,3 % en 2 de 5) |
| rotación de memtable | 0 % | 0 % (0,1 % en 1 de 5) |
| resto de `make_room`, resto del motor, fuera del motor | 0 % | 0 % |
| **Atribuido a fases con nombre** | **100 %** (99,8–100) | **100 %** |
| Commits de la cola con escritura de fondo del motor durante el `fdatasync` | 41 de 50 (11–46) | 24 de 50 (6–46) |
| … de todos los commits medidos | 741 de 5 000 (15 %) | 376 de 5 000 (8 %) |
| Commits de la cola en el primer quinto de la fase medida | 25 de 50 (14–30) | 26 de 50 (18–33) |
| Compactación pendiente al empezar la fase medida | 78 MB (72–80); L1 con 61 MB y objetivo de 32 MiB | 63 MB (46–69) |
| Ralentizaciones durante la fase medida | 0 en las 3 | 0 en 2 de 5; 11–37 en 3 de 5 |

Con la máquina muy cargada, la parte de la cola sin escritura de fondo del propio motor crece: el `fdatasync` compite también con la escritura de otros procesos (compilaciones de otros paquetes en el mismo disco), que es el mismo mecanismo con otro origen.

### 3.3 Métricas estructurales (`kerno`, `results.jsonl`; las de `results-5reps.jsonl` coinciden)

| | `agent_memory` | LiveSpec | Cauce | `append_seq` |
|---|---:|---:|---:|---:|
| Episodios de bloqueo (memtables · L0) | 0 · 0 | 0 · 0 | 0 · 0 | 0 · 0 |
| Commits ralentizados (toda la ejecución) | 33 (1–72), todos en la carga inicial en las ejecuciones con traza poco cargadas | 0 | 0 | 0 |
| Máximo de archivos en L0 | 9 (8–10) | 5 | 5 | 5 |
| Compactaciones de L0 · archivos de L1 reescritos / presentes | 8 · 19 / 30 (63 %) | 4 · 4 / 12 (33 %) | 2 · 4 / 4 | 5 · 12 / 16 (75 %) |
| Entrada máxima de una compactación de L0 | 74 MB (64–105) | 62 MB | 59 MB | 60 MB |
| Bytes escritos por compactación / por flush | 881 / 316 MB (**2,8×**) | 178 / 133 MB (1,3×) | 88 / 73 MB (1,2×) | 230 / 164 MB (1,4×) |
| Compactaciones de niveles profundos con L0 en frenado | 0 (0–3) | 0 | 0 | 0 |
| Flush más largo · compactación más larga | 53 ms · 236 ms | 30 ms · 162 ms | 31 ms · 170 ms | 40 ms · 172 ms |
| `wal_sync` p99 con escritura de fondo · sin ella | **12,0 ms · 4,0 ms** | 11,9 · 2,4 ms | 8,4 · 0,68 ms | 7,4 · 0,83 ms |
| Commits con escritura de fondo durante el `fdatasync` | 29 % | 14 % | 4 % | 32 % |
| Rotaciones · tiempo total · máxima | 37 · 59 ms · 11 ms | 17 · 13 ms · 7 ms | 10 · 4 ms · 0,6 ms | 23 · 6 ms · 0,4 ms |

Las cifras de `wal_sync` y de rotación cubren todos los commits de la ejecución, incluida la carga inicial.

### 3.4 Experimentos A/B con `kerno-tuned`

Cada variante se ejecutó intercalada con `kerno` en la misma sesión; «variante / `kerno`» compara las medianas de esa sesión.

| `KERNO_TUNING` | Carga | p99 escritura de recuerdo | p99.9 | pasos/s | `wal_sync` p99 con escritura de fondo | Ralentizaciones · máx. L0 · bloqueos |
|---|---|---:|---:|---:|---:|---|
| `slowdown=off` | 1,9–2,1 | 1,15× | 0,93× | 0,96× | 1,09× | 0 · 8 · 0 (`kerno`: 36 · 9 · 0) |
| `bytes_per_sync=1048576` | 2,0–2,4 | **0,61×** (4,9 → 3,0 ms) | 0,72× | 0,85× | **0,46×** (13,2 → 6,1 ms) | 153 · 12 · 0–2 (`kerno`: 55 · 10 · 0) |
| ídem, 5 repeticiones | 3,2–13 | 1,04× | 1,00× | 0,75× | 0,58× | 119 · 12 · 1 |
| ídem, sesión con traza | 4,9–5,6 | 1,60× | — | — | — | compactación pendiente al empezar la fase medida: 150 MB frente a 78 MB |
| ídem, traza, 5 repeticiones | 10–12 | 0,79× | — | — | — | 150 MB frente a 63 MB |
| `l0_slowdown=20,l0_stop=36` | 2,4–4,7 | 1,29× | 1,73× | 0,55× | 1,01× | 0 · 8 · 0 |
| `slowdown=off,bytes_per_sync=1048576,l0_slowdown=12` | 4,7–5,6 | 0,93× | 1,05× | 0,64× | 0,54× | 0 · 12 · 1–2 (140–270 ms bloqueado por ejecución) |
| ídem, 5 repeticiones | 8–14 | 0,85× | 0,59× | 1,52× | 0,63× | 0 · 12 · 1–2 (130–910 ms bloqueado por ejecución) |

Lectura:

- **`bytes_per_sync`** reduce a la mitad, en todas las sesiones, el p99 del `fdatasync` del WAL mientras hay escritura de fondo: sincronizar la SST cada MiB acota los datos sucios que el `fdatasync` del WAL encuentra delante (en ext4 con `data=ordered`, la confirmación del diario que exige el `fdatasync` de un archivo que crece debe escribir antes los bloques de datos ya asignados de los demás archivos). Pero sincronizar más a menudo hace la compactación más lenta: con la carga inicial de `agent_memory` la deuda al empezar la fase medida se duplica, L0 llega a 12 archivos, aparecen ralentizaciones (las de la carga inicial) y algún bloqueo. El efecto en el p99 de escritura es positivo en la sesión menos cargada y se pierde en el ruido en las demás.
- **Quitar el frenado** no cambia la cola (no había frenado en la fase medida) y, con `bytes_per_sync`, deja que L0 llegue a la parada: 1–2 bloqueos por ejecución, que suman 130–910 ms (el más largo, 0,9 s).
- **Subir los límites de L0** empeora: más archivos de L0 que leer en cada consulta (pasos/s 0,55×) y compactaciones de L0 más grandes, sin evitar ningún bloqueo porque no los había.

## 4. Veredicto por hipótesis

### H1 — flushes multi-prefijo: **confirmada en lo estructural; rechazada como causa directa de la cola**

Evidencia a favor del mecanismo: en `agent_memory` cada compactación de L0 reescribe de media el 63 % de los archivos de L1 (19 de 30 en 8 compactaciones; 72 % en `results-5reps.jsonl`) y hasta 74–105 MB de entrada por compactación, aunque los datos nuevos (ids crecientes) solo pueden coincidir con la última SST de `emb/`: el rango de cada archivo de L0, de `emb/<id viejo>` a `mem/<id nuevo>`, contiene todas las SST de `mem/` antiguos. Resultado: la compactación escribe 2,8× los bytes del flush, el doble que las otras tres cargas (1,2–1,4×).

Evidencia en contra de la consecuencia que predecía («L0 crece, cada commit duerme 1 ms con L0 ≥ 8 y se bloquea con 12»): 0 episodios de bloqueo en las cuatro cargas y en todas las repeticiones; L0 llega a 8–10 archivos solo durante la carga inicial; ninguna ralentización en la fase medida con la máquina poco cargada, y como mucho el 2,3 % del tiempo de la cola con la máquina muy cargada. H1 actúa **a través de H3**: multiplica la escritura de fondo con la que compite el WAL.

### H2 — deuda tras la carga: **confirmada (la deuda); rechazada (L0 sin prioridad)**

La fase medida empieza con 63–80 MB de compactación pendiente (L1 al doble de su objetivo de 32 MiB) y 6–7 archivos en L0; durante ella se ejecutan ~9 compactaciones más (de 24). La cola se concentra al principio: la mitad de los 50 commits más lentos cae en el primer quinto de la fase medida (25 de 50; 10 si se repartieran por igual). Cuando `bytes_per_sync` ralentiza la compactación, la deuda al empezar se duplica (150 MB). En cambio, «L0 sin prioridad» no se observa: ninguna compactación de un nivel profundo se eligió con L0 en frenado (0; como mucho 3 en una ejecución), y la deuda está en L1, no en L0.

### H3 — interferencia de escritura: **confirmada; es el mecanismo de la cola**

- El 98,6–99,4 % del tiempo de la cola es `fdatasync` del WAL (§3.2).
- Con un flush o una compactación del motor en curso, el p99 de `wal_sync` es 3–12× el de sin ellos en las cuatro cargas (§3.3), y ambos están muy por encima del `fdatasync` aislado de 4 KiB (p99 0,42 ms en `env.json`).
- Con la máquina poco cargada, 41 de los 50 commits de la cola sincronizaron con escritura de fondo del motor, frente al 15 % de todos los commits medidos.
- Acotar los datos sucios de las SST (`bytes_per_sync`) divide por dos el p99 de esas sincronizaciones en todas las sesiones (§3.4).
- Con la máquina muy cargada, la escritura de otros procesos al mismo disco produce el mismo efecto (más commits lentos sin escritura de fondo del motor): el `fdatasync` del WAL espera detrás de cualquier escritura pendiente.

### H4 — rotación en primer plano: **rechazada**

Ningún commit de la cola de las ejecuciones con traza poco cargadas cambió de memtable (como mucho 1 de 50 con la máquina muy cargada, el 0,1 % del tiempo). Las rotaciones son el 0,7 % de los commits (37 de 5 550) y cuestan 59 ms en total por ejecución; la más lenta, 8–24 ms (`sync_dir` con el disco ocupado), puede aparecer en el p99.9 o el máximo, no en el p99. Según §10.2 del diseño, el WAL precreado es un recorte que solo se justifica si H4 se confirma: no se justifica.

## 5. Consecuencias para WP05

- Las palancas de **contrapresión** (frenado por tasa, prioridad de L0) no atacan la cola observada: en la fase medida no hay bloqueos ni frenado. Siguen siendo útiles como red de seguridad cuando la compactación se retrasa (como con `bytes_per_sync`, que lleva L0 a la parada).
- Lo que reduce la cola es **menos escritura de fondo y más repartida**: *flush split* (H1: evita reescribir los `mem/` antiguos en cada compactación de L0), compactación parcial de L0 y corte por solape con el abuelo (compactaciones más pequeñas), y `bytes_per_sync` (H3), cuyo coste en velocidad de compactación hay que compensar con lo anterior. Ninguna palanca de la tabla de §5.4 cambia la parte de la cola que se debe a escritura de otros procesos.
- Una palanca que la tabla de §5.4 no incluye y que ataca directamente el mecanismo: que el `fdatasync` del WAL no tenga que confirmar el diario del sistema de archivos (WAL con tamaño ya asignado y escrito, como el reciclado de logs de RocksDB). El diseño lo deja fuera de la Fase 1 (§9, «reciclado y preasignación del WAL»); si WP05 no alcanza T1, es candidata para el memorando de decisión, previa medición.
- Medir siempre con `--trace true` además de las ejecuciones limpias: el reparto por fases y `wal_sync_busy`/`wal_sync_quiet` distinguen una mejora real de un cambio en el ruido de la máquina.
- Las cifras de T1 de este documento son provisionales; el punto de decisión de WP05 debe repetir la medición en la misma sesión con spike-btree y fjall, y la de referencia es la de WP14.

## 6. Limitaciones

- **Máquina compartida.** Todas las latencias absolutas y proporciones son provisionales (§2.2). Las conclusiones se apoyan en métricas estructurales y en el reparto por fases, que se mantienen con cargas de 1,4 a 14.
- **Una sola máquina y un solo sistema de archivos** (ext4 sobre un disco virtual). El peso de H3 depende de cómo el sistema de archivos ordena la confirmación del diario y de la latencia del dispositivo; en un NVMe local o en otro sistema de archivos puede ser distinto (R-S3).
- **`wal_sync_busy`** marca una sincronización como concurrente con escritura de fondo si había un flush o una compactación en curso al empezar o al terminar; una escritura de fondo que empieza y termina dentro de la sincronización no se ve. La escritura que el núcleo hace por su cuenta (`writeback`) tampoco.
- **Histogramas aproximados.** Los percentiles del motor tienen un error < 25 %; la atribución usa sumas exactas (al microsegundo) por commit.
- **La traza** añade dos llamadas a `stats()` por paso de `agent_memory` (fuera de la medición) y por eso no se usa para comparar motores.
