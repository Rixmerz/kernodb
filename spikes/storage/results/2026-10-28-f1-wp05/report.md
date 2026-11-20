
#### `bulk_load` — n = 200000, durability = sync

| engine | reps | load_kops | write_amp | space_amp_closed | close_ms | reopen_ms | recovery_ms | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 24.2 | 10.1 | 1.65 | 10.8 (10.0–17.6) | 1.21 (1.16–1.49) | 0.76 | 89.9 |
| lsm | 3 | 187 | 3.40 | 1.03 | 726 | 3.36 (3.00–3.89) | 6.23 | 24.7 |
| kerno | 3 | 209 | 2.96 | 1.03 | 204 | 3.75 | 6.11 (5.87–7.44) | 33.7 |
| fjall | 3 | 193 | 1.58 | 1.58 | 69.7 | 384 | 414 | 136 |

Load average (1 min) when the runs started: 1.37–1.50.

#### `append_seq` — n = 200000, durability = sync

| engine | reps | append_kops | update_kops | update_commit100_p99_us | write_amp | space_amp | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 140 (122–145) | 115 | 1749 (1691–1969) | 2.27 | 1.62 | 74.0 |
| lsm | 3 | 174 (160–189) | 143 | 6070 | 3.45 | 2.08 | 22.3 |
| kerno | 3 | 183 | 149 (145–174) | 3774 | 2.87 | 2.02 | 22.9 |
| fjall | 3 | 179 | 162 (134–163) | 1425 (1139–1444) | 1.74 | 3.24 (3.24–5.41) | 155 |

Load average (1 min) when the runs started: 1.17–1.43.

#### `history_sim` — n = 200000, durability = sync

| engine | reps | revisions_per_s_k | rev_commit100_p99_us | space_amp | history_scan_p50_us | history_scan_p99_us | reconstruct_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 11.1 | 14148 (13615–15974) | 2.04 | 4.94 | 23.4 (23.2–26.9) | 3.95 |
| lsm | 3 | 81.9 | 8196 | 1.88 | 14.3 | 42.7 (38.2–47.3) | 9.72 (9.38–11.3) |
| kerno | 3 | 78.7 (69.2–86.1) | 5239 | 1.67 | 13.0 | 42.3 (39.6–49.1) | 9.02 (8.34–11.0) |
| fjall | 3 | 75.3 (71.9–84.9) | 3035 (1942–3224) | 7.74 | 19.8 | 47.1 (42.8–50.8) | 23.7 |

Load average (1 min) when the runs started: 1.32–1.49.

#### `livespec` — n = 200000, durability = sync

| engine | reps | changesets_per_s | changeset_commit_p50_us | changeset_commit_p99_us | symbol_get_p99_us | module_scan_p50_us | module_scan_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 485 (476–556) | 1404 (1231–1479) | 3514 (3426–4092) | 32.8 | 151 | 306 |
| lsm | 3 | 774 | 511 (491–578) | 4362 (3742–4514) | 29.6 (28.8–33.4) | 167 | 666 (553–737) |
| kerno | 3 | 791 | 459 | 3021 (2537–3398) | 31.6 | 172 | 617 (596–724) |
| fjall | 3 | 832 | 448 (436–562) | 1255 (1194–2103) | 23.4 | 212 | 448 |

Load average (1 min) when the runs started: 1.13–1.28.

#### `cauce` — n = 200000, durability = sync

| engine | reps | steps_per_s | step_commit_p50_us | step_commit_p99_us | step_read_p99_us | recovery_ms |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 1458 | 604 | 1543 (1311–1622) | 57.2 | 1.26 (0.66–2.59) |
| lsm | 3 | 4485 | 162 | 484 (470–564) | 73.5 (70.8–84.2) | 9.13 (8.78–11.2) |
| kerno | 3 | 4202 | 172 | 1117 (915–1135) | 77.6 (77.2–94.6) | 8.16 (8.11–9.96) |
| fjall | 3 | 4294 | 169 | 483 (451–535) | 74.8 | 367 (359–422) |

Load average (1 min) when the runs started: 1.22–1.37.

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 1348 | 471 | 1342 (1182–1617) | 37.0 (35.5–42.0) | 54.8 (53.0–62.4) | 7418 | 3.93 |
| lsm | 3 | 1276 (1173–1399) | 344 (330–431) | 4825 (3602–4956) | 56.8 | 68.5 (61.3–78.0) | 5136 (1976–8004) | 2.45 (1.78–2.46) |
| kerno | 3 | 1368 | 344 (341–399) | 2480 | 55.4 | 65.1 | 10813 | 3.00 |
| fjall | 3 | 1892 (1606–1955) | 292 (278–347) | 795 (756–1183) | 42.1 (41.1–47.7) | 67.3 (64.3–76.9) | 2890 (2884–4653) | 3.14 |

Load average (1 min) when the runs started: 1.09–1.18.

