# Consulta asociativa: modelo y ruta de referencia (E7-01)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Hecho** en E7-01 |
| Acta | [Etapa 7](etapa7-acta.md) §3 (fila E7-01), §4, §8 (decisión 5: cotas duras) |
| Código | `crates/kerno-units/src/assoc.rs` (modelo y ruta de referencia), exportado desde `lib.rs` |
| Pruebas | `crates/kerno-units/tests/assoc_reference.rs` |
| Conjunto de evaluación | [etapa7-conjunto-evaluacion.md](etapa7-conjunto-evaluacion.md) (congelado; este paquete no lo modifica) |

Este documento describe el código tal como está. Cada cifra sale de los comandos de §11 o de las mutaciones de §9.1. Este paquete no usa el índice de adyacencia (es E7-02), no toca `planner.rs`, no ordena por puntuación (es E7-04) y no cambia el formato persistente.

## 1. Qué añade

- `AssocQuery`: la consulta, con cotas obligatorias y sus máximos duros.
- `UnitSnapshot::reference_assoc_query`: la ruta de referencia, un recorrido en anchura construido **solo** sobre `reference_relations_of` (nunca sobre `relations_of`). Es la definición de lo que la ruta por índice de E7-02 deberá devolver sobre la misma instantánea (I-32).
- `AssocResult`: los resultados, cada uno con su rastro, y la razón por la que el recorrido terminó.

## 2. El modelo de consulta

| Campo de `AssocQuery` | Tipo | Significado |
|---|---|---|
| `seeds` | `Vec<UnitId>` | Una o más semillas. Las repetidas cuentan una vez |
| `relation_types` | `Option<BTreeSet<String>>` | Tipos de relación que se siguen; `None`, cualquiera |
| `direction` | `AssocDirection` | `Out` (relaciones que salen de la unidad), `In` (que llegan) o `Both` |
| `max_depth` | `u32` | Saltos máximos desde una semilla |
| `limit` | `usize` | Resultados máximos |
| `visit_budget` | `u64` | Entradas de relación examinadas como máximo |
| `window` | `Option<SeqWindow>` | Ventana inclusiva de `commit_seq`, aplicada al `created_seq` de cada relación que se sigue |
| `provenance` | `ProvenanceFilter` | `kinds` (clases de `ProvenanceKind`), `actor` y `source` del conjunto de cambios que creó la relación; cada campo presente debe cumplirse |

`AssocDirection` es un tipo propio porque `Direction` solo tiene `Out` e `In`, y `Direction` es el de las lecturas de un salto. `AssocQuery::new(seeds, max_depth, limit, visit_budget)` pide las tres cotas (no tienen valor por defecto) y deja el resto en «cualquier tipo, ambas direcciones, sin ventana, sin filtro».

El actor y la fuente del filtro se comparan con los del registro `CHANGESET` del `created_seq` de la relación (`UnitSnapshot::change_set`). La relación no guarda actor ni fuente: guarda solo su `created_seq` (ADR-0014). Un conjunto de cambios sin actor no cumple un filtro de actor. Un `created_seq` sin registro `CHANGESET` es `Storage(Corruption)`. Los registros leídos se recuerdan dentro de una consulta, por `commit_seq`.

## 3. Cotas

Los máximos duros son constantes públicas (acta §8, decisión 5). Quien llama puede bajar las cotas, no subirlas.

| Constante | Valor | Se rechaza |
|---|---|---|
| `MAX_ASSOC_DEPTH` | 4 | `max_depth` 0 o mayor que 4 |
| `MAX_ASSOC_LIMIT` | 1 000 | `limit` 0 o mayor que 1 000 |
| `MAX_ASSOC_VISITS` | 100 000 | `visit_budget` 0 o mayor que 100 000 |

`AssocQuery::validate` comprueba la consulta sin leer nada, y `reference_assoc_query` la llama antes de abrir ninguna unidad. Los errores son del tipo `AssocQueryError`, y `AssocError::Query` los lleva:

