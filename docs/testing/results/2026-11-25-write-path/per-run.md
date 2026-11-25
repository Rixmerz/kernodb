Rounds 1-3 ran base first, rounds 4-6 ran head first.

| n | row | round | first | base kerno s | base sqlite s | head kerno s | head sqlite s | load at start (base / head) |
|---|---|---|---|---|---|---|---|---|
| 10000 | store x10000, batches of 10000 | 1 | base | 0.0971 | 0.06704 | 0.09494 | 0.06138 | 19.14 / 16.42 |
| 10000 | store x10000, batches of 10000 | 2 | base | 0.1011 | 0.06082 | 0.09803 | 0.06135 | 14.04 / 13.16 |
| 10000 | store x10000, batches of 10000 | 3 | base | 0.1035 | 0.06901 | 0.09767 | 0.0606 | 12.34 / 10.67 |
| 10000 | store x10000, batches of 10000 | 4 | head | 0.09775 | 0.06141 | 0.09579 | 0.06547 | 3.98 / 3.77 |
| 10000 | store x10000, batches of 10000 | 5 | head | 0.09455 | 0.06287 | 0.09693 | 0.06401 | 3.60 / 3.74 |
| 10000 | store x10000, batches of 10000 | 6 | head | 0.09893 | 0.07465 | 0.1 | 0.08309 | 3.78 / 3.67 |
| 10000 | update x10000 keeping history, batches | 1 | base | 0.1329 | 0.07602 | 0.1147 | 0.07527 | 19.14 / 16.42 |
| 10000 | update x10000 keeping history, batches | 2 | base | 0.1461 | 0.0874 | 0.1463 | 0.08068 | 14.04 / 13.16 |
| 10000 | update x10000 keeping history, batches | 3 | base | 0.1559 | 0.1148 | 0.1265 | 0.08211 | 12.34 / 10.67 |
| 10000 | update x10000 keeping history, batches | 4 | head | 0.1264 | 0.07741 | 0.1222 | 0.07928 | 3.98 / 3.77 |
| 10000 | update x10000 keeping history, batches | 5 | head | 0.1197 | 0.07943 | 0.1275 | 0.08427 | 3.60 / 3.74 |
| 10000 | update x10000 keeping history, batches | 6 | head | 0.1507 | 0.1211 | 0.1636 | 0.1222 | 3.78 / 3.67 |
| 10000 | store x2000, one commit each | 1 | base | 2.049 | 2.029 | 1.901 | 2.048 | 19.14 / 16.42 |
| 10000 | store x2000, one commit each | 2 | base | 1.858 | 2.293 | 1.929 | 2.204 | 14.04 / 13.16 |
| 10000 | store x2000, one commit each | 3 | base | 2.064 | 2.034 | 1.87 | 2.311 | 12.34 / 10.67 |
| 10000 | store x2000, one commit each | 4 | head | 2.025 | 2.17 | 1.964 | 2.287 | 3.98 / 3.77 |
| 10000 | store x2000, one commit each | 5 | head | 1.773 | 2.22 | 2.439 | 2.121 | 3.60 / 3.74 |
| 10000 | store x2000, one commit each | 6 | head | 2.043 | 2.18 | 2.001 | 2.239 | 3.78 / 3.67 |
| 100000 | store x100000, batches of 10000 | 1 | base | 1.14 | 0.6576 | 1.098 | 0.6442 | 9.98 / 7.02 |
| 100000 | store x100000, batches of 10000 | 2 | base | 1.192 | 0.6459 | 1.098 | 0.6475 | 5.18 / 4.39 |
| 100000 | store x100000, batches of 10000 | 3 | base | 1.037 | 0.6928 | 1.054 | 0.6574 | 4.19 / 3.35 |
| 100000 | store x100000, batches of 10000 | 4 | head | 1.106 | 0.6804 | 1.052 | 0.6938 | 3.31 / 3.58 |
| 100000 | store x100000, batches of 10000 | 5 | head | 1.036 | 0.6363 | 1.074 | 0.6331 | 2.63 / 2.89 |
| 100000 | store x100000, batches of 10000 | 6 | head | 1.061 | 0.6592 | 1.078 | 0.6576 | 2.66 / 2.67 |
| 100000 | update x100000 keeping history, batches | 1 | base | 1.691 | 0.8451 | 1.688 | 0.8082 | 9.98 / 7.02 |
| 100000 | update x100000 keeping history, batches | 2 | base | 1.699 | 0.8077 | 1.717 | 0.8114 | 5.18 / 4.39 |
| 100000 | update x100000 keeping history, batches | 3 | base | 1.621 | 0.8046 | 1.679 | 0.8542 | 4.19 / 3.35 |
| 100000 | update x100000 keeping history, batches | 4 | head | 1.739 | 0.7984 | 1.649 | 0.8735 | 3.31 / 3.58 |
| 100000 | update x100000 keeping history, batches | 5 | head | 1.612 | 0.8143 | 1.682 | 0.8107 | 2.63 / 2.89 |
| 100000 | update x100000 keeping history, batches | 6 | head | 1.714 | 0.8525 | 1.635 | 0.9394 | 2.66 / 2.67 |
| 100000 | store x2000, one commit each | 1 | base | 1.827 | 1.949 | 1.764 | 2.087 | 9.98 / 7.02 |
| 100000 | store x2000, one commit each | 2 | base | 1.837 | 2.079 | 2.322 | 2.065 | 5.18 / 4.39 |
| 100000 | store x2000, one commit each | 3 | base | 2.322 | 2.123 | 2.356 | 2.071 | 4.19 / 3.35 |
| 100000 | store x2000, one commit each | 4 | head | 2.507 | 2.284 | 1.854 | 2.163 | 3.31 / 3.58 |
| 100000 | store x2000, one commit each | 5 | head | 2.371 | 2.139 | 1.822 | 2.048 | 2.63 / 2.89 |
| 100000 | store x2000, one commit each | 6 | head | 1.755 | 1.984 | 1.916 | 2.408 | 2.66 / 2.67 |
