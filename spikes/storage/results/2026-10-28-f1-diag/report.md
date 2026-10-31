# WP04: diagnóstico de colas — tablas

Generado con `spike-bench report` (mediana; entre paréntesis el rango cuando supera el 15 %). Análisis en [`docs/testing/f1-diagnostico-colas.md`](../../../../docs/testing/f1-diagnostico-colas.md). Todas las ejecuciones en la máquina compartida del 2026-10-28 (`env.json`; `runs.log` registra `uptime` antes y después de cada invocación): **provisionales**.

## 1. Diagnóstico, 3 repeticiones (`results.jsonl`)

Las filas `@m0` son las del M0 (`spikes/storage/results/2026-10-27/results.jsonl`, otra máquina y otro día: solo orientativas), filtradas a estas cargas y motores con
`jq -c 'select(.n==200000 and (.workload|IN("agent_memory","livespec","cauce","append_seq")) and (.engine|IN("lsm","btree","fjall")) and .durability=="sync" and .cache=="warm")'`
y añadidas con `spike-bench report --input results.jsonl --baseline <filtradas> --baseline-label m0`.

#### `append_seq` — n = 200000, durability = sync

| engine | reps | append_kops | update_kops | update_commit100_p99_us | write_amp | space_amp | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 161 (82.4–177) | 136 (131–160) | 1556 (935–1696) | 2.27 | 1.62 | 74.1 |
| btree@m0 | 3 | 102 | 92.9 | 2306 | 2.27 | 1.62 | 73.9 |
| lsm | 3 | 210 (172–218) | 175 | 5231 | 3.45 | 2.08 | 22.6 |
| lsm@m0 | 3 | 135 | 115 | 6840 | 3.45 | 2.08 | 21.8 |
| kerno | 3 | 207 | 197 | 5018 | 3.45 | 2.08 | 22.0 |
| fjall | 3 | 254 | 214 (190–230) | 728 (676–856) | 1.74 | 5.41 (3.24–5.41) | 155 |
| fjall@m0 | 3 | 140 | 86.5 (84.0–98.1) | 2634 (1890–2675) | 1.74 | 3.24 | 155 |

Load average (1 min) when the runs started: 1.95–2.35.

#### `livespec` — n = 200000, durability = sync

| engine | reps | changesets_per_s | changeset_commit_p50_us | changeset_commit_p99_us | symbol_get_p99_us | module_scan_p50_us | module_scan_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 3 | 458 (191–613) | 1210 (1184–5300) | 7795 (2161–12394) | 26.7 (23.5–28.3) | 130 (123–150) | 263 (234–278) |
| btree@m0 | 3 | 408 | 1676 | 3762 (3383–4022) | 42.8 | 211 | 440 |
| lsm | 3 | 1011 | 385 (351–422) | 4168 (3541–5177) | 24.9 | 141 | 450 (440–522) |
| lsm@m0 | 3 | 667 | 572 | 4764 (4456–5448) | 43.7 | 202 | 657 |
| kerno | 3 | 946 (460–982) | 382 (380–445) | 4452 (4375–9060) | 25.3 (24.2–28.3) | 151 | 510 (306–531) |
| fjall | 3 | 1006 | 334 (325–407) | 780 (618–1052) | 20.4 (17.5–21.0) | 186 | 356 |
| fjall@m0 | 3 | 522 | 716 | 2425 (1605–2478) | 60.0 (58.8–68.8) | 295 | 1296 |

Load average (1 min) when the runs started: 1.77–3.06.

#### `cauce` — n = 200000, durability = sync

