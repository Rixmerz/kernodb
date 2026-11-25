| scale | workload | metric | engine | runs | before (base 9f8c60a) | after (head) |
|---|---|---|---|---|---:|---:|
| 10000 | bulk_load | `close_ms` | kerno | 3 / 3 | 25.1 (24.9-30.1) | 24.5 |
| 10000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 2.8 (2.5-3.3) | 2.4 (2.1-2.8) |
| 10000 | bulk_load | `reopen_ms` | kerno | 3 / 3 | 2.8 | 2.7 |
| 10000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 | 0.4 |
| 10000 | bulk_load | `recovery_ms` | kerno | 3 / 3 | 8.7 (8.2-10.3) | 3.7 (3.6-4.2) |
| 10000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.8) | 0.4 (0.4-0.6) |
| 10000 | cauce | `recovery_ms` | kerno | 3 / 3 | 19.9 (15.1-22.1) | 10.8 (8.8-24.1) |
| 10000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.5) | 0.4 (0.4-0.5) |
| 100000 | bulk_load | `close_ms` | kerno | 8 / 8 | 51.4 (24.9-198.5) | 42.3 (30.4-187.6) |
| 100000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 5.4 (5.3-6.2) | 5.6 (5.5-8.7) |
| 100000 | bulk_load | `reopen_ms` | kerno | 8 / 8 | 4.0 (3.6-17.8) | 4.4 (3.7-10.1) |
| 100000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.5 (0.4-1.6) | 0.4 (0.4-0.6) |
| 100000 | bulk_load | `recovery_ms` | kerno | 8 / 8 | 10.6 (7.6-43.7) | 5.2 (5.0-6.4) |
| 100000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.4 | 0.4 (0.4-0.5) |
| 100000 | cauce | `recovery_ms` | kerno | 3 / 3 | 36.5 (34.4-41.1) | 14.6 |
| 100000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.5 (0.4-0.5) | 0.5 |

Load average (1 min) at the start of each run: min 2.05, max 13.81
