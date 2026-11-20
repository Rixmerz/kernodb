## Durability `sync`: kerno `Sync` against SQLite WAL + `synchronous=FULL` (the equivalent pair)

### `bulk_load` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 69.6 [47.5-74.2] | 18.2 [13.4-23.4] | 3.81 | better |
| `write_amp` | 1.01 [1.01-1.01] | 6.25 [6.22-6.33] | 0.16 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.46-1.50] | 0.70 | better |
| `close_ms` | 52.0 [47.8-78.1] | 8.24 [7.23-13.6] | 6.32 | worse |
| `reopen_ms` | 5.96 [5.47-8.86] | 2.87 [2.70-4.59] | 2.07 | worse |
| `recovery_ms` | 22.0 [16.6-37.4] | 2.82 [1.32-3.75] | 7.81 | worse |
| `peak_rss_mb` | 9.86 [9.58-9.89] | 11.0 [10.8-11.1] | 0.90 | better |

### `point_reads` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 363 [329-374] | 171 [168-205] | 2.12 | better |
| `get_1t_p50_us` | 1.82 [1.69-1.93] | 3.02 [2.96-3.17] | 0.60 | better |
| `get_1t_p95_us` | 2.85 [2.77-3.14] | 6.01 [5.70-6.04] | 0.47 | better |
| `get_1t_p99_us` | 3.87 [3.82-4.27] | 8.73 [7.95-9.58] | 0.44 | better |
| `get_4t_kops` | 901 [725-1218] | 447 [340-545] | 2.01 | better |
| `get_4t_p99_us` | 3.33 [3.05-3.68] | 13.2 [12.0-17.4] | 0.25 | better |

### `range_scans` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 13.1 [9.95-14.3] | 6.75 [6.39-7.74] | 1.94 | better |
| `scan100_p50_us` | 52.6 [51.1-57.2] | 96.4 [89.3-99.5] | 0.55 | better |
| `scan100_p95_us` | 80.8 [75.1-85.7] | 156 [125-157] | 0.52 | better |
| `scan100_p99_us` | 791 [119-1779] | 1845 [1829-1852] | 0.43 | better |

### `small_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 335 [295-392] | 290 [234-314] | 1.16 | tie |
| `commit_p50_us` | 2509 [2185-2681] | 2798 [2675-3219] | 0.90 | tie |
| `commit_p95_us` | 6447 [4686-8372] | 8118 [6361-9781] | 0.79 | tie |
| `commit_p99_us` | 10797 [8069-13263] | 11674 [11420-17056] | 0.92 | tie |
| `commit_p999_us` | 23085 [16974-24674] | 45253 [37945-50528] | 0.51 | better |
| `write_amp` | 1.17 [1.17-1.17] | 30.0 [29.9-30.1] | 0.04 | better |
| `commit_fsyncs_per_commit` | 1.00 [1.00-1.00] | n/a | n/a | n/a |

### `point_deletes` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 332 [278-349] | 282 [177-366] | 1.17 | tie |
| `delete_commit_p50_us` | 2415 [2302-2714] | 2912 [2492-4264] | 0.83 | tie |
| `delete_commit_p99_us` | 10363 [9650-15008] | 11280 [7877-21463] | 0.92 | tie |
| `get_deleted_p50_us` | 1.00 [0.85-1.33] | 2.44 [1.89-3.02] | 0.41 | better |
| `get_deleted_p99_us` | 4.77 [2.71-8.03] | 11.1 [7.91-23.6] | 0.43 | tie |
| `get_live_p99_us` | 11.4 [6.25-18.6] | 14.1 [10.9-28.3] | 0.81 | tie |

### `append_seq` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 36.2 [33.4-41.8] | 30.1 [20.2-32.2] | 1.20 | better |
| `update_kops` | 34.1 [30.7-35.7] | 31.2 [22.7-35.3] | 1.09 | tie |
| `update_commit100_p99_us` | 7168 [5464-9552] | 6656 [4641-15581] | 1.08 | tie |
| `write_amp` | 1.02 [1.02-1.02] | 2.35 [2.35-2.38] | 0.43 | better |
| `space_amp` | 3.05 [3.04-3.06] | 4.27 [4.26-4.28] | 0.71 | better |
| `peak_rss_mb` | 11.0 [10.9-11.1] | 8.37 [8.22-8.53] | 1.31 | worse |

### `mixed` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 7.30 [6.75-11.0] | 2.77 [2.50-3.55] | 2.64 | better |
| `read_p50_us` | 5.12 [3.59-6.63] | 26.7 [24.0-30.4] | 0.19 | better |
| `read_p99_us` | 1304 [22.0-1712] | 724 [435-1443] | 1.80 | tie |
| `write_p50_us` | 4514 [2245-4657] | 11829 [9896-14213] | 0.38 | better |
| `write_p99_us` | 15245 [12861-16114] | 36176 [32817-44696] | 0.42 | better |

