
#### `bulk_load` — n = 200000, durability = sync

| engine | reps | load_kops | write_amp | space_amp_closed | close_ms | reopen_ms | recovery_ms | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 22.4 | 10.1 | 1.65 | 14.2 (13.7–18.6) | 1.39 (1.19–1.96) | 0.96 (0.65–0.98) | 89.9 |
| lsm | 3 | 187 | 3.28 | 1.03 | 830 | 3.57 (3.00–4.44) | 6.45 (6.10–7.13) | 25.4 |
| kerno | 3 | 181 | 3.31 | 1.04 | 195 (180–308) | 4.72 (3.17–11.8) | 8.22 (7.34–9.40) | 27.7 |
| kerno@before | 3 | 162 | 3.33 | 1.04 | 185 | 3.08 (3.06–4.43) | 7.34 | 29.0 |
| fjall | 3 | 185 | 1.58 | 1.58 | 75.4 (64.7–100.0) | 389 | 395 | 136 |

Load average (1 min) when the runs started: 1.23–1.54.

#### `bulk_load` — n = 1000000, durability = sync

| engine | reps | load_kops | write_amp | space_amp_closed | close_ms | reopen_ms | recovery_ms | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 1 | 95.9 | 5.87 | 1.04 | 336 | 14.3 | 17.7 | 42.5 |
| kerno@before | 1 | 92.0 | 5.89 | 1.04 | 376 | 16.4 | 14.7 | 42.2 |

Load average (1 min) when the runs started: 1.26–1.41.

#### `point_reads` — n = 200000, durability = sync

| engine | reps | get_1t_kops | get_1t_p50_us | get_1t_p95_us | get_1t_p99_us | get_4t_kops | get_4t_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 238 | 4.02 | 5.86 | 9.32 (8.68–10.7) | 603 (473–685) | 26.6 (24.9–33.8) |
| lsm | 3 | 273 (229–316) | 2.54 (2.10–3.05) | 6.91 (5.87–8.34) | 9.52 (8.10–14.2) | 866 (849–1008) | 23.4 |
| kerno | 3 | 244 | 3.01 | 7.16 | 11.2 | 800 | 23.5 |
| kerno@before | 3 | 250 | 2.90 | 7.03 | 10.9 | 841 | 25.0 |
| fjall | 3 | 330 (227–334) | 2.47 (2.46–3.59) | 4.55 (4.54–6.91) | 5.99 (5.86–11.2) | 889 (839–1129) | 7.89 (6.96–8.80) |

Load average (1 min) when the runs started: 1.23–1.75.

#### `range_scans` — n = 200000, durability = sync

| engine | reps | scan100_kops | scan100_p50_us | scan100_p95_us | scan100_p99_us |
|---|---|---:|---:|---:|---:|
| btree | 3 | 12.1 | 78.6 | 124 | 157 |
| lsm | 3 | 15.2 | 58.2 | 100 | 132 |
| kerno | 3 | 11.8 (10.5–12.9) | 79.4 (73.8–85.8) | 119 (105–138) | 150 (121–185) |
| kerno@before | 3 | 11.7 | 79.9 | 121 | 152 |
| fjall | 3 | 12.9 | 70.4 | 95.7 (92.4–109) | 121 (120–164) |

Load average (1 min) when the runs started: 1.40–1.69.

#### `small_commits` — n = 200000, durability = sync

| engine | reps | commits_per_s | commit_p50_us | commit_p95_us | commit_p99_us | commit_p999_us | write_amp | commit_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 2118 | 452 | 658 | 905 | 2189 (2126–2605) | 112 | 2.00 |
| lsm | 3 | 5665 | 157 | 268 | 401 | 843 (745–1924) | 1.09 | 1.00 |
| kerno | 3 | 5695 | 154 | 298 | 542 (403–553) | 1509 (999–2408) | 1.17 | 1.00 |
| kerno@before | 3 | 5661 | 159 | 293 (272–379) | 417 (371–641) | 2574 (1915–3029) | 4.05 (1.17–5.01) | 1.00 |
| fjall | 3 | 5457 | 160 | 306 | 434 | 1539 (1176–2288) | 1.21 | — |

