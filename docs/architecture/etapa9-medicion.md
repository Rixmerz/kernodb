# Etapa 9 · E9-20 — Medición única de E8-10

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Estado | **Medido una sola vez.** Todos los umbrales de E8-10 que fija el acta 9 §5 se cumplen (T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5) y los dos cocientes de tiempo (P1, P2) están dentro de su cota en las cuatro corridas. Ningún umbral, fichero ni peso se cambió después de medir; después de la corrida solo se aplicó formato (`cargo fmt`) a los dos ficheros de código y la salida de calidad se reprodujo idéntica después |
| Acta | [Etapa 9](etapa9-acta.md) §5 (umbrales), §3 fila E9-20, decisiones D7 y D22 (§8.1); P1 y P2 se definen en el [acta de la Etapa 8](etapa8-acta.md) §6.3 |
| Conjunto | `crates/kerno-testkit/data/derived-eval-2/` (congelado en E9-19) · [conjunto de evaluación](etapa9-conjunto-evaluacion.md) |
| Código | `crates/kerno-units/tests/derived_measure2.rs` (calidad; se ejecuta con `cargo test`) · `crates/kerno-units/examples/derived_bench2.rs` (P1 y P2; no se ejecuta con `cargo test`) |
| Salida cruda | [etapa9-medicion-salida.txt](etapa9-medicion-salida.txt) |

Las cifras de calidad y de tiempo salen de la salida cruda. No salen de ella el número de pruebas de `derived_measure2` (5; lo cuenta el runner de `cargo test` en las ejecuciones de la §4) ni los resultados de las 7 mutaciones (§4; salen de ejecuciones sobre copias del árbol, con las líneas `FAILED` tal como se imprimieron). En la salida cruda, «acta §6.3» se refiere al acta de la Etapa 8. Lo que no se afirma está en §5.

## 1. Método

- Se midió **una vez**: una invocación de la prueba de calidad y una de `derived_bench2`, en ese orden. La carga de la máquina (`uptime`) antes de cada una consta en la cabecera de la salida cruda: 2,27 / 2,44 / 2,67 antes de la calidad y 1,25 / 2,16 / 2,57 antes de `derived_bench2`; la carga a 1 minuto fue inferior a 4 en ambas (T-SQ1). Había otro trabajo activo en la máquina durante la corrida, así que los dos cocientes de tiempo (P1, P2) **no** se midieron en una máquina en reposo. Las cifras de calidad no dependen de la carga y una repetición posterior de la prueba de calidad, hecha por la revisión, las reprodujo línea por línea.
- T-E10-R: la prueba comprueba el SHA-256 de `corpus.txt`, `vectors.txt` y `queries.txt` contra `MANIFEST` antes de cualquier cifra; coincide. Si no coincidiera, la medición se niega (`a_manifest_that_does_not_match_refuses_to_measure`).
- Brazos, corte en 10, sobre las 100 consultas (todas llevan vector de consulta): `simple` (orden de referencia), `ranked` (`RankMode::On`, pesos por defecto, señal vectorial apagada), `vector` (búsqueda coseno exacta sobre los vectores persistidos, leídos de la base reabierta), `hybrid` (`ranked` con la señal vectorial) y `brute` (fuerza bruta independiente sobre los vectores enteros del conjunto).
- Las unidades sin juzgar del top 10 se excluyen antes de calcular las métricas y se cuentan por brazo y por clase (T-E10-Q5). El ideal de nDCG y el total de relevantes de recall siguen siendo los de toda la lista de juicios.
- Los dos motores (`emulated_log` y `kerno_db`) dan cifras idénticas: la prueba compara las 85 líneas de cada informe y falla si difieren.
- **Lectura de «sin señal».** El acta usa la expresión con dos sentidos (T-E10-V2 y el documento de congelación: el brazo solo-vector; Q4 de E8: el brazo `ranked` sin vectores). La lectura **primaria** se fijó en el código antes de medir como el brazo solo-vector. La segunda lectura se calcula en la misma corrida y se informa como informativa.

