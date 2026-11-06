# Etapa 5 · E5-03 — `ExecutionObservation`

Contrato de E5-03 ([acta de la Etapa 5](etapa5-acta.md) §3 y decisiones de §8). El código está en `crates/kerno-units/src/observation.rs` (observador, niveles, retención) y en los dos puntos de `planner.rs` donde `query` y `query_relations` informan de lo ocurrido. Campos de partida: [ejecución adaptativa §2](adaptive-execution.md); privacidad: [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md).

## 1. Niveles

Cada `UnitDb` tiene un `Observer` (`UnitDb::observer()`) que comparten todas sus instantáneas. Empieza en `Off`; ningún ajuste, lectura de entorno ni otro nivel lo sube: `Diagnostic` rige solo después de `Observer::set_level(Diagnostic)`.

| Nivel | Conserva |
|---|---|
| `Off` (por defecto, acta §8.2) | Nada. Una consulta paga una carga atómica |
| `Aggregate` | Contadores (`ObservationStats`): ejecuciones, errores, fallbacks, filas, duración total y máxima, por camino. Sin registros, sin huella, sin texto |
| `Fingerprint` | Los contadores y un `ExecutionObservation` por ejecución, con la huella con clave de la forma de la consulta |
| `Diagnostic` | Lo anterior y el texto de la forma (`fingerprint_text`, p. ej. `units[unit_type=?text]`) |

Bajar el nivel actúa sobre lo ya guardado: por debajo de `Diagnostic` se borran los textos retenidos; por debajo de `Fingerprint`, los registros. Los contadores solo los borra `clear`.

La clave de la huella (acta §8.4) se genera de una fuente del sistema la primera vez que se pasa a `Fingerprint` o más, vive en memoria y no se exporta; si el sistema no puede darla, `set_level` devuelve el error y el nivel no cambia. `set_key` existe para pruebas reproducibles.

## 2. Qué lleva un registro

`ExecutionObservation`: `observation_id` (local, creciente), `fingerprint`, `fingerprint_text` (solo `Diagnostic`), `path` (el camino que respondió; `None` si falló antes), `plan_version` (`PLANNER_VERSION`), `commit_seq` de la instantánea, `duration_ns`, `rows_returned`, `outcome`, `fallback` y `recorded_at`.

No tiene campo para un filtro, una consulta de relaciones, una unidad, una carga útil, una clave ni un mensaje de error: la consulta se reduce a su forma y se descarta, y el error se guarda como clase (`ErrorKind`), nunca su mensaje, que puede citar una clave. La prueba de centinelas sobre todos los niveles es E5-04 ([informe](etapa5-centinelas.md)).

No se registran `rows_examined` ni `bytes_read`: ningún camino los cuenta y contarlos obliga a instrumentar los bucles calientes. `cache` y `learned` son de la Etapa 6 (acta §3).

## 3. Registrar nunca falla ni bloquea la consulta

- El resultado de la consulta se calcula primero y se devuelve intacto; registrar viene después y no puede cambiarlo.
- El registro toma el cerrojo del observador con `try_lock`. Si otro hilo lo tiene, la observación **se pierde y se cuenta** (`ObservationStats::dropped`) en lugar de esperar. Una falta de clave cuenta igual.
- Un cerrojo envenenado se recupera (su estado son contadores y una cola; un pánico no los deja incoherentes).
- Si el nivel se bajó mientras la consulta corría, se registra solo lo que el nivel actual permite.

## 4. Retención

Solo en memoria (acta §8.3), acotada por tamaño y por edad (`ObservationConfig`): como máximo `max_records` (por defecto 1024) y ninguno más antiguo que `max_age` (por defecto 1 h) respecto del más nuevo. Ambas cotas se aplican al añadir y al leer; los expulsados se cuentan en `evicted`. Una configuración que no retiene nada (0 registros o edad 0) se rechaza.

ponytail: la edad se comprueba solo en el frente de la cola, de modo que un reloj que retrocede puede dejar un registro viejo detrás de otros más nuevos hasta que la cota de tamaño lo expulse. Mejora: recorrer la cola al leer.

