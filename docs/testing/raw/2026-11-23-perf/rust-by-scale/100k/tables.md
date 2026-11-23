## Durability `sync`: kerno `Sync` against SQLite WAL + `synchronous=FULL` (the equivalent pair)

### `bulk_load` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 183 [81.5-233] | 7.23 [6.51-8.50] | 25.35 | better |
| `write_amp` | 2.66 [2.51-2.67] | 32.9 [32.7-32.9] | 0.08 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.48] | 0.70 | better |
| `close_ms` | 46.0 [37.0-174] | 4.95 [4.44-7.81] | 9.29 | worse |
| `reopen_ms` | 3.61 [3.33-4.42] | 0.44 [0.39-0.74] | 8.23 | worse |
| `recovery_ms` | 9.04 [7.40-14.0] | 0.46 [0.39-0.95] | 19.56 | worse |
| `peak_rss_mb` | 24.9 [21.9-26.0] | 40.0 [39.8-40.7] | 0.62 | better |

### `point_reads` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 636 [528-786] | 222 [175-251] | 2.87 | better |
| `get_1t_p50_us` | 1.43 [1.18-1.62] | 4.13 [3.76-4.63] | 0.35 | better |
| `get_1t_p95_us` | 2.27 [1.73-3.27] | 6.74 [5.58-11.6] | 0.34 | better |
| `get_1t_p99_us` | 3.24 [2.38-4.40] | 10.9 [7.04-16.3] | 0.30 | better |
| `get_4t_kops` | 2436 [2267-2712] | 486 [409-542] | 5.01 | better |
| `get_4t_p99_us` | 3.35 [2.85-4.21] | 24.9 [23.7-28.7] | 0.13 | better |

### `range_scans` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 20.8 [17.4-28.1] | 18.0 [17.7-20.5] | 1.16 | tie |
| `scan100_p50_us` | 36.5 [33.0-44.7] | 49.5 [47.0-50.4] | 0.74 | better |
| `scan100_p95_us` | 77.6 [40.1-100] | 79.2 [56.4-81.9] | 0.98 | tie |
| `scan100_p99_us` | 111 [46.5-129] | 107 [69.9-164] | 1.04 | tie |

### `small_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 907 [600-1007] | 905 [753-969] | 1.00 | tie |
| `commit_p50_us` | 882 [849-940] | 858 [833-878] | 1.03 | tie |
| `commit_p95_us` | 1378 [1158-6889] | 1291 [1239-4432] | 1.07 | tie |
| `commit_p99_us` | 7762 [6328-8798] | 8004 [7139-8765] | 0.97 | tie |
| `commit_p999_us` | 16876 [14729-23003] | 25510 [25051-31212] | 0.66 | better |
| `write_amp` | 1.17 [1.17-1.17] | 37.2 [37.2-37.4] | 0.03 | better |
| `commit_fsyncs_per_commit` | 1.00 [1.00-1.00] | n/a | n/a | n/a |

### `point_deletes` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 1146 [960-1258] | 913 [858-1022] | 1.25 | tie |
| `delete_commit_p50_us` | 748 [733-929] | 854 [848-868] | 0.88 | tie |
| `delete_commit_p99_us` | 2051 [1218-6955] | 7401 [1767-8552] | 0.28 | tie |
| `get_deleted_p50_us` | 0.55 [0.52-0.57] | 3.62 [3.56-3.71] | 0.15 | better |
| `get_deleted_p99_us` | 1.18 [0.98-2.58] | 6.44 [6.25-6.96] | 0.18 | better |
| `get_live_p99_us` | 3.74 [3.31-7.89] | 6.82 [6.38-7.41] | 0.55 | tie |

### `append_seq` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 77.3 [48.1-84.0] | 68.6 [66.3-69.8] | 1.13 | tie |
| `update_kops` | 77.0 [69.8-78.2] | 69.0 [56.3-69.4] | 1.12 | better |
| `update_commit100_p99_us` | 5648 [3705-7705] | 10273 [9060-10813] | 0.55 | better |
| `write_amp` | 2.29 [2.29-2.29] | 3.22 [3.18-3.23] | 0.71 | better |
| `space_amp` | 1.76 [1.75-1.76] | 1.42 [1.41-1.42] | 1.24 | worse |
| `peak_rss_mb` | 19.7 [19.2-19.9] | 39.4 [39.0-39.7] | 0.50 | better |