| Variante | Cuándo |
|---|---|
| `NoSeeds` | Sin semillas |
| `DepthOutOfRange { depth }` | Profundidad fuera de `1..=4` |
| `LimitOutOfRange { limit }` | Límite fuera de `1..=1000` |
| `BudgetOutOfRange { budget }` | Presupuesto fuera de `1..=100000` |
| `EmptyRelationTypes` | Conjunto de tipos presente y vacío (no seguiría nada) |
| `EmptyProvenanceKinds` | Conjunto de clases presente y vacío |
| `InvertedWindow { lo, hi }` | Ventana con `lo > hi` |

Si falla más de una comprobación, gana la primera de este orden: semillas, profundidad, límite, presupuesto, tipos, clases, ventana. Los valores `1`, `4`, `1000` y `100000` son válidos.

Una semilla que no es una unidad activa en la instantánea (no existe, está archivada o está borrada) rechaza la consulta entera con `AssocError::SeedNotActive { unit }`, aunque haya otras semillas buenas. Es un rechazo en ejecución, no de validación, porque depende de la instantánea.

## 4. Orden y desempate (I-26)

Los resultados salen en el orden **(distancia en saltos, identificador de unidad)**. Es el orden «saltos, luego identificador» del acta (§6.2, §7 punto 3), y por tanto el que E7-04 devuelve con el ranking apagado.

Cuál es el camino que lleva un resultado lo decide el orden de examen, que es el desempate documentado:

1. Las unidades de un nivel se expanden en orden ascendente de identificador. Las semillas son el nivel 0 y se ordenan igual: el orden en que quien llama las pasa no cambia nada.
2. Para una unidad, primero las relaciones que salen de ella (`Out`) y después las que llegan (`In`), si la consulta sigue ambas.
3. Dentro de una dirección, por bytes del tipo de relación y luego por bytes del identificador de relación (el orden de `reference_relations_of`).

Una unidad se devuelve una sola vez, con el primer camino que la alcanza en ese orden. Las semillas no son resultados, aunque otra semilla las alcance. Un ciclo, una relación paralela (mismos extremos y tipo) o una relación de una unidad consigo misma no duplican resultados: la unidad ya está vista.

Nada depende de la iteración de un mapa hash (se usan `BTreeSet` y `BTreeMap`) ni de un reloj. Un nivel se examina entero y sus resultados se ordenan por identificador; ahí está la única consecuencia de este orden que conviene conocer (§6).

## 5. Presupuesto de visitas

Una entrada se **examina** cuando el recorrido mira una relación adyacente a la unidad que expande, en una dirección que sigue y con un tipo del conjunto de tipos de la consulta (cualquiera si no hay conjunto). Cuenta antes de aplicar la ventana y el filtro de procedencia, que necesitan el registro de la relación, y cuenta aunque no lleve a una unidad nueva. Dos consecuencias que las pruebas fijan:

- Las entradas de un tipo que la consulta no sigue no se examinan. Es lo que lee un rango del índice por tipo, así que la ruta por índice de E7-02 cuenta lo mismo.
- Una relación de una unidad consigo misma tiene dos entradas de adyacencia: con `Both` se examina dos veces.

El presupuesto se comprueba justo antes de examinar una entrada. Un presupuesto que alcanza exactamente para terminar no se agota; uno menos, sí.

## 6. Razones de fin

`AssocResult::ending` es una de tres, y solo una:

| `AssocEnding` | Significa | Qué pueden asumir quienes llaman |
|---|---|---|
| `Exhausted` | No quedaba nada dentro de las cotas de la consulta (también cuando el recorrido llegó a la profundidad máxima) | Los resultados son **toda** la respuesta |
| `LimitReached` | Existen más resultados que `limit`. Se encontró uno más | Los resultados son los `limit` primeros de la respuesta en el orden de §4 |
| `BudgetExhausted` | El presupuesto se acabó antes de poder decidir | Los resultados son válidos pero puede faltar alguno |

Precisiones:

- «Nada que quede» incluye el límite de profundidad: una consulta de profundidad 1 que devuelve todos los vecinos termina `Exhausted`, porque la profundidad es parte de la pregunta.
- Con `limit = k` y exactamente `k` resultados posibles, la razón es `Exhausted`, no `LimitReached`. `LimitReached` exige haber visto un resultado `k + 1`. Cuando los `k` primeros ya están completos, el recorrido para en cuanto aparece el siguiente; si el límite se cruza dentro de un nivel, termina el nivel y corta por identificador.
- Si el presupuesto se agota a mitad de un nivel, los resultados de ese nivel que ya se habían descubierto salen ordenados por identificador, y pueden faltar unidades con identificador menor que algunas devueltas. Cada resultado devuelto es uno de la respuesta completa, con el mismo camino (la prueba `bounded_answers_agree_with_the_unbounded_one` lo comprueba para todo presupuesto). Por eso `BudgetExhausted` no promete un prefijo y `LimitReached` sí. Si se agota el presupuesto y hay más de `limit` resultados, se corta en `limit` y la razón sigue siendo `BudgetExhausted`: es la afirmación más débil y no se pierde.
- `visited` informa de las entradas examinadas y nunca supera el presupuesto.

## 7. El rastro

Cada resultado (`AssocHit`) lleva:

| Campo | Contenido |
|---|---|
| `unit` | La unidad |
| `distance` | Saltos desde la semilla más cercana; igual a `path.len()` |
| `revision` | `RevNo` de la cabeza de la unidad en la instantánea |
| `path` | `Vec<TraceStep>`: las relaciones seguidas, de la semilla a la unidad, en orden |

Cada `TraceStep` guarda `relation` (identificador), `relation_type`, `from`, `to`, `created_seq` y `provenance` (un `ProvenanceKind`). `from` y `to` son los extremos de la relación tal como se guardó, no la dirección en que se siguió: una relación `u2 → u1` seguida desde `u1` aparece con `from = u2`. El paso lleva la clase de procedencia y no sus datos, y una relación inferida nunca aparece como declarada (I-39).

Con este rastro, quien llama puede comprobar un resultado con `get_relation`, `get_unit` y `change_set` sobre la misma instantánea. La prueba `frozen_corpus_traces_are_verifiable` lo hace (§9.2). La prueba general de I-40 es de E7-03.

## 8. Unidades archivadas y borradas

La capa **no permite** archivar ni borrar una unidad que una relación toca (ADR-0014; error `UnitHasRelations`). En una instantánea sana, todo extremo de toda relación está `Active`, así que no hay nada que decidir. El recorrido decide de todos modos qué hace si no es así (un daño que `verify_units` informa como UNIT-36):

| Caso | Decisión |
|---|---|
| Semilla inexistente, archivada o borrada | La consulta se rechaza con `SeedNotActive` |
| Otro extremo inexistente, archivado o borrado | Se trata como ausente: no se devuelve y no se atraviesa. La entrada que llevó a él cuenta como examinada |

Es la regla de la ruta de referencia de Etapa 4 («una consulta ve solo unidades activas», [`reference.rs`](../../crates/kerno-units/src/reference.rs)). Se eligió sobre las alternativas por esto: devolver la unidad con su estado obligaría a quien llama a filtrar, y fallar con corrupción convertiría una consulta de lectura en un verificador. El daño se informa con `verify_units`. Una instantánea anterior al archivado sigue viendo la unidad y sus relaciones: no hay otro efecto del archivado sobre una consulta.

## 9. Pruebas y mutaciones

`tests/assoc_reference.rs` tiene **19 funciones de prueba**, ejecutadas sobre `Emulated<LogEngine>` y sobre `KernoDb`: **38** pruebas. Las expectativas son vectores escritos a mano sobre dos grafos pequeños y, para el corpus congelado, un recorrido en anchura independiente escrito en la propia prueba sobre todas las relaciones (`oracle`).

| Grafo | Contenido |
|---|---|
| Rombo | 7 unidades `a..g`; 9 relaciones: `a→b`, `a→c`, `b→d`, `c→d`, `d→a` (ciclo), `d→e` (tipo `u`), `e→f`, `a→b` (paralela), `c→c` (propia). `g` está aislada |
| Procedencia | 6 unidades y 7 relaciones de las cuatro clases, con actores `alice`, `importer`, `bot`, uno sin actor, y fuentes `doc/1`, `doc/2`, `wiki/1`, `job/1`, `job/2` |

Conteo de pruebas del paquete `kerno-testkit` + `kerno-units` con `--release --all-features`: **586 aprobadas** con este cambio; el conteo anterior es **548** (586 − 38; derivado, no se ejecutó en el árbol anterior). El corredor cuenta las 38: `assoc_reference` aparece con `38 passed`.

