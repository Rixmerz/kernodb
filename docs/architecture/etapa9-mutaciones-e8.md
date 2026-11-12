# E9-15: evidencia de mutación de E8-06 y E8-07

Paquete E9-15 de la Etapa 9 ([acta](etapa9-acta.md), §3; decisión D20 (ii); hallazgo F-10 de [`etapa8-cierre.md`](etapa8-cierre.md)). Este documento registra, para cada prueba de `crates/kerno-units/tests/consolidation.rs` (E8-06) y de `crates/kerno-units/tests/derived_measure.rs` (E8-07), una mutación aplicada a una copia del árbol, la línea mutada y la salida de la prueba. No cambia código ni pruebas.

## 1. Método

- **Base:** commit `5f7aadc`. Las líneas citadas son las de ese árbol. Toda ruta de la columna «Fichero:línea» es completa desde la raíz del repositorio: `crates/kerno-units/src/consolidation.rs` (producción) y `crates/kerno-units/tests/consolidation.rs` (la prueba) son ficheros distintos, y **ninguna mutación toca la prueba de consolidación**.
- **Copia:** una sola copia del árbol (sin `.git` ni `target`) con su propio directorio `target`, compilada solo en release y borrada al terminar. El árbol de trabajo no se tocó.
- **Una mutación a la vez.** Cada mutación se aplicó con `sed` sobre una línea de la copia; antes de compilar, `git diff --no-index` contra el fichero original confirmó que el cambio estaba aplicado. Después de la corrida el fichero se restauró desde el original y `git diff --no-index --quiet` lo confirmó; al final, `git diff --no-index --stat crates <copia>/crates` no dio diferencias (salida 0).
- **Comando de cada mutación** (en la copia): `cargo test -p kerno-units --all-features --release --test consolidation` (filas M1 a M15) o `--test derived_measure` (filas D1 a D4).
- **Columna «Resultado»:** los totales de la línea `test result: FAILED. N passed; M failed; ...` de esa corrida, copiados del registro. Cada corrida terminó con `error: test failed` (código de salida 101).
- **Qué se muta.** Código de producción cuando la prueba comprueba producción. Donde la lógica comprobada vive en la propia prueba de medición (`check_manifest`, `score`), se muta esa función; donde la entrada es un dato congelado, se altera el dato en la copia.
- **Máquina:** carga de 1 minuto (`load1`) 0,77 al empezar y 1,78 antes de repetir D3 y D4; 4,2 GB libres antes de copiar. Ningún otro trabajo de compilación corrió a la vez (§3.2).

### Conteo de pruebas

- Línea base en la copia, sin mutación: `consolidation` → `test result: ok. 30 passed; 0 failed` (15 pruebas × 2 motores); `derived_measure` → `test result: ok. 4 passed; 0 failed`.
- Este paquete no añade ni retira pruebas: el conteo antes y después es el mismo (T-CNT).

### Los dos motores

Las pruebas de consolidación se instancian con `emulated_log` y con `kerno_db` (macro `engines!`). En **cada** fila de §2 la salida contiene `test emulated_log::<prueba> ... FAILED` **y** `test kerno_db::<prueba> ... FAILED` para la prueba objetivo. `quality_over_the_frozen_set` mide los dos motores dentro de una sola prueba.

Ninguna mutación dejó la prueba objetivo en verde: no hay hallazgo de ese tipo (§4.4).

## 2. Pruebas de `crates/kerno-units/tests/consolidation.rs` (E8-06): 15 pruebas, 15 mutaciones

| # | Prueba | Fichero:línea | Cambio | Resultado |
|---|---|---|---|---|
| M1 | `a_summary_records_its_exact_sources_and_a_full_descriptor` | `crates/kerno-units/src/consolidation.rs:108` | `len().min(self.max_bytes)` → `len().min(self.max_bytes + 1)` | 20 passed; 10 failed |
| M2 | `errors_say_what_happened_without_content` | `crates/kerno-units/src/consolidation.rs:133` | texto `reserved name` → `reserved nam` | 28 passed; 2 failed |
| M3 | `a_summary_with_no_sources_is_an_error` | `crates/kerno-units/src/consolidation.rs:198` | `if units.is_empty() {` → `if false {` | 28 passed; 2 failed |
| M4 | `the_kind_parameter_is_reserved_and_separates_summaries` | `crates/kerno-units/src/consolidation.rs:154` | `.is_some()` → `.is_none()` (amplia) | 2 passed; 28 failed |
| M5 | `a_summarizer_that_gives_nothing_leaves_a_failed_summary` | `crates/kerno-units/src/consolidation.rs:203` | `summarizer.summarize(inputs)` → `Some(Vec::new())` | 26 passed; 4 failed |
| M6 | `the_original_stays_readable_through_every_summary_operation` | `crates/kerno-units/src/consolidation.rs:240` | `self.value(id)` → `self.value(id).or_else(\|\| Some(b"x".to_vec()))` | 22 passed; 8 failed |
| M7 | `a_source_change_marks_the_summary_stale_in_the_same_changeset` | `crates/kerno-units/src/derived_stale.rs:599` | `if inner.registry.transition(..)` → `if false && inner.registry.transition(..)` | 18 passed; 12 failed |
| M8 | `a_new_method_or_version_creates_a_new_summary_and_supersedes_the_old` | `crates/kerno-units/src/consolidation.rs:230` | `summarizer.version()` → `summarizer.version() + 1` | 24 passed; 6 failed |
| M9 | `rebuilding_needs_the_summarizer_that_made_the_summary` | `crates/kerno-units/src/consolidation.rs:166` | `d.version() == s.version()` → `true` | 28 passed; 2 failed |
| M10 | `deleting_any_one_source_deletes_the_summary_in_the_same_changeset` | `crates/kerno-units/src/derived_stale.rs:580` | `crate::suppression::remove_dependents_of_unit(&mut inner, unit);` → `let _ = unit;` | 24 passed; 6 failed |
| M11 | `a_stale_or_superseded_summary_is_deleted_with_its_source_too` | `crates/kerno-units/src/suppression.rs:59` | `inner.registry.dependents_of_unit(unit);` → `inner.registry.dependents_of_unit(unit).into_iter().take(1).collect::<Vec<_>>();` | 26 passed; 4 failed |
| M12 | `summaries_are_rebuilt_on_open_from_their_sources` | `crates/kerno-units/src/consolidation.rs:183` | `if !made_by(d, summarizer)? {` → `if false {` | 28 passed; 2 failed |
| M13 | `a_summary_leaves_nothing_on_disk` | `crates/kerno-units/src/consolidation.rs:201` (inserción antes de la línea) | `build_summary` abre un `ChangeSet`, crea una unidad (`payload` `m`) y confirma, antes de construir | 26 passed; 4 failed |
| M14 | `reads_of_a_source_are_the_same_with_and_without_summaries` | `crates/kerno-units/src/consolidation.rs:201` (inserción antes de la línea) | `build_summary` actualiza la primera fuente a `zz` y confirma, antes de construir (amplia) | 8 passed; 22 failed |
| M15 | `a_failed_commit_leaves_the_summaries_and_the_sources_as_they_were` | `crates/kerno-units/src/derived_stale.rs:551` | `let info = self.cs.commit()?;` → `match self.cs.commit() { Ok(i) => i, Err(e) => { inner.values.clear(); return Err(e); } }` | 28 passed; 2 failed |

