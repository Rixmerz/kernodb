## Durabilidad `sync`: kerno `Sync` frente a SQLite WAL + `synchronous=FULL`

### `bulk_load` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 123 [112–137] | 54.1 [53.4–59.8] | 2.26 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 59.1 [55.2–59.4] | 12.6 [10.1–15.5] | 4.68 |
| `reopen_ms` | 12.0 [11.9–16.0] | 0.41 [0.37–0.49] | 29.45 |
| `recovery_ms` | 51.8 [31.7–59.9] | 0.44 [0.43–0.51] | 116.65 |
| `peak_rss_mb` | 9.62 [9.39–9.63] | 10.8 [10.7–10.9] | 0.89 |

### `point_reads` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 1130 [1077–1536] | 705 [586–811] | 1.60 |
| `get_1t_p50_us` | 0.54 [0.53–0.55] | 0.95 [0.90–1.04] | 0.57 |
| `get_1t_p95_us` | 0.74 [0.68–0.81] | 1.90 [1.65–2.16] | 0.39 |
| `get_1t_p99_us` | 0.98 [0.87–1.08] | 2.74 [2.19–2.92] | 0.36 |
| `get_4t_kops` | 2533 [2417–2600] | 1296 [1187–1425] | 1.95 |
| `get_4t_p99_us` | 1.10 [1.01–1.12] | 4.36 [3.94–4.94] | 0.25 |

### `range_scans` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 49.2 [46.1–53.4] | 36.6 [35.5–37.7] | 1.34 |
| `scan100_p50_us` | 14.3 [14.0–14.3] | 23.6 [23.4–24.0] | 0.60 |
| `scan100_p95_us` | 21.6 [16.0–24.0] | 32.2 [30.8–36.1] | 0.67 |
| `scan100_p99_us` | 34.8 [32.9–43.6] | 48.4 [47.4–52.3] | 0.72 |

### `small_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 419 [257–899] | 532 [363–652] | 0.79 |
| `commit_p50_us` | 3871 [116–3998] | 175 [113–3990] | 22.06 |
| `commit_p95_us` | 4018 [4004–7614] | 4008 [4006–4678] | 1.00 |
| `commit_p99_us` | 7799 [4817–9773] | 5370 [4940–8004] | 1.45 |
| `commit_p999_us` | 9953 [7999–19988] | 11182 [8020–15409] | 0.89 |
| `write_amp` | 1.17 [1.17–1.17] | 30.0 [29.9–30.1] | 0.04 |
| `commit_fsyncs_per_commit` | 1.00 [1.00–1.00] | — | — |

### `point_deletes` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 370 [299–472] | 301 [266–475] | 1.23 |
| `delete_commit_p50_us` | 3992 [581–3998] | 3996 [223–3996] | 1.00 |
| `delete_commit_p99_us` | 8004 [7995–8012] | 8028 [8004–9593] | 1.00 |
| `get_deleted_p50_us` | 0.36 [0.36–0.36] | 0.83 [0.80–0.86] | 0.44 |
| `get_deleted_p99_us` | 0.72 [0.64–0.82] | 2.81 [2.48–3.44] | 0.26 |
| `get_live_p99_us` | 1.95 [1.83–2.15] | 3.17 [2.78–3.79] | 0.61 |

### `append_seq` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 19.5 [16.2–28.6] | 25.2 [22.7–50.4] | 0.78 |
| `update_kops` | 25.0 [23.6–87.7] | 25.5 [21.9–29.1] | 0.98 |
| `update_commit100_p99_us` | 8086 [7954–11231] | 9921 [8921–21496] | 0.82 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.1 [10.9–11.2] | 8.27 [8.05–8.37] | 1.34 |

### `mixed` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 8.32 [7.85–12.4] | 2.69 [2.68–2.99] | 3.09 |
| `read_p50_us` | 2.52 [1.89–2.57] | 11.6 [11.2–11.8] | 0.22 |
| `read_p99_us` | 10.4 [7.97–12.5] | 105 [97.1–125] | 0.10 |
| `write_p50_us` | 3987 [2441–4051] | 15663 [14729–15770] | 0.25 |
| `write_p99_us` | 12042 [12001–12993] | 32571 [31883–36307] | 0.37 |

### `history_sim` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 14.5 [12.8–15.8] | 13.4 [11.6–14.4] | 1.08 |
| `rev_commit100_p99_us` | 16400 [11944–20070] | 23921 [15925–54428] | 0.69 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.18 [2.12–2.21] | 3.84 [3.76–3.89] | 0.57 |
| `history_scan_p99_us` | 5.03 [4.93–5.82] | 12.1 [11.6–15.0] | 0.42 |
| `reconstruct_p99_us` | 2.26 [1.95–2.70] | 4.86 [4.50–5.68] | 0.47 |