| engine | reps | steps_per_s | step_commit_p50_us | step_commit_p99_us | step_read_p99_us | recovery_ms |
|---|---|---:|---:|---:|---:|---:|
| btree | 3 | 1759 (1482–1802) | 498 | 1148 (1072–7959) | 44.6 | 0.53 (0.45–0.65) |
| btree@m0 | 3 | 1111 | 818 | 1966 | 76.1 | 0.94 (0.83–1.07) |
| lsm | 3 | 5241 | 131 | 409 (347–441) | 61.6 (52.5–66.6) | 8.36 (7.36–9.84) |
| lsm@m0 | 3 | 3087 | 246 | 713 | 98.0 | 9.73 |
| kerno | 3 | 5144 | 137 | 412 (384–587) | 62.5 | 7.96 (7.42–10.0) |
| fjall | 3 | 5321 (3228–5386) | 143 | 360 (334–3949) | 68.8 | 330 |
| fjall@m0 | 3 | 2941 | 250 | 658 | 118 (91.8–165) | 363 (350–453) |

Load average (1 min) when the runs started: 1.95–2.67.

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 3 | 1687 | 370 | 807 (782–3314) | 29.5 | 39.3 | 5706 (5030–5968) | 3.93 |
| btree@m0 | 3 | 1073 | 622 | 1573 (1334–1662) | 49.4 (48.2–56.0) | 72.1 (71.7–114) | 8430 (7590–9740) | 3.93 |
| lsm | 3 | 1450 (803–1454) | 255 (240–306) | 5276 (5194–7770) | 51.1 | 59.9 (40.5–60.2) | 6164 (5235–8569) | 2.63 (1.78–2.80) |
| lsm@m0 | 3 | 1037 | 478 (414–486) | 5837 (5284–6877) | 100 (97.7–114) | 234 (106–260) | 7369 (4690–11985) | 2.64 |
| kerno | 3 | 1506 (1437–1778) | 245 (237–320) | 4997 (4502–7471) | 47.5 | 49.9 | 8708 (1757–9912) | 2.54 (1.75–2.63) |
| fjall | 3 | 2479 | 205 (194–227) | 454 | 33.7 | 52.6 | 2787 (1962–5214) | 3.14 |
| fjall@m0 | 3 | 1279 | 433 | 1090 (1077–1510) | 72.5 (62.8–81.2) | 153 (105–206) | 6476 | 3.14 |

Load average (1 min) when the runs started: 1.35–1.85.


## 2. Diagnóstico, 5 repeticiones (`results-5reps.jsonl`)

#### `append_seq` — n = 200000, durability = sync

| engine | reps | append_kops | update_kops | update_commit100_p99_us | write_amp | space_amp | peak_rss_mb |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 5 | 124 (15.1–136) | 106 (57.8–131) | 2026 (1215–11944) | 2.27 | 1.62 | 74.1 |
| lsm | 5 | 185 (181–222) | 161 (139–180) | 5640 (5226–7487) | 3.45 | 2.08 | 22.6 |
| kerno | 5 | 201 (179–216) | 152 (59.8–181) | 5661 (5427–8290) | 3.45 | 2.08 | 22.1 |
| fjall | 5 | 209 (125–224) | 192 (94.8–199) | 946 (773–4207) | 1.74 | 5.41 (3.24–5.41) | 155 |

Load average (1 min) when the runs started: 2.03–3.11.

#### `livespec` — n = 200000, durability = sync

| engine | reps | changesets_per_s | changeset_commit_p50_us | changeset_commit_p99_us | symbol_get_p99_us | module_scan_p50_us | module_scan_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| btree | 5 | 553 (473–609) | 1265 | 2523 (2324–7541) | 28.1 (25.6–32.3) | 142 (127–149) | 298 (266–323) |
| lsm | 5 | 858 (265–912) | 422 (397–3340) | 4084 (3123–10371) | 26.9 | 161 (155–181) | 451 (392–625) |
| kerno | 5 | 904 (267–992) | 403 (356–3295) | 4641 (3062–12214) | 26.4 | 154 (145–179) | 534 (356–596) |
| fjall | 5 | 898 | 387 (354–426) | 1505 (1170–2947) | 20.9 | 196 | 385 (359–433) |

Load average (1 min) when the runs started: 4.69–8.22.

#### `cauce` — n = 200000, durability = sync

