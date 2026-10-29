# Informe de la Fase 1 — salida (WP14)

Estado: resultados del 2026-10-29 · Relacionado con: [ADR-0001](../adr/0001-motor-de-almacenamiento.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md), [objetivos T1–T10](f1-objetivos-rendimiento.md), [decisión de la condición 1](f1-decision-cond1.md), [informe de M0](m0-report.md), [plan de pruebas](test-plan.md), [matriz de fallos](../architecture/failure-matrix.md)

Este informe recoge la evidencia de salida de la Fase 1 (SAD §16): primero la **correctitud**, después el **rendimiento** frente a T1–T10, y al final lo que queda **pendiente o abierto**. Los criterios de salida que no se cumplen o no se pudieron ejecutar se dicen como tales; ninguno se da por cumplido por omisión.

## 1. Resumen de las puertas de salida

| Puerta | Resultado | Evidencia |
|---|---|---|
| `cargo fmt`, `clippy -D warnings`, `cargo test --workspace --all-features --release` | Verificados en el cierre de este paquete (ver §6) | comandos de §6 |
| `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture` | **Cumple**: ninguna prueba fallida; `kerno` con 0 «inocuos» y 0 «datos erróneos» | [`tests-seeds1000.log`](../../spikes/storage/results/2026-10-29-f1/tests-seeds1000.log) |
| `kill -9` real de un escritor (`StdVfs`), `KERNO_KILL_ITERS=200` | **Cumple**: 200 muertes en `Sync` y 200 en `Buffered`, 2 pruebas en verde | §3 |
| LazyFS (dispositivo con pérdida de páginas) | **Pendiente**: sin registro de ejecución | §4 |
| Benchmarks en la máquina objetivo, T1–T10 | **Ejecutados en la clase de VM de M0** (la máquina objetivo, C-09, no está nombrada). **T1 no se cumple**, y hay otros incumplimientos documentados en §5 | [`results.jsonl`](../../spikes/storage/results/2026-10-29-f1/results.jsonl), [`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md), [`env.json`](../../spikes/storage/results/2026-10-29-f1/env.json) |
| Congelación del formato v1 (ADR-0013 aceptado, bit `DRAFT` retirado) | **Congelación pendiente**: ADR-0013 sigue «Propuesto» | §7 |

## 2. Correctitud con 1000 semillas

Comando: `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture 2>&1 | tee spikes/storage/results/2026-10-29-f1/tests-seeds1000.log`, sobre el commit `50583e2`. Todas las líneas `test result:` del log son `ok` y con 0 fallos (el log contiene mensajes de `panicked`, que son los de las autopruebas del arnés que comprueban que un defecto inyectado se detecta; ninguna prueba termina en `FAILED`).

**Outcomes de corrupción del motor del producto (`kerno`, 1000 semillas por variante):**

| Variante | Truncamientos | Bit flips |
|---|---|---|
| `kerno` (por defecto) | 1000 detectados al abrir · 0 al leer · **0 inocuos** | 256 al abrir · 744 al leer · **0 inocuos** |
| `kerno-inline` | 1000 · 0 · **0** | 260 · 740 · **0** |
| `kerno-serial` | 1000 · 0 · **0** | 269 · 731 · **0** |
| `kerno-m0` (perfil M0) | 1000 · 0 · **0** | 245 · 755 · **0** |

Ningún dato erróneo ni retroceso. En los barridos de vectores de directorio (`damage`), de truncamientos y de bit flips sobre archivos del formato v1 —varios cientos de miles de ejecuciones por escenario— todas las ejecuciones se informaron: 0 inocuas y 0 con datos erróneos (por ejemplo, `bit flips basic-clean: 89880 runs, 89880 reported, 0 harmless, 0 wrong`). El único «inocuo» no nulo del log es el del motor de referencia `log` (9 de 1000), que no es el motor del producto.

## 3. `kill -9` real

`KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill -- --nocapture`:

```
process_kill sync: 200 kills (152 after progress), recovered sequence 13142
test kill_dash_9_writer_recovers_acked_prefix_sync ... ok
process_kill buffered: 200 kills (152 after progress), recovered sequence 89534
test kill_dash_9_writer_recovers_acked_prefix_buffered ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

En ambas pruebas, tras cada `SIGKILL` la base se reabre, la secuencia recuperada es ≥ la última confirmada por el escritor y el contenido coincide con el modelo. (En la pasada de 1000 semillas del log se ejecutó con las 20 iteraciones por defecto.)

## 4. LazyFS: pendiente

`crates/kerno-storage/tests/lazyfs.rs` y `spikes/storage/run-lazyfs.sh` existen, y `.github/workflows/lazyfs.yml` (manual y semanal) está añadido para que lo ejecute el propietario. **No se ha registrado ninguna ejecución**: en el log de 1000 semillas las dos pruebas aparecen como `ignored, needs a LazyFS mount`, y en este entorno no hay LazyFS instalado ni compilado (existe `/dev/fuse` y `fusermount`, pero no el binario de LazyFS, que habría que obtener y compilar desde fuera del repositorio). El criterio de salida «escenario A sobre un dispositivo con pérdida de páginas» queda **pendiente y no se marca como cumplido**; hasta que exista el registro bajo `spikes/storage/results/<fecha>-f1/`, la evidencia equivalente es `SimVfs` (corte de energía simulado) más el `kill -9` real de §3, que no pierde páginas del caché del sistema operativo.

## 5. Rendimiento frente a T1–T10

### 5.1 Entorno y método

| | |
|---|---|
| Máquina | La misma clase de VM que M0: Intel Xeon 2,80 GHz, 4 núcleos, 15,7 GiB, ext4, kernel 6.18.44 ([`env.json`](../../spikes/storage/results/2026-10-29-f1/env.json)). **No es la máquina objetivo**: C-09 sigue sin nombrar |
| `fdatasync` de 4 KiB | p50 173 µs · p95 326 µs · p99 410 µs (M0: 113 / 400 / 568 µs) |
| Compilación | `rustc 1.97.0`, `release`, commit `50583e2` |
| Método | [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) y [`f1-objetivos-rendimiento.md`](f1-objetivos-rendimiento.md): `medium` (200 000 registros), `Sync`, mediana de 3 repeticiones intercaladas, `ENGINES=btree,lsm,kerno,redb,fjall,sqlite`. Las proporciones con `lsm` y `fjall` usan motores medidos **en esta misma sesión**; las filas `@m0` del informe solo sirven para comparar con el 2026-10-27 |
| Carga de la máquina | Media de 1 minuto entre 0,97 y 1,90 al empezar cada grupo de ejecuciones (2,81 al inicio de la sesión): reposo razonable, **no** una máquina vacía. Estas cifras sustituyen a las provisionales de WP04 y WP05 |
| Escala `large` | **Incompleta**: `results.jsonl` solo tiene `bulk_load` y `point_reads` con 1 000 000 de registros (1 repetición). `range_scans` y `small_commits` en `large` **no se ejecutaron** en esta sesión (en `report.md` solo aparecen sus filas `@m0`). No afecta a ningún T salvo a la parte `large` de T4, que sí se midió |
| `concurrent_commits` con 1 escritor | Se midió aparte (`--threads 1`, 3 repeticiones, `medium`) en [`results-threads1.jsonl`](../../spikes/storage/results/2026-10-29-f1/results-threads1.jsonl), porque el barrido principal solo usa 8 escritores |
| Datos | 552 ejecuciones del barrido principal; el informe completo contra M0 se generó con `spike-bench report --input … --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` ([`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md)) |

### 5.2 T1–T10 (medianas de `kerno`, escala `medium`, `Sync`)

| Id | Métrica | M0 | `kerno` ahora | Objetivo | Resultado |
|---|---|---|---|---|---|
| T1 | `agent_memory` p99 de escritura | LSM 5 837 µs (3,71× fjall, 5,36× btree) | 3 938 µs; fjall 1 418; btree 1 748; spike-lsm 9 339 | ≤ 2,0× fjall **y** ≤ 1,5× btree | ❌ **No cumple**: 2,78× fjall y 2,25× btree |
| T2 | p99 de commit, LiveSpec | 4 764 µs | 4 391 µs (spike-lsm 7 016) = 0,63× | ≤ 1,2× spike-lsm | ✅ |
| | Cauce | 713 µs | 1 609 µs (spike-lsm 827) = 1,95× | ≤ 1,2× spike-lsm | ❌ |
| | `append_seq` inserción | 4 139 µs | 2 652 µs (5 591) = 0,47× | ≤ 1,2× spike-lsm | ✅ |
| | `append_seq` actualización | 6 840 µs | 4 747 µs (8 602) = 0,55× | ≤ 1,2× spike-lsm y ≤ 2× fjall | ✅ frente a lsm · ❌ 2,28× fjall (2 081 µs) |
| | `history_sim` | 9 118 µs | 7 533 µs (11 928) = 0,63× | ≤ 1,2× spike-lsm | ✅ |
| T3 | p99 de lectura, LiveSpec module-scan | 657 µs | 951 µs (spike-lsm 931) = 1,02× | ≤ 1,2× spike-lsm; ≤ 1,5× spike-btree | ✅ frente a lsm · ❌ 1,79× btree (532 µs) |
| | LiveSpec symbol-get | 43,7 µs | 55,5 µs (54,7) = 1,01× | ≤ 1,2× spike-lsm | ✅ |
| | Cauce step-read | 98,0 µs | 149 µs (153) = 0,97× | ≤ 1,2× spike-lsm | ✅ |
| | recall | 100 µs | 107 µs (119) = 0,90× | ≤ 1,2× spike-lsm | ✅ |
| | recent-scan | 234 µs | 159 µs (164) = 0,97× | ≤ 1,2× spike-lsm | ✅ |
| T4 | Cierre tras `bulk_load` `medium` | 759 ms | 251 ms (rango 235–296) | ≤ 250 ms | ❌ **Por 1 ms, dentro del ruido**: no se declara cumplido |
| | Cierre tras `bulk_load` `large` | 3 587 ms | 397 ms (1 repetición) | ≤ 1 000 ms | ✅ |
| T5 | Rendimiento por carga frente a spike-lsm | — | `bulk_load` 0,85× · `point_reads` 1 hilo 0,80× (4 hilos 1,02×) · `range_scans` 0,71× · `cauce` 0,87× · `rmw_counters` 0,899× · `small_commits` 0,94× · `append_seq` 0,96× / 1,01× · `history_sim` 1,01× · `livespec` 1,02× · `agent_memory` 1,08× · `point_deletes` 1,00× · `mixed` 1,73× · `concurrent_commits` 3,67× | ≥ 0,9× spike-lsm | ❌ en `bulk_load`, `point_reads` (1 hilo), `range_scans`, `cauce` y, por 0,001, `rmw_counters`; ✅ en el resto |
| T6 | `fsync` por commit | 1,00 | 1 escritor: 1,00 (`small_commits` y `concurrent_commits`). 8 escritores: **0,22** | 1 escritor = 1,00; 8 escritores ≤ 0,5 y ≥ 2× el rendimiento de 1 escritor | ✅ 8 983 commits/s con 8 escritores frente a 3 606 con 1 = 2,49× |
| T7 | Amplificación de escritura `small_commits` | 1,09× | 4,29× (rango 1,17–5,01; spike-lsm 1,09) | ≤ 1,20× | ❌ **No cumple** y es inestable entre repeticiones |
| | `bulk_load` | 3,4× | 3,33× | ≤ 3,6× | ✅ |
| T8 | Reapertura limpia | 3,4 ms | 3,59 ms (spike-lsm 4,29) = 0,84× | ≤ 1,0× spike-lsm | ✅ |
| | Recuperación sucia | 9,73 ms | 8,52 ms (7,35) = 1,16× | ≤ 1,2× spike-lsm | ✅ (margen estrecho) |
| T9 | Coste RMW (`rmw_counters`) | — | 2 267 txn/s · p99 5 521 µs · 1,00 `fsync`/commit; spike-lsm 2 522 txn/s y p99 14 893 µs; fjall 3 228 txn/s | se informa | informado |
| T10 | Rendimiento frente a fjall | 0,8–1,3× | 0,70× (`rmw_counters`) a 1,12× (`point_deletes`) en las cargas de throughput; 1,93× en `mixed` y 2,72× en `concurrent_commits` | se informa; incumplimientos de 2× documentados | informado: ninguna carga de throughput queda por debajo de 0,5× fjall; el incumplimiento de 2× de cola está en T1 (2,78×) y T2 (2,28×) |

(Cifras exactas, con rangos entre repeticiones, en [`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md); las proporciones se calcularon sobre las medianas de `results.jsonl`.)

### 5.3 Incumplimientos documentados

Se documentan todos; ninguno se corrigió ni se modificaron los objetivos T1–T10.

1. **T1** (p99 de escritura de `agent_memory`): 2,78× fjall y 2,25× spike-btree frente a ≤ 2,0× y ≤ 1,5×. Mejora respecto de M0 (5 837 µs → 3 938 µs, 0,42× el spike-lsm de esta sesión), pero no alcanza el objetivo. Es el caso que [`f1-decision-cond1.md`](f1-decision-cond1.md) dejó como candidato a NO-GO con medidas provisionales; esta medición de referencia **no lo revierte**. Según el riesgo del paquete, un incumplimiento de T1 en la salida activa la **revisión de contingencia de ADR-0001 (R-10/R-11) por el propietario**; la decisión es suya y sigue abierta.
2. **T2**: p99 de commit de Cauce 1,95× spike-lsm (objetivo ≤ 1,2×); p99 de actualización de `append_seq` 2,28× fjall (objetivo ≤ 2×).
3. **T3**: p99 de LiveSpec module-scan 1,79× spike-btree (objetivo ≤ 1,5×): según el objetivo, corresponde la revisión de la condición 5.
4. **T4** `medium`: 251 ms frente a ≤ 250 ms (rango 235–296 ms). Un miss de 1 ms dentro del ruido; se informa como miss.
5. **T5**: por debajo de 0,9× spike-lsm en `bulk_load` (0,85×), `point_reads` con 1 hilo (0,80×), `range_scans` (0,71×), `cauce` (0,87×) y `rmw_counters` (0,899×, en el límite).
6. **T7** `small_commits`: 4,29× frente a ≤ 1,20×, con rango de 1,17 a 5,01 entre repeticiones (la amplificación depende de si el trabajo de compactación entra en la ventana medida).
7. El hueco de datos de `large` (§5.1) y la máquina no vacía son limitaciones de la medición, no incumplimientos.

## 6. Verificación de las pruebas del paquete

Los comandos de aceptación que se repiten al cierre son `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` y `cargo test --workspace --all-features --release`. La pasada de 1000 semillas de §2 se hizo con la salida completa (`--nocapture`) en el commit `50583e2`; esta revisión solo añade documentación y resultados de benchmarks, sin cambios de código del motor.

## 7. Congelación del formato: pendiente

[ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) sigue en estado «Propuesto»: el propietario no lo ha aceptado. Por gobernanza de ADR-0003 la congelación **no se hizo**: el bit `DRAFT` sigue en `WRITE_INCOMPAT` y `KNOWN_INCOMPAT` (`crates/kerno-storage/src/format/features.rs`), los vectores no se regeneraron, no existe el vector negativo `draft-v1` y [v1.md](../file-format/v1.md) sigue como «Candidato a congelación». Cuando el propietario acepte ADR-0013 hay que aplicar el proceso de v1.md §15 (retirar `DRAFT`, regenerar los vectores de estructura y de directorio, añadir el vector negativo `draft-v1` esperando `Incompatible('pre-freeze draft format')`, estado «Congelado» y fila de historial).

## 8. Riesgos residuales y decisiones abiertas del propietario

| Id | Riesgo o decisión | Estado |
|---|---|---|
| 1 | Revisión de contingencia de ADR-0001 por el incumplimiento de T1 (R-10/R-11) | Abierta, del propietario |
| 2 | Aceptación de ADR-0013 y congelación de v1 | Abierta, del propietario |
| 3 | Aprobación de los objetivos T1–T10 (siguen «pendiente» en [`f1-objetivos-rendimiento.md`](f1-objetivos-rendimiento.md)) | Abierta, del propietario |
| 4 | Nombrar la máquina objetivo (C-09) y repetir la medición allí (R-S3) | Abierta |
| 5 | Registro de LazyFS (§4) | Pendiente |
| 6 | Escala `large` incompleta para `range_scans` y `small_commits` (§5.1) | Pendiente; repetir con `LARGE=1` en una máquina en reposo |
| 7 | Misses de T2, T3, T4, T5 y T7 (§5.3) | Sin corregir en este paquete |
