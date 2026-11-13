# Informe de comparación de kerno con SQLite — nueva pasada, sin umbrales (E9-17)

| Campo | Valor |
|---|---|
| Estado | Resultados del 2026-11-12. **Este informe no fija umbrales** (D6 de [`etapa9-acta.md`](../architecture/etapa9-acta.md) §8.1): es una comparación descriptiva |
| Paquete | E9-17 de [`etapa9-acta.md`](../architecture/etapa9-acta.md) §3 |
| Sustituye a | La pasada de E8-S0 ([`sqlite-report.md`](sqlite-report.md)) solo como medición en calma; ese informe se conserva |
| Datos crudos | [`spikes/storage/results/2026-11-12-e9/`](../../spikes/storage/results/2026-11-12-e9/) |

## 0. Qué es y qué no es

- Comparación **sin umbrales**: ninguna cifra es un objetivo ni un listón. Las razones kerno / SQLite son cocientes de medianas observadas.
- Se informan **todas** las cargas, también aquellas en que kerno es más lento (§4).
- A diferencia de E8-S0, esta pasada se hizo **en calma** (§2). Eso no convierte ninguna cifra en un valor general: es una sola máquina, una sola escala y 5 repeticiones.

## 1. Método

- **Binario existente, sin cambios**: `spike-bench` (`spikes/storage/spike-bench`) tal como está en el repositorio, compilado en `release` con `--features candidates,kerno`. No se modificó ningún archivo del binario ni de ningún crate. Los pares de durabilidad son los de [`sqlite-report.md`](sqlite-report.md) §1: kerno `Sync` frente a SQLite WAL + `synchronous=FULL`, y kerno `Buffered` frente a SQLite WAL + `synchronous=NORMAL`. **La garantía ante un corte de energía no es idéntica en el segundo par** y este informe no la mide.
- **Cargas.** Las 12 de [`sqlite-report.md`](sqlite-report.md) §1. `rmw_counters` queda fuera, por la misma razón (en SQLite corre en una capa de emulación y no es la misma carga).
- **Escala** `tiny` (5 000 registros; todo cabe en memoria).
- **Repeticiones: 5 por motor y carga** (semillas `42 + repetición`, iguales para los dos), intercaladas kerno, SQLite, kerno… Cada celda de las tablas sale de **5 ejecuciones**; un par de durabilidad son 120 ejecuciones (12 cargas × 5 × 2 motores), 240 en total. Ninguna ejecución falló y no hay ninguna descartada.
- **Dispersión.** Mediana de las 5 repeticiones y, entre corchetes, mínimo y máximo (rango observado, no un intervalo de confianza). La última columna es el cociente de medianas, kerno / SQLite. Cuando los rangos de los dos motores se solapan, la diferencia de medianas no distingue un motor del otro.
- **Agregación.** [`aggregate.py`](../../spikes/storage/results/2026-11-12-e9/aggregate.py) (el de E8-S0 con la lista de ficheros de esta pasada) produce [`tables.md`](../../spikes/storage/results/2026-11-12-e9/tables.md), que contiene todas las tablas (§3).

### 1.1 Comandos exactos

Desde la raíz del repositorio (`DATA` es un directorio de trabajo temporal, borrado al terminar):

```
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench env --dir $DATA > spikes/storage/results/2026-11-12-e9/env.json
W=bulk_load,point_reads,range_scans,small_commits,point_deletes,append_seq,mixed,history_sim,livespec,cauce,agent_memory,concurrent_commits
target/release/spike-bench suite --engines kerno,sqlite --workloads $W --scales tiny --reps 5 \
  --durability sync --dir $DATA --out spikes/storage/results/2026-11-12-e9/results-sync.jsonl
target/release/spike-bench suite --engines kerno,sqlite --workloads $W --scales tiny --reps 5 \
  --durability buffered --dir $DATA --out spikes/storage/results/2026-11-12-e9/results-buffered.jsonl
python3 -I spikes/storage/results/2026-11-12-e9/aggregate.py > spikes/storage/results/2026-11-12-e9/tables.md
```

### 1.2 Archivos crudos