Load average (1 min) when the runs started: 1.29–1.50.

#### `point_deletes` — n = 200000, durability = sync

| engine | reps | delete_commits_per_s | delete_commit_p50_us | delete_commit_p99_us | get_deleted_p50_us | get_deleted_p99_us | get_live_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 1953 (1860–2307) | 485 (411–502) | 950 | 2.45 | 12.8 (11.0–15.1) | 15.2 (14.8–18.1) |
| lsm | 3 | 6009 | 156 | 342 | 0.61 (0.59–0.71) | 1.46 (1.34–1.88) | 12.1 (11.9–14.5) |
| kerno | 3 | 5649 | 158 | 375 (359–427) | 0.66 | 1.61 (1.37–1.87) | 15.3 (12.9–18.7) |
| kerno@before | 3 | 5327 | 160 | 410 (362–442) | 0.92 (0.69–0.95) | 2.18 | 23.0 (22.0–31.3) |
| fjall | 3 | 5397 | 160 | 375 (370–562) | 1.31 (1.25–1.83) | 3.56 (2.87–3.78) | 8.48 (7.60–10.8) |

Load average (1 min) when the runs started: 1.22–1.43.

#### `append_seq` — n = 200000, durability = sync

| engine | reps | append_kops | update_kops | update_commit100_p99_us | write_amp | space_amp | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 125 (119–138) | 116 | 1525 | 2.27 | 1.62 | 74.4 |
| lsm | 3 | 166 | 138 | 5599 | 3.45 | 2.08 | 23.1 |
| kerno | 3 | 161 (149–189) | 154 (131–161) | 2927 (2791–3656) | 2.88 | 2.04 | 24.6 |
| kerno@before | 3 | 165 | 148 (129–164) | 3097 (2834–3985) | 2.88 | 2.04 | 24.4 |
| fjall | 3 | 185 | 155 | 1002 | 1.74 | 5.41 (3.24–5.41) | 156 |

Load average (1 min) when the runs started: 1.26–1.41.

#### `mixed` — n = 200000, durability = sync

| engine | reps | mixed_kops | read_p50_us | read_p99_us | write_p50_us | write_p99_us |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 14.9 | 8.65 | 40.1 | 2447 | 6451 |
| lsm | 3 | 35.9 | 7.94 | 41.8 | 928 | 3041 |
| kerno | 3 | 65.9 | 9.85 (8.60–10.4) | 49.7 | 426 | 1334 (1071–1488) |
| kerno@before | 3 | 67.0 (65.1–77.5) | 9.42 | 49.8 (40.5–51.8) | 422 (356–431) | 1660 (878–2062) |
| fjall | 3 | 37.6 (30.1–39.6) | 5.99 (5.74–7.08) | 33.9 (33.1–40.7) | 928 (868–1146) | 3338 (3033–3783) |

Load average (1 min) when the runs started: 1.30–1.66.

#### `history_sim` — n = 200000, durability = sync

| engine | reps | revisions_per_s_k | rev_commit100_p99_us | space_amp | history_scan_p50_us | history_scan_p99_us | reconstruct_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 11.5 | 12935 | 2.04 | 5.15 | 26.5 (26.1–30.9) | 4.16 |
| lsm | 3 | 87.4 | 8368 | 1.88 | 13.0 | 38.8 | 9.07 |
| kerno | 3 | 86.0 | 4870 | 1.68 | 12.7 | 40.0 (31.4–41.8) | 7.07 (6.01–11.0) |
| kerno@before | 3 | 86.3 | 5022 | 1.68 | 12.5 (11.8–18.0) | 37.7 (36.7–53.2) | 6.64 (5.35–9.41) |
| fjall | 3 | 82.2 (56.5–89.6) | 1862 (1717–3361) | 7.74 | 19.1 (18.5–23.9) | 47.6 (45.7–58.0) | 23.5 (17.5–35.4) |

Load average (1 min) when the runs started: 1.24–1.43.

#### `livespec` — n = 200000, durability = sync

