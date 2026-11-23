## Durability `sync`: kerno `Sync` against SQLite WAL + `synchronous=FULL` (the equivalent pair)

### `bulk_load` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 332 [97.5-341] | 47.0 [44.5-48.8] | 7.06 | better |
| `write_amp` | 1.01 [1.01-1.01] | 12.2 [12.1-12.2] | 0.08 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.49] | 0.70 | better |
| `close_ms` | 21.6 [20.5-72.9] | 2.29 [1.86-2.55] | 9.44 | worse |
| `reopen_ms` | 2.39 [2.25-9.37] | 0.38 [0.36-0.39] | 6.37 | worse |
| `recovery_ms` | 6.33 [5.62-7.41] | 0.38 [0.38-0.40] | 16.61 | worse |
| `peak_rss_mb` | 13.8 [13.6-14.4] | 15.8 [15.6-15.9] | 0.87 | better |

### `point_reads` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 1285 [1206-1530] | 717 [635-763] | 1.79 | better |
| `get_1t_p50_us` | 0.67 [0.53-0.72] | 1.12 [1.03-1.25] | 0.60 | better |
| `get_1t_p95_us` | 1.14 [0.90-1.28] | 2.21 [2.06-2.54] | 0.52 | better |
| `get_1t_p99_us` | 1.40 [1.13-1.61] | 2.81 [2.56-3.49] | 0.50 | better |
| `get_4t_kops` | 5063 [4574-5223] | 1817 [1496-1911] | 2.79 | better |
| `get_4t_p99_us` | 1.47 [1.31-1.61] | 12.0 [11.3-13.5] | 0.12 | better |

### `range_scans` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 57.7 [50.6-61.5] | 29.5 [27.1-32.0] | 1.95 | better |
| `scan100_p50_us` | 15.3 [14.6-16.6] | 32.1 [30.6-33.6] | 0.48 | better |
| `scan100_p95_us` | 22.2 [18.0-31.5] | 47.2 [37.5-56.8] | 0.47 | better |
| `scan100_p99_us` | 38.7 [21.9-44.6] | 65.0 [44.6-81.2] | 0.59 | tie |

### `small_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 951 [768-1088] | 727 [652-959] | 1.31 | tie |
| `commit_p50_us` | 927 [830-928] | 978 [934-1000] | 0.95 | better |
| `commit_p95_us` | 1272 [1248-3396] | 1965 [1349-6447] | 0.65 | tie |
| `commit_p99_us` | 2392 [2177-8024] | 7819 [2193-8323] | 0.31 | tie |
| `commit_p999_us` | 14098 [6836-20611] | 17875 [14352-19235] | 0.79 | tie |
| `write_amp` | 1.17 [1.17-1.17] | 33.1 [33.0-33.2] | 0.04 | better |
| `commit_fsyncs_per_commit` | 1.00 [1.00-1.00] | n/a | n/a | n/a |

### `point_deletes` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 885 [806-1159] | 828 [797-913] | 1.07 | tie |
| `delete_commit_p50_us` | 843 [781-868] | 951 [944-980] | 0.89 | better |
| `delete_commit_p99_us` | 7406 [1384-9699] | 7180 [2052-7766] | 1.03 | tie |
| `get_deleted_p50_us` | 0.36 [0.36-0.40] | 0.85 [0.83-1.12] | 0.42 | better |
| `get_deleted_p99_us` | 0.66 [0.57-0.86] | 2.63 [2.58-3.08] | 0.25 | better |
| `get_live_p99_us` | 1.87 [1.68-2.29] | 3.08 [2.87-3.46] | 0.61 | better |

### `append_seq` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 103 [61.3-110] | 76.5 [74.0-80.4] | 1.35 | tie |
| `update_kops` | 87.8 [53.6-96.9] | 83.6 [61.3-91.5] | 1.05 | tie |
| `update_commit100_p99_us` | 2660 [1754-3283] | 6185 [2037-12902] | 0.43 | tie |
| `write_amp` | 1.35 [1.35-1.35] | 2.89 [2.87-2.90] | 0.47 | better |
| `space_amp` | 1.50 [1.50-1.51] | 2.78 [2.77-2.80] | 0.54 | better |
| `peak_rss_mb` | 16.5 [16.5-16.8] | 10.6 [10.0-10.8] | 1.56 | worse |

### `mixed` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 21.1 [19.9-22.5] | 7.49 [7.17-8.83] | 2.82 | better |
| `read_p50_us` | 2.46 [2.33-2.52] | 13.2 [12.7-14.5] | 0.19 | better |
| `read_p99_us` | 7.89 [7.23-9.16] | 96.4 [82.0-112] | 0.08 | better |
| `write_p50_us` | 1780 [1640-1798] | 3885 [3777-4145] | 0.46 | better |
| `write_p99_us` | 6988 [3043-8749] | 22249 [20382-23773] | 0.31 | better |

