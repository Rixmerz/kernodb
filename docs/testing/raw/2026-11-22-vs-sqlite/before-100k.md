| Workload | Kerno | SQLite | Kerno / SQLite | |
|---|---|---|---|---|
| store x2000, one commit each | 4.01 [3.34-4.38] | 2.43 [2.01-4.31] | 1.65 |  |
| store x100000, batches of 10000 | 2.88 [2.3-3.77] | 0.768 [0.681-1.09] | 3.75 |  |
| point read x2000 | 0.0177 [0.0163-0.0222] | 0.013 [0.0118-0.0153] | 1.36 |  |
| scan all 102000 units of a type | 0.434 [0.399-0.48] | 0.0988 [0.0962-0.105] | 4.39 |  |
| update x100000 keeping history, batches | 3.45 [2.87-9.04] | 0.912 [0.765-3.19] | 3.78 |  |
| relate x100000, batches | 3.88 [3.71-15.2] | 0.522 [0.397-0.63] | 7.42 |  |
| related() x1000 | 0.0277 [0.0266-0.0599] | 0.00478 [0.00464-0.00886] | 5.79 |  |
| two-hop traversal x200 | 0.0148 [0.0134-0.0354] | 0.00179 [0.0015-0.00275] | 8.26 |  |
| all relations of a type | 0.181 [0.176-0.395] | 0.0717 [0.0686-0.115] | 2.52 |  |
| history x1000 | 0.0254 [0.0252-0.0646] | 0.00489 [0.00485-0.00867] | 5.20 |  |
| store x100000 substring-indexed, batches | 2.83 [2.21-8.46] | 1.87 [1.69-6.27] | 1.51 | kerno has no substring index |
| substring search x20 vs LIKE scan | 20 [19.1-29.7] | 0.351 [0.308-0.407] | 57.03 | kerno scan in Python |
| substring search x20 vs FTS5 trigram | 20 [19.1-29.7] | 0.179 [0.163-0.215] | 112.16 | kerno scan in Python |
| substring hits | 186413 [186413-186413] | 186413 [186413-186413] | 1.00 | matches over all needles |
| store x100000 keyword-indexed, batches | 19.8 [12.4-26.6] | 1.05 [0.959-1.33] | 18.86 |  |
| keyword search x100, top 10 | 1.78 [1.6-1.98] | 0.28 [0.265-0.316] | 6.36 |  |
| close | 0.2 [0.188-0.28] | 0.00244 [0.0019-0.00284] | 82.15 |  |
| disk MiB | 207 [207-207] | 162 [162-162] | 1.27 | after close |
| reopen | 0.0303 [0.0191-0.306] | 0.0339 [0.000905-0.0385] | 0.89 |  |