### `mixed` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 19.2 [16.9-21.7] | 8.56 [7.97-9.36] | 2.24 | better |
| `read_p50_us` | 4.03 [3.94-4.20] | 18.8 [18.2-19.2] | 0.21 | better |
| `read_p99_us` | 12.2 [11.7-14.0] | 122 [98.9-126] | 0.10 | better |
| `write_p50_us` | 1755 [1663-1895] | 3740 [3402-3891] | 0.47 | better |
| `write_p99_us` | 9642 [2933-13066] | 21053 [17514-24756] | 0.46 | better |

### `history_sim` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 49.6 [37.8-61.8] | 6.09 [5.29-7.37] | 8.13 | better |
| `rev_commit100_p99_us` | 11370 [9699-20070] | 65470 [49349-77398] | 0.17 | better |
| `space_amp` | 1.32 [1.32-3.39] | 1.94 [1.94-1.94] | 0.68 | tie |
| `history_scan_p50_us` | 4.73 [4.57-10.6] | 6.26 [6.02-6.88] | 0.76 | tie |
| `history_scan_p99_us` | 13.3 [13.1-32.4] | 17.3 [13.8-23.5] | 0.77 | tie |
| `reconstruct_p99_us` | 2.77 [2.71-6.48] | 7.04 [3.37-8.25] | 0.39 | tie |

### `livespec` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 504 [503-548] | 268 [219-297] | 1.88 | better |
| `changeset_commit_p50_us` | 912 [885-940] | 1357 [1299-1475] | 0.67 | better |
| `changeset_commit_p99_us` | 10895 [7762-11280] | 25657 [19022-36045] | 0.42 | better |
| `symbol_get_p99_us` | 21.2 [18.6-22.6] | 32.5 [31.2-37.7] | 0.65 | better |
| `module_scan_p50_us` | 111 [104-116] | 157 [154-162] | 0.71 | better |
| `module_scan_p99_us` | 356 [306-367] | 422 [403-436] | 0.84 | better |

### `cauce` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 915 [701-1101] | 798 [646-829] | 1.15 | tie |
| `step_commit_p50_us` | 865 [782-935] | 896 [864-970] | 0.97 | tie |
| `step_commit_p99_us` | 7578 [2072-8008] | 8430 [8294-10371] | 0.90 | better |
| `step_read_p99_us` | 76.0 [64.4-89.2] | 164 [160-182] | 0.46 | better |
| `recovery_ms` | 28.6 [25.5-50.1] | 0.39 [0.38-0.41] | 72.93 | worse |

### `agent_memory` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 786 [706-851] | 456 [425-494] | 1.72 | better |
| `memory_write_p50_us` | 845 [824-892] | 1183 [1144-1215] | 0.71 | better |
| `memory_write_p99_us` | 8286 [7483-10060] | 9544 [8352-9765] | 0.87 | tie |
| `recall_p99_us` | 37.6 [33.1-42.8] | 97.3 [94.4-103] | 0.39 | better |
| `recent_scan_p99_us` | 52.3 [45.5-54.1] | 158 [156-160] | 0.33 | better |
| `expire_commit_p99_us` | 11379 [2550-16843] | 14918 [13582-15639] | 0.76 | tie |
| `space_amp` | 2.83 [1.95-2.84] | 3.87 [3.86-3.87] | 0.73 | better |

### `concurrent_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 4077 [3392-4417] | 840 [778-878] | 4.85 | better |
| `cc_commit_p50_us` | 1872 [1762-1919] | 8102 [7905-8495] | 0.23 | better |
| `cc_commit_p99_us` | 9560 [4485-11706] | 35127 [31392-42041] | 0.27 | better |
| `cc_fsyncs_per_commit` | 0.22 [0.21-0.22] | n/a | n/a | n/a |

## Durability `buffered`: kerno `Buffered` against SQLite WAL + `synchronous=NORMAL` (NOT equivalent guarantees)

### `bulk_load` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 182 [97.3-300] | 4.54 [4.41-4.97] | 40.11 | better |
| `write_amp` | 2.21 [2.06-2.23] | 32.9 [32.7-32.9] | 0.07 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.48] | 0.70 | better |
| `close_ms` | 187 [185-197] | 9.94 [4.84-11.4] | 18.78 | worse |
| `reopen_ms` | 3.45 [3.39-3.95] | 0.92 [0.41-1.02] | 3.73 | worse |
| `recovery_ms` | 9.85 [8.30-12.8] | 0.90 [0.38-0.92] | 10.90 | worse |
| `peak_rss_mb` | 29.7 [29.4-31.1] | 39.8 [39.3-40.4] | 0.75 | better |

