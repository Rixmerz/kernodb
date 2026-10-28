# Objetivos de rendimiento de la Fase 1

| Campo | Valor |
|---|---|
| Estado | **Propuesta pendiente de aprobación del propietario** (C-06; [diseño de la Fase 1](../architecture/phase1-design.md) §13, decisión 1) |
| Fecha de registro | 2026-10-28, en WP04, **antes** de cualquier medición de WP05 |
| Fuente | [diseño de la Fase 1](../architecture/phase1-design.md) §7.2, copiado sin cambios |
| Relacionado | [ADR-0001](../adr/0001-motor-de-almacenamiento.md) (condición 1), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [informe M0](m0-report.md), [diagnóstico de colas](f1-diagnostico-colas.md) |

Estos objetivos se registran antes de medir para que ninguna cifra de WP05 o WP14 pueda elegir su propio umbral a posteriori. Los valores de la tabla son los del diseño (§7.2) y no se modifican en este paquete: cualquier cambio requiere la aprobación del propietario y queda registrado en este documento con su fecha y su motivo.

## Método

- Escala `medium` (200 000 registros), `Sync`, mediana de 3 repeticiones intercaladas, misma sesión y misma máquina con `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` ([ADR-0012](../adr/0012-metodologia-de-benchmarks.md)).
- Valores M0 del 2026-10-27 (VM, 4 núcleos, ext4, `fdatasync` 4 KiB p50 113 / p99 568 µs).
- Las proporciones («≤ 1,2× spike-lsm», «≤ 2,0× fjall») se calculan con motores medidos **en la misma sesión**; nunca contra cifras de otra máquina u otro día. Para comparar con el M0, `spike-bench report --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` etiqueta esas filas como `lsm@m0`, etc., y nunca las mezcla con las nuevas.
- Las mediciones de WP04 y WP05 se hacen con otros paquetes compilando y probando en la misma máquina: son **provisionales**. La medición de referencia es la de WP14, con la máquina en reposo (§11 del diseño).

## Objetivos

| Id | Métrica | M0 | Objetivo |
|---|---|---|---|
| T1 | `agent_memory` p99 de escritura de memoria | LSM 5 837 µs; B+ tree 1 573; fjall 1 090 (3,71× / 5,36×) | ≤ 2,0× fjall **y** ≤ 1,5× spike-btree |
| T2 | p99 de commit: LiveSpec / Cauce / `append_seq` inserción / `append_seq` actualización / `history_sim` | 4 764 / 713 / 4 139 / 6 840 / 9 118 µs | ≤ 1,2× spike-lsm; además `append_seq` actualización ≤ 2× fjall (M0 2,60×); incumplimientos documentados |
| T3 | p99 de lectura: LiveSpec module-scan / symbol-get, Cauce, recall, recent-scan | 657 / 43,7 / 98,0 / 100 / 234 µs | ≤ 1,2× spike-lsm; LiveSpec module-scan ≤ 1,5× spike-btree (M0 1,49×), si no, revisión de la condición 5 |
| T4 | Cierre limpio tras `bulk_load` | 759 ms (`medium`), 3 587 ms (`large`); fjall 96 / 919 | ≤ 250 ms y ≤ 1 000 ms con el presupuesto por defecto |
| T5 | Rendimiento de cada carga | p. ej. 4 044 commits/s, carga 162 kops/s, lecturas 230 kops/s | ≥ 0,9× spike-lsm |
| T6 | `fsync` por commit | 1,00 | 1 escritor = 1,00; `concurrent_commits` con 8 escritores ≤ 0,5 y ≥ 2× el rendimiento de 1 escritor |
| T7 | Amplificación de escritura `small_commits` / `bulk_load` | 1,09× / 3,4× | ≤ 1,20× (la cabecera de trama añade 16 B por commit) / ≤ 3,6× |
| T8 | Reapertura limpia / recuperación sucia | 3,4 ms / 9,73 ms | ≤ 1,0× / ≤ 1,2× spike-lsm |
| T9 | Coste RMW (`rmw_counters`) | — | se informa (R-S6) |
| T10 | Rendimiento frente a fjall | 0,8–1,3× | se informa; incumplimientos de 2× documentados (C-07) |

## Cómo se mide cada objetivo

| Id | Carga (`spike-bench`) | Columnas |
|---|---|---|
| T1 | `agent_memory` | `memory_write_p99_us` |
| T2 | `livespec`, `cauce`, `append_seq`, `history_sim` | `changeset_commit_p99_us`, `step_commit_p99_us`, `append_commit100_p99_us`, `update_commit100_p99_us`, `rev_commit100_p99_us` |
| T3 | `livespec`, `cauce`, `agent_memory` | `module_scan_p99_us`, `symbol_get_p99_us`, `step_read_p99_us`, `recall_p99_us`, `recent_scan_p99_us` |
| T4 | `bulk_load` (`medium` y `large`) | `close_ms` |
| T5 | todas | la columna de rendimiento de cada carga (`commits_per_s`, `load_kops`, `get_1t_kops`, …) |
| T6 | `small_commits`; `concurrent_commits --threads 1` y `--threads 8` | `commit_fsyncs_per_commit`; `cc_fsyncs_per_commit`, `cc_commits_per_s` |
| T7 | `small_commits`, `bulk_load` | `write_amp` |
| T8 | `bulk_load` | `reopen_ms`, `recovery_ms` |
| T9 | `rmw_counters` (WP13a) | — |
| T10 | todas | proporción `kerno` / `fjall` de la columna de rendimiento |

`concurrent_commits` (WP04) ejecuta 20 000 commits `Sync` pequeños (un `put` de 200 bytes con claves disjuntas) repartidos entre `--threads` hilos; con 8 escritores, T6 solo puede cumplirse cuando exista group commit (WP12).

## Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| — | pendiente | El propietario aprueba o ajusta T1–T10 antes de que WP05 mida (diseño §13, decisión 1). |
