## Durability `sync`: kerno `Sync` against SQLite WAL + `synchronous=FULL` (the equivalent pair)

### `bulk_load` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 211 [81.5-341] | 26.5 [6.51-48.8] | 7.96 | better |
| `write_amp` | 1.76 [1.01-2.67] | 22.5 [12.1-32.9] | 0.08 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.49] | 0.70 | better |
| `close_ms` | 39.0 [20.5-174] | 3.49 [1.86-7.81] | 11.16 | worse |
| `reopen_ms` | 3.45 [2.25-9.37] | 0.39 [0.36-0.74] | 8.92 | worse |
| `recovery_ms` | 7.40 [5.62-14.0] | 0.40 [0.38-0.95] | 18.75 | worse |
| `peak_rss_mb` | 18.2 [13.6-26.0] | 27.9 [15.6-40.7] | 0.65 | tie |

### `point_reads` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 996 [528-1530] | 443 [175-763] | 2.25 | tie |
| `get_1t_p50_us` | 0.95 [0.53-1.62] | 2.50 [1.03-4.63] | 0.38 | tie |
| `get_1t_p95_us` | 1.51 [0.90-3.27] | 4.06 [2.06-11.6] | 0.37 | tie |
| `get_1t_p99_us` | 2.00 [1.13-4.40] | 5.26 [2.56-16.3] | 0.38 | tie |
| `get_4t_kops` | 3643 [2267-5223] | 1019 [409-1911] | 3.57 | better |
| `get_4t_p99_us` | 2.23 [1.31-4.21] | 18.6 [11.3-28.7] | 0.12 | better |

### `range_scans` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 39.3 [17.4-61.5] | 23.8 [17.7-32.0] | 1.65 | tie |
| `scan100_p50_us` | 24.8 [14.6-44.7] | 40.3 [30.6-50.4] | 0.61 | tie |
| `scan100_p95_us` | 35.8 [18.0-100] | 56.6 [37.5-81.9] | 0.63 | tie |
| `scan100_p99_us` | 45.5 [21.9-129] | 71.5 [44.6-164] | 0.64 | tie |

### `small_commits` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 915 [600-1088] | 805 [652-969] | 1.14 | tie |
| `commit_p50_us` | 907 [830-940] | 906 [833-1000] | 1.00 | tie |
| `commit_p95_us` | 1325 [1158-6889] | 1663 [1239-6447] | 0.80 | tie |
| `commit_p99_us` | 7719 [2177-8798] | 7889 [2193-8765] | 0.98 | tie |
| `commit_p999_us` | 15704 [6836-23003] | 22143 [14352-31212] | 0.71 | tie |
| `write_amp` | 1.17 [1.17-1.17] | 35.2 [33.0-37.4] | 0.03 | better |
| `commit_fsyncs_per_commit` | 1.00 [1.00-1.00] | n/a | n/a | n/a |

### `point_deletes` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 1027 [806-1258] | 889 [797-1022] | 1.15 | tie |
| `delete_commit_p50_us` | 816 [733-929] | 906 [848-980] | 0.90 | tie |
| `delete_commit_p99_us` | 4695 [1218-9699] | 7213 [1767-8552] | 0.65 | tie |
| `get_deleted_p50_us` | 0.46 [0.36-0.57] | 2.34 [0.83-3.71] | 0.20 | better |
| `get_deleted_p99_us` | 0.92 [0.57-2.58] | 4.67 [2.58-6.96] | 0.20 | tie |
| `get_live_p99_us` | 2.80 [1.68-7.89] | 4.92 [2.87-7.41] | 0.57 | tie |

### `append_seq` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 81.7 [48.1-110] | 71.9 [66.3-80.4] | 1.14 | tie |
| `update_kops` | 77.6 [53.6-96.9] | 69.3 [56.3-91.5] | 1.12 | tie |
| `update_commit100_p99_us` | 3494 [1754-7705] | 9138 [2037-12902] | 0.38 | tie |
| `write_amp` | 1.82 [1.35-2.29] | 3.04 [2.87-3.23] | 0.60 | better |
| `space_amp` | 1.63 [1.50-1.76] | 2.10 [1.41-2.80] | 0.78 | tie |
| `peak_rss_mb` | 18.0 [16.5-19.9] | 24.9 [10.0-39.7] | 0.72 | tie |

