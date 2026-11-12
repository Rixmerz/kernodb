# Informe de comparación de kerno con SQLite — sin umbrales (E8-S0)

| Campo | Valor |
|---|---|
| Estado | Resultados del 2026-11-12. **Este informe no fija umbrales**: es una comparación descriptiva, por decisión del propietario del 2026-11-11 (punto 2) |
| Paquete | E8-S0 de [`etapa8-acta.md`](../architecture/etapa8-acta.md) §3 |
| Relacionado | [`f1-objetivos-rendimiento.md`](f1-objetivos-rendimiento.md) (durabilidad equivalente), [`f1-report.md`](f1-report.md) (método y convenciones), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) |
| Datos crudos | [`spikes/storage/results/2026-11-12-e8-s0/`](../../spikes/storage/results/2026-11-12-e8-s0/) |

## 0. Qué es y qué no es este informe

- Es una **comparación sin umbrales**. No declara «cumple» ni «no cumple» para ninguna cifra, y **ninguna cifra de este informe es un objetivo**, una meta ni un listón para trabajo futuro. Las razones kerno / SQLite son solo cocientes de medianas observadas; no se comparan con ningún valor de referencia.
- Se informan **todos** los resultados, incluidos aquellos en los que kerno es más lento (§5).
- La máquina estaba **compartida y muy cargada** durante toda la medición (§2). Las cifras absolutas no son reproducibles tal cual y las diferencias pequeñas no son interpretables.

## 1. Método

- **Binario existente, sin cambios.** Se usa `spike-bench` (`spikes/storage/spike-bench`) tal como está en el repositorio; no se modificó ningún archivo del binario ni de ningún crate. El adaptador de SQLite (`adapters.rs`, `SqliteEngine`) ya empareja la durabilidad como declara [`f1-objetivos-rendimiento.md`](f1-objetivos-rendimiento.md): con `--durability sync` abre SQLite con `journal_mode=WAL` y `synchronous=FULL`; con `--durability buffered`, con `journal_mode=WAL` y `synchronous=NORMAL`. En kerno, `sync` es `Durability::Sync` y `buffered` es `Durability::Buffered`.
- **Pares de durabilidad.**

  | Par | kerno | SQLite |
  |---|---|---|
  | Sync | `Sync` (un `fsync` por commit en un solo escritor) | WAL + `synchronous=FULL` |
  | Buffered | `Buffered` (sin `fsync` por commit) | WAL + `synchronous=NORMAL` |

  **La garantía ante un corte de energía no es idéntica en el segundo par.** En ambos casos un commit confirmado no está en disco de forma duradera en el momento de confirmarse: SQLite con WAL + `NORMAL` sincroniza el WAL solo en los puntos de control; kerno `Buffered` no sincroniza por commit. Qué parte de los últimos commits se pierde tras un corte de energía y cómo se recupera cada motor son propiedades distintas de cada uno y **este informe no las mide**. Por eso las cifras del par Buffered miden coste de ejecución sin `fsync` por commit, no una equivalencia de garantías. El par Sync es el equivalente que declara el documento de objetivos.
- **Cargas.** Las 12 cargas de `spike-bench` que ambos motores ejecutan tal cual: `bulk_load`, `point_reads`, `range_scans`, `small_commits`, `point_deletes`, `append_seq`, `mixed`, `history_sim`, `livespec`, `cauce`, `agent_memory`, `concurrent_commits` (8 escritores).
  - **Excluida: `rmw_counters`.** En kerno corre sobre transacciones nativas; en SQLite el binario la ejecuta a través de una capa de emulación (`Emulated`), de modo que no es la misma carga en los dos motores. En el par Buffered las filas crudas existen en `results-buffered.jsonl` pero no se tabulan; en el par Sync no llegó a ejecutarse (§6).