| Archivo | Contenido |
|---|---|
| `env.json` | Salida de `spike-bench env`, **reducida** a núcleos, memoria, commit, carga y latencia de `fdatasync`; se quitaron a mano los campos que describen el entorno sin ser necesarios para reproducir (T-NM) |
| `results-sync.jsonl`, `progress-sync.log` | Par Sync, 120 ejecuciones |
| `results-buffered.jsonl`, `progress-buffered.log` | Par Buffered, 120 ejecuciones |
| `aggregate.py`, `tables.md` | Agregación y su salida |

## 2. Entorno y carga de la máquina (T-SQ1)

- 4 núcleos, 15,7 GiB. Código: commit `0be147b`.
- Espacio libre antes de medir: 7,1 GiB.
- **Nada más compilaba ni corría** durante las dos tandas (la compilación de `spike-bench` terminó antes). Carga de 1 minuto al lanzar cada tanda: 1,83 (Sync) y 1,69 (Buffered), por debajo de 4 (T-SQ1). La carga que registra el propio binario en cada ejecución incluye al propio benchmark:

  | Archivo | Ejecuciones | Carga (1 min), mínimo – mediana – máximo |
  |---|---|---|
  | `results-sync.jsonl` | 120 | 1,33 – 1,54 – 1,90 |
  | `results-buffered.jsonl` | 120 | 1,58 – 1,69 – 1,89 |

  Ninguna ejecución superó 4, así que no hubo que repetir ningún bloque.
- `fdatasync` de 4 KiB (`env.json`): p50 93,0 µs · p95 160 µs · p99 198 µs · máx 1 014 µs. En E8-S0 el mismo comando midió p50 3 994 µs. Esto cambia sobre todo el par Sync: **las cifras de Sync de esta pasada y las de E8-S0 no son comparables entre sí** (otro dispositivo en otro momento); dentro de cada pasada la comparación kerno / SQLite sí es válida.

## 3. Resultados

Todas las tablas (cada cifra, mediana [mín–máx] de 5 ejecuciones) están en [`tables.md`](../../spikes/storage/results/2026-11-12-e9/tables.md); §7 las reproduce íntegras, sin retocar.

## 4. Observaciones (descriptivas; sin conclusión de cumplimiento)

Cada punto cita medianas de las tablas.

**Donde kerno es más lento o mayor que SQLite:**

- **Apertura y recuperación.** `recovery_ms` de `bulk_load`: 3,54 frente a 0,38 ms (Sync) y 3,96 frente a 0,32 (Buffered); de `cauce`: 49,6 frente a 0,36 (Sync) y 63,5 frente a 0,35 (Buffered). `reopen_ms` de `bulk_load`: 0,62 frente a 0,30 (Sync) y 0,77 frente a 0,33 (Buffered).
- **Cierre tras `bulk_load`.** `close_ms`: 11,1 frente a 4,48 ms (Sync) y 14,0 frente a 3,73 (Buffered).
- **`livespec`, lectura de módulo.** `module_scan_p99_us`: 258 frente a 165 µs (Sync) y 503 frente a 162 (Buffered); `module_scan_p50_us` es igual en Sync (82,8 en ambos).
- **`small_commits` en Sync.** `commits_per_s` 7 226 frente a 8 047 ([6 749–7 384] y [7 327–8 735]; rangos solapados, así que la diferencia de medianas no distingue un motor del otro); `commit_p50_us` 127 frente a 101 µs.
- **`append_seq`.** `peak_rss_mb` 11,0 frente a 8,42 MB (Sync); `update_kops` en Sync 227 frente a 252, con rangos solapados.
- **`mixed` en Buffered**, `write_p50_us`: 25,3 frente a 23,2 µs, con rangos solapados.
- **`concurrent_commits`**, `cc_commit_p50_us`: 377 frente a 131 µs (Sync) y 87,2 frente a 18,2 (Buffered), aunque la tasa de commits de kerno es mayor.

**Donde kerno es más rápido o menor:**