### `livespec` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 205 [199–209] | 220 [213–226] | 0.93 |
| `changeset_commit_p50_us` | 3604 [3596–3645] | 3586 [3574–3590] | 1.01 |
| `changeset_commit_p99_us` | 11657 [11567–13042] | 14311 [14049–15622] | 0.81 |
| `symbol_get_p99_us` | 7.69 [7.17–9.70] | 11.5 [10.8–12.0] | 0.67 |
| `module_scan_p50_us` | 165 [164–173] | 110 [106–116] | 1.51 |
| `module_scan_p99_us` | 367 [353–534] | 216 [202–244] | 1.70 |

### `cauce` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 276 [256–5195] | 285 [256–4240] | 0.97 |
| `step_commit_p50_us` | 3983 [156–3983] | 3922 [155–3922] | 1.02 |
| `step_commit_p99_us` | 7991 [448–7995] | 8139 [604–9191] | 0.98 |
| `step_read_p99_us` | 34.4 [30.5–36.5] | 111 [98.6–119] | 0.31 |
| `recovery_ms` | 107 [62.7–124] | 0.43 [0.37–0.47] | 249.92 |

### `agent_memory` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 241 [175–2394] | 245 [238–1338] | 0.98 |
| `memory_write_p50_us` | 3889 [270–4078] | 3555 [315–3578] | 1.09 |
| `memory_write_p99_us` | 11469 [2163–11911] | 11330 [2587–11477] | 1.01 |
| `recall_p99_us` | 24.6 [19.2–26.0] | 68.6 [61.7–72.5] | 0.36 |
| `recent_scan_p99_us` | 40.0 [39.3–46.0] | 109 [105–118] | 0.37 |
| `expire_commit_p99_us` | 11919 [627–33440] | 11788 [4452–15565] | 1.01 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

### `concurrent_commits` (sync; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 2315 [1478–3824] | 290 [265–318] | 7.99 |
| `cc_commit_p50_us` | 3740 [572–4059] | 28017 [28000–30523] | 0.13 |
| `cc_commit_p99_us` | 10863 [8503–15917] | 68944 [60031–76087] | 0.16 |
| `cc_fsyncs_per_commit` | 0.23 [0.22–0.23] | — | — |

## Durabilidad `buffered`: kerno `Buffered` frente a SQLite WAL + `synchronous=NORMAL`

### `bulk_load` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `load_kops` | 288 [240–520] | 60.0 [50.5–69.4] | 4.79 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 61.9 [53.6–66.5] | 4.99 [4.26–10.2] | 12.40 |
| `reopen_ms` | 11.9 [8.00–12.0] | 0.38 [0.35–0.42] | 31.68 |
| `recovery_ms` | 35.9 [25.9–39.8] | 0.40 [0.40–0.47] | 89.30 |
| `peak_rss_mb` | 9.91 [9.75–9.93] | 11.6 [11.2–11.7] | 0.86 |

### `point_reads` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `get_1t_kops` | 929 [800–1095] | 667 [597–770] | 1.39 |
| `get_1t_p50_us` | 0.53 [0.52–0.58] | 0.94 [0.91–0.94] | 0.57 |
| `get_1t_p95_us` | 0.72 [0.68–0.89] | 1.74 [1.68–1.84] | 0.41 |
| `get_1t_p99_us` | 0.92 [0.88–1.10] | 2.38 [2.21–2.62] | 0.38 |
| `get_4t_kops` | 1324 [1005–1516] | 651 [545–705] | 2.03 |
| `get_4t_p99_us` | 1.03 [0.95–1.30] | 4.23 [3.82–6.22] | 0.24 |

### `range_scans` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `scan100_kops` | 41.0 [40.0–42.9] | 32.3 [29.7–37.1] | 1.27 |
| `scan100_p50_us` | 14.5 [14.2–14.9] | 24.1 [23.7–24.8] | 0.60 |
| `scan100_p95_us` | 21.8 [17.2–24.7] | 34.6 [29.9–41.5] | 0.63 |
| `scan100_p99_us` | 39.1 [33.4–49.5] | 55.3 [45.1–63.3] | 0.71 |

### `small_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `commits_per_s` | 163754 [103114–234767] | 30547 [27705–33672] | 5.36 |
| `commit_p50_us` | 2.20 [2.09–3.21] | 7.42 [7.08–7.82] | 0.30 |
| `commit_p95_us` | 4.24 [3.93–5.72] | 11.3 [10.6–12.2] | 0.37 |
| `commit_p99_us` | 6.47 [5.79–13.0] | 25.5 [24.2–28.6] | 0.25 |
| `commit_p999_us` | 27.9 [24.2–70.7] | 12018 [10830–12550] | <0.01 |
| `write_amp` | 1.27 [1.27–1.27] | 29.7 [29.6–29.8] | 0.04 |
| `commit_fsyncs_per_commit` | 0.00 [0.00–0.00] | — | — |