| engine | reps | steps_per_s | step_commit_p50_us | step_commit_p99_us | step_read_p99_us | recovery_ms |
|---|---|---:|---:|---:|---:|---:|
| btree | 5 | 1289 (1160–1822) | 560 (472–578) | 7950 (1084–7963) | 52.9 (50.2–61.1) | 0.70 (0.44–0.89) |
| lsm | 5 | 4056 (3869–5259) | 175 (138–176) | 622 (339–1606) | 82.8 (75.5–88.4) | 8.82 (7.42–14.1) |
| kerno | 5 | 4533 (2371–4773) | 162 | 452 (433–3979) | 81.4 (72.5–89.3) | 11.4 (8.00–45.0) |
| fjall | 5 | 4356 (1078–4818) | 161 (144–168) | 457 (375–5276) | 72.5 (65.6–80.9) | 334 (312–465) |

Load average (1 min) when the runs started: 2.52–4.39.

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| btree | 5 | 1271 (418–1577) | 417 (403–610) | 7778 (947–9921) | 33.6 (31.2–44.9) | 50.1 (42.0–61.4) | 12526 (5992–38633) | 3.93 |
| lsm | 5 | 1384 (732–1599) | 312 (248–339) | 6877 (4264–8561) | 49.0 (42.5–60.4) | 61.3 (40.4–71.1) | 8180 (2220–12345) | 2.63 (1.79–2.83) |
| kerno | 5 | 1402 (328–1425) | 317 (284–1571) | 6681 (4731–9363) | 52.3 (50.1–63.3) | 61.3 (57.8–69.8) | 4485 (1999–12165) | 2.64 (2.15–2.83) |
| fjall | 5 | 1996 (598–2059) | 260 (214–447) | 1314 (644–7786) | 43.1 (39.2–50.3) | 66.9 (64.9–89.7) | 4391 (2740–9077) | 3.15 |

Load average (1 min) when the runs started: 2.31–7.87.


## 3. A/B con `kerno-tuned`, 3 repeticiones (`ab.jsonl`), una sesión por variante

### Sesión `KERNO_TUNING=slowdown=off`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 3 | 1577 | 271 (268–309) | 5210 | 45.8 (44.6–52.8) | 51.0 (45.8–57.2) | 9445 | 2.63 (2.11–2.83) |
| kerno-tuned[slowdown=off] | 3 | 1520 (1140–1608) | 274 (249–294) | 5992 (4674–7758) | 47.6 | 53.3 (46.8–67.5) | 8086 (1691–8163) | 2.54 (1.74–2.64) |

Load average (1 min) when the runs started: 1.89–2.07.


### Sesión `KERNO_TUNING=bytes_per_sync=1048576`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 3 | 1695 (1155–1759) | 253 (226–268) | 4936 (4854–7758) | 44.1 | 48.4 (40.9–53.2) | 8151 (1780–9159) | 2.61 |
| kerno-tuned[bytes_per_sync=1048576] | 3 | 1433 (1200–1490) | 299 (292–360) | 3013 (2376–4428) | 46.8 (46.0–54.3) | 52.4 (45.5–60.6) | 4841 | 2.10 (1.36–2.64) |

Load average (1 min) when the runs started: 1.98–2.26.


### Sesión `KERNO_TUNING=l0_slowdown=20,l0_stop=36`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 3 | 1209 (429–1515) | 291 (277–467) | 6324 (5612–8196) | 46.9 | 46.3 (45.1–56.9) | 8823 (5771–13574) | 2.46 (2.11–2.83) |
| kerno-tuned[l0_slowdown=20,l0_stop=36] | 3 | 666 (254–1244) | 344 (250–3742) | 8131 (6947–11379) | 49.4 (43.0–82.2) | 47.6 (43.3–87.0) | 8335 (4223–8585) | 2.46 (1.78–2.80) |

Load average (1 min) when the runs started: 2.29–3.53.


### Sesión `KERNO_TUNING=slowdown=off,bytes_per_sync=1048576,l0_slowdown=12`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 3 | 1470 (398–1564) | 287 (278–547) | 5722 (4633–10641) | 46.4 (46.0–56.1) | 50.9 | 8569 (1651–34537) | 2.67 (1.75–2.83) |
| kerno-tuned[slowdown=off,bytes_per_sync=1048576,l0_slowdown=12] | 3 | 941 (774–1274) | 331 (278–346) | 5300 (2249–7741) | 50.4 | 52.0 (49.5–57.5) | 6038 (4260–8069) | 1.35 |