- Lecturas puntuales: `get_1t_kops` 1 628 frente a 895 (Sync); `get_4t_kops` 4 928 frente a 1 995.
- Escaneos de rango: `scan100_kops` 60,4 frente a 36,5 (Sync).
- Escrituras: `bulk_load` `load_kops` 366 frente a 89,3 (Sync); `concurrent_commits` 20 842 frente a 6 769 commits/s (Sync), con 0,22 `fsync` por commit en kerno. En Buffered, `small_commits` 332 291 frente a 57 444 commits/s y `cauce` 123 628 frente a 13 121 pasos/s; **no es una ventaja de garantías equivalentes** (§1).
- Amplificación: `write_amp` de `bulk_load` 1,01 frente a 6,25; de `small_commits` 1,17 frente a 30,0 (Sync).
- Colas de commit en cargas con `fsync`: `changeset_commit_p99_us` de `livespec` 940 frente a 5 730 µs (Sync); `cc_commit_p99_us` 715 frente a 11 665 µs (Sync).

**Cargas con cociente cercano a 1 en Sync:** `commit_p95_us` de `small_commits` (0,99), `delete_commit_p50_us` (1,05), `step_commit_p50_us` de `cauce` (1,06) y `step_commit_p99_us` (0,94).

## 5. Lo que **no** se puede concluir

- Ninguna conclusión de cumplimiento: no hay umbral.
- Nada sobre un corte de energía ni sobre pérdida de datos.
- Nada sobre conjuntos mayores que la memoria ni sobre las escalas `small`, `medium` o `large`.
- Nada preciso sobre latencias de cola: con 5 repeticiones, p99, p99.9 y máximos tienen rangos amplios en varias cargas (por ejemplo `get_4t_p99_us` de SQLite, [4,96–14,4] µs).
- Nada sobre otras máquinas ni sobre otro dispositivo de disco: el `fsync` de esta pasada es mucho más rápido que el de E8-S0 (§2) y cambiaría sobre todo el par Sync.
- No es una comparación entre dos motores ajustados: el adaptador de SQLite es el del binario (una conexión escritora y hasta cuatro lectoras, sin ajustes adicionales) y el de kerno usa la configuración por defecto del binario.
- Cada razón es de medianas de 5 repeticiones, sin intervalo de confianza ni prueba de significación.

## 6. Límites de la ejecución

1. Una sola pasada por par; las dos tandas se midieron seguidas, una tras otra, y no se compara una tanda con la otra.
2. `rmw_counters` no se ejecutó.
3. `env.json` es una versión reducida de la salida del binario (§1.2); el resto de los archivos crudos está tal como los escribió el binario.

## 7. Tablas

### Durabilidad `sync`: kerno `Sync` frente a SQLite WAL + `synchronous=FULL`

#### `bulk_load` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 366 [314–385] | 89.3 [78.5–101] | 4.10 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 11.1 [10.2–11.5] | 4.48 [4.25–5.60] | 2.48 |
| `reopen_ms` | 0.62 [0.60–0.71] | 0.30 [0.30–0.45] | 2.07 |
| `recovery_ms` | 3.54 [3.52–5.15] | 0.38 [0.37–0.51] | 9.32 |
| `peak_rss_mb` | 9.50 [9.39–9.63] | 11.0 [10.9–11.1] | 0.86 |

#### `point_reads` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 1628 [1484–1700] | 895 [888–969] | 1.82 |
| `get_1t_p50_us` | 0.52 [0.50–0.52] | 0.86 [0.82–0.87] | 0.60 |
| `get_1t_p95_us` | 0.64 [0.61–0.78] | 1.60 [1.45–1.65] | 0.40 |
| `get_1t_p99_us` | 0.74 [0.70–0.89] | 2.11 [1.90–2.32] | 0.35 |
| `get_4t_kops` | 4928 [4535–5307] | 1995 [1827–2876] | 2.47 |
| `get_4t_p99_us` | 1.01 [0.96–1.11] | 12.7 [4.96–14.4] | 0.08 |