- **Escala.** `tiny` = 5 000 registros (el binario la define en `scale_n`). Es una escala **moderada a propósito**, porque la máquina tiene 4 CPU compartidas. Todo el conjunto de datos cabe en memoria.
- **Repeticiones.** 5 por motor y carga, con semillas `42 + repetición` (las mismas para los dos motores). Cada ejecución es un proceso nuevo; dentro de cada carga, las repeticiones se intercalan (kerno, SQLite, kerno, SQLite…).
- **Estadística.** Para cada métrica se da la **mediana de las 5 repeticiones** y, entre corchetes, el **mínimo y el máximo** (el rango observado, no un intervalo de confianza). La última columna es el cociente de las medianas, kerno / SQLite; su lectura depende de la métrica (en rendimiento, >1 significa que kerno hizo más por segundo; en latencias, tiempos, amplificación y memoria, >1 significa un valor mayor en kerno). `<0.01` indica un cociente menor que 0,005.
- **Tablas.** Generadas por [`aggregate.py`](../../spikes/storage/results/2026-11-12-e8-s0/aggregate.py) a partir de los JSONL crudos (salida archivada en [`tables.md`](../../spikes/storage/results/2026-11-12-e8-s0/tables.md)). Las métricas son las de cabecera de cada carga (`headline()` en `spike-bench/src/main.rs`).

### 1.1 Comandos exactos

Desde la raíz del repositorio (`DATA` es un directorio de trabajo temporal, borrado al terminar):

```
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench env --dir $DATA > spikes/storage/results/2026-11-12-e8-s0/env.json

# Par Sync, primera pasada (13 cargas × 5 repeticiones × 2 motores = 130 ejecuciones previstas)
target/release/spike-bench suite --engines kerno,sqlite --workloads all --scales tiny --reps 5 \
  --durability sync --dir $DATA --out spikes/storage/results/2026-11-12-e8-s0/results-sync.jsonl

# Par Sync, repetición completa de concurrent_commits
target/release/spike-bench suite --engines kerno,sqlite --workloads concurrent_commits --scales tiny --reps 5 \
  --durability sync --dir $DATA --out spikes/storage/results/2026-11-12-e8-s0/results-sync-concurrent.jsonl

# Par Buffered
target/release/spike-bench suite --engines kerno,sqlite --workloads all --scales tiny --reps 5 \
  --durability buffered --dir $DATA --out spikes/storage/results/2026-11-12-e8-s0/results-buffered.jsonl

python3 -I spikes/storage/results/2026-11-12-e8-s0/aggregate.py > spikes/storage/results/2026-11-12-e8-s0/tables.md
```

El comando de la primera pasada Sync no se archivó cuando se lanzó; el que se muestra es el reconstruido a partir de su registro de progreso (`progress-sync.log`: contador `[n/130]`, escala `tiny`, 5 repeticiones, solo `kerno` y `sqlite`, cargas en el orden de `WORKLOADS`).

### 1.2 Archivos crudos

| Archivo | Contenido |
|---|---|
| `env.json` | Entorno (de la primera pasada Sync) |
| `results-sync.jsonl`, `progress-sync.log` | Primera pasada Sync. **Interrumpida** (§6). La última línea del JSONL está truncada y se conserva tal cual |
| `results-sync-concurrent.jsonl`, `progress-sync-concurrent.log` | Repetición completa de `concurrent_commits` en Sync |
| `results-buffered.jsonl`, `progress-buffered.log` | Par Buffered, 130 ejecuciones (incluye `rmw_counters`, no tabulado) |
| `aggregate.py`, `tables.md` | Agregación y su salida |

## 2. Entorno y carga de la máquina

| | |
|---|---|
| Máquina | VM, **4 núcleos**, 15,7 GiB |
| Compilación | `release`; SQLite es el incluido (`bundled`) según el `Cargo.lock` |
| Código | Commit `0d05269` (`env.json`); el binario se reconstruyó para la segunda y tercera tanda sobre el mismo código |
| `fdatasync` de 4 KiB (`env.json`) | p50 3 994 µs · p95 4 069 µs · p99 7 586 µs · máx 7 999 µs |

**La máquina estaba compartida.** Otros trabajos pesados corrían a la vez en los mismos 4 CPU y en el mismo disco. La media de carga de 1 minuto registrada por el propio binario en cada ejecución fue:

| Archivo | Ejecuciones | Carga (1 min), mínimo – mediana – máximo |
|---|---|---|
| `results-sync.jsonl` | 114 | 5,04 – 8,99 – 14,18 |
| `results-sync-concurrent.jsonl` | 10 | 10,98 – 13,125 – 13,56 |
| `results-buffered.jsonl` | 130 | 8,98 – 11,52 – 12,06 |

Con 4 CPU, una carga de 5 a 14 significa que había entre una y tres veces más tareas ejecutables que CPU. Efectos sobre los números:

- Los procesos de benchmark compiten por CPU con otros trabajos: la **dispersión crece** y las **latencias de cola (p99, p99.9) y los máximos** quedan inflados de forma impredecible. Ningún motor está a salvo, pero no tienen por qué sufrirlo en la misma medida.
- La latencia de `fdatasync` de la propia máquina (≈ 4 ms en p50) es mucho mayor que la registrada en informes anteriores en esta clase de VM (113 a 173 µs, p50, en M0 y en [`f1-report.md`](f1-report.md) §5.1) y es compartida con otros procesos. En el par Sync, casi todo commit de ambos motores queda **acotado por ese `fsync`** (de ahí las medianas cercanas a 4 000 µs en ambos), de modo que muchas diferencias del par Sync reflejan el dispositivo del momento y no el motor.
- Algunas métricas muestran dos regímenes entre repeticiones (por ejemplo `small_commits` Sync de kerno, mediana de `commit_p50_us` 3 871 µs con mínimo 116 µs): en algunas repeticiones el `fsync` fue rápido y en otras lento.
- La carga cambió entre tandas (la repetición de `concurrent_commits` y la tanda Buffered se midieron con más carga que buena parte de la primera pasada Sync). **Comparar un par con otro, o una tanda con otra, no es válido**; la comparación válida es kerno frente a SQLite dentro del mismo par y la misma carga, cuyas repeticiones están intercaladas.

## 3. Resultados del par Sync: kerno `Sync` frente a SQLite WAL + `synchronous=FULL`

Escala `tiny` (5 000 registros), 5 repeticiones por motor. Formato de cada celda: mediana [mín–máx].

#### `bulk_load` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 123 [112–137] | 54.1 [53.4–59.8] | 2.26 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 59.1 [55.2–59.4] | 12.6 [10.1–15.5] | 4.68 |
| `reopen_ms` | 12.0 [11.9–16.0] | 0.41 [0.37–0.49] | 29.45 |
| `recovery_ms` | 51.8 [31.7–59.9] | 0.44 [0.43–0.51] | 116.65 |
| `peak_rss_mb` | 9.62 [9.39–9.63] | 10.8 [10.7–10.9] | 0.89 |

#### `point_reads` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 1130 [1077–1536] | 705 [586–811] | 1.60 |
| `get_1t_p50_us` | 0.54 [0.53–0.55] | 0.95 [0.90–1.04] | 0.57 |
| `get_1t_p95_us` | 0.74 [0.68–0.81] | 1.90 [1.65–2.16] | 0.39 |
| `get_1t_p99_us` | 0.98 [0.87–1.08] | 2.74 [2.19–2.92] | 0.36 |
| `get_4t_kops` | 2533 [2417–2600] | 1296 [1187–1425] | 1.95 |
| `get_4t_p99_us` | 1.10 [1.01–1.12] | 4.36 [3.94–4.94] | 0.25 |

#### `range_scans` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 49.2 [46.1–53.4] | 36.6 [35.5–37.7] | 1.34 |
| `scan100_p50_us` | 14.3 [14.0–14.3] | 23.6 [23.4–24.0] | 0.60 |
| `scan100_p95_us` | 21.6 [16.0–24.0] | 32.2 [30.8–36.1] | 0.67 |
| `scan100_p99_us` | 34.8 [32.9–43.6] | 48.4 [47.4–52.3] | 0.72 |