Load average (1 min) when the runs started: 4.69–6.33.


## 4. A/B con `kerno-tuned`, 5 repeticiones (`ab-5reps.jsonl`)

### Sesión `KERNO_TUNING=bytes_per_sync=1048576`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 5 | 368 (218–698) | 614 (372–3736) | 11338 (8954–15720) | 60.1 (56.6–90.7) | 69.2 (53.7–78.8) | 8430 (7541–26821) | 2.30 (1.35–2.67) |
| kerno-tuned[bytes_per_sync=1048576] | 5 | 277 (216–319) | 3453 (1992–3746) | 11747 | 61.9 (53.2–79.8) | 68.0 (58.3–77.6) | 11944 (9740–36012) | 2.65 (2.30–3.57) |

Load average (1 min) when the runs started: 3.24–12.83.


### Sesión `KERNO_TUNING=slowdown=off,bytes_per_sync=1048576,l0_slowdown=12`

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 5 | 350 (243–779) | 784 (281–3738) | 11674 (9527–12993) | 55.7 (49.2–74.6) | 58.1 (55.2–71.0) | 15892 (9781–46137) | 1.75 (1.73–2.63) |
| kerno-tuned[slowdown=off,bytes_per_sync=1048576,l0_slowdown=12] | 5 | 531 (204–892) | 340 (312–3736) | 9871 (6537–13402) | 57.1 (49.0–64.3) | 65.7 (58.3–70.0) | 8188 (5497–16638) | 2.47 (1.35–2.67) |

Load average (1 min) when the runs started: 8.00–14.41.


## 5. Ejecuciones con traza (`--trace true`; `trace.jsonl`, `trace-5reps.jsonl`)

Las columnas `attr_*` (atribución de la cola) están en los JSON; aquí solo las de cabecera.

### 3 repeticiones

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 3 | 1384 (346–1409) | 303 (298–1630) | 4796 (4030–9486) | 49.6 (42.3–49.9) | 47.0 (42.5–55.8) | 6795 (3934–8602) | 2.64 |
| kerno-tuned[bytes_per_sync=1048576] | 3 | 444 (304–1244) | 694 (332–3621) | 7692 (3783–8176) | 52.9 | 53.5 | 10519 (5280–11715) | 2.64 (2.10–2.67) |

Load average (1 min) when the runs started: 4.90–5.61.


### 5 repeticiones

#### `agent_memory` — n = 200000, durability = sync

| engine | reps | steps_per_s | memory_write_p50_us | memory_write_p99_us | recall_p99_us | recent_scan_p99_us | expire_commit_p99_us | space_amp |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| kerno | 5 | 392 (345–1383) | 429 (235–2404) | 10101 (4563–11477) | 54.7 (50.0–79.6) | 64.2 (54.9–220) | 12583 (2238–36012) | 2.63 |
| kerno-tuned[bytes_per_sync=1048576] | 5 | 488 (355–1115) | 449 (286–497) | 7959 (4215–8864) | 53.3 | 65.1 (49.9–66.1) | 10346 (4045–12034) | 2.63 (2.10–2.67) |

Load average (1 min) when the runs started: 10.00–12.72.


## 6. Coste de la instrumentación (`overhead.jsonl`, `small`)

Nota: en `small_commits` la repetición 0 de los dos motores es atípica (1144 y 1254 commits/s, frente a unos 6500–6900 en las otras), con una dispersión de commits/s del 89 % (`lsm`) y del 82 % (`kerno`), por encima del 15 % de la regla de riesgo. Esta ejecución no basta para dar por cumplido el criterio ≥ 0,97×; el veredicto queda para WP14 (véase `docs/testing/f1-diagnostico-colas.md`).

#### `point_reads` — n = 20000, durability = sync