### `history_sim` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 70.4 [67.5-77.4] | 15.7 [8.94-17.8] | 4.49 | better |
| `rev_commit100_p99_us` | 1752 [1596-2206] | 22561 [11780-47383] | 0.08 | better |
| `space_amp` | 4.59 [4.59-4.59] | 4.79 [4.79-4.79] | 0.96 | better |
| `history_scan_p50_us` | 2.27 [2.10-3.78] | 3.92 [3.87-5.05] | 0.58 | better |
| `history_scan_p99_us` | 4.61 [3.84-12.4] | 11.7 [9.55-13.9] | 0.39 | tie |
| `reconstruct_p99_us` | 1.77 [1.64-2.23] | 3.67 [3.12-4.20] | 0.48 | better |

### `livespec` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 571 [441-659] | 371 [321-434] | 1.54 | better |
| `changeset_commit_p50_us` | 1087 [1005-1143] | 1295 [1269-1338] | 0.84 | better |
| `changeset_commit_p99_us` | 7823 [4260-10805] | 17580 [15131-21365] | 0.45 | better |
| `symbol_get_p99_us` | 11.5 [9.14-12.6] | 16.4 [14.7-17.6] | 0.70 | better |
| `module_scan_p50_us` | 134 [121-145] | 101 [97.2-103] | 1.32 | worse |
| `module_scan_p99_us` | 317 [310-346] | 261 [247-273] | 1.21 | worse |

### `cauce` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 848 [688-987] | 808 [613-840] | 1.05 | tie |
| `step_commit_p50_us` | 935 [894-950] | 974 [965-990] | 0.96 | better |
| `step_commit_p99_us` | 7889 [2677-8495] | 7942 [5964-10379] | 0.99 | tie |
| `step_read_p99_us` | 35.1 [31.7-36.4] | 176 [163-182] | 0.20 | better |
| `recovery_ms` | 16.4 [15.6-20.6] | 0.39 [0.38-0.45] | 41.93 | worse |

### `agent_memory` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 772 [606-811] | 481 [451-526] | 1.60 | better |
| `memory_write_p50_us` | 961 [947-1013] | 1262 [1204-1284] | 0.76 | better |
| `memory_write_p99_us` | 7995 [3979-10428] | 8106 [6124-9699] | 0.99 | tie |
| `recall_p99_us` | 26.9 [23.8-30.1] | 92.5 [82.6-98.3] | 0.29 | better |
| `recent_scan_p99_us` | 51.3 [44.6-55.2] | 155 [150-177] | 0.33 | better |
| `expire_commit_p99_us` | 9003 [1923-14246] | 8643 [3594-9503] | 1.04 | tie |
| `space_amp` | 1.18 [1.18-1.18] | 2.02 [2.02-2.03] | 0.58 | better |

### `concurrent_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 4265 [4100-4573] | 788 [757-872] | 5.42 | better |
| `cc_commit_p50_us` | 1879 [1769-1893] | 8413 [8204-8651] | 0.22 | better |
| `cc_commit_p99_us` | 6271 [3633-8684] | 38601 [36635-42107] | 0.16 | better |
| `cc_fsyncs_per_commit` | 0.21 [0.21-0.22] | n/a | n/a | n/a |

## Durability `buffered`: kerno `Buffered` against SQLite WAL + `synchronous=NORMAL` (NOT equivalent guarantees)

### `bulk_load` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 617 [594-632] | 37.7 [18.6-39.2] | 16.37 | better |
| `write_amp` | 1.01 [1.01-1.01] | 12.2 [12.1-12.2] | 0.08 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.47-1.49] | 0.70 | better |
| `close_ms` | 35.9 [34.3-107] | 3.23 [2.60-4.54] | 11.09 | worse |
| `reopen_ms` | 2.62 [2.42-12.7] | 0.36 [0.35-0.69] | 7.23 | worse |
| `recovery_ms` | 11.2 [7.75-64.3] | 0.49 [0.38-0.82] | 22.61 | worse |
| `peak_rss_mb` | 13.5 [13.3-13.6] | 15.2 [15.0-15.3] | 0.88 | better |

### `point_reads` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 1543 [1339-1619] | 701 [664-761] | 2.20 | better |
| `get_1t_p50_us` | 0.52 [0.50-0.63] | 1.14 [1.04-1.22] | 0.46 | better |
| `get_1t_p95_us` | 0.90 [0.83-1.08] | 2.25 [2.10-2.38] | 0.40 | better |
| `get_1t_p99_us` | 1.18 [1.05-1.32] | 2.88 [2.65-3.06] | 0.41 | better |
| `get_4t_kops` | 4989 [4575-5655] | 1761 [1615-1790] | 2.83 | better |
| `get_4t_p99_us` | 1.46 [1.23-1.68] | 13.2 [11.7-19.3] | 0.11 | better |