## 5. I-34, parte de observaciones

El observador no escribe en el motor, no abre transacciones y no lee el `Clock` de la capa (un reloj de pasos de prueba cambiaría las marcas de tiempo de los commits posteriores): tiene el suyo, `SystemClock` por defecto. `observing_does_not_change_the_data` (`tests/observation.rs`) ejecuta los mismos commits y lecturas en dos bases, una con `Off` y otra con `Diagnostic`, en `Emulated<LogEngine>` y en `KernoDb`, e iguala las respuestas (unidades, caminos, relaciones), las secuencias de commit y los bytes del motor. Comprueba además que la ejecución observada sí registró (4 + 4 × 6 observaciones), para que no sea una comparación entre dos ejecuciones mudas.

## 6. Conteo de pruebas y mutación

`cargo test --workspace --exclude kerno-storage --all-features --release`: **590 → 611** (+21: 13 unitarias de `observation.rs` y 4 de `tests/observation.rs` por dos motores). `kerno-units` solo: 238 → 259.

Cada prueba nueva se vio fallar con una mutación y se restauró. Las mutaciones se aplicaron al código de producción con copia previa y se restauró desde ella:

| Mutación | Pruebas que la matan |
|---|---|
| nivel inicial `Aggregate` | `default_is_off_and_off_keeps_nothing`, `the_default_is_off_and_keeps_nothing` (×2), `observing_does_not_change_the_data` (×2) |
| `begin` siempre devuelve `Some` | `default_is_off_and_off_keeps_nothing` |
| `Aggregate` conserva registros | `aggregate_counts_and_keeps_no_record`, `a_query_that_outlives_a_lowering_…` |
| texto de la forma desde `Fingerprint` | `fingerprint_records_the_hash_and_diagnostic_adds_the_text`, `queries_report_what_ran` (×2) |
| bajar el nivel no borra textos / registros | `lowering_the_level_drops_what_it_no_longer_allows` |
| se ignora el nivel bajado durante la consulta | `a_query_that_outlives_a_lowering_…` |
| sin cota de tamaño | `retention_is_bounded_by_size`, `concurrent_recording_…` (×2) |
| sin cota de edad | `retention_is_bounded_by_age` |
| expulsados sin contar | `retention_is_bounded_by_size`, `retention_is_bounded_by_age`, `concurrent_recording_…` (×2) |
| configuración sin validar | `a_config_that_keeps_nothing_is_refused` |
| pérdida por espera sin contar | `recording_that_would_wait_is_lost_and_counted`, `concurrent_recording_…` (×2) |
| cerrojo envenenado se propaga | `a_poisoned_lock_is_recovered_not_propagated` |
| falta de clave sin contar | `a_missing_key_is_a_counted_loss` |
| `ErrorKind::Corruption` → `Storage` | `error_kinds_carry_no_message` |
| un error conserva el camino | `a_failed_query_is_recorded_as_a_class_without_a_path` |
| el camino se pierde del registro | `fingerprint_records_the_hash_…`, `queries_report_what_ran` (×2) |
| la consulta observada devuelve otra respuesta | `observing_does_not_change_the_data` (×2), `queries_report_what_ran` (×2), `concurrent_recording_…` (×2) |
| `commit_seq` fijo en 0 | `queries_report_what_ran` (×2) |
| las relaciones se observan con la forma de unidades | `queries_report_what_ran` (×2), solo tras añadir la aserción sobre `QueryKind` |

La última fila fue un hallazgo: la mutación se mantenía en verde, es decir, la prueba no distinguía el tipo de consulta observada. Se añadió la aserción `fingerprint.kind()` y la mutación pasó a rojo.

## 7. Coste de observar (umbral ≤ 5 % de la p99 de la consulta sin observar, acta §8.5)

`index_bench` mide `secondary` (W1, filtro por tipo, ~1 % de selectividad) con el observador en `off` y en cada nivel, en cinco rondas que alternan los niveles, modo completo y sistema de archivos real. El coste es el p50 añadido de la mejor ronda de cada nivel frente a `off`, dividido por la p99 de la mejor ronda de `off`.