### `point_reads` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 778 [750-785] | 256 [236-260] | 3.04 | better |
| `get_1t_p50_us` | 1.20 [1.19-1.21] | 3.69 [3.63-3.95] | 0.33 | better |
| `get_1t_p95_us` | 1.73 [1.70-1.90] | 5.37 [5.28-6.10] | 0.32 | better |
| `get_1t_p99_us` | 2.31 [2.29-3.00] | 6.94 [6.40-8.15] | 0.33 | better |
| `get_4t_kops` | 2943 [2808-2980] | 553 [497-572] | 5.32 | better |
| `get_4t_p99_us` | 2.56 [2.40-2.71] | 21.5 [18.8-25.2] | 0.12 | better |

### `range_scans` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 26.9 [23.4-28.1] | 19.9 [19.9-20.4] | 1.35 | better |
| `scan100_p50_us` | 33.7 [32.8-36.4] | 47.9 [47.3-48.4] | 0.70 | better |
| `scan100_p95_us` | 43.9 [40.4-56.2] | 59.7 [56.2-60.0] | 0.73 | tie |
| `scan100_p99_us` | 61.6 [51.0-72.0] | 75.9 [68.7-82.9] | 0.81 | tie |

### `small_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 173127 [161331-187354] | 8007 [7121-12079] | 21.62 | better |
| `commit_p50_us` | 3.71 [3.60-4.15] | 15.1 [14.9-15.2] | 0.24 | better |
| `commit_p95_us` | 6.64 [6.54-7.63] | 41.2 [29.7-42.1] | 0.16 | better |
| `commit_p99_us` | 9.20 [8.72-11.6] | 51.8 [43.2-57.5] | 0.18 | better |
| `commit_p999_us` | 17.8 [15.8-23.9] | 37454 [36340-38273] | <0.01 | better |
| `write_amp` | 2.12 [1.99-3.68] | 37.7 [37.7-37.7] | 0.06 | better |
| `commit_fsyncs_per_commit` | 0.00 [0.00-0.00] | n/a | n/a | n/a |

### `point_deletes` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 286781 [275543-289996] | 7707 [4893-8881] | 37.21 | better |
| `delete_commit_p50_us` | 3.19 [3.16-3.26] | 17.6 [16.5-18.1] | 0.18 | better |
| `delete_commit_p99_us` | 6.27 [6.14-6.55] | 162 [132-254] | 0.04 | better |
| `get_deleted_p50_us` | 0.51 [0.43-0.53] | 3.65 [3.56-3.72] | 0.14 | better |
| `get_deleted_p99_us` | 0.95 [0.82-1.26] | 7.00 [6.62-8.96] | 0.14 | better |
| `get_live_p99_us` | 3.37 [3.13-4.16] | 7.17 [7.00-9.43] | 0.47 | better |

### `append_seq` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 730 [302-797] | 125 [83.2-137] | 5.84 | better |
| `update_kops` | 633 [281-650] | 106 [96.7-140] | 5.97 | better |
| `update_commit100_p99_us` | 190 [168-346] | 47907 [14934-58589] | <0.01 | better |
| `write_amp` | 2.10 [2.04-2.12] | 3.22 [3.18-3.23] | 0.65 | better |
| `space_amp` | 3.74 [3.57-3.81] | 1.42 [1.41-1.42] | 2.64 | worse |
| `peak_rss_mb` | 29.9 [28.4-31.5] | 39.8 [39.6-40.2] | 0.75 | better |

### `mixed` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 1082 [1031-1109] | 62.4 [53.2-101] | 17.34 | better |
| `read_p50_us` | 2.33 [2.29-2.60] | 21.2 [16.7-21.4] | 0.11 | better |
| `read_p99_us` | 4.99 [4.60-5.19] | 97.7 [81.8-111] | 0.05 | better |
| `write_p50_us` | 10.3 [9.58-10.7] | 31.4 [24.3-33.8] | 0.33 | better |
| `write_p99_us` | 31.9 [28.8-33.0] | 303 [238-361] | 0.11 | better |

### `history_sim` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 148 [114-205] | 7.09 [5.81-7.60] | 20.90 | better |
| `rev_commit100_p99_us` | 1027 [971-1238] | 88080 [79430-117375] | 0.01 | better |
| `space_amp` | 2.70 [2.67-2.72] | 1.94 [1.94-1.94] | 1.39 | worse |
| `history_scan_p50_us` | 12.2 [11.3-14.8] | 6.24 [6.01-6.50] | 1.96 | worse |
| `history_scan_p99_us` | 34.0 [29.7-44.6] | 16.8 [13.8-29.9] | 2.02 | tie |
| `reconstruct_p99_us` | 6.86 [5.52-8.68] | 6.92 [3.37-9.21] | 0.99 | tie |