## 2. Calidad (acta 9 §5)

| Umbral | Cota | Valor medido | Estado |
|---|---|---|---|
| **T-E10-R** `MANIFEST` | igualdad de SHA-256 | coincide | **cumple** |
| **T-E10-V1** consultas con vector, por clase | ≥ 25 en cada clase | 25, 25, 25, 25 | **cumple** |
| **T-E10-V2** nDCG@10 sin señal, en conjunto | ≤ 0,96 | 0,4216 | **cumple** |
| **T-E10-Q1** recall@10 de la búsqueda exacta persistida frente a fuerza bruta | = 1,000 | 1,000 (mínimo y media); 100 de 100 listas idénticas y en el mismo orden | **cumple** |
| **T-E10-Q2** nDCG@10 con señal, en conjunto | ≥ 0,80 | 0,9738 | **cumple** |
| **T-E10-Q3** recall@10 con señal, en conjunto | ≥ 0,90 | 0,9594 | **cumple** |
| **T-E10-S1** nDCG@10 con señal − sin señal, en conjunto | ≥ 0,02 | +0,5522 | **cumple** |
| **T-E10-S2** con − sin señal, por clase | ≥ −0,01 en cada una | vecindad vectorial +0,2982; temporal +0,6505; procedencia +0,6312; combinada +0,6291 | **cumple** en las 4 |
| **T-E10-Q5** sin juzgar en el top 10, por brazo y clase | ≤ 10 % | máximo 1,6 % (`ranked`, clase combinada, 4 de 250) | **cumple** |

Segunda lectura de «sin señal» (brazo `ranked`, informativa): S1 = +0,0651 (cumple); S2 = +0,0000 (vecindad vectorial), +0,0281 (temporal), +0,1832 (procedencia), +0,0490 (combinada): también se cumple en las cuatro clases. Esta lectura no cambia ningún veredicto.

nDCG@10 por brazo, en conjunto: `simple` 0,9014; `ranked` 0,9087; `vector` 0,4216; `hybrid` 0,9738. Con T-E10-V2 cumplido (0,4216 ≤ 0,96) el conjunto no está saturado en la lectura primaria, que es lo que F-07 pedía. En `ranked` sí hay un techo parcial en conjunto (0,9087), y por eso la segunda lectura da una mejora mucho menor (+0,0651) que la primaria. El techo anotado como F-07 se repite por completo en la clase vecindad vectorial: `simple`, `ranked` e `hybrid` valen 1,0000 de nDCG@10 y la segunda lectura da S2 = +0,0000 en esa clase.

## 3. Cocientes de tiempo (P1 y P2)

`derived_bench2` **no** corre sobre `derived-eval-2`: usa una historia generada de escala Performance con la misma semilla, sin los vectores de `derived_eval2`; por eso P2 solo cronometra 25 consultas de vecindad vectorial. 20 000 unidades, `KernoDb` sobre el sistema de ficheros real, 4 corridas de 12 rondas de 64 pares, alternando el orden. Sin umbral absoluto de latencia (D7): cada umbral es un cociente de dos p50 de la misma sesión.

| Medida | Cota | Corrida 1 | Corrida 2 | Corrida 3 | Corrida 4 | Estado |
|---|---|---|---|---|---|---|
| **P1** p50 con descriptor / sin descriptor | ≤ 1,50 | 1,173 | 1,163 | 1,141 | 1,140 | **cumple** (0 de 4 sobre la cota) |
| **P2** p50 persistido / en memoria | ≤ 1,10 | 0,997 | 1,001 | 1,000 | 1,000 | **cumple** (0 de 4 sobre la cota) |

## 4. Verificación del código de medición