#### `small_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 419 [257–899] | 532 [363–652] | 0.79 |
| `commit_p50_us` | 3871 [116–3998] | 175 [113–3990] | 22.06 |
| `commit_p95_us` | 4018 [4004–7614] | 4008 [4006–4678] | 1.00 |
| `commit_p99_us` | 7799 [4817–9773] | 5370 [4940–8004] | 1.45 |
| `commit_p999_us` | 9953 [7999–19988] | 11182 [8020–15409] | 0.89 |
| `write_amp` | 1.17 [1.17–1.17] | 30.0 [29.9–30.1] | 0.04 |
| `commit_fsyncs_per_commit` | 1.00 [1.00–1.00] | — | — |

#### `point_deletes` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 370 [299–472] | 301 [266–475] | 1.23 |
| `delete_commit_p50_us` | 3992 [581–3998] | 3996 [223–3996] | 1.00 |
| `delete_commit_p99_us` | 8004 [7995–8012] | 8028 [8004–9593] | 1.00 |
| `get_deleted_p50_us` | 0.36 [0.36–0.36] | 0.83 [0.80–0.86] | 0.44 |
| `get_deleted_p99_us` | 0.72 [0.64–0.82] | 2.81 [2.48–3.44] | 0.26 |
| `get_live_p99_us` | 1.95 [1.83–2.15] | 3.17 [2.78–3.79] | 0.61 |

#### `append_seq` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 19.5 [16.2–28.6] | 25.2 [22.7–50.4] | 0.78 |
| `update_kops` | 25.0 [23.6–87.7] | 25.5 [21.9–29.1] | 0.98 |
| `update_commit100_p99_us` | 8086 [7954–11231] | 9921 [8921–21496] | 0.82 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.1 [10.9–11.2] | 8.27 [8.05–8.37] | 1.34 |

#### `mixed` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 8.32 [7.85–12.4] | 2.69 [2.68–2.99] | 3.09 |
| `read_p50_us` | 2.52 [1.89–2.57] | 11.6 [11.2–11.8] | 0.22 |
| `read_p99_us` | 10.4 [7.97–12.5] | 105 [97.1–125] | 0.10 |
| `write_p50_us` | 3987 [2441–4051] | 15663 [14729–15770] | 0.25 |
| `write_p99_us` | 12042 [12001–12993] | 32571 [31883–36307] | 0.37 |

#### `history_sim` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 14.5 [12.8–15.8] | 13.4 [11.6–14.4] | 1.08 |
| `rev_commit100_p99_us` | 16400 [11944–20070] | 23921 [15925–54428] | 0.69 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.18 [2.12–2.21] | 3.84 [3.76–3.89] | 0.57 |
| `history_scan_p99_us` | 5.03 [4.93–5.82] | 12.1 [11.6–15.0] | 0.42 |
| `reconstruct_p99_us` | 2.26 [1.95–2.70] | 4.86 [4.50–5.68] | 0.47 |

#### `livespec` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 205 [199–209] | 220 [213–226] | 0.93 |
| `changeset_commit_p50_us` | 3604 [3596–3645] | 3586 [3574–3590] | 1.01 |
| `changeset_commit_p99_us` | 11657 [11567–13042] | 14311 [14049–15622] | 0.81 |
| `symbol_get_p99_us` | 7.69 [7.17–9.70] | 11.5 [10.8–12.0] | 0.67 |
| `module_scan_p50_us` | 165 [164–173] | 110 [106–116] | 1.51 |
| `module_scan_p99_us` | 367 [353–534] | 216 [202–244] | 1.70 |

#### `cauce` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 276 [256–5195] | 285 [256–4240] | 0.97 |
| `step_commit_p50_us` | 3983 [156–3983] | 3922 [155–3922] | 1.02 |
| `step_commit_p99_us` | 7991 [448–7995] | 8139 [604–9191] | 0.98 |
| `step_read_p99_us` | 34.4 [30.5–36.5] | 111 [98.6–119] | 0.31 |
| `recovery_ms` | 107 [62.7–124] | 0.43 [0.37–0.47] | 249.92 |