### `point_deletes` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `delete_commits_per_s` | 308953 [277862–513881] | 20770 [18128–21715] | 14.87 |
| `delete_commit_p50_us` | 1.68 [1.67–1.70] | 9.35 [9.13–12.8] | 0.18 |
| `delete_commit_p99_us` | 3.72 [3.63–4.04] | 52.4 [46.3–66.1] | 0.07 |
| `get_deleted_p50_us` | 0.37 [0.37–0.38] | 0.82 [0.80–1.36] | 0.45 |
| `get_deleted_p99_us` | 0.82 [0.78–0.88] | 2.60 [2.43–3.84] | 0.32 |
| `get_live_p99_us` | 2.19 [1.92–2.42] | 2.88 [2.83–4.42] | 0.76 |

### `append_seq` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `append_kops` | 873 [783–906] | 340 [179–464] | 2.56 |
| `update_kops` | 471 [318–662] | 196 [189–266] | 2.40 |
| `update_commit100_p99_us` | 4135 [127–6066] | 8192 [3383–12616] | 0.50 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.5 [11.3–11.6] | 8.80 [8.69–9.09] | 1.30 |

### `mixed` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `mixed_kops` | 644 [364–779] | 70.0 [66.1–77.7] | 9.20 |
| `read_p50_us` | 0.87 [0.85–1.23] | 8.60 [8.34–9.60] | 0.10 |
| `read_p99_us` | 2.40 [1.84–32.9] | 200 [39.6–300] | 0.01 |
| `write_p50_us` | 1.98 [1.94–3.76] | 8.72 [8.29–12.4] | 0.23 |
| `write_p99_us` | 9.10 [6.84–158] | 4108 [3789–4415] | <0.01 |

### `history_sim` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `revisions_per_s_k` | 147 [112–296] | 39.3 [33.2–45.3] | 3.74 |
| `rev_commit100_p99_us` | 8479 [1699–12345] | 15860 [14090–24297] | 0.53 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.24 [2.21–2.29] | 3.82 [3.78–3.86] | 0.59 |
| `history_scan_p99_us` | 5.49 [4.19–6.00] | 11.5 [11.3–12.3] | 0.48 |
| `reconstruct_p99_us` | 2.01 [1.96–2.23] | 4.34 [4.23–4.79] | 0.46 |

### `livespec` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `changesets_per_s` | 2216 [1941–2412] | 875 [794–895] | 2.53 |
| `changeset_commit_p50_us` | 44.0 [42.6–48.5] | 126 [120–138] | 0.35 |
| `changeset_commit_p99_us` | 184 [111–243] | 17875 [15778–19382] | 0.01 |
| `symbol_get_p99_us` | 4.92 [4.58–5.35] | 10.3 [9.13–11.8] | 0.48 |
| `module_scan_p50_us` | 86.8 [84.8–104] | 87.2 [84.7–95.0] | 0.99 |
| `module_scan_p99_us` | 479 [308–1621] | 236 [177–437] | 2.03 |

### `cauce` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 70464 [49722–83416] | 8103 [7174–8181] | 8.70 |
| `step_commit_p50_us` | 3.14 [3.03–4.43] | 14.2 [11.8–15.6] | 0.22 |
| `step_commit_p99_us` | 8.90 [8.19–14.3] | 87.6 [73.7–128] | 0.10 |
| `step_read_p99_us` | 17.5 [14.6–27.4] | 72.0 [62.8–77.1] | 0.24 |
| `recovery_ms` | 118 [96.4–126] | 0.39 [0.37–4.50] | 301.43 |

### `agent_memory` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `steps_per_s` | 10496 [9820–11323] | 1483 [1382–1639] | 7.08 |
| `memory_write_p50_us` | 7.65 [7.46–8.38] | 46.4 [41.8–49.0] | 0.16 |
| `memory_write_p99_us` | 32.0 [31.4–45.0] | 8098 [6431–9503] | <0.01 |
| `recall_p99_us` | 15.6 [14.8–17.7] | 63.1 [51.6–68.3] | 0.25 |
| `recent_scan_p99_us` | 37.2 [33.2–44.7] | 117 [84.9–122] | 0.32 |
| `expire_commit_p99_us` | 558 [112–4102] | 15827 [14852–33784] | 0.04 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

### `concurrent_commits` (buffered; repeticiones: kerno 5, sqlite 5)

| Métrica | kerno mediana [mín–máx] | sqlite mediana [mín–máx] | kerno / sqlite (medianas) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 287350 [221103–377416] | 21173 [17554–22022] | 13.57 |
| `cc_commit_p50_us` | 1.65 [1.64–2.44] | 11.3 [6.69–18.1] | 0.15 |
| `cc_commit_p99_us` | 111 [18.9–215] | 8544 [4719–11272] | 0.01 |