### `mixed` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 20.0 [16.9-22.5] | 8.43 [7.17-9.36] | 2.38 | better |
| `read_p50_us` | 3.23 [2.33-4.20] | 16.4 [12.7-19.2] | 0.20 | better |
| `read_p99_us` | 10.4 [7.23-14.0] | 108 [82.0-126] | 0.10 | better |
| `write_p50_us` | 1767 [1640-1895] | 3797 [3402-4145] | 0.47 | better |
| `write_p99_us` | 8077 [2933-13066] | 21742 [17514-24756] | 0.37 | better |

### `history_sim` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 64.6 [37.8-77.4] | 8.16 [5.29-17.8] | 7.92 | better |
| `rev_commit100_p99_us` | 5953 [1596-20070] | 48366 [11780-77398] | 0.12 | tie |
| `space_amp` | 3.99 [1.32-4.59] | 3.36 [1.94-4.79] | 1.19 | tie |
| `history_scan_p50_us` | 4.17 [2.10-10.6] | 5.54 [3.87-6.88] | 0.75 | tie |
| `history_scan_p99_us` | 12.7 [3.84-32.4] | 13.9 [9.55-23.5] | 0.92 | tie |
| `reconstruct_p99_us` | 2.47 [1.64-6.48] | 4.03 [3.12-8.25] | 0.61 | tie |

### `livespec` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 515 [441-659] | 309 [219-434] | 1.67 | better |
| `changeset_commit_p50_us` | 972 [885-1143] | 1325 [1269-1475] | 0.73 | better |
| `changeset_commit_p99_us` | 9789 [4260-11280] | 19907 [15131-36045] | 0.49 | better |
| `symbol_get_p99_us` | 15.6 [9.14-22.6] | 24.4 [14.7-37.7] | 0.64 | tie |
| `module_scan_p50_us` | 119 [104-145] | 129 [97.2-162] | 0.93 | tie |
| `module_scan_p99_us` | 317 [306-367] | 338 [247-436] | 0.94 | tie |

### `cauce` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 878 [688-1101] | 803 [613-840] | 1.09 | tie |
| `step_commit_p50_us` | 912 [782-950] | 966 [864-990] | 0.94 | tie |
| `step_commit_p99_us` | 7836 [2072-8495] | 8378 [5964-10379] | 0.94 | tie |
| `step_read_p99_us` | 50.4 [31.7-89.2] | 172 [160-182] | 0.29 | better |
| `recovery_ms` | 23.0 [15.6-50.1] | 0.39 [0.38-0.45] | 58.83 | worse |

### `agent_memory` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 779 [606-851] | 472 [425-526] | 1.65 | better |
| `memory_write_p50_us` | 920 [824-1013] | 1213 [1144-1284] | 0.76 | better |
| `memory_write_p99_us` | 8165 [3979-10428] | 8511 [6124-9765] | 0.96 | tie |
| `recall_p99_us` | 31.6 [23.8-42.8] | 94.7 [82.6-103] | 0.33 | better |
| `recent_scan_p99_us` | 51.8 [44.6-55.2] | 158 [150-177] | 0.33 | better |
| `expire_commit_p99_us` | 9208 [1923-16843] | 11543 [3594-15639] | 0.80 | tie |
| `space_amp` | 1.57 [1.18-2.84] | 2.94 [2.02-3.87] | 0.53 | tie |

### `concurrent_commits` (sync; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 4232 [3392-4573] | 836 [757-878] | 5.06 | better |
| `cc_commit_p50_us` | 1875 [1762-1919] | 8278 [7905-8651] | 0.23 | better |
| `cc_commit_p99_us` | 7621 [3633-11706] | 37356 [31392-42107] | 0.20 | better |
| `cc_fsyncs_per_commit` | 0.21 [0.21-0.22] | n/a | n/a | n/a |

## Durability `buffered`: kerno `Buffered` against SQLite WAL + `synchronous=NORMAL` (NOT equivalent guarantees)

### `bulk_load` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 447 [97.3-632] | 11.8 [4.41-39.2] | 37.87 | better |
| `write_amp` | 1.53 [1.01-2.23] | 22.5 [12.1-32.9] | 0.07 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.49] | 0.70 | better |
| `close_ms` | 146 [34.3-197] | 4.69 [2.60-11.4] | 31.07 | worse |
| `reopen_ms` | 3.42 [2.42-12.7] | 0.55 [0.35-1.02] | 6.19 | worse |
| `recovery_ms` | 10.2 [7.75-64.3] | 0.67 [0.38-0.92] | 15.22 | worse |
| `peak_rss_mb` | 21.5 [13.3-31.1] | 27.3 [15.0-40.4] | 0.79 | tie |