#### `agent_memory` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 241 [175–2394] | 245 [238–1338] | 0.98 |
| `memory_write_p50_us` | 3889 [270–4078] | 3555 [315–3578] | 1.09 |
| `memory_write_p99_us` | 11469 [2163–11911] | 11330 [2587–11477] | 1.01 |
| `recall_p99_us` | 24.6 [19.2–26.0] | 68.6 [61.7–72.5] | 0.36 |
| `recent_scan_p99_us` | 40.0 [39.3–46.0] | 109 [105–118] | 0.37 |
| `expire_commit_p99_us` | 11919 [627–33440] | 11788 [4452–15565] | 1.01 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 2315 [1478–3824] | 290 [265–318] | 7.99 |
| `cc_commit_p50_us` | 3740 [572–4059] | 28017 [28000–30523] | 0.13 |
| `cc_commit_p99_us` | 10863 [8503–15917] | 68944 [60031–76087] | 0.16 |
| `cc_fsyncs_per_commit` | 0.23 [0.22–0.23] | — | — |

## 4. Resultados del par Buffered: kerno `Buffered` frente a SQLite WAL + `synchronous=NORMAL`

Misma escala y método. Recordatorio: la garantía ante corte de energía **no es idéntica** en este par (§1).

#### `bulk_load` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 288 [240–520] | 60.0 [50.5–69.4] | 4.79 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 61.9 [53.6–66.5] | 4.99 [4.26–10.2] | 12.40 |
| `reopen_ms` | 11.9 [8.00–12.0] | 0.38 [0.35–0.42] | 31.68 |
| `recovery_ms` | 35.9 [25.9–39.8] | 0.40 [0.40–0.47] | 89.30 |
| `peak_rss_mb` | 9.91 [9.75–9.93] | 11.6 [11.2–11.7] | 0.86 |

#### `point_reads` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 929 [800–1095] | 667 [597–770] | 1.39 |
| `get_1t_p50_us` | 0.53 [0.52–0.58] | 0.94 [0.91–0.94] | 0.57 |
| `get_1t_p95_us` | 0.72 [0.68–0.89] | 1.74 [1.68–1.84] | 0.41 |
| `get_1t_p99_us` | 0.92 [0.88–1.10] | 2.38 [2.21–2.62] | 0.38 |
| `get_4t_kops` | 1324 [1005–1516] | 651 [545–705] | 2.03 |
| `get_4t_p99_us` | 1.03 [0.95–1.30] | 4.23 [3.82–6.22] | 0.24 |

#### `range_scans` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 41.0 [40.0–42.9] | 32.3 [29.7–37.1] | 1.27 |
| `scan100_p50_us` | 14.5 [14.2–14.9] | 24.1 [23.7–24.8] | 0.60 |
| `scan100_p95_us` | 21.8 [17.2–24.7] | 34.6 [29.9–41.5] | 0.63 |
| `scan100_p99_us` | 39.1 [33.4–49.5] | 55.3 [45.1–63.3] | 0.71 |

#### `small_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 163754 [103114–234767] | 30547 [27705–33672] | 5.36 |
| `commit_p50_us` | 2.20 [2.09–3.21] | 7.42 [7.08–7.82] | 0.30 |
| `commit_p95_us` | 4.24 [3.93–5.72] | 11.3 [10.6–12.2] | 0.37 |
| `commit_p99_us` | 6.47 [5.79–13.0] | 25.5 [24.2–28.6] | 0.25 |
| `commit_p999_us` | 27.9 [24.2–70.7] | 12018 [10830–12550] | <0.01 |
| `write_amp` | 1.27 [1.27–1.27] | 29.7 [29.6–29.8] | 0.04 |
| `commit_fsyncs_per_commit` | 0.00 [0.00–0.00] | — | — |

#### `point_deletes` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 308953 [277862–513881] | 20770 [18128–21715] | 14.87 |
| `delete_commit_p50_us` | 1.68 [1.67–1.70] | 9.35 [9.13–12.8] | 0.18 |
| `delete_commit_p99_us` | 3.72 [3.63–4.04] | 52.4 [46.3–66.1] | 0.07 |
| `get_deleted_p50_us` | 0.37 [0.37–0.38] | 0.82 [0.80–1.36] | 0.45 |
| `get_deleted_p99_us` | 0.82 [0.78–0.88] | 2.60 [2.43–3.84] | 0.32 |
| `get_live_p99_us` | 2.19 [1.92–2.42] | 2.88 [2.83–4.42] | 0.76 |

