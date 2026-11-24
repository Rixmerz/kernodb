| scale | workload | metric | engine | runs | before (base 9f8c60a) | after (head) |
|---|---|---|---|---|---:|---:|
| 10000 | bulk_load | `close_ms` | kerno | 3 / 3 | 23.5 (22.1-34.2) | 22.9 |
| 10000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 2.1 (1.9-2.8) | 2.8 |
| 10000 | bulk_load | `reopen_ms` | kerno | 3 / 3 | 2.6 (2.5-3.3) | 2.3 |
| 10000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 |
| 10000 | bulk_load | `recovery_ms` | kerno | 3 / 3 | 7.6 (6.1-13.6) | 6.7 (4.3-10.1) |
| 10000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.4 | 0.5 (0.4-0.9) |
| 10000 | cauce | `recovery_ms` | kerno | 3 / 3 | 18.8 | 8.5 (7.6-9.2) |
| 10000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 (0.4-0.5) |
| 100000 | bulk_load | `close_ms` | kerno | 8 / 8 | 40.1 (27.6-203.6) | 34.3 (28.0-52.2) |
| 100000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 4.7 (4.1-5.5) | 4.5 (4.4-6.7) |
| 100000 | bulk_load | `reopen_ms` | kerno | 8 / 8 | 3.6 (3.5-4.6) | 3.8 (3.6-4.8) |
| 100000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 (0.4-2.1) |
| 100000 | bulk_load | `recovery_ms` | kerno | 8 / 8 | 10.3 (9.2-19.4) | 5.6 (4.3-8.9) |
| 100000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.5 (0.4-0.7) | 0.8 (0.4-0.9) |
| 100000 | cauce | `recovery_ms` | kerno | 3 / 3 | 33.3 (25.8-54.5) | 16.1 (13.3-17.1) |
| 100000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 |

Load average (1 min) at the start of each run: min 1.74, max 3.19