| engine | reps | get_1t_kops | get_1t_p50_us | get_1t_p95_us | get_1t_p99_us | get_4t_kops | get_4t_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| lsm | 3 | 1336 | 0.63 | 0.90 | 1.11 | 2855 | 1.36 |
| kerno | 3 | 1330 | 0.64 | 0.92 | 1.14 (1.06–1.26) | 3057 | 1.23 |

Load average (1 min) when the runs started: 4.73–4.88.

#### `small_commits` — n = 20000, durability = sync

| engine | reps | commits_per_s | commit_p50_us | commit_p95_us | commit_p99_us | commit_p999_us | write_amp | commit_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| lsm | 3 | 6465 (1144–6887) | 133 | 251 (241–4002) | 352 (347–4375) | 1097 (715–8135) | 1.09 | 1.00 |
| kerno | 3 | 6672 (1254–6694) | 134 | 240 (216–4002) | 340 (340–6185) | 1951 (1095–9019) | 1.09 | 1.00 |

Load average (1 min) when the runs started: 4.88–5.17.


### 6.1 Repetición sobre el commit final del paquete

`e1575d3` (WP04 integrado con WP03a; el código del motor no cambia desde `b742393`), con la máquina más cargada (1 min: 9,8–12,2). Con esa carga la comparación es sobre todo ruido; la medición de referencia de este criterio es la de WP14.

3 repeticiones (`overhead-final.jsonl`):


##### `point_reads` — n = 20000, durability = sync

| engine | reps | get_1t_kops | get_1t_p50_us | get_1t_p95_us | get_1t_p99_us | get_4t_kops | get_4t_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| lsm | 3 | 1127 (969–1149) | 0.72 | 1.06 | 1.37 (1.24–1.52) | 1346 (1005–1558) | 1.53 (1.50–1.86) |
| kerno | 3 | 1039 | 0.74 | 1.08 | 1.40 | 1160 (921–1770) | 1.54 (1.37–1.77) |

Load average (1 min) when the runs started: 10.59–10.59.

##### `small_commits` — n = 20000, durability = sync

| engine | reps | commits_per_s | commit_p50_us | commit_p95_us | commit_p99_us | commit_p999_us | write_amp | commit_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| lsm | 3 | 3313 (1823–3354) | 168 | 671 (629–3994) | 3998 | 5665 (5210–7324) | 1.09 | 1.00 |
| kerno | 3 | 4316 (2659–5211) | 144 | 416 (367–1724) | 1963 (1026–4006) | 4248 (3426–7987) | 1.09 | 1.00 |

Load average (1 min) when the runs started: 10.73–12.23.

5 repeticiones (`overhead-final-5reps.jsonl`; regla de riesgo: dispersión > 15 % en la anterior):


##### `point_reads` — n = 20000, durability = sync

| engine | reps | get_1t_kops | get_1t_p50_us | get_1t_p95_us | get_1t_p99_us | get_4t_kops | get_4t_p99_us |
|---|---|---:|---:|---:|---:|---:|---:|
| lsm | 5 | 837 (718–967) | 0.79 | 1.25 (1.21–1.42) | 1.68 (1.58–2.77) | 1446 (1087–1583) | 1.83 (1.47–4.14) |
| kerno | 5 | 981 (823–1146) | 0.76 | 1.20 (1.05–1.29) | 1.64 (1.35–1.69) | 1578 (1175–1676) | 1.59 (1.52–2.33) |

Load average (1 min) when the runs started: 9.86–10.19.

##### `small_commits` — n = 20000, durability = sync

| engine | reps | commits_per_s | commit_p50_us | commit_p95_us | commit_p99_us | commit_p999_us | write_amp | commit_fsyncs_per_commit |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| lsm | 5 | 735 (405–3215) | 268 (203–3574) | 3998 (608–4256) | 6320 (3150–8008) | 17105 (5419–200278) | 1.09 | 1.00 |
| kerno | 5 | 867 (426–4728) | 296 (168–1760) | 4127 (361–5214) | 7746 (567–8471) | 12100 (3697–34505) | 1.09 | 1.00 |

Load average (1 min) when the runs started: 9.79–12.02.