| Escala | `aggregate` | `fingerprint` | `diagnostic` |
|---|---|---|---|
| 20 000 unidades, ejecución 1 | −1,5 % | +3,2 % | **+5,9 %** |
| 20 000 unidades, ejecución 2 (`--seed 7`) | −1,5 % | +0,6 % | +3,6 % |
| 100 000 unidades, ejecución 1 | +3,0 % | −0,2 % | −2,6 % |
| 100 000 unidades, ejecución 2 (`--seed 7`) | −1,3 % | −0,5 % | −3,4 % |

Lectura: la máquina es compartida y las diferencias de p50 de decenas de microsegundos se confunden con el ruido (hay valores negativos). En 7 de 8 mediciones el coste cumple el umbral. La excepción es `diagnostic` en 20 000 unidades, ejecución 1 (+5,9 %); en la ejecución 2 cumple (+3,6 %). El coste real está acotado por el trabajo añadido por ejecución: la huella (BLAKE3 con clave sobre el texto de la forma) y, en `diagnostic`, copiar el texto. `diagnostic` es solo por activación explícita. **Se registra como riesgo, no como bloqueante** (acta §5 y §8): con la consulta de W1 en torno a 1,3 ms el 5 % son ~65 µs, del orden del propio coste de `diagnostic`; en consultas más lentas la fracción baja.

El planificador sobre W1 (E5-02, umbral ≤ 10 %) se midió de nuevo en estas mismas ejecuciones: p50/p99 +6,1 % / **+14,1 %** en 20 000 unidades (ejecución 1) y +2,6 % / +0,3 % en la ejecución 2; +2,4 % / +2,8 % y +2,2 % / −6,0 % en 100 000. Un valor fuera de umbral entre cuatro, que la segunda ejecución no reproduce. Es ruido de la máquina compartida más que un cambio (E5-03 no toca el planificador salvo por el informe al observador, que con `off` es una carga atómica); se anota para E5-05.

### Salida cruda

Ejecución 1 (`cargo run --release -p kerno-units --example index_bench -- --dir <dir>`):

