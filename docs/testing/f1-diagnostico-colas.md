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
- Una palanca que la tabla de §5.4 no incluye y que ataca directamente el mecanismo: que el `fdatasync` del WAL no tenga que confirmar el diario del sistema de archivos (WAL con tamaño ya asignado y escrito, como el reciclado de logs de RocksDB). El diseño lo deja fuera de la Fase 1 (§9, «reciclado y preasignación del WAL»); si WP05 no alcanza T1, es candidata para el memorando de decisión, previa medición. **Seguimiento: implementada y medida en [§8](#8-preasignación-y-reciclado-del-wal).**
- Medir siempre con `--trace true` además de las ejecuciones limpias: el reparto por fases y `wal_sync_busy`/`wal_sync_quiet` distinguen una mejora real de un cambio en el ruido de la máquina.
- Las cifras de T1 de este documento son provisionales; el punto de decisión de WP05 debe repetir la medición en la misma sesión con spike-btree y fjall, y la de referencia es la de WP14.

## 6. Limitaciones

- **Máquina compartida.** Todas las latencias absolutas y proporciones son provisionales (§2.2). Las conclusiones se apoyan en métricas estructurales y en el reparto por fases, que se mantienen con cargas de 1,4 a 14.
- **Una sola máquina y un solo sistema de archivos** (ext4 sobre un disco virtual). El peso de H3 depende de cómo el sistema de archivos ordena la confirmación del diario y de la latencia del dispositivo; en un NVMe local o en otro sistema de archivos puede ser distinto (R-S3).
- **`wal_sync_busy`** marca una sincronización como concurrente con escritura de fondo si había un flush o una compactación en curso al empezar o al terminar; una escritura de fondo que empieza y termina dentro de la sincronización no se ve. La escritura que el núcleo hace por su cuenta (`writeback`) tampoco.
- **Histogramas aproximados.** Los percentiles del motor tienen un error < 25 %; la atribución usa sumas exactas (al microsegundo) por commit.
- **La traza** añade dos llamadas a `stats()` por paso de `agent_memory` (fuera de la medición) y por eso no se usa para comparar motores.

## 7. Resultados de WP05

| Campo | Valor |
|---|---|
| Estado | **Provisional.** La máquina se compartía con otros paquetes (carga de 1 minuto entre 0,7 y 7,4 durante las ejecuciones; cada fila de `results.jsonl` lleva `load1`/`load5`/`load15` y `env.json` la carga inicial). La medición de referencia es la de WP14, con la máquina en reposo |
| Fecha | 2026-10-28 |
| Código medido | `kerno-storage` con las palancas de la condición 1 (§5.4 del diseño) sobre `8f720db`; `results.jsonl`, `report.md`, `large.jsonl`, `env.json` y los archivos `round2-*` son de la versión final, con las palancas del cierre (§7.4) |
| Datos | [`spikes/storage/results/2026-10-28-f1-wp05/`](../../spikes/storage/results/2026-10-28-f1-wp05): `results.jsonl` y [`report.md`](../../spikes/storage/results/2026-10-28-f1-wp05/report.md) (las seis cargas, escala `medium`, 3 repeticiones, `--durability sync`, `lsm`, `kerno`, `btree` y `fjall` intercalados en la misma sesión), `large.jsonl` (`bulk_load`, escala `large`, 1 repetición), `rerun-agent_memory.jsonl` (repetición de `agent_memory` con 5 repeticiones, una vez más con la máquina menos cargada), `ab.jsonl` (palanca a palanca), `cond.jsonl` y `cond-final.jsonl` (paso condicional), `cauce-ab.jsonl`, `round2-close.jsonl` (cierre de `bulk_load`, `medium` con 10 repeticiones y `large` con 3), `round2-cauce-attribution.jsonl` y `round2-compaction-syncs.jsonl` (atribución de T2 de Cauce), `round2-agent_memory.jsonl` (`bytes_per_sync` frente a T1), `env.json` |

### 7.1 Valores por defecto finales

El diagnóstico (§4) no rechazó ninguna palanca por completo, así que todas están encendidas por defecto salvo dos:

- `l0_priority = false`: H2 no mostró el efecto «L0 sin prioridad» (§4). Encenderla no cambió el p99 (2,73 ms con la palanca frente a 2,55 ms sin ella, mediana de 5 y 35 ejecuciones respectivamente, dentro del ruido).
- `max_subcompactions = 1` y `max_background_compactions = 1`: el paso condicional (§7.3) está implementado y probado, pero no demostró una mejora fuera del ruido.

El resto: `flush_split`, `l0_metric = Depth`, `l0_partial`, `l0_trivial_move`, `slowdown = Rate { 16 MiB/s }`, `bytes_per_sync = 1 MiB`, `heap_merge_threshold = 8`, `close_subcompactions = 8` (solo para las compactaciones del cierre, que además no sincronizan por tramos; §7.4), `max_compaction_bytes` 25 × y `grandparent_overlap_bytes` 10 × el tamaño objetivo; cierre `Bounded { 256 MiB, 200 ms }`. El perfil `M0` las apaga todas.

### 7.2 Efecto de cada palanca en T1 (`ab.jsonl`, `agent_memory`, `medium`)

`memory_write_p99_us`, mediana de las ejecuciones (cada variante con 5; los valores por defecto con 35 repartidas en la sesión; carga de 1 minuto 0,7–2,0):

| Variante | p99 (µs) | Frente al valor por defecto |
|---|---:|---:|
| valores por defecto | 2 554 | — |
| perfil `m0` (todas las palancas apagadas) | 5 087 | 2,0× peor |
| `bytes_per_sync = none` | 3 822 | 1,5× peor |
| `slowdown = sleep:1000` | 2 806 | +10 % (ruido) |
| `flush_split = false` | 2 718 | +6 % (ruido) |
| `l0_priority = true` | 2 728 | +7 % (ruido) |
| `l0_partial = false` | 2 570 | 0 % |
| `l0_metric = files` | 2 257 | −12 % (ruido) |

Las palancas juntas reducen a la mitad el p99 del M0. De ellas, solo `bytes_per_sync` se distingue del ruido por sí sola (coherente con H3, §4); el resto actúa en conjunto o por debajo de la resolución de estas medidas, y la dispersión entre ejecuciones del valor por defecto (2,0–3,8 ms) es mayor que la diferencia entre ellas.

### 7.3 T1 y la regla de decisión

Objetivo T1: `memory_write_p99_us` ≤ 2,0× fjall **y** ≤ 1,5× spike-btree, en la misma sesión.

| Medición | `kerno` | fjall | btree | `kerno` / fjall | `kerno` / btree | Carga (1 min) |
|---|---:|---:|---:|---:|---:|---|
| `results.jsonl`, 3 repeticiones (versión final) | 2 480 | 795 | 1 342 | 3,12× | 1,85× | 1,1–1,2 |
| `rerun-agent_memory.jsonl`, 5 repeticiones (repetición con menos carga) | 2 748 | 1 059 | 1 376 | 2,60× | 2,00× | 1,3–1,7 |
| `cond-final.jsonl`, 15 repeticiones | 2 585 | 974 | 1 340 | 2,65× | 1,93× | 0,9–1,7 |
| `cond-final.jsonl`, `max_subcompactions=4,max_background_compactions=2` | 2 372 | 974 | 1 340 | 2,43× | 1,77× | 1,2–1,7 |

(El p99 de M0 en la misma sesión: `lsm` 5 472 µs, es decir 6,0× fjall y 4,3× btree.)

1. Con los valores por defecto, T1 **no se cumple** y el desvío es > 25 % (2,65× frente a 2,0× es +33 %; 1,93× frente a 1,5× es +29 %). Se repitió esta carga una vez con la máquina menos cargada (`rerun-agent_memory.jsonl`): mismo resultado (+30 % y +33 %), así que no se debe al ruido de esa sesión.
2. Según la regla, se implementó dentro de WP05 el paso condicional: subcompactaciones (la compactación de L0 se divide en límites de archivos de L1, `std::thread::scope`, salidas instaladas en **una** edición del MANIFEST) y `max_background_compactions = 2` (conjunto `being_compacted` bajo el cerrojo del MANIFEST, `pick` excluye esas entradas, exclusión de rangos de salida en el mismo nivel, `compaction::apply` lo vuelve a comprobar), con las pruebas `subcompactions_install_single_manifest_write` y `concurrent_compactions_preserve_model`.
3. Se volvió a medir (`cond.jsonl` con cada mitad por separado y las dos juntas, `cond-final.jsonl` con las dos juntas y 15 repeticiones intercaladas con los demás motores): el p99 baja a 2 372 µs (−8 %, 2,43× fjall y 1,77× btree). Esa mejora cae dentro de la dispersión (en `cond.jsonl` las medianas de las tres variantes son 2 211–3 035 µs frente a 2 560 sin ellas, sin orden estable), de modo que **T1 sigue sin cumplirse** y el paso condicional no demuestra efecto.
4. **Decisión: GO no concedido; candidato a NO-GO, provisional hasta WP14.** Se escribe [f1-decision-cond1.md](f1-decision-cond1.md) y se señala al propietario. La ejecución secuencial continúa y el formato v1 no depende de esta decisión.

### 7.4 T2–T5 (`report.md`, `large.jsonl`; mediana de 3 repeticiones)

| Objetivo | Medida | `kerno` | Referencia | Proporción | Objetivo | |
|---|---|---:|---|---:|---|---|
| T2 | LiveSpec `changeset_commit_p99_us` | 3 021 | `lsm` 4 362 | 0,69× | ≤ 1,2× | cumple |
| T2 | `append_seq` inserción `append_commit100_p99_us` | 1 888 | `lsm` 4 313 | 0,44× | ≤ 1,2× | cumple |
| T2 | `append_seq` actualización `update_commit100_p99_us` | 3 774 | `lsm` 6 070 / fjall 1 425 | 0,62× / 2,65× | ≤ 1,2× `lsm`, ≤ 2× fjall | `lsm` sí, fjall **no** |
| T2 | `history_sim` `rev_commit100_p99_us` | 5 239 | `lsm` 8 196 | 0,64× | ≤ 1,2× | cumple |
| T2 | Cauce `step_commit_p99_us` | 1 117 | `lsm` 484 | 2,31× | ≤ 1,2× | **no cumple** (atribuido a `bytes_per_sync`; conflicto con T1) |
| T3 | LiveSpec `symbol_get_p99_us` | 31,6 | `lsm` 29,6 | 1,07× | ≤ 1,2× | cumple |
| T3 | LiveSpec `module_scan_p99_us` | 617 | `lsm` 666 / btree 306 | 0,93× / 2,02× | ≤ 1,2× `lsm`, ≤ 1,5× btree | `lsm` sí, btree **no** (el M0 tampoco: `lsm` 2,18× btree) |
| T3 | Cauce `step_read_p99_us` | 77,6 | `lsm` 73,5 | 1,06× | ≤ 1,2× | cumple |
| T3 | `recall_p99_us` / `recent_scan_p99_us` | 55,4 / 65,1 | `lsm` 56,8 / 68,5 | 0,98× / 0,95× | ≤ 1,2× | cumple |
| T4 | `bulk_load` `close_ms`, `medium` | 204 (192–222) | `lsm` 726, fjall 70 | — | ≤ 250 ms | cumple |
| T4 | `bulk_load` `close_ms`, `large` (1 repetición) | 376 | `lsm` 3 401, fjall 970 | — | ≤ 1 000 ms | cumple |
| T5 | rendimiento: LiveSpec / Cauce / `append_seq` / `agent_memory` / `history_sim` / `bulk_load` | 791 / 4 202 / 183 / 1 368 / 78,7 / 209 | `lsm` 774 / 4 485 / 174 / 1 276 / 81,9 / 187 | 1,02× / 0,94× / 1,05× / 1,07× / 0,96× / 1,12× | ≥ 0,9× | cumple |

Carga de 1 minuto de `results.jsonl`: 1,1–1,5; de `large.jsonl`: 1,2–1,6.

Lectura:

- **T4: se cumple con las palancas del cierre.** La primera medición (587 ms `medium`, 1 270 ms `large`) se quedaba en el tramo **obligatorio** del cierre: `bulk_load` carga en orden aleatorio, así que cada archivo de L0 abarca todas las claves y cada uno que baja reescribe todo L1. Desglose de un cierre `medium` antes de los cambios de abajo (ms, carga ~1): ~25 esperar al flush en curso del hilo de fondo, ~10 volcar la memtable, ~135 compactar ~108 MB de entrada (6 archivos de L0 y 8 de L1) y ~50 borrar esas entradas (el sistema de archivos tarda ~0,5 ms por MB en liberar archivos recién escritos, también en paralelo: 56 frente a 58 ms para 112 MB en una prueba aparte). Cuatro cambios, todos solo del cierre, sin relajar el suelo obligatorio (la métrica de L0 sigue quedando por debajo de `l0_trigger` tras cada cierre limpio):
  1. El tramo obligatorio toma, con `l0_partial`, solo los archivos más viejos que hacen falta para dejar la métrica por debajo de `l0_trigger` (3 de 6 en `bulk_load`; ~90 MB de entrada en vez de ~108). Prueba: `close_floor_takes_only_the_oldest_files_it_needs`.
  2. Las compactaciones del cierre se parten en subintervalos (`close_subcompactions = 8`); las de fondo siguen con `max_subcompactions = 1`, porque en el A/B de `agent_memory` ese valor a 4 subió el p99 de 2 609 a 3 076 µs (`round2-agent_memory.jsonl`). Prueba: `close_compaction_uses_close_subcompactions`.
  3. Las SST de esas compactaciones no hacen los `sync_data` intermedios de `bytes_per_sync`, que existen para que la escritura de fondo no compita con el `fdatasync` del WAL; al cerrar no hay WAL escribiendo.
  4. El presupuesto de tiempo ya no deja empezar una compactación que, al ritmo de las anteriores del mismo cierre (borrado incluido), acabaría después de `max_duration`: antes, una compactación empezada a los 198 ms llevó un cierre a 266 ms.

  Resultado: `medium` 204 ms de mediana (192–222, `results.jsonl`) y 210 ms (200–230, 10 repeticiones, `round2-close.jsonl`), frente a 250 ms de objetivo; `large` 376 ms (`large.jsonl`) y 358 ms (353–382, 3 repeticiones). Con la carga de 1,1–1,6 de estas sesiones hay ~40 ms de margen; la medición de referencia sigue siendo la de WP14.
- **T2 de Cauce: atribuido, y en conflicto con T1.** Con la máquina menos cargada (carga 0,9–1,8; `round2-cauce-attribution.jsonl`, 7 repeticiones por variante, medianas de `step_commit_p99_us`): `lsm` 537 µs, `kerno` 1 124 µs (2,09×), `kerno-tuned[m0]` 540 µs, `bytes_per_sync=none` 529 µs, `slowdown=off` 1 125 µs, `flush_split=false` 1 057 µs. La regresión se debe a `bytes_per_sync`; el frenado por tasa y `flush_split` no influyen. Dentro de `bytes_per_sync`, son los `sync_data` intermedios de las **compactaciones**: quitarlos solo de ellas (`round2-compaction-syncs.jsonl`, 6–7 repeticiones intercaladas, carga 1,3–1,5) deja Cauce en 620 µs frente a 525 de `lsm` (1,18×, dentro del objetivo), pero sube el p99 de `agent_memory` de 2 304 a 3 382 µs (rangos 1 990–2 712 y 2 662–4 223, sin solaparse), es decir, empeora T1, que ya es el incumplimiento principal. Valores mayores del parámetro dan el mismo intercambio (`round2-agent_memory.jsonl`). Por eso se deja `bytes_per_sync = 1 MiB` en las compactaciones de fondo y la elección entre T1 y T2 de Cauce queda para el propietario ([f1-decision-cond1.md](f1-decision-cond1.md)); el interruptor de esa prueba fue temporal y no está en el código.
- **T3 de `module_scan`.** Frente a `lsm` se cumple (0,93× en `results.jsonl`; 1,10× en `round2-cauce-attribution.jsonl`): el 1,24× de la primera medición era ruido de una sesión con carga 5+. Frente a btree sigue en 2,02×, pero el M0 está igual (`lsm` 2,18× btree en la misma sesión; `kerno-tuned[m0]` 626 µs frente a 684 de `kerno` en `round2-cauce-attribution.jsonl`): es el coste de un recorrido que fusiona varios niveles de un LSM frente a un árbol B, no de una palanca de WP05.
- **T5.** `kerno` queda en 0,94–1,12× de `lsm`.

### 7.5 Aceptación del paquete

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` y `cargo test --workspace --all-features --release` pasan.
- Cierre acotado: las pruebas de `tests/close_budget.rs` (`l0_bounded_across_100_short_sessions`: profundidad de L0 < `l0_trigger` y ≤ 2 × `l0_trigger` archivos tras cada cierre; `space_bounded_across_100_short_sessions`; `budgeted_close_crash_at_every_point`).
- **T1 no cumplido; T4 cumplido** (`medium` 204 ms, `large` 376 ms) tras las palancas del cierre. Siguen sin cumplirse T2 de Cauce (atribuido a `bytes_per_sync`, en conflicto con T1), `module_scan` frente a btree (igual que el M0) y `append_seq` actualización frente a fjall. La decisión de la condición 1 queda en [f1-decision-cond1.md](f1-decision-cond1.md) para el propietario.

## 8. Preasignación y reciclado del WAL

Seguimiento de WP05: la palanca 1 de [f1-decision-cond1.md §3](f1-decision-cond1.md#3-siguientes-palancas), que ataca el mecanismo de H3 en lugar de reducir la escritura de fondo. **No cambia el estado de la decisión**: T1 sigue sin cumplirse (§8.3).

| Campo | Valor |
|---|---|
| Estado | **Provisional**, como el resto del documento: máquina compartida, carga de 1 minuto 1,1–2,0 en las ejecuciones del motor y 1,6–5,6 en las del microbanco (cada serie lleva su `uptime-*.txt`). La medición de referencia sigue siendo la de WP14 con la máquina en reposo |
| Fecha | 2026-10-28 |
| Código medido | `kerno-storage` con `Config::wal_preallocate` ([DESIGN.md](../../crates/kerno-storage/DESIGN.md), «WAL preasignado y reciclado»), sobre `b745205`. La primera implementación (`*-v1-fullzero.*`) ponía a cero el WAL reciclado entero; la final solo la parte escrita |
| Datos | [`spikes/storage/results/2026-10-28-f1-wal/`](../../spikes/storage/results/2026-10-28-f1-wal): `walsync.jsonl` y `walsync-load5.jsonl` (microbanco), `agent_memory.jsonl` (10 repeticiones), `results.jsonl` y `report.md` (seis cargas, `medium`, 5 repeticiones), `large.jsonl`, `control-*.jsonl` (controles del interruptor), `*-v1-fullzero.*` (primera implementación), `env.json`, `uptime-*.txt` |

### 8.1 Qué hace

El WAL se escribe como `cabecera ‖ ceros` del tamaño del *write buffer* antes de su primera trama, y un WAL ya volcado se sobrescribe con ceros y queda como reserva (`WAL.spare`) para la rotación siguiente. El `fdatasync` de una trama sobrescribe bloques ya asignados y escritos, así que no tiene que confirmar el tamaño del archivo. **Sin cambio de formato**: los ceros son el relleno de v1.md §6.1 y WAL-R4; el reciclado sin poner a cero necesitaría el bit `incompat` reservado y cambiar WAL-R4/REC-10a ([ADR-0013, punto 15](../adr/0013-formato-v1-y-protocolo-del-motor.md)). Interruptor: `Config::wal_preallocate` (por defecto `false`, porque el microbanco en reposo no muestra ganancia; se enciende con `KERNO_TUNING=wal_preallocate=true`; el perfil `M0` lo apaga). Pruebas: `wal.rs` (17 pruebas del WAL, 8 nuevas), `tests/wal_prealloc.rs` (11) y los barridos de caídas, corrupción y fallos compartidos, que ahora recorren WAL preasignados y reciclados (la escena del barrido `every_point` recicla WAL: `the_crash_sweep_scenario_recycles_logs`).

### 8.2 Microbanco de `fdatasync` (`spike-bench walsync`)

Cada variante añade 512 B por iteración y mide el `sync_data` siguiente (5 000 por ejecución, variantes intercaladas): `grow` (el archivo crece), `setlen` (`set_len` previo: tamaño final, bloques sin asignar) y `prewritten` (ceros escritos y sincronizados hasta el tamaño final). «Con escritura de fondo» añade un hilo que escribe ráfagas de 16 MiB con `sync_data` cada MiB y 30 ms de pausa en el mismo directorio. Mediana de las ejecuciones (entre paréntesis, mínimo–máximo), en µs:

| Sesión | Fondo | Variante | p50 | p99 |
|---|---|---|---:|---:|
| carga 1,6–1,8, 9 repeticiones (`walsync.jsonl`) | no | `grow` | 148 | 435 (326–532) |
| | no | `setlen` | 106 | 379 (299–705) |
| | no | `prewritten` | 149 | 360 (290–607) |
| | sí | `grow` | 164 | 1 629 (1 533–1 732) |
| | sí | `setlen` | 109 | 1 487 (1 318–1 669) |
| | sí | `prewritten` | 164 | 1 649 (1 341–1 798) |
| carga 4,9–5,9, 7 repeticiones (`walsync-load5.jsonl`) | no | `grow` | 156 | 3 987 (475–4 131) |
| | no | `setlen` | 110 | 723 (309–5 276) |
| | no | `prewritten` | 159 | 765 (403–5 304) |
| | sí | `grow` | 181 | 3 957 (1 871–7 963) |
| | sí | `setlen` | 127 | 2 128 (1 507–3 879) |
| | sí | `prewritten` | 186 | 2 458 (1 396–3 148) |

Lectura: **con la máquina en reposo el microbanco no muestra ganancia** (p99 de `grow` y `prewritten` indistinguibles, también con la escritura de fondo propia); con la carga de otros procesos (carga ≥ 5) el p99 de `grow` es 3–5× el de `prewritten`, pero con rangos que llegan a solaparse (una ejecución de `prewritten` llegó a 5,3 ms). La ganancia del `fdatasync` aislado depende, por tanto, de la contención con escritura ajena, justo el escenario de H3, y no se reproduce con la escritura de fondo del propio banco. `setlen` tiene un p50 más bajo (~110 frente a ~150 µs) y un p99 que no se distingue de `prewritten`; no se probó en el motor.

### 8.3 `agent_memory` y T1

Sesiones del A/B (`kerno` con la palanca, `kerno-tuned` con `wal_preallocate=false`; `agent_memory` `medium`, `Sync`, repeticiones intercaladas, mediana y rango de `memory_write_p99_us`, µs):

| Sesión | Código | Reps | `kerno` p99 | sin palanca p99 | `kerno` p50 | sin palanca p50 | fjall p99 | btree p99 | Carga |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `agent_memory-v1-fullzero.jsonl` | primera impl. | 7 | 1 754 (1 449–3 017) | 2 370 (2 212–2 621) | 217 | 361 | 814 | 1 247 | 1,7–4,0 |
| `results-v1-fullzero.jsonl` | primera impl. | 5 | 2 783 (2 312–3 133) | 2 548 (1 679–3 027) | 231 | 401 | 1 132 | 1 182 | 1,2–1,7 |
| `agent_memory.jsonl` | final | 10 | 1 958 (1 367–2 738) | 2 563 (1 884–2 771) | 197 | 331 | 1 032 | 1 068 | 1,1–1,5 |
| `results.jsonl` | final | 5 | 2 437 (1 657–2 781) | 2 548 (2 347–2 841) | 215 | 364 | 1 116 | 1 127 | 1,4–1,6 |
| `control-tuned-*.jsonl` | final | 6 | 2 416 (2 048–2 877) | 2 030 (1 688–2 421) | 214 | 326 | — | — | 1,1 |

(En el control, `kerno-tuned[wal_preallocate=true]` da 2 430 µs y 203 µs de p50: el perfil `Tuned` no añade nada por sí mismo.)

Con el código final, las 15 repeticiones de las dos sesiones que incluyen fjall y btree (`agent_memory.jsonl` y `results.jsonl`):

| | p99 (µs), mediana (cuartiles) | p50 (µs) | frente a fjall (objetivo ≤ 2,0×) | frente a btree (objetivo ≤ 1,5×) |
|---|---:|---:|---:|---:|
| `kerno` con la palanca | 2 173 (1 708–2 466) | 206 | **2,08×** | **1,93×** |
| `kerno` sin la palanca | 2 548 (2 347–2 763) | 348 | 2,44× | 2,27× |
| fjall | 1 044 (657–1 147) | 286 | — | — |
| btree | 1 123 (1 022–1 225) | 445 | — | — |

1. **El p50 de escritura baja un 40 % de forma estable** (206 frente a 348 µs; 197–231 frente a 326–401 µs en todas las series y en el control) y el rendimiento de `agent_memory` sube entre un 7 y un 28 % (`steps_per_s`, según la serie): es el efecto esperado de no confirmar el tamaño en cada commit. En `cauce` el p50 no cambia (169 frente a 162 µs en el control).
2. **El p99 mejora de media un 15 % (8 % con las 27 repeticiones de las tres series con la palanca), pero no de forma estable**: en las cinco series los rangos con y sin la palanca se solapan, y en el control es peor (2 416 frente a 2 030 µs). La primera implementación, que ponía a cero el WAL entero, empeoró el p99 en una sesión (2 783 frente a 2 548): la escritura de ceros compite con el `fdatasync` igual que cualquier otra escritura de fondo, y por eso el reciclado solo reescribe la parte escrita.
3. **T1 no se cumple**: 2,08× fjall (objetivo ≤ 2,0×) y 1,93× btree (≤ 1,5×); sin la palanca, 2,44× y 2,27×. La palanca acerca T1 un 15 %, pero la proporción con btree sigue un 29 % por encima del objetivo; lo que queda es la cola de la escritura de fondo que ninguna palanca de WAL toca (§4).

### 8.4 T2–T5 (`report.md`; mediana de 5 repeticiones, `Sync`, carga de 1 minuto 1,1–2,0)

Las proporciones son frente a `lsm` (M0) de la misma sesión; «sin palanca» es `kerno-tuned[wal_preallocate=false]`.

| Objetivo | Medida | `kerno` | sin palanca | `lsm` | fjall | btree | `kerno`/`lsm` | Objetivo | |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| T2 | LiveSpec `changeset_commit_p99_us` | 1 605 | 2 658 | 4 342 | 1 658 | 2 793 | 0,37× | ≤ 1,2× | cumple |
| T2 | `append_seq` `append_commit100_p99_us` | 1 508 | 2 043 | 3 803 | 973 | 1 692 | 0,40× | ≤ 1,2× | cumple |
| T2 | `append_seq` `update_commit100_p99_us` | 2 339 | 3 654 | 6 054 | 1 378 | 1 635 | 0,39× (1,70× fjall) | ≤ 1,2× `lsm`, ≤ 2× fjall | **cumple las dos** (WP05: 2,65× fjall) |
| T2 | `history_sim` `rev_commit100_p99_us` | 4 506 | 5 698 | 8 331 | 2 261 | 13 959 | 0,54× | ≤ 1,2× | cumple |
| T2 | Cauce `step_commit_p99_us` | 998 | 1 094 | 504 | 514 | 1 320 | 1,98× | ≤ 1,2× | **no cumple** (WP05: 2,31×; el p50 no cambia: 173 frente a 171 µs) |
| T3 | LiveSpec `symbol_get_p99_us` | 31,3 | 30,4 | 28,1 | 21,7 | 29,6 | 1,11× | ≤ 1,2× | cumple |
| T3 | LiveSpec `module_scan_p99_us` | 711 | 627 | 540 | 398 | 295 | 1,32× (sin palanca 1,16×) | ≤ 1,2× `lsm`, ≤ 1,5× btree | **no cumple con la palanca** (ver abajo) |
| T3 | Cauce `step_read_p99_us` | 76,7 | 83,6 | 74,7 | 75,2 | 54,0 | 1,03× | ≤ 1,2× | cumple |
| T3 | `recall_p99_us` / `recent_scan_p99_us` | 53,2 / 59,3 | 54,4 / 65,6 | 55,7 / 61,1 | 41,5 / 64,4 | 34,6 / 51,5 | 0,95× / 0,97× | ≤ 1,2× | cumple |
| T4 | `bulk_load` `close_ms`, `medium` | 219 | 223 | 731 | 77 | 11 | — | ≤ 250 ms | cumple |
| T4 | `bulk_load` `close_ms`, `large` (3 repeticiones, `large.jsonl`) | 409 | 449 | 3 446 | 982 | — | — | ≤ 1 000 ms | cumple |
| T5 | rendimiento: LiveSpec / Cauce / `append_seq` / `agent_memory` / `history_sim` / `bulk_load` | 925 / 4 293 / 205 / 1 553 / 84,2 / 209 | 810 / 4 326 / 176 / 1 268 / 72,9 / 203 | 873 / 4 373 / 179 / 1 318 / 80,3 / 189 | | | 1,06× / 0,98× / 1,15× / 1,18× / 1,05× / 1,11× | ≥ 0,9× | cumple |

- **`module_scan_p99_us` (T3).** Con la palanca es 667–711 µs en tres sesiones frente a 604–627 sin ella y 540–577 de `lsm` (`control-livespec-*.jsonl`: 667 y 691 frente a 604 µs, rangos solapados 591–724 y 509–728; el p50 es igual, 161–167 µs). Es una diferencia de ~10–14 % que cae dentro de la dispersión y que no se ha explicado (las lecturas no usan el WAL); no se descarta que sea real. En WP05 era 0,93× `lsm`.
- **Cauce (T2)** pasa de 2,31× a 1,98× `lsm`: mejora, pero el objetivo (≤ 1,2×) sigue sin cumplirse y su atribución a `bytes_per_sync` ([§7.4](#74-t2t5-reportmd-largejsonl-mediana-de-3-repeticiones)) no cambia.
- **LiveSpec, `append_seq` e `history_sim` mejoran un 30–40 % en p99** (y el p50 de LiveSpec un 16 %: 381 frente a 457 µs) con la palanca.

### 8.5 Costes

| Medida (`bulk_load` `medium`, mediana de 5) | con la palanca | sin ella |
|---|---:|---:|
| `write_amp` | 3,97 | 2,97 |
| `reopen_ms` | 17,0 (16,3–22,4) | 3,71 |
| `recovery_ms` | 28,9 | 6,36 |
| `cauce` `recovery_ms` | 33,2 | 8,89 |
| `history_sim` `space_amp` | 2,06 (2,06–2,85) | 1,68 |
| `append_seq` `space_amp` | 2,24 | 2,04 |

- **Escritura:** el WAL se escribe dos veces (la trama y los ceros del reciclado, solo la parte escrita), de ahí +1,0 en `write_amp` de `bulk_load` (+1,0 en `large`: 6,49 frente a 5,52).
- **Apertura:** `reopen_ms` sube 13 ms y `recovery_ms` 22 ms (crear el WAL de recuperación con el tamaño del *write buffer*, ocho `sync_data` de 1 MiB, y leer sus ceros al reproducir; no se midió cuánto corresponde a cada cosa). Un handle que se abre para pocas escrituras paga ese tiempo.
- **Espacio:** hasta un WAL más (el de reserva) mientras el handle está abierto; los archivos se borran al cerrar (`space_amp_closed` igual: 1,04).
- **Disco lleno:** el espacio del WAL se toma al crearlo, no al hacer commit; un commit que cabe en lo preasignado no necesita espacio nuevo. Un fallo al preparar la reserva no envenena el handle (`a_failure_preparing_the_spare_is_not_an_engine_failure`).

### 8.6 Límites

- Misma máquina y mismo sistema de archivos que el resto (ext4 sobre disco virtual); la ganancia de `fdatasync` depende de cómo el sistema de archivos ordene la confirmación del diario y de la contención con otros procesos (R-S3).
- El microbanco con la máquina en reposo no reproduce la ganancia del microbanco con carga; la palanca queda apagada por defecto porque el microbanco no muestra ganancia en reposo; la evidencia a favor de encenderla es la del motor (p50 −40 %, T2 −30–40 %) y queda pendiente de que la medición de referencia de WP14 la confirme para cambiar el valor por defecto. **Decidido en [§9.1](#91-wal_preallocate-ayuda-se-queda-apagado)** con 15 parejas intercaladas más Cauce y LiveSpec: se queda apagada.
- El p99 de T1 mejora por debajo de la dispersión entre ejecuciones (cuartiles 1 708–2 466 µs con la palanca): la mejora del 15 % es una mediana, no una diferencia que separe las distribuciones.

## 9. Seguimiento de la condición 1: `wal_preallocate`, volcado incremental y niveles dinámicos

Seguimiento de [§8](#8-preasignación-y-reciclado-del-wal) y de las palancas 4 y 5 de [f1-decision-cond1.md §3](f1-decision-cond1.md#3-siguientes-palancas). **No cambia el estado de la decisión**: T1 sigue sin cumplirse (§9.5).

| Campo | Valor |
|---|---|
| Estado | **Provisional**, como el resto del documento: máquina compartida con otros paquetes que compilaban y probaban (cada serie lleva la carga de 1 minuto de cada fila y [`README.txt`](../../spikes/storage/results/2026-10-28-f1-cond1b/README.txt) la resume). La medición de referencia sigue siendo la de WP14 con la máquina en reposo |
| Fecha | 2026-10-28 |
| Código medido | `kerno-storage` sobre `1764311` con `File::start_writeback` / `wait_writeback`, `CompactionConfig::{bytes_per_writeback, dynamic_base_level, file_picker}` ([DESIGN.md](../../crates/kerno-storage/DESIGN.md)) |
| Datos | [`spikes/storage/results/2026-10-28-f1-cond1b/`](../../spikes/storage/results/2026-10-28-f1-cond1b) |

### 9.1 `wal_preallocate`: ¿ayuda? Se queda apagado

`agent_memory` `medium`, `Sync`, `kerno` (apagado, el valor por defecto) frente a `kerno-tuned[wal_preallocate=true]`, intercalados con fjall y btree. Dos series de la misma orden, la segunda con el orden de los motores invertido para quitar el efecto del orden dentro de la repetición; 15 repeticiones en total, mediana del `memory_write_p99_us` (µs):

| Serie | Carga | `kerno` apagado | con la palanca | diferencia pareada (mediana) | con la palanca mejor en | fjall | btree |
|---|---|---:|---:|---:|---:|---:|---:|
| A (7 rep., `kerno` primero) | 2,6–6,1 | 4 927 (3 928–9 593) | 5 628 (3 363–8 126) | −12 % | 4 de 7 | 2 026 | 1 916 |
| B (8 rep., `kerno-tuned` primero) | 1,4–1,8 | 3 873 (3 256–4 952) | 3 448 (3 006–4 067) | −15 % | 6 de 8 | 949 | 1 518 |
| A + B | 1,4–6,1 | 4 016 (cuartiles 3 828–4 936) | 3 582 (3 363–5 628) | **−12 %** | **10 de 15** | 1 050 | 1 625 |

El p50 de escritura baja un 44 % (570 → 322 µs) en las 15 parejas, como en [§8.3](#83-agent_memory-y-t1). Con la palanca T1 queda en 3,41× fjall y 2,20× btree (3,83× y 2,47× sin ella): **no se cumple de ninguna de las dos formas**. Las otras cargas que dependen del WAL (`livespec` y `cauce`, 7 repeticiones intercaladas con `lsm`, carga 1,3–2,0, `walprealloc-t3.jsonl`):

| Medida | `kerno` apagado | con la palanca | `lsm` |
|---|---:|---:|---:|
| LiveSpec `changeset_commit_p99_us` | 3 119 (2 468–5 329) | 2 513 (2 095–3 023) | 6 410 |
| LiveSpec `module_scan_p99_us` | 911 (779–945) | 977 (818–1 008) | 856 |
| Cauce `step_commit_p99_us` | 1 321 (1 213–1 535) | 1 499 (1 342–1 716) | 686 |
| Cauce `step_commit_p50_us` | 263 | 267 | 264 |

**Decisión: `wal_preallocate` se queda en `false`.** A favor: el p50 (−44 %), una mejora de la mediana del p99 de T1 de entre el 12 y el 15 % en las tres series independientes (§8.3 y estas dos) y el p99 de commit de LiveSpec (−19 %). En contra: la mejora del p99 de T1 no separa las distribuciones (10 de 15 parejas; rangos y cuartiles solapados) y T1 no se cumple; el p99 de Cauce (T2) empeora un 13 % (también con rangos solapados) y `module_scan` un 7 %; y el coste de [§8.5](#85-costes) incluye +1,0 de `write_amp` en `bulk_load` (≈ 3,2 → 4,2 frente al límite de 3,6 de T7) y +13 ms de apertura. Una palanca que no cumple el objetivo que la motivó, que lleva T7 a incumplirse y cuyo efecto es del orden del ruido no es un valor por defecto razonable. La evidencia a favor de encenderla (p50, T2 de LiveSpec) no es un objetivo de T1.

### 9.2 Palanca 5: volcado incremental de las SST con `sync_file_range`

**Implementación.** `File::start_writeback(offset, len)` y `File::wait_writeback(offset, len)` (por defecto no hacen nada; `StdVfs` en Linux: `sync_file_range` con `SYNC_FILE_RANGE_WRITE` y con `WAIT_BEFORE | WAIT_AFTER`; en otras plataformas, nada). `SimVfs` las modela como operaciones mutantes sin efecto (`OpKind::StartWriteback` y `WaitWriteback`): cuentan para los puntos de caída y de fallo inyectado y salen en las trazas, pero **no hacen duradero nada** (lo no sincronizado se pierde con un corte de energía exactamente como sin ellas: `an_sst_writeback_request_never_makes_data_durable`). Opción: `CompactionConfig::bytes_per_writeback`.

**Microbanco primero** (`spike-bench walsync --writeback true`; p99 del `fdatasync` de 512 B que crece, o de un archivo ya escrito, mientras otro hilo escribe ráfagas de 16 MiB en el mismo directorio; mediana de 6–7 repeticiones intercaladas, carga 1,7–3,0, µs):

| Cómo acota sus datos sucios la ráfaga | archivo que crece (WAL por defecto) | archivo preasignado |
|---|---:|---:|
| sin escritura de fondo | 460 | 458 |
| nada hasta el final de la ráfaga (M0) | 9 880–10 519 | 9 814–10 559 |
| `sync_data` cada MiB (`bytes_per_sync`) | 2 494–2 724 | 2 234–2 482 |
| `sync_file_range(WRITE)` cada MiB, sin esperar | 10 117–10 166 | 10 260–10 707 |
| ídem y espera del rango anterior (dos rangos en vuelo) | 5 222 | 4 413 |
| ídem y espera del mismo rango (uno en vuelo) | 3 608 | 2 867 |

Pedir el volcado sin esperar **no cambia la cola**: solo adelanta la ráfaga y el `fdatasync` del WAL sigue esperando detrás de toda la cola del dispositivo. Lo que acota la cola es no dejar más de un rango en vuelo; con esa espera, `sync_file_range` se queda a ~1,3× del `fdatasync` por MiB. En el microbanco **ninguna variante mejora a `bytes_per_sync`**.

**En el motor** el escritor de la SST pide el volcado de cada MiB y espera ese mismo rango (la variante «un rango en vuelo»), en lugar de sincronizar (`bytes_per_sync = None`). A/B contra el valor por defecto de entonces (`bytes_per_sync = 1 MiB`), intercalados, 6 repeticiones por serie, mediana (rango) y diferencia pareada:

| Serie | Carga | Medida | por defecto | `bytes_per_writeback` | diferencia pareada | mejor en |
|---|---|---|---:|---:|---:|---:|
| 1 MiB | 1,7–1,9 | `agent_memory` p99 | 3 763 (3 168–4 264) | 2 696 (2 134–3 318) | **−24 %** | 6 de 6 |
| 1 MiB (otra sesión) | 1,2–2,0 | `agent_memory` p99 | 3 358 (2 910–4 575) | 2 982 (2 550–3 719) | −10 % | 4 de 6 |
| 1 MiB (otra sesión) | 1,6–1,8 | Cauce `step_commit_p99_us` | 1 423 (1 200–1 584) | 1 148 (953–1 251) | **−21 %** | 6 de 6 |
| 256 KiB | 1,3–1,8 | `agent_memory` p99 | 3 167 (2 243–3 836) | 2 563 (2 195–3 736) | −4 % | 4 de 6 |
| 256 KiB | 1,5–1,8 | Cauce `step_commit_p99_us` | 1 327 (997–1 457) | 853 (839–976) | **−34 %** | 6 de 6 |

El valor de 1 MiB mejora el p99 de T1 entre un 10 y un 24 % por serie (mediana de las 12 parejas: −18 %, mejor en 10 de 12) y el de Cauce entre un 21 % (1 MiB) y un 34 % (256 KiB), mejor en 6 de 6 repeticiones en las dos series: **resuelve la mitad del conflicto T1 / T2 de Cauce** descrito en [§7.4](#74-t2t5-reportmd-largejsonl-mediana-de-3-repeticiones), porque quita la barrera del diario de los `sync_data` intermedios de las compactaciones sin dejar la escritura sin acotar. No cumple T1 por sí solo (3 358–3 763 → 2 696–2 982 µs).

### 9.3 Palanca 4: niveles dinámicos y prioridad por bytes

**`dynamic_base_level`.** Con L1 vacío, ninguna compactación en vuelo hacia él y L2 lo bastante pequeño para que una compactación de L0 con `l0_trigger` archivos quepa en `max_compaction_bytes`, L0 se compacta directamente en L2: el dato se reescribe una vez en lugar de una por nivel mientras la base de datos es pequeña. La condición de que L1 esté vacío no es un detalle: un archivo en L1 sería más viejo que lo que lo esquiva, y `compaction::apply` lo rechaza como `Corruption` interna ([DESIGN.md](../../crates/kerno-storage/DESIGN.md)). Un cierre limpio **no** salta L1 (fusionar L0 con todo L2 cuesta mucho más que con L1; sin esa excepción el cierre de `bulk_load` `medium` subía un 6–17 % en los A/B).

**`file_picker = MinOverlap`** (prioridad por bytes: del nivel por encima de su objetivo se compacta el archivo que reescribe menos bytes del nivel de abajo por byte que baja).

A/B contra el valor por defecto de entonces, intercalados, 6 repeticiones, `agent_memory` `medium`:

| Variante | Carga | p99 por defecto | p99 con la palanca | diferencia pareada | mejor en | compactaciones · MB escritos por compactación (por defecto → palanca) |
|---|---|---:|---:|---:|---:|---|
| `dynamic_base_level` | 1,3–1,8 | 3 591 (2 734–4 284) | 2 527 (2 380–2 900) | **−30 %** | 6 de 6 | 22 → 17 · 561 → 523 (−7 %) |
| `file_picker = MinOverlap` | 1,5–1,8 | 3 718 (2 718–4 452) | 3 039 (2 464–3 514) | −17 % | 6 de 6 | 23 → 18 · 611 → 467 (−23 %) |
| `bytes_per_writeback` + `dynamic_base_level` | 1,4–1,8 | 3 474 (2 804–4 059) | **2 240 (1 894–2 562)** | **−31 %** | 6 de 6 | 23 → 14 · 611 → 530 (−13 %) |

La amplificación de escritura de `bulk_load` no cambia (3,0–3,3 en todas las variantes): la carga en orden aleatorio reescribe todo L1 por cada archivo de L0 y esas palancas no tocan eso. En `agent_memory` la escritura de compactación baja entre un 7 y un 23 %, y los bytes de flush (318 MB) son los mismos.

**Costes.**

- `dynamic_base_level`: el p99 de commit de LiveSpec sube un 54 % (5 118 frente a 3 332 µs con la palanca apagada; 16 y 8 repeticiones; `attr-livespec.jsonl`): compactaciones de L0 en L2 más largas. Sigue en 0,74–0,87× `lsm` (objetivo ≤ 1,2×). En el A/B aislado el cierre de `bulk_load` `medium` subía un 6 % (299 → 325 ms) y un 17 % junto con `bytes_per_writeback` (297 → 318 ms); con la excepción del cierre de más arriba, la sesión final lo mide en 179 ms frente a 252 con los valores anteriores (§9.4; no se midió la excepción aislada).
- `MinOverlap` **no es el valor por defecto** (no se midió con la corrección siguiente, así que sus cifras de la tabla no se han repetido). Tal como se midió, retrasaba justo los archivos que más basura contienen y fallaba la prueba compartida `space_is_reclaimed_after_range_deletes` con el perfil `Inline` (3 375 523 bytes en disco tras la recarga del ciclo 6, límite 3 172 072). Ahora el selector pasa primero los archivos con fragmentos de borrado de rango, la prueba pasa y `Inline` incluye `MinOverlap`. Sigue el límite de amplificación de espacio que preveía [f1-decision-cond1.md §3](f1-decision-cond1.md#3-siguientes-palancas).

### 9.4 Nuevos valores por defecto y T1–T5

Valores por defecto de `CompactionConfig` que cambian: `bytes_per_sync = None`, `bytes_per_writeback = Some(1 MiB)`. `dynamic_base_level = false` (apagada por defecto hasta que el propietario decida: T1 mejora, el p99 de commit de LiveSpec empeora 1,43×), `file_picker = RoundRobin` y `wal_preallocate = false` no cambian. El perfil `M0` apaga las tres. En una plataforma sin `sync_file_range` el valor por defecto deja las SST sin acotar (solo afecta a la cola del `sync` del WAL, y las garantías de `Sync` solo están verificadas en Linux).

Sesión final (`results.jsonl`): `kerno` con los valores nuevos tal como estaban entonces (`dynamic_base_level` encendida; ya no es el valor por defecto), `kerno-tuned[bytes_per_sync=1048576,bytes_per_writeback=none,dynamic_base_level=false]` (los valores por defecto anteriores), `lsm`, btree y fjall, `medium`, `Sync`, 5 repeticiones intercaladas, mediana. **Carga de 1 minuto: 1,14–2,38 (mediana 1,41).**

| T | carga | métrica | kerno | rango | defecto anterior | lsm | fjall | btree | k/lsm | k/fjall | k/btree | k/anterior |
| T1 | agent_memory | `memory_write_p99_us` | 2339 | 2093–2425 | 3709 | 6947 | 783 | 1399 | 0.34× | 2.99× | 1.67× | 0.63× |
| T2 | livespec | `changeset_commit_p99_us` | 4387 | 4094–5394 | 3062 | 5911 | 1428 | 3406 | 0.74× | 3.07× | 1.29× | 1.43× |
| T2 | append_seq | `append_commit100_p99_us` | 2486 | 2048–2623 | 2486 | 5059 | 1483 | 2632 | 0.49× | 1.68× | 0.94× | 1.00× |
| T2 | append_seq | `update_commit100_p99_us` | 4936 | 4172–5358 | 4772 | 7778 | 1637 | 2468 | 0.63× | 3.01× | 2.00× | 1.03× |
| T2 | history_sim | `rev_commit100_p99_us` | 7078 | 6275–7995 | 7016 | 11026 | 2851 | 19055 | 0.64× | 2.48× | 0.37× | 1.01× |
| T2 | cauce | `step_commit_p99_us` | 1197 | 901–1346 | 1402 | 671 | 574 | 2030 | 1.78× | 2.09× | 0.59× | 0.85× |
| T3 | livespec | `symbol_get_p99_us` | 47.1 | 44.8–52.7 | 48.2 | 47.4 | 37.2 | 44.6 | 0.99× | 1.27× | 1.06× | 0.98× |
| T3 | livespec | `module_scan_p99_us` | 842 | 700–898 | 871 | 848 | 565 | 478 | 0.99× | 1.49× | 1.76× | 0.97× |
| T3 | cauce | `step_read_p99_us` | 127 | 108–142 | 130 | 138 | 109 | 90.3 | 0.92× | 1.17× | 1.41× | 0.98× |
| T3 | agent_memory | `recall_p99_us` | 71.9 | 67.6–80.3 | 85.6 | 80.8 | 61.1 | 52.2 | 0.89× | 1.18× | 1.38× | 0.84× |
| T3 | agent_memory | `recent_scan_p99_us` | 89.3 | 84.0–111 | 109 | 99.9 | 98.0 | 81.7 | 0.89× | 0.91× | 1.09× | 0.82× |
| T4 | bulk_load | `close_ms` | 179 | 175–186 | 252 | 1058 | 187 | 21.6 | 0.17× | 0.96× | 8.28× | 0.71× |
| T5 | livespec | `changesets_per_s` | 623 | 571–671 | 610 | 631 | 639 | 379 | 0.99× | 0.98× | 1.65× | 1.02× |
| T5 | cauce | `steps_per_s` | 2882 | 2809–3009 | 2693 | 2983 | 3002 | 1000 | 0.97× | 0.96× | 2.88× | 1.07× |
| T5 | append_seq | `append_kops` | 113 | 110–136 | 124 | 126 | 128 | 82.9 | 0.90× | 0.88× | 1.36× | 0.91× |
| T5 | agent_memory | `steps_per_s` | 1100 | 1059–1202 | 915 | 949 | 1338 | 1016 | 1.16× | 0.82× | 1.08× | 1.20× |
| T5 | history_sim | `revisions_per_s_k` | 57.3 | 54.2–60.7 | 54.6 | 58.1 | 53.1 | 7.24 | 0.99× | 1.08× | 7.91× | 1.05× |
| T5 | bulk_load | `load_kops` | 118 | 105–130 | 129 | 116 | 134 | 15.6 | 1.02× | 0.88× | 7.56× | 0.91× |
| T7 | bulk_load | `write_amp` | 3.27 | 3.25–3.31 | 3.18 | 3.37 | 1.58 | 10.1 | 0.97× | 2.07× | 0.32× | 1.03× |
| T8 | bulk_load | `reopen_ms` | 4.08 | 3.81–13.3 | 4.30 | 6.59 | 517 | 1.42 | 0.62× | 0.01× | 2.88× | 0.95× |
| T8 | bulk_load | `recovery_ms` | 8.37 | 7.78–13.4 | 9.44 | 8.34 | 512 | 1.04 | 1.00× | 0.02× | 8.04× | 0.89× |
| - | agent_memory | `memory_write_p50_us` | 496 | 440–505 | 485 | 445 | 417 | 630 | 1.11× | 1.19× | 0.79× | 1.02× |
| - | agent_memory | `space_amp` | 3.02 | 3.02–3.02 | 3.02 | 2.46 | 3.15 | 3.93 | 1.23× | 0.96× | 0.77× | 1.00× |
| - | append_seq | `write_amp` | 2.74 | 2.74–2.75 | 2.87 | 3.45 | 1.74 | 2.27 | 0.80× | 1.57× | 1.21× | 0.96× |

(`k/anterior` < 1 es mejora en las métricas de latencia y de espacio, > 1 en las de rendimiento. `bulk_load` `large`, 3 repeticiones, carga 1,2–2,6: `close_ms` 531 (453–632) frente a 598 con los valores anteriores, `lsm` 4 658 y fjall 1 405; objetivo ≤ 1 000 ms.)

Lectura:

- **T1 no se cumple**: 2 339 µs frente a 783 de fjall en la misma sesión (**2,99×**, objetivo ≤ 2,0×) y 1 399 de btree (**1,67×**, ≤ 1,5×). Los valores nuevos bajan el p99 un 37 % respecto de los anteriores de la misma sesión (3 709 µs: 4,74× fjall y 2,65× btree) y acercan la proporción con btree a un 11 % del objetivo, pero la proporción con fjall empeora frente a las de §7–§8 (2,1–2,7×) porque fjall midió aquí 783 µs, menos que en las otras sesiones (1 044–1 059): la proporción con fjall depende tanto de fjall como de `kerno`.
- **T2**: LiveSpec 0,74× `lsm` y `append_seq` inserción y actualización 0,49× y 0,63× (cumplen); `history_sim` 0,64× (cumple); `append_seq` actualización 3,01× fjall (objetivo ≤ 2×: no cumple, como en el M0 y en WP05); **Cauce 1,78× `lsm`** (antes 2,09×; el objetivo es ≤ 1,2×: sigue sin cumplirse, mejor).
- **T3**: `symbol_get` 0,99×, Cauce `step_read` 0,92×, `recall` 0,89× y `recent_scan` 0,89× (cumplen); `module_scan` 0,99× `lsm` (cumple) y 1,76× btree (no cumple, como el M0: 2,18×).
- **T4**: `medium` 179 ms (objetivo ≤ 250 ms) y `large` 531 ms (≤ 1 000 ms): se cumple. Con la carga de estas sesiones (1,3–2,0) el valor por defecto anterior medía 252 ms.
- **T5**: entre 0,90× y 1,16× `lsm` en las seis cargas (≥ 0,9×: cumple, `append_seq` en el límite).
- **T7**: `write_amp` de `bulk_load` 3,27 (≤ 3,6: cumple; 3,18 con los valores anteriores).
- **T8**: apertura limpia 4,1 ms y recuperación 8,4 ms, 0,62× y 1,00× `lsm`.

### 9.5 Estado de la decisión y límites

- **T1 sigue sin cumplirse** (2,99× fjall y 1,67× btree frente a ≤ 2,0× y ≤ 1,5×): el estado de [f1-decision-cond1.md](f1-decision-cond1.md) sigue siendo *candidato a NO-GO, provisional*. No se modifica ningún objetivo.
- Todas las cifras son de una máquina compartida (carga de 1 minuto 0,7–6,1; las series con carga > 3 se marcan) y de un solo sistema de archivos (ext4 sobre disco virtual). Una mejora del 10–15 % del p99 de T1 está dentro de la dispersión entre repeticiones; las que superan el 25 % (`dynamic_base_level`, la combinación) se repiten en 6 de 6 repeticiones pero con 6 repeticiones por serie.
- Los valores por defecto nuevos son una decisión tomada con estas series y quedan pendientes de que WP14 las confirme con la máquina en reposo. `dynamic_base_level` (apagada por defecto) cambia el compromiso entre objetivos (T1 mejor, p99 de commit de LiveSpec peor, aún dentro de T2); si el propietario prefiere lo contrario, se enciende con `KERNO_TUNING=dynamic_base_level=true` o `CompactionConfig::dynamic_base_level`.
- La prueba compartida `space_is_reclaimed_after_range_deletes` y `space_is_reclaimed` de los perfiles con hilos (`threads`, `m0_profile`) tuvieron fallos aislados al ejecutar el conjunto completo con otros procesos compilando (también en el perfil `M0`, que no lleva ninguna palanca nueva); aislados pasan siempre (6 de 6). No se han atribuido.