### `point_reads` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 1062 [750-1619] | 462 [236-761] | 2.30 | tie |
| `get_1t_p50_us` | 0.91 [0.50-1.21] | 2.42 [1.04-3.95] | 0.38 | tie |
| `get_1t_p95_us` | 1.39 [0.83-1.90] | 3.83 [2.10-6.10] | 0.36 | better |
| `get_1t_p99_us` | 1.81 [1.05-3.00] | 4.73 [2.65-8.15] | 0.38 | tie |
| `get_4t_kops` | 3778 [2808-5655] | 1094 [497-1790] | 3.45 | better |
| `get_4t_p99_us` | 2.04 [1.23-2.71] | 19.1 [11.7-25.2] | 0.11 | better |

### `range_scans` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 44.3 [23.4-63.9] | 25.6 [19.9-33.3] | 1.73 | tie |
| `scan100_p50_us` | 23.7 [13.9-36.4] | 39.1 [29.1-48.4] | 0.61 | tie |
| `scan100_p95_us` | 29.5 [17.0-56.2] | 48.8 [35.5-60.0] | 0.60 | tie |
| `scan100_p99_us` | 39.9 [20.5-72.0] | 66.4 [38.3-82.9] | 0.60 | tie |

### `small_commits` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 193183 [161331-220713] | 18714 [7121-31641] | 10.32 | better |
| `commit_p50_us` | 3.64 [3.53-4.15] | 13.9 [12.8-15.2] | 0.26 | better |
| `commit_p95_us` | 6.55 [6.32-7.63] | 23.5 [16.5-42.1] | 0.28 | better |
| `commit_p99_us` | 8.71 [7.71-11.6] | 39.2 [26.2-57.5] | 0.22 | better |
| `commit_p999_us` | 15.1 [12.2-23.9] | 19363 [1632-38273] | <0.01 | better |
| `write_amp` | 1.68 [1.37-3.68] | 35.3 [32.8-37.7] | 0.05 | better |
| `commit_fsyncs_per_commit` | 0.00 [0.00-0.00] | n/a | n/a | n/a |

### `point_deletes` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 288869 [275543-291295] | 10433 [4893-15844] | 27.69 | better |
| `delete_commit_p50_us` | 3.16 [3.13-3.26] | 16.4 [15.1-18.1] | 0.19 | better |
| `delete_commit_p99_us` | 6.27 [6.14-6.55] | 119 [81.6-254] | 0.05 | better |
| `get_deleted_p50_us` | 0.43 [0.37-0.53] | 2.23 [0.84-3.72] | 0.19 | better |
| `get_deleted_p99_us` | 0.79 [0.53-1.26] | 4.73 [2.44-8.96] | 0.17 | better |
| `get_live_p99_us` | 2.69 [1.58-4.16] | 5.11 [2.92-9.43] | 0.53 | tie |

### `append_seq` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 789 [302-1003] | 155 [83.2-256] | 5.10 | better |
| `update_kops` | 604 [281-650] | 147 [96.7-300] | 4.11 | tie |
| `update_commit100_p99_us` | 168 [104-346] | 8281 [1399-58589] | 0.02 | better |
| `write_amp` | 1.58 [1.08-2.12] | 3.04 [2.87-3.23] | 0.52 | better |
| `space_amp` | 3.46 [3.25-3.81] | 2.10 [1.41-2.80] | 1.65 | worse |
| `peak_rss_mb` | 23.1 [17.3-31.5] | 25.1 [10.2-40.2] | 0.92 | tie |

### `mixed` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 1102 [1031-1479] | 104 [53.2-134] | 10.59 | better |
| `read_p50_us` | 2.06 [1.33-2.60] | 16.7 [14.1-21.4] | 0.12 | better |
| `read_p99_us` | 4.62 [3.53-6.23] | 74.0 [56.0-111] | 0.06 | better |
| `write_p50_us` | 10.5 [9.58-13.9] | 24.3 [22.0-33.8] | 0.43 | better |
| `write_p99_us` | 33.4 [28.8-46.6] | 203 [136-361] | 0.16 | better |

### `history_sim` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 277 [114-361] | 9.84 [5.81-27.0] | 28.18 | better |
| `rev_commit100_p99_us` | 709 [318-1238] | 63373 [16572-117375] | 0.01 | better |
| `space_amp` | 3.66 [2.67-4.59] | 3.36 [1.94-4.79] | 1.09 | tie |
| `history_scan_p50_us` | 6.96 [2.03-14.8] | 5.05 [3.82-6.50] | 1.38 | tie |
| `history_scan_p99_us` | 17.5 [3.62-44.6] | 14.1 [9.50-29.9] | 1.24 | tie |
| `reconstruct_p99_us` | 3.80 [1.65-8.68] | 3.40 [3.12-9.21] | 1.12 | tie |

