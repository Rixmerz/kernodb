| scale | workload | metric | engine | runs | before (base 9f8c60a) | after (head) |
|---|---|---|---|---|---:|---:|
| 10000 | bulk_load | `close_ms` | kerno | 3 / 3 | 25.2 (25.1-30.5) | 28.9 (21.0-31.1) |
| 10000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 2.5 (2.0-2.6) | 2.1 (2.0-2.9) |
| 10000 | bulk_load | `reopen_ms` | kerno | 3 / 3 | 2.9 (2.5-3.1) | 2.7 (2.3-3.9) |
| 10000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 | 0.4 |
| 10000 | bulk_load | `recovery_ms` | kerno | 3 / 3 | 9.1 (8.2-10.0) | 4.6 (3.5-6.1) |
| 10000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.6 (0.5-0.9) | 0.5 |
| 10000 | cauce | `recovery_ms` | kerno | 3 / 3 | 19.8 (14.2-22.9) | 10.0 (9.6-13.6) |
| 10000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 (0.4-0.7) |
| 100000 | bulk_load | `close_ms` | kerno | 8 / 8 | 31.5 (27.6-238.8) | 36.2 (26.5-105.0) |
| 100000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 4.8 (4.7-5.6) | 5.4 |
| 100000 | bulk_load | `reopen_ms` | kerno | 8 / 8 | 3.9 | 3.9 (3.6-10.3) |
| 100000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 | 0.5 (0.4-0.9) |
| 100000 | bulk_load | `recovery_ms` | kerno | 8 / 8 | 11.7 (9.7-139.3) | 5.7 (4.5-12.3) |
| 100000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.5 (0.4-1.3) | 0.4 |
| 100000 | cauce | `recovery_ms` | kerno | 3 / 3 | 31.7 (30.9-48.5) | 17.3 (14.0-17.5) |
| 100000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.4 | 0.4 |

Load average (1 min) at the start of each run: min 2.05, max 4.70