#### `range_scans` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 60.4 [53.0–62.4] | 36.5 [34.3–39.7] | 1.66 |
| `scan100_p50_us` | 13.8 [13.6–14.4] | 23.4 [22.0–24.0] | 0.59 |
| `scan100_p95_us` | 19.8 [16.9–22.6] | 37.5 [29.4–39.5] | 0.53 |
| `scan100_p99_us` | 34.0 [32.4–43.0] | 59.2 [44.9–62.4] | 0.57 |

#### `small_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 7226 [6749–7384] | 8047 [7327–8735] | 0.90 |
| `commit_p50_us` | 127 [122–129] | 101 [91.1–107] | 1.25 |
| `commit_p95_us` | 219 [200–225] | 221 [203–269] | 0.99 |
| `commit_p99_us` | 355 [317–366] | 327 [276–376] | 1.08 |
| `commit_p999_us` | 677 [654–1000] | 2548 [1912–5841] | 0.27 |
| `write_amp` | 1.17 [1.17–1.17] | 30.0 [29.9–30.1] | 0.04 |
| `commit_fsyncs_per_commit` | 1.00 [1.00–1.00] | — | — |

#### `point_deletes` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 7160 [6961–8672] | 6040 [5638–6653] | 1.19 |
| `delete_commit_p50_us` | 127 [108–136] | 121 [113–126] | 1.05 |
| `delete_commit_p99_us` | 323 [285–360] | 432 [339–493] | 0.75 |
| `get_deleted_p50_us` | 0.36 [0.34–0.37] | 0.79 [0.77–0.84] | 0.45 |
| `get_deleted_p99_us` | 0.59 [0.57–0.77] | 2.44 [2.27–2.75] | 0.24 |
| `get_live_p99_us` | 1.84 [1.78–2.12] | 2.58 [2.56–3.24] | 0.71 |

#### `append_seq` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 238 [131–281] | 159 [130–182] | 1.50 |
| `update_kops` | 227 [201–280] | 252 [194–268] | 0.90 |
| `update_commit100_p99_us` | 628 [456–2652] | 720 [580–1032] | 0.87 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.0 [10.9–11.1] | 8.42 [8.03–8.49] | 1.30 |

#### `mixed` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 128 [113–139] | 52.8 [48.9–53.9] | 2.43 |
| `read_p50_us` | 1.65 [1.57–1.71] | 10.4 [10.1–10.9] | 0.16 |
| `read_p99_us` | 12.8 [10.3–19.2] | 68.5 [67.3–73.1] | 0.19 |
| `write_p50_us` | 270 [262–299] | 525 [499–562] | 0.51 |
| `write_p99_us` | 557 [489–865] | 2165 [2087–2415] | 0.26 |

#### `history_sim` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 122 [89.1–140] | 52.2 [47.9–56.9] | 2.34 |
| `rev_commit100_p99_us` | 995 [871–4207] | 4108 [3607–4481] | 0.24 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.24 [2.14–3.62] | 3.66 [3.38–3.90] | 0.61 |
| `history_scan_p99_us` | 4.27 [3.52–6.32] | 11.1 [10.7–14.6] | 0.38 |
| `reconstruct_p99_us` | 2.58 [1.92–3.73] | 4.13 [4.04–6.12] | 0.62 |

#### `livespec` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 1588 [1527–1728] | 1058 [1029–1071] | 1.50 |
| `changeset_commit_p50_us` | 346 [294–367] | 452 [447–467] | 0.77 |
| `changeset_commit_p99_us` | 940 [875–979] | 5730 [5612–6025] | 0.16 |
| `symbol_get_p99_us` | 5.43 [5.21–6.43] | 9.46 [8.94–10.2] | 0.57 |
| `module_scan_p50_us` | 82.8 [80.7–90.3] | 82.8 [82.3–83.2] | 1.00 |
| `module_scan_p99_us` | 258 [218–301] | 165 [160–179] | 1.57 |

#### `cauce` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 6739 [6495–7195] | 5638 [5394–6142] | 1.20 |
| `step_commit_p50_us` | 127 [121–131] | 121 [111–125] | 1.06 |
| `step_commit_p99_us` | 325 [289–334] | 344 [267–372] | 0.94 |
| `step_read_p99_us` | 20.5 [18.2–24.4] | 63.1 [54.9–74.0] | 0.33 |
| `recovery_ms` | 49.6 [48.0–54.4] | 0.36 [0.34–0.38] | 136.29 |