### `history_sim` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 30.9 [29.2-32.3] | 10.8 [10.6-11.1] | 2.85 | better |
| `rev_commit100_p99_us` | 8692 [8172-11903] | 18301 [16941-20562] | 0.47 | better |
| `space_amp` | 4.59 [4.59-4.59] | 7.85 [7.85-7.85] | 0.58 | better |
| `history_scan_p50_us` | 5.69 [4.36-6.83] | 5.98 [4.50-9.18] | 0.95 | tie |
| `history_scan_p99_us` | 16.0 [12.2-20.5] | 14.9 [11.8-26.5] | 1.07 | tie |
| `reconstruct_p99_us` | 2.69 [2.13-3.44] | 3.94 [3.65-9.47] | 0.68 | better |

### `livespec` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 400 [272-498] | 293 [125-305] | 1.37 | tie |
| `changeset_commit_p50_us` | 1212 [1117-1522] | 1634 [1547-3619] | 0.74 | better |
| `changeset_commit_p99_us` | 10191 [7586-11723] | 17809 [17252-36209] | 0.57 | better |
| `symbol_get_p99_us` | 15.5 [10.8-24.8] | 28.7 [23.8-129] | 0.54 | tie |
| `module_scan_p50_us` | 210 [179-317] | 178 [146-514] | 1.18 | tie |
| `module_scan_p99_us` | 940 [598-2583] | 380 [336-3869] | 2.47 | tie |

### `cauce` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 506 [292-776] | 376 [259-541] | 1.35 | tie |
| `step_commit_p50_us` | 1275 [989-2378] | 1309 [987-2243] | 0.97 | tie |
| `step_commit_p99_us` | 8536 [7598-15647] | 13165 [9372-18203] | 0.65 | tie |
| `step_read_p99_us` | 91.7 [44.8-162] | 651 [370-1731] | 0.14 | better |
| `recovery_ms` | 94.0 [61.8-520] | 0.56 [0.42-2.49] | 166.64 | worse |

### `agent_memory` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 369 [243-572] | 255 [145-384] | 1.44 | tie |
| `memory_write_p50_us` | 1826 [1151-2568] | 1710 [1389-3025] | 1.07 | tie |
| `memory_write_p99_us` | 9241 [8020-15450] | 11583 [8782-14893] | 0.80 | tie |
| `recall_p99_us` | 88.2 [28.5-125] | 276 [97.6-2189] | 0.32 | tie |
| `recent_scan_p99_us` | 236 [54.1-959] | 1009 [176-3383] | 0.23 | tie |
| `expire_commit_p99_us` | 10789 [5808-19464] | 15122 [10306-244187] | 0.71 | tie |
| `space_amp` | 1.89 [1.88-1.89] | 2.03 [2.02-2.03] | 0.93 | better |

### `concurrent_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 1785 [1249-2835] | 430 [355-544] | 4.15 | better |
| `cc_commit_p50_us` | 3240 [2245-5349] | 15630 [14426-20136] | 0.21 | better |
| `cc_commit_p99_us` | 14639 [9535-20955] | 54166 [41026-58589] | 0.27 | better |
| `cc_fsyncs_per_commit` | 0.21 [0.21-0.22] | n/a | n/a | n/a |

## Durability `buffered`: kerno `Buffered` against SQLite WAL + `synchronous=NORMAL` (NOT equivalent guarantees)

### `bulk_load` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `load_kops` | 350 [342-406] | 34.8 [29.2-54.7] | 10.04 | better |
| `write_amp` | 1.01 [1.01-1.01] | 6.25 [6.22-6.33] | 0.16 | better |
| `space_amp_closed` | 1.03 [1.03-1.03] | 1.48 [1.46-1.50] | 0.70 | better |
| `close_ms` | 27.8 [25.6-52.0] | 3.03 [2.87-4.07] | 9.18 | worse |
| `reopen_ms` | 3.38 [3.26-4.33] | 0.60 [0.53-0.69] | 5.65 | worse |
| `recovery_ms` | 14.7 [10.3-25.2] | 0.74 [0.57-0.96] | 19.93 | worse |
| `peak_rss_mb` | 9.66 [9.32-9.85] | 10.9 [10.7-11.0] | 0.89 | better |