### `livespec` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 1867 [1740-2076] | 404 [344-610] | 4.62 | better |
| `changeset_commit_p50_us` | 44.3 [43.2-51.7] | 308 [231-391] | 0.14 | better |
| `changeset_commit_p99_us` | 126 [109-155] | 70648 [25592-91554] | <0.01 | better |
| `symbol_get_p99_us` | 10.3 [6.63-16.5] | 18.6 [12.8-30.1] | 0.56 | tie |
| `module_scan_p50_us` | 110 [91.6-120] | 118 [80.8-162] | 0.93 | tie |
| `module_scan_p99_us` | 289 [252-334] | 273 [245-404] | 1.06 | tie |

### `cauce` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 46144 [37755-79748] | 4165 [2362-5116] | 11.08 | better |
| `step_commit_p50_us` | 5.46 [4.69-6.24] | 26.8 [22.4-56.1] | 0.20 | better |
| `step_commit_p99_us` | 12.2 [9.72-20.8] | 257 [180-368] | 0.05 | better |
| `step_read_p99_us` | 25.6 [8.43-37.1] | 73.3 [69.4-94.7] | 0.35 | better |
| `recovery_ms` | 50.6 [19.0-60.7] | 0.40 [0.36-0.82] | 128.20 | worse |

### `agent_memory` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 5485 [3254-12034] | 1172 [752-1371] | 4.68 | better |
| `memory_write_p50_us` | 13.3 [12.1-14.2] | 81.9 [77.6-89.0] | 0.16 | better |
| `memory_write_p99_us` | 55.8 [25.6-1526] | 7543 [1535-13394] | 0.01 | better |
| `recall_p99_us` | 22.4 [11.9-38.2] | 78.9 [46.1-92.5] | 0.28 | better |
| `recent_scan_p99_us` | 31.2 [19.6-51.2] | 137 [81.2-150] | 0.23 | better |
| `expire_commit_p99_us` | 825 [123-109904] | 7772 [2611-13591] | 0.11 | tie |
| `space_amp` | 2.62 [2.21-2.87] | 2.94 [2.02-3.87] | 0.89 | tie |

### `concurrent_commits` (buffered; repetitions: kerno 10, sqlite 10)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 267716 [205688-296865] | 9486 [7966-14111] | 28.22 | better |
| `cc_commit_p50_us` | 26.2 [23.8-29.6] | 37.0 [32.9-41.6] | 0.71 | better |
| `cc_commit_p99_us` | 72.1 [62.0-94.4] | 19530 [14344-31932] | <0.01 | better |

## Summary

| Pair | kerno better | not distinguishable (ranges overlap) | kerno worse |
|---|---|---|---|
| `sync` | 25 | 38 | 4 |
| `buffered` | 42 | 20 | 5 |

### Every metric where kerno is worse and the ranges do not overlap

| Pair | Workload | Metric | kerno | sqlite | kerno / sqlite |
|---|---|---|---|---|---|
| sync | bulk_load | `close_ms` | 39.0 [20.5-174] | 3.49 [1.86-7.81] | 11.16 |
| sync | bulk_load | `reopen_ms` | 3.45 [2.25-9.37] | 0.39 [0.36-0.74] | 8.92 |
| sync | bulk_load | `recovery_ms` | 7.40 [5.62-14.0] | 0.40 [0.38-0.95] | 18.75 |
| sync | cauce | `recovery_ms` | 23.0 [15.6-50.1] | 0.39 [0.38-0.45] | 58.83 |
| buffered | bulk_load | `close_ms` | 146 [34.3-197] | 4.69 [2.60-11.4] | 31.07 |
| buffered | bulk_load | `reopen_ms` | 3.42 [2.42-12.7] | 0.55 [0.35-1.02] | 6.19 |
| buffered | bulk_load | `recovery_ms` | 10.2 [7.75-64.3] | 0.67 [0.38-0.92] | 15.22 |
| buffered | append_seq | `space_amp` | 3.46 [3.25-3.81] | 2.10 [1.41-2.80] | 1.65 |
| buffered | cauce | `recovery_ms` | 50.6 [19.0-60.7] | 0.40 [0.36-0.82] | 128.20 |