#### `agent_memory` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 3068 [2757–3460] | 1463 [1287–1562] | 2.10 |
| `memory_write_p50_us` | 223 [211–255] | 293 [272–313] | 0.76 |
| `memory_write_p99_us` | 813 [599–1137] | 1377 [1163–2238] | 0.59 |
| `recall_p99_us` | 14.6 [13.1–18.6] | 57.6 [51.5–77.5] | 0.25 |
| `recent_scan_p99_us` | 33.3 [30.9–39.0] | 112 [91.6–155] | 0.30 |
| `expire_commit_p99_us` | 902 [423–1764] | 4188 [3809–6607] | 0.22 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 20842 [20295–23449] | 6769 [6582–6967] | 3.08 |
| `cc_commit_p50_us` | 377 [343–393] | 131 [126–135] | 2.88 |
| `cc_commit_p99_us` | 715 [584–791] | 11665 [11297–11813] | 0.06 |
| `cc_fsyncs_per_commit` | 0.22 [0.22–0.22] | — | — |

### Durabilidad `buffered`: kerno `Buffered` frente a SQLite WAL + `synchronous=NORMAL`

#### `bulk_load` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 531 [370–563] | 91.1 [79.7–97.1] | 5.82 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 14.0 [12.7–16.8] | 3.73 [3.51–4.28] | 3.75 |
| `reopen_ms` | 0.77 [0.60–0.83] | 0.33 [0.29–0.33] | 2.30 |
| `recovery_ms` | 3.96 [3.03–5.13] | 0.32 [0.30–0.36] | 12.41 |
| `peak_rss_mb` | 9.45 [9.30–9.54] | 11.1 [11.0–11.1] | 0.86 |

#### `point_reads` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 1603 [1552–1714] | 872 [725–951] | 1.84 |
| `get_1t_p50_us` | 0.52 [0.49–0.53] | 0.88 [0.83–1.01] | 0.59 |
| `get_1t_p95_us` | 0.64 [0.62–0.68] | 1.67 [1.47–2.00] | 0.38 |
| `get_1t_p99_us` | 0.77 [0.74–0.85] | 2.26 [1.90–2.83] | 0.34 |
| `get_4t_kops` | 4800 [4456–5117] | 2427 [2176–2478] | 1.98 |
| `get_4t_p99_us` | 0.96 [0.95–1.08] | 12.5 [10.1–14.7] | 0.08 |

#### `range_scans` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 61.1 [56.8–64.7] | 38.9 [37.5–40.0] | 1.57 |
| `scan100_p50_us` | 13.8 [13.2–14.3] | 22.9 [22.0–23.2] | 0.60 |
| `scan100_p95_us` | 17.4 [15.9–18.4] | 32.8 [29.6–34.8] | 0.53 |
| `scan100_p99_us` | 32.9 [31.4–33.4] | 50.0 [48.0–51.1] | 0.66 |

#### `small_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 332291 [291183–345897] | 57444 [53937–60207] | 5.78 |
| `commit_p50_us` | 1.94 [1.92–2.09] | 6.09 [5.94–6.14] | 0.32 |
| `commit_p95_us` | 3.81 [3.74–4.06] | 9.56 [8.47–9.84] | 0.40 |
| `commit_p99_us` | 10.1 [8.38–11.0] | 21.9 [20.8–23.5] | 0.46 |
| `commit_p999_us` | 24.4 [21.0–28.9] | 1496 [613–3271] | 0.02 |
| `write_amp` | 1.27 [1.27–1.27] | 29.7 [29.6–29.8] | 0.04 |
| `commit_fsyncs_per_commit` | 0.00 [0.00–0.00] | — | — |