#### `append_seq` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 873 [783–906] | 340 [179–464] | 2.56 |
| `update_kops` | 471 [318–662] | 196 [189–266] | 2.40 |
| `update_commit100_p99_us` | 4135 [127–6066] | 8192 [3383–12616] | 0.50 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.5 [11.3–11.6] | 8.80 [8.69–9.09] | 1.30 |

#### `mixed` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 644 [364–779] | 70.0 [66.1–77.7] | 9.20 |
| `read_p50_us` | 0.87 [0.85–1.23] | 8.60 [8.34–9.60] | 0.10 |
| `read_p99_us` | 2.40 [1.84–32.9] | 200 [39.6–300] | 0.01 |
| `write_p50_us` | 1.98 [1.94–3.76] | 8.72 [8.29–12.4] | 0.23 |
| `write_p99_us` | 9.10 [6.84–158] | 4108 [3789–4415] | <0.01 |

#### `history_sim` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 147 [112–296] | 39.3 [33.2–45.3] | 3.74 |
| `rev_commit100_p99_us` | 8479 [1699–12345] | 15860 [14090–24297] | 0.53 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.24 [2.21–2.29] | 3.82 [3.78–3.86] | 0.59 |
| `history_scan_p99_us` | 5.49 [4.19–6.00] | 11.5 [11.3–12.3] | 0.48 |
| `reconstruct_p99_us` | 2.01 [1.96–2.23] | 4.34 [4.23–4.79] | 0.46 |

#### `livespec` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 2216 [1941–2412] | 875 [794–895] | 2.53 |
| `changeset_commit_p50_us` | 44.0 [42.6–48.5] | 126 [120–138] | 0.35 |
| `changeset_commit_p99_us` | 184 [111–243] | 17875 [15778–19382] | 0.01 |
| `symbol_get_p99_us` | 4.92 [4.58–5.35] | 10.3 [9.13–11.8] | 0.48 |
| `module_scan_p50_us` | 86.8 [84.8–104] | 87.2 [84.7–95.0] | 0.99 |
| `module_scan_p99_us` | 479 [308–1621] | 236 [177–437] | 2.03 |

#### `cauce` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 70464 [49722–83416] | 8103 [7174–8181] | 8.70 |
| `step_commit_p50_us` | 3.14 [3.03–4.43] | 14.2 [11.8–15.6] | 0.22 |
| `step_commit_p99_us` | 8.90 [8.19–14.3] | 87.6 [73.7–128] | 0.10 |
| `step_read_p99_us` | 17.5 [14.6–27.4] | 72.0 [62.8–77.1] | 0.24 |
| `recovery_ms` | 118 [96.4–126] | 0.39 [0.37–4.50] | 301.43 |

#### `agent_memory` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 10496 [9820–11323] | 1483 [1382–1639] | 7.08 |
| `memory_write_p50_us` | 7.65 [7.46–8.38] | 46.4 [41.8–49.0] | 0.16 |
| `memory_write_p99_us` | 32.0 [31.4–45.0] | 8098 [6431–9503] | <0.01 |
| `recall_p99_us` | 15.6 [14.8–17.7] | 63.1 [51.6–68.3] | 0.25 |
| `recent_scan_p99_us` | 37.2 [33.2–44.7] | 117 [84.9–122] | 0.32 |
| `expire_commit_p99_us` | 558 [112–4102] | 15827 [14852–33784] | 0.04 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 287350 [221103–377416] | 21173 [17554–22022] | 13.57 |
| `cc_commit_p50_us` | 1.65 [1.64–2.44] | 11.3 [6.69–18.1] | 0.15 |
| `cc_commit_p99_us` | 111 [18.9–215] | 8544 [4719–11272] | 0.01 |

## 5. Qué se puede concluir y qué no

### 5.1 Observaciones (descriptivas, en esta máquina compartida y a escala `tiny`)

