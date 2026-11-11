# Ranking de resultados asociativos y modo diagnóstico (E7-04)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Hecho** en E7-04 |
| Acta | [Etapa 7](etapa7-acta.md) §3 (fila E7-04), §4 (I-41, I-37), §7 punto 3 |
| ADR | [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md): vectores en memoria, coseno y producto escalar, búsqueda exacta |
| Código | `crates/kerno-units/src/assoc_rank.rs`, exportado desde `lib.rs` con una línea (`pub mod assoc_rank;`) |
| Pruebas | `crates/kerno-units/tests/assoc_rank.rs` |
| Documentos previos | [consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [ejecución indexada (E7-02)](etapa7-ejecucion-indexada.md) |

Este documento describe el código tal como está. Cada cifra sale de los comandos de §9 o de las mutaciones de §8. Este paquete no cambia el formato persistente, ni el conjunto de evaluación, ni ningún umbral del acta, ni `assoc.rs`, el planificador, el acta o el roadmap. **No lee las respuestas esperadas del conjunto congelado** y los pesos no se ajustaron contra él: no hay ninguna consulta suya en las pruebas. Tampoco decide si el ranking es el orden por defecto: eso lo decide la medición de E7-05 (acta §7 punto 3). Aquí el valor por defecto es **apagado**.

## 1. Qué añade

- `UnitSnapshot::rank_assoc(&AssocResult, &RankOptions) -> Result<Vec<RankedHit>, RankError>`: ordena los resultados de una consulta asociativa sobre la instantánea en que se ejecutó.
- `RankMode`: `Off` (por defecto), `On` y `Diagnostic`.
- `RankWeights` y las constantes `WEIGHT_*`: los pesos, con su razón (§4).
- `VectorSet`, `VectorQuery`, `VectorMetric`, `VectorError`: los vectores de ADR-0011 y la búsqueda exacta (§6).

Es un módulo aparte porque `assoc.rs` lo edita otro paquete (E7-03). Quien llama pasa el resultado que ya tiene; el módulo no ejecuta ninguna consulta.

## 2. Modos

| Modo | Orden | `score` | `contributions` | Lee la instantánea |
|---|---|---|---|---|
| `Off` (por defecto) | El del resultado: el de la ruta de referencia, «saltos, luego identificador» | `None` | `None` | No |
| `On` | `(puntuación descendente, identificador ascendente)` | `Some` | `None` | Sí |
| `Diagnostic` | El de `On` | `Some` | `Some`: la aportación de cada señal | Sí |

Con `Off` el orden es el de `AssocResult::hits` sin tocarlo, así que coincide con el de la ruta de referencia y, por I-32, con el de la ruta por índice. Si se suministran vectores, se comprueban en los tres modos y antes de cualquier otra cosa.

`RankedHit` lleva `unit`, `distance`, `score`, `contributions` y `similarity` (la similitud exacta en punto fijo, solo en `Diagnostic` y solo si la unidad tiene vector). No lleva el camino ni tipos de relación: eso lo tiene quien llama en su `AssocResult`.

## 3. Señales y fórmula

Todas las aportaciones son enteros (`i64`). Una puntuación mayor es mejor. La puntuación es **exactamente** la suma de las cinco aportaciones (I-41): `Contributions::sum` es la definición de la puntuación, y la prueba compara cada aportación con un oráculo escrito aparte (§7).

`score = hops + relation_type + recency + provenance + similarity`

| Señal | Lee | Aportación |
|---|---|---|
| Saltos | `distance` | `w_hops · (MAX_ASSOC_DEPTH + 1 − distance)`: 4 pasos de peso para un salto, 1 para cuatro |
| Tipo de relación | los tipos de los pasos del camino | `w_type · pasos_preferidos / pasos`, división entera (hacia cero). Vale 0 si el conjunto `preferred_types` está vacío |
| Recencia | el `commit_seq` más reciente del que depende el resultado | `w_recency · H / (H + edad)`, con `H = RECENCY_HALF_LIFE = 64` y `edad = seq_instantánea − más_reciente` |
| Procedencia | la clase más débil del camino | `w_prov · confianza(clase)`: `Declared` 3, `Imported` 2, `Algorithmic` 1, `Inferred` 0 |
| Similitud | la similitud exacta con el vector de quien llama | `w_sim · clamp(sim, −1, 1)`, con `sim` en punto fijo (`SIMILARITY_SCALE` = 1 000 000 por 1,0). Vale 0 si no hay vectores o la unidad no tiene vector |

Decisiones de la definición:

- **Tipo de relación.** El significado de un tipo es del dominio de quien llama, no del núcleo; por eso la señal es relativa a un conjunto que la persona indica (`RankOptions::preferred_types`) y no a una lista fija de tipos. Sin conjunto la señal no existe (0 para todos). Medirla con un conjunto concreto es trabajo de E7-05.
- **Procedencia: el eslabón más débil.** Una cadena es tan fiable como su paso menos fiable; un camino de dos pasos con uno inferido no se presenta mejor que el inferido (I-39). El orden `Declared > Imported > Algorithmic > Inferred` es el del enum `ProvenanceKind` y sigue la idea de ADR-0014 (la afirmación explícita antes que la derivada; la confianza numérica solo existe en `Inferred`); ADR-0014 no fija una escala de confianza, así que esta es una elección de este paquete, sin medir.
- **Recencia.** Se mira lo más reciente entre la revisión cabeza de la unidad (`Unit::updated_seq`, leída de la instantánea) y la relación que la alcanzó (`created_seq` del último paso). Es una secuencia de commit, nunca un reloj (I-26). `H` es la edad a la que queda la mitad del peso; la función es monótona, acotada en `[0, w]` y solo usa enteros. Una edad por encima de `u32::MAX` pesa como `u32::MAX` (la aportación ya es 0 ahí).
- **Una unidad sin vector** aporta 0, igual que una de coseno 0; la salida de diagnóstico las distingue (`similarity: None` frente a `Some(0)`).
- **Un resultado de otra instantánea** (una unidad que ya no es activa, o cuya revisión cabeza no es la del resultado) se rechaza con `RankError::NotFromThisSnapshot`, que no nombra la unidad. Con `Off` no se lee nada y no hay comprobación.
- **Cota.** El ranking reordena lo que la consulta devolvió. Un resultado que la consulta cortó (`LimitReached`, `BudgetExhausted`) no se ve: ranquear antes del corte exigiría cambiar el recorrido de `assoc.rs` (§10).

## 4. Pesos y por qué

Las constantes son públicas y `RankWeights::default()` las usa. Un peso 0 apaga una señal y uno negativo la invierte (la prueba de I-41 usa pesos al azar, incluidos negativos).

| Constante | Valor | Razón |
|---|---|---|
| `WEIGHT_HOPS` | 1 000 | Es la unidad en que se miden las demás. La cercanía en el grafo es la evidencia estructural de asociación y la única señal que todo resultado tiene sin que nadie la suministre |
| `WEIGHT_PROVENANCE` | 250 por paso (0 a 750) | 3 · 250 < 1 000: la procedencia sola nunca da la vuelta a un salto |
| `WEIGHT_TYPE` | 500 | Por debajo de un salto: el gusto de quien llama por un tipo afina, no supera a la cercanía |
| `WEIGHT_RECENCY` | 400 | La más débil: algo tocado hace poco no está por eso más relacionado; solo desempata contiendas cercanas |
| `WEIGHT_SIMILARITY` | 1 500 | Entre uno y dos saltos: una coincidencia semántica fuerte por sí sola puede superar un salto de distancia, nunca dos |
| `RECENCY_HALF_LIFE` | 64 commits | Una escala de decenas de commits. El valor exacto no tiene una medición detrás: es una elección a revisar en E7-05 |

Las relaciones entre los pesos son la justificación de los valores, y por eso están fijadas en tiempo de compilación (`const _: () = assert!(…)`): ninguna señal que no sea de saltos supera sola un salto (`3·p < h`, `t < h`, `r < h`); las dos más fuertes sumadas sí (`3·p + t > h`); la similitud supera uno y no dos (`h < s < 2h`). **Los valores concretos no están afinados**: no se midieron contra el conjunto congelado ni contra ningún otro. Si E7-05 cambia un valor, cambia la constante y esta tabla en el mismo cambio, diciendo qué medición lo justifica.

## 5. Determinismo

- Todo es entero salvo la similitud vectorial (§6). Ningún orden compara un `f32`/`f64`.
- El orden es total: `(puntuación descendente, identificador de unidad ascendente)`. Un empate de puntuación lo resuelve el identificador.
- Nada depende de un mapa hash (se usan `BTreeSet` y `BTreeMap`), del reloj, del orden en que quien llama pasó los vectores ni de la ruta que produjo el resultado: por I-32 el resultado es el mismo por la referencia, el índice y el planificador, y la prueba lo comprueba sobre el ranking completo, aportaciones incluidas.
- Las sumas no pueden desbordar: los pesos son `i32`, cada aportación es un peso por un factor pequeño y `i64` sobra.

## 6. Vectores (ADR-0011)

Los vectores los entrega quien llama, viven en memoria y no se escriben en ninguna parte. El módulo no depende de red ni de modelo (I-31) y no añade dependencias (`Cargo.toml` no cambia).

| Tipo | Contenido |
|---|---|
| `VectorSet` | Una dimensión (mayor que 0), una métrica y los vectores por unidad. `insert` comprueba cada vector; `search(query, k)` es la fuerza bruta exacta; `similarity(unit, query)` da la de una unidad |
| `VectorQuery` | La métrica y el vector de consulta, comprobado al construirse |
| `VectorMetric` | `Cosine` o `Dot` |
| `VectorError` | `ZeroDimension`, `Empty`, `DimensionMismatch { expected, got }`, `MetricMismatch { set, query }`, `NonFinite { index }`, `ZeroVector` |

Rechazos, todos antes de ordenar nada y en los tres modos:

| Caso | Error | Dónde |
|---|---|---|
| Conjunto de dimensión 0 | `ZeroDimension` | `VectorSet::new` |
| Vector vacío | `Empty` | `insert`, `VectorQuery::new` |
| Dimensión distinta de la del conjunto | `DimensionMismatch` | `insert`, `VectorSet::check` (consulta) |
| Métrica de la consulta distinta de la del conjunto | `MetricMismatch` | `VectorSet::check` |
| Componente NaN o infinita (con su posición) | `NonFinite` | `insert`, `VectorQuery::new` |
| Vector de norma cero con coseno (`0.0` y `-0.0` lo son) | `ZeroVector` | `insert`, `VectorQuery::new` |

Orden de comprobación: vacío, dimensión, no finito, cero; y en la comparación, métrica y luego dimensión. Un vector rechazado no se guarda. Con producto escalar un vector cero es válido. Ningún error lleva un valor de un vector, solo posiciones y dimensiones.

### Cómo se hace estable la señal vectorial

Es la única operación con coma flotante. Se hace estable así, y se dice aquí para que se pueda discutir:

1. Toda entrada es `f32`, que `f64` representa sin error, y los productos de dos `f32` son exactos en `f64`.
2. Los componentes se recorren en orden de índice con un acumulador por magnitud (`dot`, `‖a‖²`, `‖b‖²`) y solo se usan `+`, `*`, `/` y `sqrt`, que IEEE-754 define redondeadas correctamente. Rust no reasocia ni fusiona (FMA) operaciones de coma flotante, y no se llama a ninguna función de `libm` (`exp`, `ln`, `pow`, `acos`). Por tanto el mismo vector da los mismos bits en cualquier plataforma IEEE-754 de las que Rust soporta, en cualquier ejecución.
3. El resultado se **redondea a punto fijo** (`round`, mitad lejos de cero) a seis decimales antes de tocar ninguna otra cifra. La puntuación no contiene un flotante y ningún orden compara uno. Un `f64` fuera del rango de `i64` satura; un coseno que el redondeo deja una unidad en el último lugar por encima de 1,0 vuelve a 1 000 000 por el mismo redondeo (por eso no hay un `clamp` al calcularlo; el `clamp` está al convertir en aportación).

Límite honesto: dos plataformas que difirieran en un `f64` justo en una frontera de redondeo de la sexta cifra darían una unidad de diferencia en la similitud. El diseño lo evita por construcción (punto 2) pero no lo demuestra con otra plataforma: solo se probó en esta. No lo detectaría una prueba de este árbol.

### Producto escalar y la puntuación

El producto escalar no está acotado. La similitud exacta se informa sin recortar (`similarity` en el diagnóstico, `search`), pero la aportación a la puntuación se recorta a `[−1, 1]` antes de multiplicar por `WEIGHT_SIMILARITY`: **por encima de 1,0 el producto escalar deja de distinguir** unidades en la puntuación. Quien necesite resolución con magnitudes grandes debe normalizar la escala de sus vectores o usar `VectorSet::search` directamente. No se añadió un parámetro de escala porque nadie lo pidió.

## 7. Pruebas

`tests/assoc_rank.rs` tiene 10 funciones: 9 se ejecutan sobre `Emulated<LogEngine>` y `KernoDb` (18 pruebas) y la de similitud, que no toca ningún motor, una vez: **19 pruebas**. El conjunto congelado no se usa.

| Prueba | Qué demuestra |
|---|---|
| `contributions_add_up_to_the_score_and_match_an_oracle` (I-41) | 8 semillas, 2 métricas, 25 consultas cada una sobre grafos de 8 a 14 unidades, pesos al azar (también negativos), conjuntos de tipos preferidos y vectores sobre la mayoría de las unidades. Para cada resultado: las cinco aportaciones son las de un oráculo que parte de `get_relation` y `get_unit` (no del rastro), la puntuación es su suma, el orden es `(puntuación, id)`, `On` ordena como `Diagnostic` sin mostrar aportaciones. Exige más de 400 resultados ordenados, más de 100 con vector y más de 20 con alguna aportación negativa |
| `hand_vectors_without_a_vector_signal` | Un mundo de cinco unidades con cada cifra calculada a mano con los pesos por defecto (§7.1): 5 626, 4 650, 4 382 y 3 643, y el orden `b, d, c, e` frente al de referencia `b, c, d, e` |
| `hand_vectors_with_cosine_and_dot` | Las mismas con coseno (`+1500`, `0`, `−1500`, y la unidad sin vector sin cambio) y con producto escalar (0,25 se queda, 2,0 se recorta a 1,0 y se informa como 2 000 000). Fija los valores por defecto de los pesos |
| `same_state_and_query_give_the_same_order` | 6 mundos, 30 consultas cada uno: el ranking completo, aportaciones incluidas, es el mismo para el resultado de la ruta de referencia, la ruta por índice y el planificador, repetido, y desde una segunda instantánea del mismo estado |
| `ties_are_broken_by_identifier` | Cuatro hojas que empatan en todas las señales salen por identificador ascendente; con todos los pesos en 0, todo empata y el orden es el identificador puro, distinto del de referencia (que pone antes lo más cercano) |
| `ranking_off_is_the_reference_order` | 5 mundos, 40 consultas: con las opciones por defecto, el orden es el de la ruta de referencia y el de la de índice, sin puntuación; `Off` es el modo por defecto; el orden de referencia es `(saltos, id)`; con `On` el orden cambia en más de 10 casos (la prueba no es vacía) |
| `a_result_from_another_state_is_refused_unless_ranking_is_off` | Tras modificar una unidad, el resultado antiguo se rechaza en `On` y `Diagnostic` y se acepta en `Off`; sobre su propia instantánea se acepta |
| `every_vector_rejection` | Todos los casos de §6, con las dos métricas; un vector rechazado no se guarda; el orden métrica-dimensión; y que `rank_assoc` rechaza en los tres modos, también con un resultado sin resultados |
| `similarity_matches_an_independent_oracle` | Vectores de octavos (`k/8`, exactos en `f32` y `f64`), 5 dimensiones, 2 métricas, 6 consultas cada una (más de 1 000 comparaciones). Para el producto escalar el oráculo es exacto: `15 625 · D`. Para el coseno, una comprobación en enteros de que `m` está a media unidad de `10⁶ · D / √(‖a‖²‖b‖²)`, sin flotantes. La búsqueda compara todos los vectores una vez, ordena `(similitud desc, id asc)` y `k` da un prefijo. Valores a mano (1,0; 0,6; 0; 50 000 000) y saturación con `f32::MAX` |
| `sentinels_no_literal_outside_the_diagnostic_output` (I-37) | Un tipo de relación y componentes centinela. Ni el `Debug` de `VectorSet`, `VectorQuery`, `RankOptions`, ni la salida con `Off` y `On`, ni ninguno de los errores (`Display` y `Debug`: siete de vectores, uno de dimensión en `rank_assoc`, otro `NotFromThisSnapshot`) los contienen, ni nombran una unidad; la salida de diagnóstico, que solo lleva enteros, tampoco. Controles: el resultado sí contiene el tipo, y la salida contiene `RankedHit` y `Contributions`. El ranking no deja ningún registro en el observador a nivel `diagnostic` |

### 7.1 Los vectores a mano

Mundo: unidades `a b c d e` (seq 1); `a –cites→ b` Declared (seq 2); `a –mentions→ c` Inferred (seq 3); `b –cites→ d` Declared (seq 4); `c –cites→ e` Imported (seq 5); `d` se actualiza (seq 6). Consulta desde `a`, saliendo, profundidad 2; tipo preferido `cites`; la instantánea está en seq 6. Pesos por defecto.

| Unidad | Saltos | Tipo | Recencia | Procedencia | Total |
|---|---|---|---|---|---|
| `b` | 4 000 | 500 (1 de 1) | 376 (`400·64/(64+4)`, más reciente seq 2) | 750 (Declared) | **5 626** |
| `d` | 3 000 | 500 (2 de 2) | 400 (seq 6, edad 0) | 750 (Declared, Declared) | **4 650** |
| `c` | 4 000 | 0 (`mentions`) | 382 (`400·64/(64+3)`, seq 3) | 0 (Inferred) | **4 382** |
| `e` | 3 000 | 250 (1 de 2) | 393 (`400·64/(64+1)`, seq 5) | 0 (Inferred es el más débil) | **3 643** |

Cada entrada pasa por la división entera: `25 600 / 68 = 376,5…` → 376.

## 8. Mutaciones

Cada mutación se aplicó a `assoc_rank.rs` sin mutar con un guion que sustituye un texto, ejecuta `cargo test -p kerno-units --release --test assoc_rank --no-fail-fast`, anota las pruebas rojas y restaura el fichero (comprobado con `diff` contra una copia). Las 42 compilaron salvo un primer intento de M34 con un cambio incompleto, que se rehízo; las 42 fueron detectadas. Una mutación que sobrevivía no apareció, pero una revisión previa mostró que el `clamp(-1, 1)` del coseno era redundante con el redondeo a punto fijo (§6) y se eliminó en lugar de dejarlo sin guardia. «Prueba» abrevia el nombre de la función; todas fallan en sus dos motores salvo `similarity_matches_an_independent_oracle`.

| # | Mutación | Pruebas que fallaron |
|---|---|---|
| M01 | El desempate pasa a identificador descendente | `contributions_…`, `ties_are_broken_by_identifier` |
| M02 | La puntuación ordena de menor a mayor | `contributions_…`, `hand_vectors_without_…`, `hand_vectors_with_…` |
| M03 | `Off` ordena por puntuación | `ranking_off_is_the_reference_order`, `a_result_from_another_state_…` |
| M04 | El modo por defecto es `On` | las mismas dos |
| M05 | Los saltos puntúan más cuanto más lejos | `contributions_…`, las dos de vectores a mano |
| M06 | El tipo no se divide por el número de pasos | las mismas tres |
| M07 | La procedencia toma el eslabón más fuerte | las mismas tres |
| M08 | `Imported` se fía como `Declared` | `contributions_…` |
| M09 | La recencia ignora la relación que alcanzó la unidad | las mismas tres |
| M10 | La recencia ignora la edad | las mismas tres |
| M11 | La similitud no se recorta a `[−1, 1]` | `contributions_…`, `hand_vectors_with_…` |
| M12 | La suma omite la similitud | las mismas dos |
| M13 | La puntuación es la suma más 1 | `contributions_…`, las dos de vectores a mano, `ties_…` |
| M14 | El diagnóstico no muestra las aportaciones | `contributions_…`, las dos a mano, `sentinels_…` |
| M15 | Los vectores no se comprueban | `every_vector_rejection` |
| M16 | `Off` no comprueba los vectores | `every_vector_rejection` |
| M17 | Se comprueba la dimensión antes que la métrica | `every_vector_rejection` |
| M18 | La métrica no se comprueba | `every_vector_rejection`, `sentinels_…` |
| M19 | Un vector cero se acepta con coseno | `every_vector_rejection`, `sentinels_…` |
| M20 | Un infinito se acepta | `every_vector_rejection`, `sentinels_…` |
| M21 | `insert` no comprueba la dimensión | `every_vector_rejection`, `sentinels_…` |
| M22 | Un vector de consulta vacío se acepta | `every_vector_rejection` |
| M23 | `insert` comprueba la dimensión antes que el vacío | `every_vector_rejection` |
| M24 | Un conjunto de dimensión 0 se acepta | `every_vector_rejection`, `sentinels_…` |
| M25 | El coseno divide solo por una norma | `hand_vectors_with_…`, `similarity_matches_an_independent_oracle` |
| M26 | La similitud se trunca en vez de redondear | `similarity_matches_an_independent_oracle` |
| M27 | La búsqueda ordena de menos a más similar | `similarity_matches_an_independent_oracle` |
| M28 | El empate de la búsqueda va por identificador descendente | `similarity_matches_an_independent_oracle` |
| M29 | La búsqueda ignora `k` | `similarity_matches_an_independent_oracle` |
| M30 | Cualquier resultado se acepta como de esta instantánea | `a_result_from_another_state_…`, `sentinels_…` |
| M31 | La secuencia de la revisión cabeza vale 0 | `contributions_…`, las dos a mano |
| M32 | El `Debug` de `VectorSet` imprime los componentes | `sentinels_…` |
| M33 | El `Debug` de `VectorQuery` imprime los componentes | `sentinels_…` |
| M34 | El `Debug` de `RankOptions` imprime los tipos | `sentinels_…` |
| M35 | El producto escalar se calcula como coseno | `hand_vectors_with_…`, `similarity_matches_an_independent_oracle` |
| M36 | `WEIGHT_HOPS` pasa de 1 000 a 1 100 | las dos de vectores a mano |
| M37 | `RECENCY_HALF_LIFE` pasa de 64 a 32 | `contributions_…`, las dos a mano |
| M38 | Una unidad sin vector cuenta como similitud −1 | `contributions_…`, las dos a mano |
| M39 | La señal de tipo se invierte | `contributions_…`, las dos a mano |
| M40 | Los pasos se cuentan por `distance + 1` y no por el camino | `contributions_…`, las dos a mano |
| M41 | La recencia suma un bit del reloj | `contributions_…`, las dos a mano, `same_state_and_query_give_the_same_order`, `ties_…` |
| M42 | Los empates se dejan en el orden de entrada invertido, sin identificador | `contributions_…`, las dos a mano, `ties_…` |

M30 también hace fallar `sentinels_…` porque esa prueba comprueba el rechazo de un resultado de otra instantánea. Las pruebas se editaron después de la primera ronda (el centinela numérico y la función de similitud sin motor); M26, M28, M29, M32, M33, M34 y M35 se volvieron a ejecutar con el fichero final y siguen detectadas.

## 9. Verificación

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_rank
```

Conteo de pruebas de `kerno-testkit` + `kerno-units` con `--release --all-features`: **630 aprobadas, 0 fallidas, 8 ignoradas** con este cambio; antes eran 611 (E7-02). Las 19 nuevas son las de `assoc_rank`; el corredor las cuenta (`assoc_rank`: `19 passed`).

## 10. Límites y lo que habría añadido a `assoc.rs`

- **La recencia necesita un dato que `AssocHit` no lleva.** `AssocHit` trae `revision` (`RevNo`) pero no el `commit_seq` de esa revisión. Se lee de la instantánea con la API pública: `reference_get(hit.unit)` y `Unit::updated_seq`, por el identificador del resultado (una lectura por resultado, con verificación de la carga útil). Lo que habría añadido a `assoc.rs`: un campo `head_seq: CommitSeq` en `AssocHit`, que el recorrido ya tiene en la mano en `traverse` (`reference_get(there)` devuelve la unidad); evitaría una lectura por resultado y la comprobación `NotFromThisSnapshot` sería innecesaria para este dato. El rastro (`TraceStep`) ya lleva tipo, `created_seq` y clase de procedencia, y no hizo falta nada más.
- **El corte se hace antes del ranking.** `limit` y `BudgetExhausted` cortan por «saltos, luego identificador»; el ranking solo reordena lo que quedó. Si E7-05 mostrara que lo relevante está más allá del corte, habría que ranquear dentro del recorrido (cambio de `assoc.rs`) o pedir un límite mayor.
- **Los pesos no están medidos.** Son una justificación estructural (§4), no un resultado. La señal de tipo no existe sin un conjunto de tipos preferidos y no hay uno por defecto.
- **Producto escalar** saturado a ±1,0 en la puntuación (§6).
- **Estabilidad entre plataformas** por construcción, no comprobada en otra plataforma (§6).
- **Coste.** Una lectura de unidad por resultado y, con vectores, una comparación por resultado; la búsqueda exacta es lineal en el tamaño del conjunto. No se afirma nada de rendimiento (E7-05).
- **Sin observaciones.** `rank_assoc` no informa al observador: no es una ejecución del planificador. Si E7-05 o E7-06 quisieran contarla, sería una observación nueva y estaría sujeta a I-37.
- **Dependencias.** Ninguna nueva; `Cargo.toml` no cambia.

## 11. Documentos relacionados

- [Acta de la Etapa 7](etapa7-acta.md) §3 (E7-04), §4 (I-41, I-37), §7 punto 3
- [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md)
- [Consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [ejecución indexada (E7-02)](etapa7-ejecucion-indexada.md)
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): procedencia de las relaciones
- [Invariantes](invariants.md): I-26 (el reloj no ordena), I-32, I-37, I-39