#### `point_deletes` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 504718 [448818–522184] | 31415 [30829–36515] | 16.07 |
| `delete_commit_p50_us` | 1.65 [1.62–1.69] | 8.37 [7.63–10.1] | 0.20 |
| `delete_commit_p99_us` | 3.59 [3.19–3.72] | 45.4 [40.1–57.5] | 0.08 |
| `get_deleted_p50_us` | 0.37 [0.35–0.39] | 0.80 [0.77–0.81] | 0.46 |
| `get_deleted_p99_us` | 0.62 [0.55–0.85] | 2.38 [2.16–2.54] | 0.26 |
| `get_live_p99_us` | 1.42 [1.29–1.90] | 2.72 [2.38–2.92] | 0.52 |

#### `append_seq` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 887 [871–918] | 511 [421–543] | 1.73 |
| `update_kops` | 791 [779–817] | 389 [324–406] | 2.03 |
| `update_commit100_p99_us` | 122 [110–134] | 435 [324–659] | 0.28 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.0 [10.9–11.1] | 8.50 [8.34–8.59] | 1.30 |

#### `mixed` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 679 [632–705] | 136 [130–139] | 5.01 |
| `read_p50_us` | 1.92 [1.85–2.08] | 15.1 [13.8–15.4] | 0.13 |
| `read_p99_us` | 20.8 [15.7–25.1] | 94.5 [86.5–160] | 0.22 |
| `write_p50_us` | 25.3 [23.8–27.6] | 23.2 [20.3–24.4] | 1.09 |
| `write_p99_us` | 99.7 [92.9–151] | 356 [247–491] | 0.28 |

#### `history_sim` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 331 [245–343] | 60.4 [55.2–63.7] | 5.49 |
| `rev_commit100_p99_us` | 344 [302–382] | 9855 [9675–10461] | 0.03 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.25 [2.16–3.17] | 3.80 [3.76–3.92] | 0.59 |
| `history_scan_p99_us` | 4.16 [3.71–15.3] | 12.7 [10.6–15.9] | 0.33 |
| `reconstruct_p99_us` | 2.71 [2.07–5.00] | 5.39 [4.20–5.95] | 0.50 |

#### `livespec` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 3026 [2898–3082] | 1383 [1366–1426] | 2.19 |
| `changeset_commit_p50_us` | 41.0 [40.7–42.1] | 105 [100–106] | 0.39 |
| `changeset_commit_p99_us` | 160 [147–171] | 11411 [11125–11526] | 0.01 |
| `symbol_get_p99_us` | 5.23 [4.95–5.42] | 8.96 [8.00–9.52] | 0.58 |
| `module_scan_p50_us` | 84.3 [81.9–85.4] | 80.3 [79.4–82.1] | 1.05 |
| `module_scan_p99_us` | 503 [470–532] | 162 [156–191] | 3.10 |

#### `cauce` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 123628 [110694–125224] | 13121 [12407–14130] | 9.42 |
| `step_commit_p50_us` | 2.96 [2.94–3.29] | 11.8 [11.1–12.6] | 0.25 |
| `step_commit_p99_us` | 7.54 [7.27–9.92] | 63.8 [59.2–78.7] | 0.12 |
| `step_read_p99_us` | 14.8 [12.5–16.1] | 62.5 [57.9–66.6] | 0.24 |
| `recovery_ms` | 63.5 [59.1–73.0] | 0.35 [0.35–0.47] | 179.97 |

#### `agent_memory` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 15497 [14144–15728] | 2216 [2036–2433] | 6.99 |
| `memory_write_p50_us` | 7.00 [6.99–7.45] | 40.3 [36.4–43.2] | 0.17 |
| `memory_write_p99_us` | 34.2 [32.0–36.9] | 574 [530–672] | 0.06 |
| `recall_p99_us` | 16.4 [14.8–17.8] | 54.7 [48.1–66.5] | 0.30 |
| `recent_scan_p99_us` | 38.8 [34.3–39.9] | 93.3 [78.8–135] | 0.42 |
| `expire_commit_p99_us` | 143 [117–858] | 9372 [8643–15843] | 0.02 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 85971 [82277–89258] | 26204 [25032–28192] | 3.28 |
| `cc_commit_p50_us` | 87.2 [84.7–90.8] | 18.2 [16.8–21.8] | 4.79 |
| `cc_commit_p99_us` | 262 [253–290] | 8569 [8110–8774] | 0.03 |