Cada punto se cita con las medianas de las tablas; todo punto está sujeto a los límites de §2 y §6.

**Casos en los que kerno es más lento o mayor que SQLite** (en ambos pares salvo indicación):

- **Reapertura y recuperación.** `reopen_ms` de `bulk_load`: kerno 12,0 ms frente a 0,41 ms (Sync) y 11,9 frente a 0,38 (Buffered). `recovery_ms` de `bulk_load`: 51,8 frente a 0,44 ms (Sync) y 35,9 frente a 0,40 (Buffered); en `cauce`, 107 frente a 0,43 ms (Sync) y 118 frente a 0,39 (Buffered). En estas cargas SQLite recupera en cientos de microsegundos y kerno en decenas de milisegundos.
- **Cierre tras `bulk_load`.** `close_ms`: kerno 59,1 frente a 12,6 ms (Sync) y 61,9 frente a 4,99 (Buffered).
- **`livespec`, lectura de módulo.** `module_scan_p50_us`: kerno 165 frente a 110 µs (Sync; en Buffered 86,8 frente a 87,2, sin diferencia apreciable). `module_scan_p99_us`: 367 frente a 216 µs (Sync) y 479 frente a 236 (Buffered); el máximo observado de kerno en Buffered es 1 621 µs, así que la cola está afectada por la carga.
- **`small_commits` en Sync.** `commits_per_s`: kerno 419 frente a 532, con rangos que se solapan ([257–899] y [363–652]); `commit_p99_us` 7 799 frente a 5 370 µs. Ambos dependen del `fsync` del momento (§2). `commit_p50_us` también es mayor en kerno (3 871 frente a 175 µs) por el efecto de dos regímenes descrito en §2, con rangos amplios en los dos motores.
- **`append_seq` en Sync.** `append_kops` 19,5 frente a 25,2 kops/s ([16,2–28,6] y [22,7–50,4]).
- **Memoria residente en `append_seq`.** `peak_rss_mb`: kerno 11,1 frente a 8,27 MB (Sync) y 11,5 frente a 8,80 (Buffered).
- **Otras cargas Sync con kerno ligeramente por debajo o igual dentro del ruido:** `livespec` `changesets_per_s` 205 frente a 220; `cauce` `steps_per_s` 276 frente a 285 y `step_commit_p50_us` 3 983 frente a 3 922 (rangos amplios y solapados en ambos).

**Casos en los que kerno es más rápido o menor que SQLite:**

- **Lecturas puntuales** (`point_reads`): `get_1t_kops` 1 130 frente a 705 (Sync) y 929 frente a 667 (Buffered); con 4 hilos, 2 533 frente a 1 296 y 1 324 frente a 651. Las latencias p99 de lectura también son menores en kerno (por ejemplo 0,98 frente a 2,74 µs a 1 hilo, Sync).
- **Escaneos de rango** (`range_scans`): `scan100_kops` 49,2 frente a 36,6 (Sync) y 41,0 frente a 32,3 (Buffered).
- **Escrituras sin `fsync` por commit (Buffered):** en `small_commits`, 163 754 frente a 30 547 commits/s; en `point_deletes`, 308 953 frente a 20 770; en `concurrent_commits`, 287 350 frente a 21 173 commits/s; en `mixed`, 644 frente a 70,0 kops/s; en `cauce`, 70 464 frente a 8 103 pasos/s. Las latencias de commit son menores en la misma proporción de orden de magnitud. Estas diferencias son grandes frente a la dispersión observada, pero **no deben leerse como una ventaja de garantías equivalentes**: ver el aviso del par Buffered en §1.
- **Amplificación de escritura:** `write_amp` de `bulk_load` 1,01 frente a 6,25; de `small_commits` 1,17 frente a 30,0 (Sync) y 1,27 frente a 29,7 (Buffered); de `append_seq` 1,02 frente a 2,35. Amplificación de espacio menor en kerno en `bulk_load`, `append_seq`, `history_sim` y `agent_memory`.
- **Lecturas en cargas mixtas y de agente:** `mixed` (Sync) `read_p99_us` 10,4 frente a 105 µs; `agent_memory` `recall_p99_us` 24,6 frente a 68,6 µs y `recent_scan_p99_us` 40,0 frente a 109 µs (Sync).
- **Escritores concurrentes en Sync** (`concurrent_commits`, 8 hilos): 2 315 frente a 290 commits/s, con 0,23 `fsync` por commit en kerno (SQLite no informa esta métrica). Esta carga se repitió completa en otra tanda con más carga de máquina (§6).
- **Escrituras de Sync dominadas por el `fsync`:** `delete_commit_p99_us`, `memory_write_p99_us`, `step_commit_p99_us` y `changeset_commit_p50_us` son prácticamente iguales en ambos motores (cociente entre 0,98 y 1,01), como cabe esperar si ambos están acotados por el mismo dispositivo.

