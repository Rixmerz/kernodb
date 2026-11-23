| Workload | Kerno | SQLite | Kerno / SQLite | |
|---|---|---|---|---|
| store x2000, one commit each | 6.2 [6.02-6.88] | 7.69 [5.91-8.46] | 0.81 |  |
| store x10000, batches of 10000 | 1.02 [0.926-1.28] | 0.217 [0.174-0.34] | 4.71 |  |
| point read x2000 | 0.0295 [0.0275-0.081] | 0.0189 [0.0181-0.0612] | 1.56 |  |
| scan all 12000 units of a type | 0.136 [0.125-0.327] | 0.047 [0.0224-0.112] | 2.90 |  |
| update x10000 keeping history, batches | 0.976 [0.755-2.19] | 0.476 [0.24-0.672] | 2.05 |  |
| relate x10000, batches | 1.15 [0.548-2.06] | 0.0556 [0.0555-0.163] | 20.67 |  |
| related() x1000 | 0.0521 [0.0363-0.107] | 0.0158 [0.00712-0.0202] | 3.30 |  |
| two-hop traversal x200 | 0.0123 [0.0113-0.0503] | 0.00194 [0.00185-0.0187] | 6.33 |  |
| all relations of a type | 0.0506 [0.0359-0.149] | 0.0181 [0.00851-0.0301] | 2.80 |  |
| history x1000 | 0.0341 [0.0141-0.0692] | 0.0277 [0.00933-0.0957] | 1.23 |  |
| store x10000 substring-indexed, batches | 0.826 [0.576-1.41] | 0.55 [0.355-0.66] | 1.50 | kerno has no substring index |
| substring search x20 vs LIKE scan | 6.58 [3.56-9.62] | 0.116 [0.0346-0.124] | 56.75 | kerno scan in Python |
| substring search x20 vs FTS5 trigram | 6.58 [3.56-9.62] | 0.0687 [0.0209-0.084] | 95.87 | kerno scan in Python |
| substring hits | 18426 [18426-18426] | 18426 [18426-18426] | 1.00 | matches over all needles |
| store x10000 keyword-indexed, batches | 10.4 [2.34-11.4] | 0.566 [0.159-0.572] | 18.33 |  |
| keyword search x100, top 10 | 0.467 [0.271-0.907] | 0.124 [0.0661-0.223] | 3.75 |  |
| close | 1.07 [0.599-1.47] | 0.00386 [0.0025-0.00553] | 278.05 |  |
| disk MiB | 22.2 [22.1-22.2] | 17 [17-17] | 1.31 | after close |
| reopen | 0.0276 [0.0253-0.0386] | 0.0081 [0.00237-0.0245] | 3.41 |  |
