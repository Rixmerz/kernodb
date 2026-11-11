# Ejecución indexada de la consulta asociativa (E7-02)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Hecho** en E7-02 |
| Acta | [Etapa 7](etapa7-acta.md) §3 (fila E7-02), §4, §5 (H-1) |
| Código | `crates/kerno-units/src/assoc.rs` (recorrido compartido y ruta por índice), `planner.rs` (candidatos, sondeos, comprobación y caída), `plan.rs`, `fingerprint.rs`, `observation.rs` |
| Pruebas | `crates/kerno-units/tests/assoc_indexed.rs`, `tests/sentinels.rs`, `tests/plan.rs` y las pruebas de unidad de `planner.rs` |
| Documentos previos | [consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [centinelas (E5-04)](etapa5-centinelas.md) · [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) |

Este documento describe el código tal como está. Cada cifra sale de los comandos de §9 o de las mutaciones de §8. Este paquete no cambia el formato persistente, ni el conjunto de evaluación, ni ningún umbral del acta, ni el comportamiento de `reference_assoc_query`. No usa la caché de resultados (acta §2.3).

## 1. Qué añade

- `UnitSnapshot::indexed_assoc_query`: el mismo recorrido en anchura que la ruta de referencia, pero con los vecinos de cada salto leídos de `relations_of` (el índice de adyacencia) y cada respuesta comprobada. Es el camino solo, elegido por quien llama; una respuesta falsa del índice es `AssocError::IndexAnswer(Violation)`.
- `UnitSnapshot::assoc_query`: el camino del planificador. Devuelve `AssocOutcome { result, path, fallback }`. Elige entre `PlanPath::AssocReference` y `PlanPath::AssocAdjacency`, comprueba y, si algo falla, responde la ruta de referencia y lo registra.
- `UnitSnapshot::explain_assoc`: el `Plan` que `assoc_query` sigue en esa instantánea.
- `QueryKind::Associative`, `NormalizedQuery::of_assoc` y `Observed::Assoc`: la huella y la observación de estas consultas.

## 2. El recorrido es uno solo

`reference_assoc_query` y la ruta por índice llaman al mismo `traverse` (en `assoc.rs`); solo cambia la función que da los vecinos de una unidad en una dirección:

| Ruta | Vecinos de `(unidad, dirección)` |
|---|---|
| Referencia | `reference_relations_of(unidad, dirección, None)` |
| Índice | `relations_of(unidad, dirección, tipo)`, una lectura por tipo del conjunto de tipos de la consulta (una sola con `None`), comprobada |

Por construcción, el orden de examen, el presupuesto de visitas, la ventana, el filtro de procedencia, los tres fines y la regla de las unidades que no están activas son los de E7-01. Las lecturas por tipo se hacen en orden de bytes del tipo (el de `BTreeSet<String>`), así que concatenadas dan el orden `(tipo, id)` de `reference_relations_of`, y las entradas de un tipo que la consulta no sigue nunca se leen. El presupuesto cuenta lo mismo que en la referencia porque el filtro de tipos se sigue aplicando antes de contar.

Que el recorrido sea común no sustituye a la prueba diferencial: los datos que cada ruta lee son distintos (§7).

## 3. El plan

Los candidatos, en orden de desempate (`PlanPath::candidates_of`):

| Candidato | Es |
|---|---|
| `AssocReference` | Recorrido sobre `reference_relations_of`. Siempre usable |
| `AssocAdjacency` | Recorrido sobre el índice. Puede ser inutilizable (§3.2) |

Una consulta asociativa no tiene estado de índice (`plan.index()` es `None`): las entradas de adyacencia se escriben en el mismo `ChangeSet` que la relación (ADR-0014) y no tienen marca de agua, así que no hay un «`Ready`» que leer. Lo que el planificador sabe de ellas es lo que ve en los rangos de claves.

### 3.1 Coste

El coste es el del primer salto, en lecturas clave-valor; los saltos siguientes repiten el mismo intercambio. `expansiones` = semillas distintas × direcciones seguidas; `a` = entradas del primer salto, sondeadas por semilla y dirección (sin mirar el tipo: es una cota superior):

| Candidato | Coste |
|---|---|
| `AssocReference` | `r × expansiones` (cada expansión lee todos los `REL`) |
| `AssocAdjacency` | `2 × a` (la entrada y el registro de cada vecino) |

`r` y `a` son sondeos de a lo sumo `PROBE` = 64 entradas, igual que en `decide_relations`: cotas inferiores cuando el sondeo llega lleno. Gana el más barato de los usables, y el primero en el orden de la tabla en un empate (una tienda sin relaciones cuesta 0 y 0, y responde la referencia).

ponytail: el coste del primer salto decide para todos y los sondeos no distinguen tipos. Camino de mejora: un recuento por unidad y tipo, si una carga muestra que el sondeo elige la ruta más lenta.

### 3.2 Cuándo el índice no es usable

El planificador sondea además los rangos enteros `REL`, `REL_OUT` y `REL_IN`. Una tienda sana escribe el registro y las dos entradas en un solo conjunto de cambios, así que los tres tienen el mismo número de entradas. De los tres sondeos (`adjacency_unusable`):

| Condición | `Unusable` |
|---|---|
| Hay relaciones y `REL_OUT` o `REL_IN` no tiene ninguna entrada | `AdjacencyAbsent` |
| Un rango completo (menos de 64) tiene menos entradas que otro rango, completo o no | `AdjacencyInconsistent` |

Un rango completo es el recuento exacto y acota a los demás por arriba. Un sondeo lleno solo es una cota inferior: que otro rango tenga 64 no contradice a nada. Por eso esta comprobación **solo ve lo que un sondeo puede ver**: en una tienda de menos de 64 relaciones, cualquier entrada que falte o sobre; en una mayor, un índice ausente. Es la parte de la comprobación de completitud que el acta permite hacer sin recorrer todos los registros.

Con el índice inutilizable, `Decision::new` lo acepta solo para `AssocAdjacency` y por esas dos causas (`PlanError::FallbackUnusable` en cualquier otro caso), nunca lo elige, y la razón del plan es `Reason::AdjacencyUnusable(causa)`. La referencia no puede ser inutilizable.

## 4. Las comprobaciones de cada respuesta del índice

Cada lectura del índice (`check_adjacent`, en `planner.rs`) pasa por esto antes de que el recorrido use una relación:

| # | Comprobación | Si falla |
|---|---|---|
| 1 | `relations_of` compara cada entrada con su registro `REL`: registro ausente, o extremos o tipo distintos | `Storage(Corruption)` → `FallbackCause::Corrupt` |
| 2 | `check_relations`: solo relaciones de la unidad y dirección pedidas y del tipo pedido, en orden estricto `(tipo, id)` | `Violation::NotMatching`, `OutOfOrder` |
| 3 | Ninguna relación con `created_seq` posterior a la instantánea: ninguna entrada de esta instantánea puede apuntar a ella | `Violation::BeyondSnapshot` |
| 4 | La entrada espejo, en el otro extremo de la relación, existe y nombra este extremo | `Violation::Unmirrored` |

Las comprobaciones 2 a 4 son las de `check_relations` de Etapa 5 más dos nuevas. La 4 cuesta una lectura puntual por relación examinada y es lo que detecta, en una tienda grande, una entrada a la que le falta su pareja o que no coincide con ella.

**Lo que no se comprueba (H-1).** Una entrada que falta en los dos extremos de una relación no deja nada que comparar, y una entrada que falta solo en el lado que se sigue es invisible si el otro extremo no se expande. En una tienda de 64 relaciones o más, el recorrido por índice contesta entonces menos que la referencia sin avisar. Lo ve `verify_units` (UNIT-36) y la prueba diferencial lo descarta sobre tiendas sanas; el acta ya lo recoge como H-1. La prueba `completeness_is_not_a_query_time_check` fija este límite para que nadie lo dé por cubierto.

## 5. La caída y su causa

Si cualquiera de las comprobaciones de un salto falla, o el índice informa de daño, se abandona el recorrido entero y responde la ruta de referencia desde el principio. El resultado de `assoc_query` lo dice en `AssocOutcome::fallback` (y la observación lo repite):

| `fallback` | Cuándo | Camino que responde |
|---|---|---|
| `None` | La ruta elegida respondió y pasó sus comprobaciones | La elegida |
| `Fallback { abandoned: AssocAdjacency, cause: Unusable(AdjacencyAbsent \| AdjacencyInconsistent) }` | El plan marcó el índice inutilizable (§3.2); no se leyó | `AssocReference` |
| `… cause: Invalid(NotMatching \| OutOfOrder \| BeyondSnapshot \| Unmirrored)` | Se eligió el índice y una comprobación del salto lo rechazó (§4, 2 a 4) | `AssocReference` |
| `… cause: Corrupt` | Se eligió el índice y informó de daño (§4, 1) | `AssocReference` |

Tres precisiones:

- Una consulta que el modelo rechaza (`AssocQueryError`) o con una semilla que no es una unidad activa no llega al planificador: el error es el mismo que da la referencia (`active_seeds`, común a las dos).
- Un error que no es daño no se esconde con un segundo intento, como en `execute_units`. Si el daño está en los datos mismos (una cabeza ilegible, por ejemplo), la referencia falla igual y el error se devuelve.
- Un índice inutilizable cuenta como caída registrada aunque el plan no lo eligiera, a diferencia de `query` con un índice de unidades `Stale`, que no deja `fallback`. Es deliberado: el índice de adyacencia no tiene un estado que la observación pueda leer, y sin esto una tienda dañada y una tienda donde la referencia es más barata serían indistinguibles en las observaciones.

## 6. Explicación y observaciones sin literales

`explain_assoc` devuelve un `Plan`: la huella de la forma de la consulta, los dos candidatos con coste y usabilidad, el elegido y la razón. No lleva semilla, tipo, actor, fuente, ventana ni cifra de la consulta.

La forma (`NormalizedQuery::of_assoc`, solo visible en `diagnostic`) sustituye todo por marcadores y lleva solo estructura:

```text
assoc[seeds=?unit_ids,dir=<out|in|both>,depth=?u32,limit=?u32,budget=?u64
      [,types=?texts][,window=?seqs][,kinds=?kinds][,actor=?bytes][,source=?bytes]]
```

La dirección es estructura (tres valores). Ni el número de semillas, de tipos o de clases entra en la forma. La huella es la de siempre: BLAKE3 con clave, `assoc/blake3-keyed/v1/…`.

`assoc_query` informa al observador igual que `query` y `query_relations`: camino, filas (`hits.len()`), `commit_seq`, `fallback` y, si falla, la clase del error (`ErrorKind::Invalid` para una consulta rechazada o una semilla inactiva, y la clase del error de lectura en otro caso), nunca su mensaje. Un contador por camino: `PATHS` pasa a 8.

El mensaje de error que recibe quien llama sigue citando sus propias cotas (`window … has its bounds inverted`) y la semilla ausente: es el error de E7-01 y no es una observación ni una explicación. Las pruebas lo fijan (§7).

`PLANNER_VERSION` sigue en 1: no cambian los candidatos, el coste ni la elección de ninguna consulta anterior.

## 7. Pruebas

Todas corren sobre `Emulated<LogEngine>` y `KernoDb`, salvo las de unidad de `planner.rs` (solo el emulado, como las anteriores del mismo módulo). La referencia es el oráculo: nunca lee el índice, así que el daño al índice no puede cambiar lo que responde.

| Prueba | Qué demuestra |
|---|---|
| `assoc_indexed::differential_seeded` (I-32) | 6 semillas, cada una con 60 consultas generadas sobre dos instantáneas (una anterior a más relaciones) sobre grafos de 10 a 17 unidades con autorrelaciones, relaciones paralelas, un concentrador, 4 tipos (dos con prefijo común) y las 4 clases de procedencia, con actores y fuentes variados. Las consultas ponen todos los campos de `AssocQuery`: semillas repetidas, ausentes y vacías, conjuntos de tipos (con uno inexistente), dirección, profundidad 0 a 6, límite, presupuesto, ventana (también invertida), clases, actor y fuente. La ruta por índice, la del planificador y la de referencia dan el mismo resultado (aciertos, orden, rastros, visitas, fin) o el mismo error; el plan es determinista; el camino ejecutado es el explicado; ninguna tienda sana cae. Exige haber cubierto los dos caminos, los tres fines, los rechazos y cada campo |
| `differential_across_scan_pages` | 600 relaciones sobre 3 unidades y una cuarta que las alcanza en un salto: el barrido de entradas cruza páginas de 256, con presupuestos en torno a esas fronteras y todos los conjuntos de tipos, direcciones y límites |
| `frozen_corpus_indexed_equals_reference` | Las 64 consultas del conjunto congelado (`crates/kerno-testkit/data/assoc-eval`, sin tocar), cada una también con tres cotas apretadas, y una de tres semillas con los máximos duros: los tres fines, mismo resultado en las tres rutas. Además `verify_units` limpio |
| `false_index_answers_are_detected_and_answered_through_the_fallback` | 10 daños escritos al motor, cada uno en una tienda pequeña (6 relaciones) y en una grande (más de 64): §7.1 |
| `completeness_is_not_a_query_time_check` | El límite de §4: la relación sin entradas en una tienda grande no se detecta y la respuesta es peor que la referencia; en la pequeña, el planificador la ve. En los dos casos `verify_units` da UNIT-36 |
| `a_failure_that_is_not_damage_is_not_hidden` | Semilla inactiva y consulta rechazada dan el mismo error por las dos rutas; una cabeza dañada falla también en la referencia y el error se devuelve |
| `the_plan_compares_the_first_hop_and_says_why` | Costes 12 y 6, 6 y 2, 12 y 0 en vectores a mano; tienda sin relaciones; la huella empieza por `assoc/blake3-keyed/v1/` |
| `observations_record_the_path_the_rows_and_the_class_of_an_error` | Camino, filas, `commit_seq`, formas exactas en `diagnostic`, clase `Invalid` sin camino ni filas para las rechazadas, contadores por camino (ningún otro camino cuenta) y la elección de la referencia con un concentrador |
| `the_shape_ignores_literals_and_sees_structure` | Otros literales, misma huella y mismo texto; cada condición y la dirección, huella distinta |
| `sentinels::no_sentinel_in_any_associative_plan_observation_or_error` (I-37) | Centinelas en el tipo de relación (y uno desmesurado), actor y fuente del conjunto de cambios, la fuente de procedencia, la cota superior de la ventana, la profundidad y los identificadores de las semillas. Se buscan en el plan, la huella, el camino y la caída, los contadores y los registros, en los cuatro niveles; y en los errores, salvo las cifras y la semilla que la consulta rechazada cita a su propio llamante. Controles: la respuesta contiene el tipo, el conjunto de cambios contiene actor y fuente, los errores contienen las cotas. Cada huella informada es la de una forma con otros literales |
| `sentinels::levels_below_diagnostic_carry_no_shape_text` | Ampliada: la forma asociativa solo existe en `diagnostic` |
| `sentinels::no_sentinel_in_any_observability_explanation_or_error_output` | Ampliada: tras dañar una cabeza, las consultas asociativas fallan o caen y siguen sin centinelas |
| `plan::associative_plans_have_their_own_candidates_and_choice` y `plan_holds_no_literal` | Candidatos exactos y en orden; el índice de adyacencia puede ser inutilizable solo por sus dos causas; la referencia, nunca; sin estado de índice; empate a la referencia; el plan no lleva semilla, tipo, actor, fuente ni ventana |
| `planner::tests::*` (4) | La tabla de sondeos de §3.2, los costes del primer salto, cada `Violation` inyectada en un salto (relación de otra unidad, desordenada, repetida, de otro tipo, posterior a la instantánea, sin entradas), el daño (`Corrupt`), el error que no es daño, y el índice inutilizable que no se lee |

### 7.1 Respuestas falsas del índice

Cada daño se escribe con `put_all` directamente al motor, sobre una tienda sana de un anillo `a→b a→c b→d c→d(u) d→e e→a` y la consulta desde `a`, profundidad 4, ambas direcciones. Antes se calcula la respuesta de la referencia. Después de dañar se comprueba que `assoc_query` devuelve exactamente la de la referencia (que no lee el índice), que el camino es `AssocReference`, que `fallback` es el esperado, que el plan lo anuncia y que la observación repite el camino y la caída. Al devolver los bytes originales, el índice vuelve a responder sin caída y `verify_units` queda limpio.

| Daño | Tienda pequeña (6 relaciones) | Tienda grande (más de 64) |
|---|---|---|
| Falta la entrada espejo de una relación que se sigue | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| Falta la entrada del lado que se sigue y se ve desde el otro extremo | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| La entrada espejo nombra otra unidad | `Invalid(Unmirrored)` | `Invalid(Unmirrored)` |
| Entrada sobrante sin registro | `Unusable(AdjacencyInconsistent)` | `Corrupt` |
| Entrada sobrante con registro y sin espejo | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| Una entrada nombra la unidad equivocada | `Corrupt` | `Corrupt` |
| Revisión equivocada: registro con `created_seq` posterior a la instantánea | `Invalid(BeyondSnapshot)` | `Invalid(BeyondSnapshot)` |
| Índice obsoleto: entradas de una relación borrada después | `Unusable(AdjacencyInconsistent)` | `Corrupt` |
| Índice obsoleto: una relación creada después no tiene entradas | `Unusable(AdjacencyInconsistent)` | **no se detecta** (§4) |
| Índice ausente: ninguna entrada saliente | `Unusable(AdjacencyAbsent)` | `Unusable(AdjacencyAbsent)` |

Los daños que cambian los datos (registro falsificado, registro con revisión futura) se comparan con la referencia sobre la tienda dañada y se comprueba que esa respuesta difiere de la sana, para que la prueba no sea vacía; los demás, que solo tocan el índice, deben dar la misma respuesta que antes de dañar.

## 8. Mutaciones

Cada mutación se aplicó a código sin mutar con un guion que sustituye el texto, ejecuta `cargo test -p kerno-units --release --no-fail-fast` sobre la biblioteca y los ficheros `assoc_indexed`, `sentinels`, `plan`, `assoc_reference`, `relations` y `observation`, anota las pruebas rojas y restaura el fichero (comprobado byte a byte). Las 29 compilaron y las 29 fueron detectadas. Dos de ellas salieron de revisar qué mutantes sobrevivirían y dieron una prueba más: la entrada espejo que nombra otra unidad (M06) y un contador por camino compartido (M19). «Prueba» abrevia el nombre de la función; las de `assoc_indexed` y `sentinels` fallan en sus dos motores.

| # | Mutación | Pruebas que fallaron |
|---|---|---|
| M01 | La ruta por índice lee los tipos en orden descendente | `differential_seeded`, `frozen_corpus_indexed_equals_reference` |
| M02 | `indexed_assoc_query` lee siempre el rango saliente | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_indexed_equals_reference`, `false_index_answers_…` |
| M03 | `indexed_assoc_query` ignora el tipo del salto | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_indexed_equals_reference` |
| M04 | El planificador nunca ejecuta la ruta por índice | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_…`, `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `observations_record_…`, `a_wrong_hop_…` |
| M05 | Una entrada espejo ausente cuenta como presente | `false_index_answers_…`, `a_wrong_hop_…` |
| M06 | La entrada espejo no se compara con este extremo | `false_index_answers_…` |
| M07 | Una relación creada en la secuencia de la instantánea se da por posterior | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_…`, `observations_record_…`, `a_wrong_hop_…` |
| M08 | Se omite la comprobación de la relación posterior a la instantánea | `false_index_answers_…`, `a_wrong_hop_…` |
| M09 | `check_relations` no se aplica a un salto | `a_wrong_hop_…` |
| M10 | Un índice ausente no se reconoce | `false_index_answers_…`, `the_adjacency_index_is_usable_only_when_the_probes_can_agree`, `an_unusable_adjacency_path_is_not_run_…` |
| M11 | Los sondeos inconsistentes no se reconocen | `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `the_adjacency_index_is_usable_…`, `assoc_costs_compare_the_first_hop` |
| M12 | El daño del índice se devuelve en vez de caer | `false_index_answers_…`, `a_wrong_hop_…` |
| M13 | Una ruta inutilizable se registra como daño | `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `an_unusable_adjacency_path_is_not_run_…` |
| M14 | Una respuesta falsa se registra como daño | `false_index_answers_…`, `a_wrong_hop_…` |
| M15 | El coste de la referencia ignora las expansiones | `the_plan_compares_the_first_hop_…`, `assoc_costs_compare_the_first_hop`, `false_index_answers_…`, `completeness_…`, `observations_record_…` |
| M16 | El coste del índice cuenta cada entrada una vez | `the_plan_compares_the_first_hop_…`, `assoc_costs_compare_the_first_hop`, `differential_seeded` |
| M17 | `assoc_query` no informa al observador | `observations_record_…`, `levels_below_diagnostic_carry_no_shape_text`, `no_sentinel_in_any_associative_plan_…`, `false_index_answers_…`, `completeness_…` |
| M18 | Una consulta rechazada se observa como `Other` | `observations_record_…` |
| M19 | El camino por índice comparte el contador del primero | `observations_record_…` |
| M20 | La forma olvida la ventana | `the_shape_ignores_literals_and_sees_structure`, `levels_below_diagnostic_…`, `observations_record_…` |
| M21 | La forma lleva los tipos de relación | `no_sentinel_in_any_associative_plan_…`, `no_sentinel_in_any_observability_…`, `the_shape_ignores_literals_…`, `levels_below_diagnostic_…`, `observations_record_…` |
| M22 | La forma lleva el actor | las mismas cinco que M21 |
| M23 | La referencia asociativa puede ser inutilizable | `associative_plans_have_their_own_candidates_and_choice` |
| M24 | La razón de un índice inutilizable es «menor coste» | `associative_plans_…`, `false_index_answers_…`, `completeness_…` |
| M25 | Las expansiones no se cuentan | `the_plan_compares_the_first_hop_…`, `differential_seeded`, `differential_across_scan_pages`, `false_index_answers_…`, `a_wrong_hop_…`, `observations_record_…`, `completeness_…` |
| M26 | `explain_assoc` planifica una consulta rechazada | `differential_seeded` |
| M27 | Un error que no es daño cae a la referencia | `a_wrong_hop_…` |
| M28 | Una semilla inactiva se acepta | `a_failure_that_is_not_damage_…`, `differential_seeded`, `observations_record_…`, `no_sentinel_in_any_associative_plan_…`, y de E7-01 `seeds_must_be_active_units`, `ends_that_are_not_active_are_absent` |
| M29 | La caída por índice inutilizable no se registra | `false_index_answers_…`, `completeness_…`, `an_unusable_adjacency_path_is_not_run_…` |

## 9. Verificación

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_indexed
```

Conteo de pruebas de `kerno-testkit` + `kerno-units` con `--release --all-features`: **611 aprobadas, 0 fallidas, 8 ignoradas** con este cambio; antes eran 586 (E7-01). Las 25 nuevas son 18 de `assoc_indexed` (9 × 2 motores), 2 de `sentinels` (1 × 2), 1 de `plan` y 4 de unidad de `planner`; el corredor las cuenta (`assoc_indexed`: `18 passed`).

## 10. Límites y riesgos

- **Completitud (H-1).** La respuesta por índice puede ser incompleta sin avisar cuando faltan entradas en los dos extremos de una relación y la tienda tiene 64 relaciones o más (§4). Es una propiedad de la comprobación en consulta, no un fallo de esta ruta; la cubre `verify_units` y la prueba diferencial sobre tiendas sanas.
- **Coste de la comprobación.** Cada relación examinada cuesta una lectura puntual más (el espejo) y cada consulta planificada, hasta 192 entradas de sondeo más las del primer salto. No se afirma nada de rendimiento (E7-05).
- **Sondeos cotas inferiores.** En tiendas grandes el planificador ve poco: el coste y la detección de §3.2 son una estimación. La ruta elegida es siempre correcta; solo puede no ser la más barata.
- **Un solo `ChangeSet` de relaciones.** Las pruebas de daño usan relaciones de un conjunto de cambios por relación; el comportamiento no depende de ello.
- **Dependencias.** Ninguna nueva; `Cargo.toml` no cambia.
- **Copia.** `tests/harness/assoc.rs` repite el cargador del corpus y la conversión de consultas de `tests/assoc_reference.rs`, que no se tocó. Pendiente: que `assoc_reference.rs` use el módulo común.

## 11. Documentos relacionados

- [Acta de la Etapa 7](etapa7-acta.md) §3 (E7-02), §4, §5 (H-1)
- [Consulta asociativa (E7-01)](etapa7-consulta-asociativa.md)
- [Centinelas (E5-04)](etapa5-centinelas.md)
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): relaciones y adyacencia transaccional
- [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md)
- [Invariantes](invariants.md): I-32, I-37