### `range_scans` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 61.9 [60.5-63.9] | 32.7 [30.8-33.3] | 1.89 | better |
| `scan100_p50_us` | 14.4 [13.9-14.6] | 29.7 [29.1-30.9] | 0.49 | better |
| `scan100_p95_us` | 17.6 [17.0-18.5] | 36.1 [35.5-41.4] | 0.49 | better |
| `scan100_p99_us` | 21.8 [20.5-28.7] | 41.2 [38.3-64.0] | 0.53 | better |

### `small_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 215175 [199012-220713] | 30585 [25348-31641] | 7.04 | better |
| `commit_p50_us` | 3.59 [3.53-3.67] | 12.9 [12.8-13.0] | 0.28 | better |
| `commit_p95_us` | 6.39 [6.32-6.58] | 16.8 [16.5-17.3] | 0.38 | better |
| `commit_p99_us` | 7.89 [7.71-8.71] | 28.3 [26.2-35.2] | 0.28 | better |
| `commit_p999_us` | 12.9 [12.2-14.5] | 1759 [1632-2386] | 0.01 | better |
| `write_amp` | 1.37 [1.37-1.37] | 32.9 [32.8-32.9] | 0.04 | better |
| `commit_fsyncs_per_commit` | 0.00 [0.00-0.00] | n/a | n/a | n/a |

### `point_deletes` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 290157 [282840-291295] | 14474 [11985-15844] | 20.05 | better |
| `delete_commit_p50_us` | 3.14 [3.13-3.17] | 15.3 [15.1-16.3] | 0.21 | better |
| `delete_commit_p99_us` | 6.26 [6.17-6.38] | 88.6 [81.6-106] | 0.07 | better |
| `get_deleted_p50_us` | 0.37 [0.37-0.43] | 0.86 [0.84-0.90] | 0.43 | better |
| `get_deleted_p99_us` | 0.57 [0.53-0.76] | 2.67 [2.44-2.85] | 0.21 | better |
| `get_live_p99_us` | 1.73 [1.58-2.25] | 2.98 [2.92-3.23] | 0.58 | better |

### `append_seq` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 1001 [782-1003] | 246 [172-256] | 4.07 | better |
| `update_kops` | 594 [410-608] | 295 [154-300] | 2.01 | better |
| `update_commit100_p99_us` | 123 [104-168] | 1542 [1399-1628] | 0.08 | better |
| `write_amp` | 1.11 [1.08-1.11] | 2.89 [2.87-2.90] | 0.39 | better |
| `space_amp` | 3.34 [3.25-3.34] | 2.78 [2.77-2.80] | 1.20 | worse |
| `peak_rss_mb` | 17.5 [17.3-17.7] | 10.5 [10.2-10.6] | 1.66 | worse |

### `mixed` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 1442 [1095-1479] | 112 [107-134] | 12.92 | better |
| `read_p50_us` | 1.35 [1.33-1.83] | 15.8 [14.1-16.8] | 0.09 | better |
| `read_p99_us` | 3.90 [3.53-6.23] | 61.8 [56.0-66.3] | 0.06 | better |
| `write_p50_us` | 10.6 [10.4-13.9] | 23.8 [22.0-24.2] | 0.45 | better |
| `write_p99_us` | 35.8 [33.8-46.6] | 142 [136-168] | 0.25 | better |

### `history_sim` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 354 [349-361] | 12.6 [12.1-27.0] | 28.05 | better |
| `rev_commit100_p99_us` | 379 [318-447] | 42926 [16572-47317] | 0.01 | better |
| `space_amp` | 4.59 [4.59-4.59] | 4.79 [4.79-4.79] | 0.96 | better |
| `history_scan_p50_us` | 2.19 [2.03-2.67] | 3.95 [3.82-4.09] | 0.56 | better |
| `history_scan_p99_us` | 4.40 [3.62-5.36] | 10.1 [9.50-19.4] | 0.44 | better |
| `reconstruct_p99_us` | 1.72 [1.65-2.08] | 3.15 [3.12-6.97] | 0.55 | better |

