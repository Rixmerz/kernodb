# Etapa 9 · E9-20 — Medición única de E8-10

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Estado | **Medido una sola vez.** Todos los umbrales de E8-10 que fija el acta 9 §5 se cumplen (T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5) y los dos cocientes de tiempo (P1, P2) están dentro de su cota en las cuatro corridas. Ningún umbral, fichero ni peso se cambió después de medir |
| Acta | [Etapa 9](etapa9-acta.md) §5 (umbrales), §3 fila E9-20, decisiones D7 y D22 (§8.1) |
| Conjunto | `crates/kerno-testkit/data/derived-eval-2/` (congelado en E9-19) · [conjunto de evaluación](etapa9-conjunto-evaluacion.md) |
| Código | `crates/kerno-units/tests/derived_measure2.rs` (calidad; se ejecuta con `cargo test`) · `crates/kerno-units/examples/derived_bench2.rs` (P1 y P2; no se ejecuta con `cargo test`) |
| Salida cruda | [etapa9-medicion-salida.txt](etapa9-medicion-salida.txt) |

Toda cifra de este documento sale de la salida cruda. Lo que no se afirma está en §5.

## 1. Método

- Se midió **una vez**: una invocación de la prueba de calidad y una de `derived_bench2`, en ese orden, con la máquina sin otro trabajo. La carga de la máquina (`uptime`) antes de cada una consta en la cabecera de la salida cruda: 2,27 antes de la calidad y 1,25 antes de `derived_bench2`, ambas por debajo de 4 (T-SQ1).
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

nDCG@10 por brazo, en conjunto: `simple` 0,9014; `ranked` 0,9087; `vector` 0,4216; `hybrid` 0,9738. Con T-E10-V2 cumplido (0,4216 ≤ 0,96) el conjunto no está saturado en la lectura primaria, que es lo que F-07 pedía. En `ranked` sí hay un techo parcial (0,9087), y por eso la segunda lectura da una mejora mucho menor (+0,0651) que la primaria.

## 3. Cocientes de tiempo (P1 y P2)

20 000 unidades, `KernoDb` sobre el sistema de ficheros real, 4 corridas de 12 rondas de 64 pares, alternando el orden. Sin umbral absoluto de latencia (D7): cada umbral es un cociente de dos p50 de la misma sesión.

| Medida | Cota | Corrida 1 | Corrida 2 | Corrida 3 | Corrida 4 | Estado |
|---|---|---|---|---|---|---|
| **P1** p50 con descriptor / sin descriptor | ≤ 1,50 | 1,173 | 1,163 | 1,141 | 1,140 | **cumple** (0 de 4 sobre la cota) |
| **P2** p50 persistido / en memoria | ≤ 1,10 | 0,997 | 1,001 | 1,000 | 1,000 | **cumple** (0 de 4 sobre la cota) |

## 4. Verificación del código de medición

La prueba nueva tiene 5 pruebas (el runner cuenta `derived_measure2` con 5; antes del paquete no existía). Cada una se vio fallar con una mutación aplicada a una copia del árbol, en compilación release y con su propio directorio de compilación, copia borrada después. Todas con `cargo test -p kerno-units --all-features --release --test derived_measure2`, sobre `crates/kerno-units/tests/derived_measure2.rs`:

| Línea | Cambio | Prueba que falla |
|---|---|---|
| 100 | `if &got != digest` → `if false && &got != digest` | `a_manifest_that_does_not_match_refuses_to_measure` |
| 173 | `.filter(\|u\| judged.contains(u))` → `.filter(\|_\| true)` | `unjudged_units_are_excluded_and_counted` |
| 114 | `gain >= S1_MIN_GAIN` → `gain > S1_MIN_GAIN` | `every_threshold_flips_at_its_own_bound` |
| 354 | `assert_eq!(reopen.served, UNITS` → `UNITS + 1` | `quality_over_the_second_frozen_set` |
| 385 | `assert_eq!(indexed, simple` → `assert_ne!` | `quality_over_the_second_frozen_set` |
| 371 | `reopen.served` → `reopen.served + std::any::type_name::<E>().len()` (informe distinto por motor) | `quality_over_the_second_frozen_set` (acuerdo entre motores) |

`the_manifest_of_the_second_frozen_set_matches` no tiene mutación propia que la ponga en rojo sin tocar los datos congelados; la cubre el rojo de la línea 100 sobre su pareja y la comprobación de la prueba de calidad. Se declara aquí como límite.

Después de la corrida registrada solo se aplicó `cargo fmt` a la prueba; no cambia ninguna cadena impresa, y la salida cruda no se regeneró.

## 5. Límites

- El conjunto es sintético y de 2 000 unidades; las cifras no se extrapolan a otras cargas.
- La lectura primaria de S1 y S2 compara contra un brazo solo-vector débil; por eso las diferencias son grandes. La segunda lectura, más exigente, también cumple.
- F-08: P2 no mide la lectura de los vectores persistidos. La lectura única de los 20 000 vectores servidos hacia un conjunto en memoria tardó 9,8 ms y se informa sin cota.
- Una sola medición, en una sola máquina; el acta no pide repetirla y no se repitió.