### `point_reads` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `get_1t_kops` | 1103 [262-1161] | 519 [161-584] | 2.13 | tie |
| `get_1t_p50_us` | 0.74 [0.65-2.00] | 1.50 [1.32-3.23] | 0.49 | tie |
| `get_1t_p95_us` | 1.28 [1.22-3.62] | 3.08 [2.77-6.50] | 0.42 | tie |
| `get_1t_p99_us` | 1.79 [1.59-6.27] | 4.32 [3.64-14.9] | 0.42 | tie |
| `get_4t_kops` | 3732 [990-4835] | 1268 [376-1449] | 2.94 | tie |
| `get_4t_p99_us` | 1.71 [1.29-4.06] | 11.2 [8.60-16.4] | 0.15 | better |

### `range_scans` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `scan100_kops` | 12.5 [12.3-23.3] | 6.78 [6.24-9.03] | 1.84 | better |
| `scan100_p50_us` | 56.6 [34.4-60.4] | 93.2 [88.6-99.8] | 0.61 | better |
| `scan100_p95_us` | 86.7 [54.0-93.1] | 168 [142-184] | 0.52 | better |
| `scan100_p99_us` | 398 [73.9-937] | 1843 [350-1864] | 0.22 | tie |

### `small_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `commits_per_s` | 36630 [27568-60326] | 6634 [4909-10209] | 5.52 | better |
| `commit_p50_us` | 8.84 [8.14-10.5] | 28.2 [26.9-29.3] | 0.31 | better |
| `commit_p95_us` | 17.8 [15.0-45.6] | 58.0 [43.4-119] | 0.31 | tie |
| `commit_p99_us` | 67.5 [29.7-140] | 1950 [614-2482] | 0.03 | better |
| `commit_p999_us` | 3858 [1724-4407] | 14721 [10289-19726] | 0.26 | better |
| `write_amp` | 1.27 [1.27-1.27] | 29.7 [29.6-29.8] | 0.04 | better |
| `commit_fsyncs_per_commit` | 0.00 [0.00-0.00] | n/a | n/a | n/a |

### `point_deletes` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `delete_commits_per_s` | 133932 [63966-141788] | 6460 [5609-7632] | 20.73 | better |
| `delete_commit_p50_us` | 5.89 [5.62-6.04] | 38.1 [37.2-40.3] | 0.15 | better |
| `delete_commit_p99_us` | 24.7 [22.0-31.2] | 957 [390-1618] | 0.03 | better |
| `get_deleted_p50_us` | 1.08 [0.89-1.30] | 2.46 [2.19-2.79] | 0.44 | better |
| `get_deleted_p99_us` | 4.65 [3.08-5.83] | 11.0 [10.3-16.0] | 0.42 | better |
| `get_live_p99_us` | 10.3 [7.31-13.8] | 12.7 [11.3-16.2] | 0.81 | tie |

### `append_seq` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `append_kops` | 369 [220-390] | 133 [120-143] | 2.77 | better |
| `update_kops` | 312 [173-352] | 147 [77.2-161] | 2.13 | better |
| `update_commit100_p99_us` | 514 [343-1532] | 2304 [2058-5538] | 0.22 | better |
| `write_amp` | 1.02 [1.02-1.02] | 2.35 [2.35-2.38] | 0.43 | better |
| `space_amp` | 3.05 [3.04-3.06] | 4.27 [4.26-4.28] | 0.71 | better |
| `peak_rss_mb` | 11.0 [10.9-11.1] | 8.39 [8.13-8.53] | 1.31 | worse |

### `mixed` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `mixed_kops` | 124 [61.6-804] | 21.8 [12.9-81.1] | 5.70 | tie |
| `read_p50_us` | 3.63 [2.21-5.08] | 33.2 [28.8-45.1] | 0.11 | better |
| `read_p99_us` | 15.1 [7.93-49.3] | 2163 [102-3434] | 0.01 | better |
| `write_p50_us` | 17.4 [8.31-27.3] | 60.1 [47.7-84.2] | 0.29 | better |
| `write_p99_us` | 4151 [93.9-4813] | 8184 [463-10969] | 0.51 | tie |

### `history_sim` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `revisions_per_s_k` | 78.6 [65.1-98.6] | 20.5 [19.1-22.6] | 3.84 | better |
| `rev_commit100_p99_us` | 1527 [1162-2576] | 17056 [15565-18825] | 0.09 | better |
| `space_amp` | 4.59 [4.59-4.59] | 7.85 [7.85-7.85] | 0.58 | better |
| `history_scan_p50_us` | 9.86 [8.94-11.3] | 13.1 [12.0-13.4] | 0.75 | better |
| `history_scan_p99_us` | 31.4 [20.0-37.7] | 38.0 [27.7-41.4] | 0.83 | tie |
| `reconstruct_p99_us` | 3.91 [3.56-8.83] | 6.98 [6.43-13.6] | 0.56 | tie |

