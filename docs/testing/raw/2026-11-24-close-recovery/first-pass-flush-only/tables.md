| scale | workload | metric | engine | runs | before (base 9f8c60a) | after (head) |
|---|---|---|---|---|---:|---:|
| 10000 | bulk_load | `close_ms` | kerno | 3 / 3 | 27.1 (26.9-92.1) | 24.5 (20.9-25.8) |
| 10000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 1.9 | 2.1 (2.1-4.4) |
| 10000 | bulk_load | `reopen_ms` | kerno | 3 / 3 | 3.2 (2.7-3.2) | 2.5 (2.3-2.9) |
| 10000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.4) | 0.5 (0.4-0.5) |
| 10000 | bulk_load | `recovery_ms` | kerno | 3 / 3 | 7.2 (7.1-9.4) | 4.8 (3.6-5.3) |
| 10000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.4 (0.4-0.9) | 0.4 (0.4-0.6) |
| 10000 | cauce | `recovery_ms` | kerno | 3 / 3 | 20.6 | 13.7 (8.4-19.1) |
| 10000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.5 (0.4-0.6) | 0.5 (0.4-1.0) |
| 100000 | bulk_load | `close_ms` | kerno | 8 / 8 | 105.2 (38.6-199.6) | 43.0 (31.7-198.0) |
| 100000 | bulk_load | `close_ms` | sqlite | 3 / 3 | 5.6 (4.4-5.9) | 5.5 (4.2-5.6) |
| 100000 | bulk_load | `reopen_ms` | kerno | 8 / 8 | 4.7 (3.7-6.0) | 4.6 (3.9-8.5) |
| 100000 | bulk_load | `reopen_ms` | sqlite | 3 / 3 | 0.5 | 0.5 |
| 100000 | bulk_load | `recovery_ms` | kerno | 8 / 8 | 11.9 (8.2-20.3) | 5.5 (4.4-7.4) |
| 100000 | bulk_load | `recovery_ms` | sqlite | 3 / 3 | 0.5 (0.4-0.5) | 0.6 (0.4-0.8) |
| 100000 | cauce | `recovery_ms` | kerno | 3 / 3 | 30.4 (30.2-57.0) | 26.4 (13.9-27.4) |
| 100000 | cauce | `recovery_ms` | sqlite | 3 / 3 | 0.5 | 0.5 (0.5-0.6) |

Load average (1 min) at the start of each run: min 1.86, max 5.00