### `livespec` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 1929 [1746-2012] | 466 [344-610] | 4.14 | better |
| `changeset_commit_p50_us` | 44.1 [43.2-45.4] | 247 [231-257] | 0.18 | better |
| `changeset_commit_p99_us` | 124 [109-129] | 78709 [50954-91554] | <0.01 | better |
| `symbol_get_p99_us` | 7.59 [6.63-7.71] | 13.2 [12.8-14.3] | 0.58 | better |
| `module_scan_p50_us` | 113 [106-120] | 82.0 [80.8-91.5] | 1.37 | worse |
| `module_scan_p99_us` | 297 [285-334] | 251 [245-257] | 1.18 | worse |

### `cauce` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 56781 [44377-79748] | 3646 [2362-5116] | 15.57 | better |
| `step_commit_p50_us` | 4.97 [4.69-5.33] | 40.7 [29.6-56.1] | 0.12 | better |
| `step_commit_p99_us` | 10.9 [9.72-20.8] | 242 [180-260] | 0.05 | better |
| `step_read_p99_us` | 13.7 [8.43-17.8] | 81.9 [73.5-94.7] | 0.17 | better |
| `recovery_ms` | 48.1 [19.0-60.7] | 0.66 [0.37-0.82] | 72.51 | worse |

### `agent_memory` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 5851 [5250-12034] | 909 [752-1304] | 6.43 | better |
| `memory_write_p50_us` | 12.2 [12.1-12.8] | 79.8 [77.6-89.0] | 0.15 | better |
| `memory_write_p99_us` | 46.8 [25.6-53.8] | 7483 [1746-7782] | 0.01 | better |
| `recall_p99_us` | 18.8 [11.9-22.3] | 77.2 [68.0-81.4] | 0.24 | better |
| `recent_scan_p99_us` | 24.8 [19.6-33.4] | 140 [129-142] | 0.18 | better |
| `expire_commit_p99_us` | 214 [123-299] | 9519 [2611-12952] | 0.02 | better |
| `space_amp` | 2.28 [2.21-2.41] | 2.02 [2.02-2.03] | 1.13 | worse |

### `concurrent_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 270156 [238319-285792] | 9122 [8673-9487] | 29.62 | better |
| `cc_commit_p50_us` | 25.6 [24.3-29.6] | 38.1 [32.9-41.6] | 0.67 | better |
| `cc_commit_p99_us` | 77.2 [71.3-87.2] | 20955 [16876-31932] | <0.01 | better |

## Summary

| Pair | kerno better | not distinguishable (ranges overlap) | kerno worse |
|---|---|---|---|
| `sync` | 45 | 15 | 7 |
| `buffered` | 58 | 0 | 9 |

### Every metric where kerno is worse and the ranges do not overlap

| Pair | Workload | Metric | kerno | sqlite | kerno / sqlite |
|---|---|---|---|---|---|
| sync | bulk_load | `close_ms` | 21.6 [20.5-72.9] | 2.29 [1.86-2.55] | 9.44 |
| sync | bulk_load | `reopen_ms` | 2.39 [2.25-9.37] | 0.38 [0.36-0.39] | 6.37 |
| sync | bulk_load | `recovery_ms` | 6.33 [5.62-7.41] | 0.38 [0.38-0.40] | 16.61 |
| sync | append_seq | `peak_rss_mb` | 16.5 [16.5-16.8] | 10.6 [10.0-10.8] | 1.56 |
| sync | livespec | `module_scan_p50_us` | 134 [121-145] | 101 [97.2-103] | 1.32 |
| sync | livespec | `module_scan_p99_us` | 317 [310-346] | 261 [247-273] | 1.21 |
| sync | cauce | `recovery_ms` | 16.4 [15.6-20.6] | 0.39 [0.38-0.45] | 41.93 |
| buffered | bulk_load | `close_ms` | 35.9 [34.3-107] | 3.23 [2.60-4.54] | 11.09 |
| buffered | bulk_load | `reopen_ms` | 2.62 [2.42-12.7] | 0.36 [0.35-0.69] | 7.23 |
| buffered | bulk_load | `recovery_ms` | 11.2 [7.75-64.3] | 0.49 [0.38-0.82] | 22.61 |
| buffered | append_seq | `space_amp` | 3.34 [3.25-3.34] | 2.78 [2.77-2.80] | 1.20 |
| buffered | append_seq | `peak_rss_mb` | 17.5 [17.3-17.7] | 10.5 [10.2-10.6] | 1.66 |
| buffered | livespec | `module_scan_p50_us` | 113 [106-120] | 82.0 [80.8-91.5] | 1.37 |
| buffered | livespec | `module_scan_p99_us` | 297 [285-334] | 251 [245-257] | 1.18 |
| buffered | cauce | `recovery_ms` | 48.1 [19.0-60.7] | 0.66 [0.37-0.82] | 72.51 |
| buffered | agent_memory | `space_amp` | 2.28 [2.21-2.41] | 2.02 [2.02-2.03] | 1.13 |