```text
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,reference,71.440,86.992,108.472,199,109
20000,type,primary,7.953,8.289,8.480,199,109
20000,type,ideal,1.083,1.538,1.653,199,109
20000,type,secondary,1.284,1.895,2.169,199,109
# 20000 type: primary/ideal p50 = 7.35x, primary p95 = 8.289 ms, secondary/ideal p50 = 1.19x -> threshold not exceeded
# round 0: planner p50/p99 = 1.255/1.864 ms, legacy = 1.277/1.937 ms
# round 1: planner p50/p99 = 1.225/1.956 ms, legacy = 1.668/2.099 ms
# round 2: planner p50/p99 = 1.199/1.911 ms, legacy = 1.163/1.808 ms
# round 3: planner p50/p99 = 1.205/1.681 ms, legacy = 1.194/1.473 ms
# round 4: planner p50/p99 = 1.239/3.031 ms, legacy = 1.131/2.125 ms
# 20000 type: planner overhead over the current path: p50 +6.1 %, p99 +14.1 % (threshold 10 %) -> exceeded
# 20000 type: observation at Aggregate adds p50 -0.0204 ms = -1.51 % of the unobserved p99 1.350 ms (threshold 5 %) -> within
# 20000 type: observation at Fingerprint adds p50 +0.0424 ms = +3.15 % of the unobserved p99 1.350 ms (threshold 5 %) -> within
# 20000 type: observation at Diagnostic adds p50 +0.0798 ms = +5.91 % of the unobserved p99 1.350 ms (threshold 5 %) -> exceeded
20000,type+schema,reference,69.322,82.248,112.954,67,109
20000,type+schema,primary,7.254,11.639,13.561,67,109
20000,type+schema,ideal,0.472,0.743,1.050,67,109
20000,type+schema,secondary,0.486,0.693,0.886,67,109
# 20000 type+schema: primary/ideal p50 = 15.36x, primary p95 = 11.639 ms, secondary/ideal p50 = 1.03x -> threshold exceeded
100000,type,reference,455.479,574.590,610.601,999,698
100000,type,primary,53.957,64.433,82.387,999,698
100000,type,ideal,9.602,10.948,12.097,999,698
100000,type,secondary,10.528,14.335,16.519,999,698
# 100000 type: primary/ideal p50 = 5.62x, primary p95 = 64.433 ms, secondary/ideal p50 = 1.10x -> threshold exceeded
# round 0: planner p50/p99 = 11.721/18.101 ms, legacy = 11.213/17.741 ms
# round 1: planner p50/p99 = 11.994/15.253 ms, legacy = 11.792/36.830 ms
# round 2: planner p50/p99 = 11.482/14.402 ms, legacy = 11.238/19.409 ms
# round 3: planner p50/p99 = 11.518/16.607 ms, legacy = 11.906/14.006 ms
# round 4: planner p50/p99 = 12.890/17.782 ms, legacy = 12.467/17.352 ms
# 100000 type: planner overhead over the current path: p50 +2.4 %, p99 +2.8 % (threshold 10 %) -> within
# 100000 type: observation at Aggregate adds p50 +0.3456 ms = +3.02 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
# 100000 type: observation at Fingerprint adds p50 -0.0260 ms = -0.23 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
# 100000 type: observation at Diagnostic adds p50 -0.2980 ms = -2.60 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
100000,type+schema,reference,434.437,519.880,552.383,333,698
100000,type+schema,primary,43.255,51.928,62.686,333,698
100000,type+schema,ideal,3.397,3.782,4.019,333,698
100000,type+schema,secondary,3.658,4.124,4.432,333,698
# 100000 type+schema: primary/ideal p50 = 12.73x, primary p95 = 51.928 ms, secondary/ideal p50 = 1.08x -> threshold exceeded
# secondary index justified by W1: true
```

Ejecución 2 (`-- --dir <dir> --seed 7`, solo las líneas de `secondary`, planificador y observación):

```text
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,secondary,1.236,1.458,1.534,201,106
# 20000 type: planner overhead over the current path: p50 +2.6 %, p99 +0.3 % (threshold 10 %) -> within
# 20000 type: observation at Aggregate adds p50 -0.0269 ms = -1.54 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
# 20000 type: observation at Fingerprint adds p50 +0.0099 ms = +0.57 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
# 20000 type: observation at Diagnostic adds p50 +0.0633 ms = +3.64 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
20000,type+schema,secondary,0.433,0.568,0.611,67,106
100000,type,secondary,11.821,13.399,13.971,999,634
# 100000 type: planner overhead over the current path: p50 +2.2 %, p99 -6.0 % (threshold 10 %) -> within
# 100000 type: observation at Aggregate adds p50 -0.1778 ms = -1.32 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
# 100000 type: observation at Fingerprint adds p50 -0.0607 ms = -0.45 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
# 100000 type: observation at Diagnostic adds p50 -0.4565 ms = -3.40 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
100000,type+schema,secondary,3.748,4.353,5.304,333,634
```

## 8. Estado frente a los criterios de la acta (§4)

| Criterio | Estado tras E5-03 |
|---|---|
| Observar no altera datos confirmados ni la secuencia de commit (I-34, solo observaciones) | Verificado por `observing_does_not_change_the_data` en dos motores |
| Nada de literales ni contenido fuera de `diagnostic` (I-37) | El registro no tiene campo para ellos; la prueba de centinelas por nivel es E5-04 |

## 9. Riesgos

- `diagnostic` superó el 5 % en una de ocho mediciones (§7). Riesgo registrado, no bloqueante.
- Con contención, las observaciones se pierden (contadas en `dropped`) en lugar de esperar: un contador bajo es información, no un fallo. Si la Etapa 6 necesita registros completos bajo carga, hará falta un recolector por hilo.