### 9.1 Mutaciones

Cada mutación se aplicó al código sin mutar, se ejecutó `cargo test -p kerno-units --release --test assoc_reference`, se anotaron las pruebas que fallaron y se restauró el fichero con `git checkout -- <fichero>`. Después de la última, `git diff` no mostraba cambios en el árbol de trabajo. Las líneas son las de `src/assoc.rs` sin mutar. Todos los nombres son de las funciones de prueba; cada una fallaba en sus dos motores. «Otras» son las pruebas que además fallaron.

| Prueba | Mutación (línea) | Falló | Otras que fallaron |
|---|---|---|---|
| `validation_rejects_every_unbounded_or_malformed_query` | 153: se quita `self.max_depth == 0 \|\|` (profundidad 0 aceptada) | esa | `seeds_must_be_active_units` |
| `hard_maxima_are_public_and_accepted` | 153: `> MAX_ASSOC_DEPTH` → `>=` (profundidad 4 rechazada) | esa | 13 más que usan profundidad 4 |
| `seeds_must_be_active_units` | 311: la comprobación de semilla activa pasa a `if false` | esa | `ends_that_are_not_active_are_absent` |
| `seeds_are_not_results_and_order_does_not_depend_on_them` | 319: `seen` empieza vacío (las semillas pueden volver como resultados) | esa | 11 más |
| `out_traversal_vector` | 337: `!seen.insert(there)` → `seen.contains(&there)` (gana el último camino) | esa | 12 más |
| `in_traversal_vector` | 85: `In` sigue `Direction::Out` | esa | `frozen_corpus_traces_are_verifiable` |
| `both_traversal_vector_with_cycle_and_self_relation` | 86: `Both` sigue solo `Out` | esa | `ends_that_are_not_active_are_absent`, `frozen_corpus_traces_are_verifiable`, `relation_type_filter`, `seeds_are_not_results_and_order_does_not_depend_on_them`, `trace_carries_the_fields_of_each_relation_and_the_head_revision` |
| `trace_carries_the_fields_of_each_relation_and_the_head_revision` | 349: `revision: unit.head` → `RevNo(1)` | solo esa | — |
| `trace_carries_the_fields_of_each_relation_and_the_head_revision` | 379: la clase del paso siempre `Declared` | esa | `provenance_filters_by_kind_actor_and_source`, `frozen_corpus_traces_are_verifiable` |
| `relation_type_filter` | 369: el filtro de tipos devuelve siempre `true` | esa | `the_three_endings_are_distinct`, `frozen_corpus_traces_are_verifiable` |
| `window_is_inclusive_and_applies_to_each_relation_followed` | 394: `<`/`>` → `<=`/`>=` (ventana exclusiva) | esa | `provenance_filters_by_kind_actor_and_source`, `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 405: el filtro de actor devuelve `true` | esa | `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 398: el filtro de clases invertido | esa | `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 406: el filtro de fuente devuelve `true` | esa | `frozen_corpus_traces_are_verifiable` |
| `the_three_endings_are_distinct` | 429: el fin por presupuesto se informa como `LimitReached` | esa | `bounded_answers_agree_with_the_unbounded_one` |
| `bounded_answers_agree_with_the_unbounded_one` | 357: no se corta en `limit` tras un nivel | esa | `the_three_endings_are_distinct`, `frozen_corpus_traces_are_verifiable` |
| `bounded_answers_agree_with_the_unbounded_one` | 329: `==` → `>` (se examina una entrada más del presupuesto) | esa | `the_three_endings_are_distinct` |
| `same_logical_graph_gives_byte_identical_results` | 354: la frontera se recorre en orden descendente de identificador | esa | `in_traversal_vector`, `out_traversal_vector`, `provenance_filters_by_kind_actor_and_source`, `relation_type_filter` |
| `repeated_runs_are_byte_identical` | 364: `visited` suma un bit del reloj | esa | 10 más que comparan `visited`; el número varía entre ejecuciones porque el bit del reloj cambia |
| `a_snapshot_does_not_see_later_commits` | 307: el resultado se memoriza por consulta y no por instantánea (variable por hilo) | solo esa | — |
| `ends_that_are_not_active_are_absent` | 340: `reference_get` → `get_unit` (se devuelven unidades archivadas y borradas) | solo esa | — |
| `the_reference_path_never_reads_the_adjacency_index` | 327: `reference_relations_of` → `relations_of` | solo esa | — |
| `frozen_corpus_traces_are_verifiable` | 346: el camino se guarda al revés (`p.reverse()`) | esa | 8 más (los vectores) |

Dos precisiones sobre esta tabla:

- Con el índice de adyacencia ya destruido en la prueba `the_reference_path_never_reads_the_adjacency_index`, la ruta de referencia responde igual: es la prueba de que no lo lee. Si la mutación 327 se aplica, `relations_of` devuelve vacío y el resultado cambia.
- `same_logical_graph_gives_byte_identical_results` escribe el mismo grafo con identificadores fijados a mano (una fuente de identificadores de prueba) y con las unidades y las relaciones creadas en cuatro órdenes distintos, las relaciones de cada una en un único conjunto de cambios para que tengan el mismo `created_seq`. Compara la representación `Debug` de los resultados. Solo tiene sentido con identificadores fijos: con un contador, el orden de creación cambia los identificadores y, con ellos, el grafo lógico. Una mutación que dependiera del orden de inserción no tiene dónde apoyarse en esta ruta (el recorrido lee por clave, no por orden de escritura), así que la mutación de la tabla es la del orden de expansión.

### 9.2 Lo que comprueba el corpus congelado

`frozen_corpus_traces_are_verifiable` carga el corpus (2 000 unidades, 3 958 relaciones, 300 conjuntos de cambios) y ejecuta las 64 consultas con las restricciones del fichero (dirección, tipos, profundidad, ventana, clases, actor y fuente) y los límites máximos. Para cada resultado comprueba:

- orden `(distancia, unidad)` sin repetidos, y que ninguna semilla es un resultado;
- cada relación del camino existe en la instantánea con los mismos extremos, tipo, `created_seq` y clase de procedencia, y cumple tipo, ventana, clase, actor y fuente de la consulta;
- cada salto conecta con el siguiente, desde una semilla hasta la unidad, en una dirección que la consulta sigue;
- la unidad está activa y `revision` es su cabeza.

Además compara los resultados con el recorrido en anchura independiente: las 64 terminan `Exhausted` y el conjunto de resultados y sus distancias son los del recorrido de la prueba. Todas las respuestas esperadas del conjunto (incluidos los distractores de grado 0) están entre los resultados, 56 de las 64 consultas alcanzan distancia 2 o más, y una consulta de tres semillas sin restricciones termina `LimitReached` con los 1 000 primeros del recorrido en anchura. Esta comprobación no mide la calidad de la recuperación (es E7-05) y no cambia ningún dato del conjunto.

## 10. Límites y riesgos de este paquete

- **Coste**: cada expansión llama a `reference_relations_of`, que lee todos los registros `REL` (lineal en el número de relaciones). Es la ruta de referencia, correcta por definición y lenta a propósito. La prueba del corpus tarda unos seis segundos para las 38 pruebas en `--release`. No se afirma nada de rendimiento (es E7-05).
- **`BudgetExhausted` no es un prefijo** (§6). Quien necesite el prefijo debe usar el límite, no el presupuesto.
- **Semilla no activa rechaza la consulta entera** (§3). Es una elección; se podría omitir la semilla e informar de ello, lo que añadiría un campo al resultado que nadie ha pedido.
- **Sin ranking ni vectores**: el orden es «saltos, luego identificador».
- **Dependencia**: ninguna nueva. `Cargo.toml` no cambia.

## 11. Verificación

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_reference
```

La última línea da `test result: ok. 38 passed` sobre los dos motores.

## 12. Documentos relacionados

- [Acta de la Etapa 7](etapa7-acta.md) §3 (E7-01), §4, §8
- [Conjunto de evaluación](etapa7-conjunto-evaluacion.md) (corpus y consultas congelados)
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): relaciones y procedencia
- [Invariantes](invariants.md): I-26 (el reloj no ordena), I-32 (equivalencia con la ruta de referencia), I-39 (inferida no es declarada)