### 5.2 Lo que **no** se puede concluir

- **Ninguna conclusión de cumplimiento.** No hay umbral contra el que decir que una cifra es buena o mala; las razones no son objetivos.
- **Nada sobre un corte de energía.** El par Buffered no iguala la garantía de durabilidad (§1) y este informe no mide pérdida de datos ni recuperación tras corte de energía en ninguno de los dos motores.
- **Nada sobre conjuntos de datos mayores que la memoria.** Con 5 000 registros todo cabe en caché; no se midió lectura en frío ni las escalas `small`, `medium` o `large`.
- **Nada preciso sobre latencias de cola.** Con la máquina cargada y 5 repeticiones, p99, p99.9 y máximos varían mucho entre repeticiones (véanse los rangos); las diferencias pequeñas, y las de las cargas acotadas por `fsync`, no son interpretables.
- **No se puede extrapolar a otras máquinas ni a una máquina en reposo.** Un dispositivo con `fdatasync` rápido cambiaría sobre todo el par Sync.
- **No es una comparación entre dos motores tuneados.** El adaptador de SQLite es el del binario existente (una conexión escritora y hasta cuatro lectoras, con 64 MiB de presupuesto de caché repartido entre ellas, sin ajustes adicionales); el de kerno usa la configuración por defecto del binario. Un ajuste específico de cada motor podría modificar las razones.
- **Cada razón es de medianas de 5 repeticiones.** No hay intervalo de confianza ni prueba de significación; cuando los rangos de los dos motores se solapan, la diferencia de medianas no distingue un motor del otro.

## 6. Límites y desviaciones de la ejecución

1. **Primera pasada Sync interrumpida por falta de espacio en disco.** La partición se llenó durante `concurrent_commits` de kerno (repetición 2): `progress-sync.log` registra `No space left on device (os error 28)` y la línea final de `results-sync.jsonl` quedó truncada (se conserva sin editar; `aggregate.py` la ignora). Ese archivo tiene 114 líneas válidas y la truncada, de las 130 ejecuciones previstas; faltan 16: 6 de `concurrent_commits` y las 10 de `rmw_counters`.
2. **`concurrent_commits` en Sync se repitió completa** (5 repeticiones por motor) en `results-sync-concurrent.jsonl`; las 2 repeticiones por motor de la primera pasada **no se usan** en las tablas, para que las 5 sean de la misma tanda. Se midieron más tarde y con más carga que el resto de la primera pasada, y no intercaladas con las demás cargas.
3. **`rmw_counters` no se tabula** (§1) y en Sync no se ejecutó.
4. **El comando de la primera pasada Sync es una reconstrucción** (§1.1).
5. **`env.json` se tomó una vez**, al inicio de la primera pasada; no se repitió en las otras tandas.
6. **Escala única (`tiny`) y 5 repeticiones**, por la limitación de CPU compartidos y por el espacio en disco (el disco estuvo cerca del límite durante toda la sesión). No se ejecutó lectura en frío.
7. **Ningún binario ni crate se modificó.** No hubo que cambiar `spike-bench` para ejecutar los pares de durabilidad.
8. **No se archivó salida de `cargo fmt`/`clippy`**: este paquete solo añade documentos y archivos de resultados.