### `livespec` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 1841 [1740-2076] | 387 [346-510] | 4.76 | better |
| `changeset_commit_p50_us` | 45.6 [43.6-51.7] | 376 [359-391] | 0.12 | better |
| `changeset_commit_p99_us` | 130 [115-155] | 63603 [25592-75432] | <0.01 | better |
| `symbol_get_p99_us` | 14.0 [12.9-16.5] | 25.8 [22.9-30.1] | 0.54 | better |
| `module_scan_p50_us` | 97.3 [91.6-111] | 156 [145-162] | 0.62 | better |
| `module_scan_p99_us` | 278 [252-294] | 367 [290-404] | 0.76 | tie |

### `cauce` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 41730 [37755-47541] | 4229 [3578-4925] | 9.87 | better |
| `step_commit_p50_us` | 5.65 [5.58-6.24] | 22.5 [22.4-23.9] | 0.25 | better |
| `step_commit_p99_us` | 12.8 [11.4-14.3] | 278 [254-368] | 0.05 | better |
| `step_read_p99_us` | 34.2 [33.4-37.1] | 71.7 [69.4-73.1] | 0.48 | better |
| `recovery_ms` | 54.0 [31.4-60.4] | 0.39 [0.36-0.40] | 137.64 | worse |

### `agent_memory` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 4119 [3254-6155] | 1179 [806-1371] | 3.49 | better |
| `memory_write_p50_us` | 14.1 [13.8-14.2] | 82.2 [81.5-82.8] | 0.17 | better |
| `memory_write_p99_us` | 811 [57.8-1526] | 7873 [1535-13394] | 0.10 | better |
| `recall_p99_us` | 35.0 [22.5-38.2] | 80.5 [46.1-92.5] | 0.43 | better |
| `recent_scan_p99_us` | 39.5 [23.8-51.2] | 134 [81.2-150] | 0.29 | better |
| `expire_commit_p99_us` | 4133 [1352-109904] | 6025 [5382-13591] | 0.69 | tie |
| `space_amp` | 2.83 [2.83-2.87] | 3.87 [3.86-3.87] | 0.73 | better |

### `concurrent_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 265275 [205688-296865] | 12600 [7966-14111] | 21.05 | better |
| `cc_commit_p50_us` | 26.8 [23.8-28.2] | 33.8 [33.6-39.6] | 0.79 | better |
| `cc_commit_p99_us` | 71.0 [62.0-94.4] | 17449 [14344-26706] | <0.01 | better |

## Summary

| Pair | kerno better | not distinguishable (ranges overlap) | kerno worse |
|---|---|---|---|
| `sync` | 42 | 20 | 5 |
| `buffered` | 54 | 6 | 7 |

### Every metric where kerno is worse and the ranges do not overlap

| Pair | Workload | Metric | kerno | sqlite | kerno / sqlite |
|---|---|---|---|---|---|
| sync | bulk_load | `close_ms` | 46.0 [37.0-174] | 4.95 [4.44-7.81] | 9.29 |
| sync | bulk_load | `reopen_ms` | 3.61 [3.33-4.42] | 0.44 [0.39-0.74] | 8.23 |
| sync | bulk_load | `recovery_ms` | 9.04 [7.40-14.0] | 0.46 [0.39-0.95] | 19.56 |
| sync | append_seq | `space_amp` | 1.76 [1.75-1.76] | 1.42 [1.41-1.42] | 1.24 |
| sync | cauce | `recovery_ms` | 28.6 [25.5-50.1] | 0.39 [0.38-0.41] | 72.93 |
| buffered | bulk_load | `close_ms` | 187 [185-197] | 9.94 [4.84-11.4] | 18.78 |
| buffered | bulk_load | `reopen_ms` | 3.45 [3.39-3.95] | 0.92 [0.41-1.02] | 3.73 |
| buffered | bulk_load | `recovery_ms` | 9.85 [8.30-12.8] | 0.90 [0.38-0.92] | 10.90 |
| buffered | append_seq | `space_amp` | 3.74 [3.57-3.81] | 1.42 [1.41-1.42] | 2.64 |
| buffered | history_sim | `space_amp` | 2.70 [2.67-2.72] | 1.94 [1.94-1.94] | 1.39 |
| buffered | history_sim | `history_scan_p50_us` | 12.2 [11.3-14.8] | 6.24 [6.01-6.50] | 1.96 |
| buffered | cauce | `recovery_ms` | 54.0 [31.4-60.4] | 0.39 [0.36-0.40] | 137.64 |