### `livespec` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `changesets_per_s` | 504 [294-672] | 244 [134-367] | 2.07 | tie |
| `changeset_commit_p50_us` | 272 [230-384] | 669 [591-1070] | 0.41 | better |
| `changeset_commit_p99_us` | 2589 [510-4415] | 35291 [24117-65634] | 0.07 | better |
| `symbol_get_p99_us` | 29.1 [22.1-54.4] | 114 [53.2-490] | 0.26 | tie |
| `module_scan_p50_us` | 464 [372-732] | 385 [358-668] | 1.20 | tie |
| `module_scan_p99_us` | 2828 [1032-4637] | 2941 [613-5329] | 0.96 | tie |

### `cauce` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 35670 [10025-43834] | 2303 [1615-4508] | 15.49 | better |
| `step_commit_p50_us` | 12.4 [10.3-14.6] | 74.0 [45.9-81.5] | 0.17 | better |
| `step_commit_p99_us` | 38.4 [31.4-430] | 2615 [260-5149] | 0.01 | tie |
| `step_read_p99_us` | 33.2 [27.4-134] | 2009 [130-3584] | 0.02 | tie |
| `recovery_ms` | 173 [150-380] | 1.07 [0.98-16.4] | 161.67 | worse |

### `agent_memory` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `steps_per_s` | 3853 [2519-8197] | 490 [418-605] | 7.86 | better |
| `memory_write_p50_us` | 26.8 [18.8-29.4] | 239 [206-247] | 0.11 | better |
| `memory_write_p99_us` | 143 [51.1-1769] | 4731 [4231-6586] | 0.03 | better |
| `recall_p99_us` | 36.8 [28.4-47.4] | 1352 [333-1828] | 0.03 | better |
| `recent_scan_p99_us` | 104 [39.8-1055] | 1915 [1790-2185] | 0.05 | better |
| `expire_commit_p99_us` | 1911 [252-3846] | 29655 [11272-32997] | 0.06 | better |
| `space_amp` | 1.89 [1.88-1.89] | 2.03 [2.02-2.03] | 0.93 | better |

### `concurrent_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min-max] | sqlite median [min-max] | kerno / sqlite | kerno is |
|---|---|---|---|---|
| `cc_threads` | 8.00 [8.00-8.00] | 8.00 [8.00-8.00] | 1.00 | n/a |
| `cc_commits_per_s` | 69806 [22934-102137] | 8579 [4891-11187] | 8.14 | better |
| `cc_commit_p50_us` | 6.29 [5.61-6.55] | 29.1 [26.6-31.2] | 0.22 | better |
| `cc_commit_p99_us` | 2976 [1866-10035] | 25133 [15032-46760] | 0.12 | better |

## Summary

| Pair | kerno better | not distinguishable (ranges overlap) | kerno worse |
|---|---|---|---|
| `sync` | 35 | 27 | 5 |
| `buffered` | 44 | 18 | 5 |

### Every metric where kerno is worse and the ranges do not overlap

| Pair | Workload | Metric | kerno | sqlite | kerno / sqlite |
|---|---|---|---|---|---|
| sync | bulk_load | `close_ms` | 52.0 [47.8-78.1] | 8.24 [7.23-13.6] | 6.32 |
| sync | bulk_load | `reopen_ms` | 5.96 [5.47-8.86] | 2.87 [2.70-4.59] | 2.07 |
| sync | bulk_load | `recovery_ms` | 22.0 [16.6-37.4] | 2.82 [1.32-3.75] | 7.81 |
| sync | append_seq | `peak_rss_mb` | 11.0 [10.9-11.1] | 8.37 [8.22-8.53] | 1.31 |
| sync | cauce | `recovery_ms` | 94.0 [61.8-520] | 0.56 [0.42-2.49] | 166.64 |
| buffered | bulk_load | `close_ms` | 27.8 [25.6-52.0] | 3.03 [2.87-4.07] | 9.18 |
| buffered | bulk_load | `reopen_ms` | 3.38 [3.26-4.33] | 0.60 [0.53-0.69] | 5.65 |
| buffered | bulk_load | `recovery_ms` | 14.7 [10.3-25.2] | 0.74 [0.57-0.96] | 19.93 |
| buffered | append_seq | `peak_rss_mb` | 11.0 [10.9-11.1] | 8.39 [8.13-8.53] | 1.31 |
| buffered | cauce | `recovery_ms` | 173 [150-380] | 1.07 [0.98-16.4] | 161.67 |