| engine | reps | changesets_per_s | changeset_commit_p50_us | changeset_commit_p99_us | symbol_get_p99_us | module_scan_p50_us | module_scan_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 543 | 1292 | 2576 (2255–2675) | 29.2 | 137 | 292 |
| lsm | 3 | 867 | 465 | 4112 (2986–4637) | 28.0 | 157 | 518 (488–587) |
| kerno | 3 | 821 (729–858) | 511 (479–564) | 2265 (2228–3455) | 30.2 | 167 | 587 |
| kerno@before | 3 | 799 | 516 (468–550) | 2605 (1930–3201) | 29.6 | 172 | 575 (520–622) |
| fjall | 3 | 828 | 443 (431–532) | 1035 (942–1306) | 22.5 (19.2–24.5) | 199 | 393 (389–461) |

Load average (1 min) when the runs started: 1.09–1.24.

#### `cauce` — n = 200000, durability = sync

| engine | reps | steps_per_s | step_commit_p50_us | step_commit_p99_us | step_read_p99_us | recovery_ms |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 1453 | 625 | 1323 | 57.5 | 0.82 (0.56–0.89) |
| lsm | 3 | 4003 (3952–4589) | 194 (164–194) | 464 (417–495) | 80.3 (76.0–89.3) | 8.15 (8.04–12.3) |
| kerno | 3 | 3852 | 197 (170–209) | 930 | 81.6 (71.3–127) | 10.6 (9.28–14.0) |
| kerno@before | 3 | 3938 (3390–4072) | 180 (168–211) | 885 (885–1105) | 81.4 | 10.4 (9.88–11.7) |
| fjall | 3 | 4293 | 175 | 497 (455–542) | 68.0 | 340 (332–402) |

Load average (1 min) when the runs started: 1.24–1.36.

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 1449 | 435 | 922 (918–1140) | 34.9 | 48.5 (45.4–65.4) | 6476 (6431–8057) | 3.93 |
| lsm | 3 | 1363 (1278–1630) | 336 (282–348) | 5620 (4248–6500) | 51.0 | 57.4 | 2080 (1725–10306) | 2.63 |
| kerno | 3 | 1420 | 364 | 2314 (1990–2423) | 51.5 | 58.8 | 3551 (2947–3899) | 2.12 (2.12–3.02) |
| kerno@before | 3 | 1438 (1197–1479) | 360 (356–449) | 2257 | 53.9 (45.6–55.7) | 63.2 (48.2–65.0) | 5022 (2761–7557) | 3.02 |
| fjall | 3 | 1944 | 289 | 760 (655–793) | 38.9 | 61.1 | 2714 (2623–3762) | 3.14 |

Load average (1 min) when the runs started: 1.22–1.61.

#### `concurrent_commits` — n = 200000, durability = sync

| engine | reps | cc_threads | cc_commits_per_s | cc_commit_p50_us | cc_commit_p99_us | cc_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 8.00 | 2308 | 2888 | 16163 | 2.00 |
| lsm | 3 | 8.00 | 4600 | 216 | 12673 | 1.00 |
| kerno | 3 | 8.00 | 15295 | 495 | 1179 | 0.22 |
| kerno@before | 3 | 8.00 | 15161 (14255–17203) | 502 (456–551) | 1071 (924–1104) | 0.22 |
| fjall | 3 | 8.00 | 6100 | 142 | 11411 | — |

Load average (1 min) when the runs started: 1.04–1.37.

#### `rmw_counters` — n = 200000, durability = sync

| engine | reps | rmw_txns_per_s | rmw_txn_p50_us | rmw_txn_p99_us | rmw_txn_p999_us | rmw_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 2750 (2259–2771) | 2103 (1383–3152) | 15999 | 21496 | 2.00 |
| lsm | 3 | 4655 (4208–5481) | 212 (173–245) | 12739 | 17908 | 1.00 |
| kerno | 3 | 4133 (3951–4823) | 1883 (1552–1966) | 3453 | 6173 (4067–8389) | 1.00 |
| kerno@before | 3 | 3790 (3649–4500) | 2001 (1634–2111) | 4375 (3887–4805) | 7688 (6234–16490) | 1.00 |
| fjall | 3 | 6752 (6047–7302) | 132 (124–149) | 11772 | 19759 | — |

Load average (1 min) when the runs started: 1.12–1.19.