Otras pruebas que la misma mutación pone en rojo (además de la objetivo, siempre en los dos motores): M5, `a_summary_records_its_exact_sources_and_a_full_descriptor`; M8, `a_summary_leaves_nothing_on_disk` y `the_original_stays_readable_through_every_summary_operation`; M10, `a_stale_or_superseded_summary_is_deleted_with_its_source_too` y `the_original_stays_readable_through_every_summary_operation`; M11, `deleting_any_one_source_deletes_the_summary_in_the_same_changeset`; M13, `reads_of_a_source_are_the_same_with_and_without_summaries`. En M2, M3, M9, M12 y M15 la objetivo es la única en rojo.

## 3. Pruebas de `crates/kerno-units/tests/derived_measure.rs` (E8-07): 4 pruebas, 4 mutaciones

| # | Prueba | Fichero:línea | Cambio | Resultado | Líneas `FAILED` |
|---|---|---|---|---|---|
| D1 | `quality_over_the_frozen_set` (P3) | `crates/kerno-units/src/derived_stale.rs:589` | `s.rev != head` → `s.rev == head` | 3 passed; 1 failed | `quality_over_the_frozen_set` |
| D2 | `the_manifest_of_the_frozen_set_matches` | `crates/kerno-testkit/data/derived-eval/vectors.txt` (final del fichero) | se añade un salto de línea al final | 1 passed; 3 failed | `the_manifest_of_the_frozen_set_matches`, `quality_over_the_frozen_set`, `a_manifest_that_does_not_match_refuses_to_measure` |
| D3 | `a_manifest_that_does_not_match_refuses_to_measure` | `crates/kerno-units/tests/derived_measure.rs:92` | `if &got != digest {` → `if false && &got != digest {` | 3 passed; 1 failed | `a_manifest_that_does_not_match_refuses_to_measure` |
| D4 | `unjudged_units_are_excluded_and_counted` | `crates/kerno-units/tests/derived_measure.rs:119` | `.filter(\|u\| judged.contains(u))` → `.filter(\|_\| true)` | 3 passed; 1 failed | `unjudged_units_are_excluded_and_counted` |

**D3 y D4 se corrieron dos veces** en la misma copia, con el mismo resultado las dos veces: `test result: FAILED. 3 passed; 1 failed`, y `quality_over_the_frozen_set` en verde. Una revisión anterior de este registro anotó `2 passed; 2 failed` para las dos filas, con `quality_over_the_frozen_set` también en rojo; aquí no se reprodujo. Por el código, esa prueba no puede ver ninguna de las dos mutaciones: con D3, `check_manifest` sigue devolviendo `Ok` porque los ficheros congelados coinciden con el `MANIFEST`; con D4, `score` solo cambia cifras que la prueba imprime (las medias y Q5) y no compara con ningún valor. Sus aserciones son la igualdad de los dos motores (ambos con la misma mutación) y P3/P4, que no leen `score`. Si una corrida muestra esa prueba en rojo con D3 o D4, la causa está fuera de la mutación y se registra aparte; la cifra de la tabla es la observada.

## 4. Resultado y límites

- **T-MU1:** pruebas con su mutación registrada = pruebas del alcance (15 de consolidación y 4 de medición, 19 de 19). Cierra F-10 de E8 en lo que toca a E8-06 y E8-07.
- Las mutaciones M4 y M14 son amplias: la prueba objetivo falla, pero también casi todas las demás. Muestran que la prueba ve la mutación, no que sea la única que la ve. M13 y M14 insertan una escritura en `build_summary`; para «un resumen no escribe nada» no hay con este código una mutación más estrecha.
- `quality_over_the_frozen_set` comprueba también P4 y la igualdad de los dos motores; D1 solo ejercita P3. No se registró una mutación aparte para P4 ni para el informe idéntico entre motores.
- Solo se corrieron los dos ficheros de prueba, antes y después de cada mutación, en la copia. Este paquete no cambia código ni pruebas.