`derived_measure2` tiene 5 pruebas (el runner de `cargo test` cuenta 5 en cada una de las ejecuciones siguientes: `4 passed; 1 failed` o `2 passed; 3 failed`; antes del paquete el fichero no existía). Se ejecutaron **7 mutaciones**, cada una sobre una copia del árbol, de una en una, en compilación release y con su propio directorio de compilación, copia borrada después. Todas con `cargo test -p kerno-units --all-features --release --test derived_measure2`, sobre `crates/kerno-units/tests/derived_measure2.rs`. La columna final es la línea `FAILED` tal como se imprimió; estos resultados no salen de la salida cruda, sino de esas ejecuciones.

| N.º | Línea | Cambio | Prueba(s) que falla(n) | Línea impresa |
|---|---|---|---|---|
| 1 | 100 | `if &got != digest` → `if &got == digest` | las tres siguientes (resultado: `2 passed; 3 failed`) | `test the_manifest_of_the_second_frozen_set_matches ... FAILED`; `test a_manifest_that_does_not_match_refuses_to_measure ... FAILED`; `test quality_over_the_second_frozen_set ... FAILED` |
| 2 | 100 | `if &got != digest` → `if false && &got != digest` | `a_manifest_that_does_not_match_refuses_to_measure` | `test a_manifest_that_does_not_match_refuses_to_measure ... FAILED` |
| 3 | 173 | `.filter(\|u\| judged.contains(u))` → `.filter(\|_\| true)` | `unjudged_units_are_excluded_and_counted` | `test unjudged_units_are_excluded_and_counted ... FAILED` |
| 4 | 114 | `gain >= S1_MIN_GAIN` → `gain > S1_MIN_GAIN` | `every_threshold_flips_at_its_own_bound` | `test every_threshold_flips_at_its_own_bound ... FAILED` |
| 5 | 354 | `assert_eq!(reopen.served, UNITS` → `UNITS + 1` | `quality_over_the_second_frozen_set` | `test quality_over_the_second_frozen_set ... FAILED` |
| 6 | 385 | `assert_eq!(indexed, simple` → `assert_ne!(indexed, simple` | `quality_over_the_second_frozen_set` | `test quality_over_the_second_frozen_set ... FAILED` |
| 7 | 371 | `reopen.served` → `reopen.served + std::any::type_name::<E>().len()` (informe distinto por motor) | `quality_over_the_second_frozen_set` (acuerdo entre motores) | `test quality_over_the_second_frozen_set ... FAILED` |

Cobertura: `the_manifest_of_the_second_frozen_set_matches` (mutación 1), `a_manifest_that_does_not_match_refuses_to_measure` (1 y 2), `unjudged_units_are_excluded_and_counted` (3), `every_threshold_flips_at_its_own_bound` (4) y `quality_over_the_second_frozen_set` (1, 5, 6 y 7). Las cinco pruebas tienen al menos una mutación. La mutación 1 no toca datos congelados.

Después de la corrida registrada solo se aplicó `cargo fmt` a los dos ficheros de código, sin cambiar ninguna cadena impresa; la salida cruda no se regeneró y una repetición posterior de la prueba de calidad reprodujo sus figuras idénticas.

## 5. Límites

- El conjunto es sintético y de 2 000 unidades; las cifras no se extrapolan a otras cargas.
- La lectura primaria de S1 y S2 compara contra un brazo solo-vector débil; por eso las diferencias son grandes. La segunda lectura, más exigente, también cumple.
- F-08: P2 no mide la lectura de los vectores persistidos. La lectura única de los 20 000 vectores servidos hacia un conjunto en memoria tardó 9,8 ms y se informa sin cota.
- Los umbrales de calidad se imprimen en la salida, no se afirman con `assert` en la prueba (solo se afirman acuerdos internos: manifiesto, motores y rutas); es un límite conocido.
- Una sola medición, en una sola máquina; el acta no pide repetirla y no se repitió.
