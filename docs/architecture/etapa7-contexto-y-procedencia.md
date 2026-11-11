# Contexto temporal y procedencia de la consulta asociativa (E7-03)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Hecho** en E7-03 |
| Acta | [Etapa 7](etapa7-acta.md) §3 (fila E7-03), §4 (I-39, I-40), §2.1 (objetivos 2 a 4) |
| Código | Ninguno de producción: `crates/kerno-units/src/assoc.rs` no cambia. Pruebas nuevas en `crates/kerno-units/tests/assoc_context.rs`; `tests/harness/assoc.rs` gana `load_corpus_with_relations` y hace público `story_provenance` |
| Pruebas | `crates/kerno-units/tests/assoc_context.rs` (7 funciones × 2 motores) |
| Documentos previos | [consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [ejecución indexada (E7-02)](etapa7-ejecucion-indexada.md) · [conjunto de evaluación](etapa7-conjunto-evaluacion.md) |

Este documento describe el código tal como está. Cada cifra sale de los comandos de §8 o de las mutaciones de §7. Este paquete no cambia el formato persistente, ni el conjunto de evaluación, ni ningún umbral del acta, ni el comportamiento de las tres rutas (`reference_assoc_query`, `indexed_assoc_query`, `assoc_query`), y no toca el ranking (E7-04).

## 1. Qué había y qué faltaba

La ventana de `commit_seq` y los filtros de clase, actor y fuente ya se aplicaban en las dos rutas desde E7-01 y E7-02, y por una sola razón: el recorrido es uno (`traverse`, en `assoc.rs`) y las rutas solo cambian de dónde leen los vecinos. Por eso no hacía falta código nuevo, y no se añadió. Lo que no estaba probado, contra la fila de E7-03 del acta:

| Hueco | Qué había | Qué faltaba |
|---|---|---|
| Rastro verificable en la ruta por índice y en la del planificador | `frozen_corpus_traces_are_verifiable` (E7-01) verifica los rastros solo de la ruta de referencia. La prueba diferencial de E7-02 compara las tres rutas entre sí | Verificar los rastros de **cada** ruta contra algo que no sea otra ruta |
| Existencia de la revisión citada | `revision == unit.head`, leída de la propia biblioteca | Que la revisión se pueda leer en la instantánea, que su `commit_seq` no pase de la instantánea y que ninguna posterior sea visible. Los grafos de las pruebas diferenciales **nunca actualizan una unidad**: toda cabeza es la revisión 1, así que ese campo nunca podía diferir |
| Oráculo de la ventana y de los filtros | Vectores a mano (E7-01) y, para el corpus, `admitted`, que lee el actor y la fuente con `snap.change_set` (la biblioteca) | Un oráculo de fuerza bruta que no lea nada de la biblioteca, con combinaciones y respuestas vacías, en las tres rutas y sobre el corpus congelado |
| I-39 en el rastro | `relations::declared_is_never_confused_with_inferred` (capa de relaciones, sin consulta asociativa) | Que el rastro de una consulta asociativa, con relaciones paralelas de clases distintas entre los mismos extremos, nunca muestre una relación inferida, importada o algorítmica como declarada |
| I-26 | `history::ordering_ignores_clock` (historial) | Que ni el orden, ni el camino, ni la ventana, ni los filtros de una consulta asociativa dependan de las marcas de tiempo |

## 2. El modelo y el oráculo

Las expectativas no salen de la biblioteca. La prueba mantiene mientras escribe un **modelo**: cada relación con su identificador, extremos, tipo, `commit_seq`, procedencia completa, actor y fuente del conjunto de cambios; y, para cada unidad, el `commit_seq` de cada una de sus revisiones. Para el corpus congelado el modelo se construye desde el relato (`corpus.changes`, `corpus.relations`, `corpus.units`), no desde la base.

`oracle(model, seq, query)` es el recorrido en anchura escrito de nuevo desde el orden documentado en `assoc.rs` (niveles por identificador, `Out` antes de `In`, `(tipo, id)` por bytes) sobre **todas** las relaciones a la vez, y devuelve los resultados, sus caminos y las entradas examinadas. `assert_answer` compara con él cada campo: unidad, distancia, relaciones del camino, tipo, extremos, `created_seq` y clase de cada paso, revisión, `visited` y el fin `Exhausted`. Solo vale para consultas cuyo límite y presupuesto no se alcanzan; con cotas apretadas se usa `verify_trace` (§3).

El mundo de prueba (`World`) crece en conjuntos de cambios de 1 a 4 relaciones con actor y fuente aleatorios (a veces ausentes), las cuatro clases de procedencia, relaciones propias y paralelas, y 0 a 2 **unidades actualizadas** por conjunto. Se toman instantáneas intermedias (`old` y `new`). Los identificadores y las secuencias dependen solo de la semilla, nunca del reloj.

## 3. Qué comprueba `verify_trace` (I-40)

Para cada resultado de cada ruta, con cualquier cota:

- el orden `(distancia, unidad)` sin repetidos; ninguna semilla es un resultado; `path.len() == distance`;
- **la revisión**: es la cabeza que dice el modelo en esa instantánea; `get_revision(unidad, revisión)` la lee; su `commit_seq` es el del modelo y no pasa de la instantánea; `get_revision(unidad, revisión + 1)` es `RevisionNotFound`;
- **cada relación del camino**: existe en el modelo con `created_seq` no posterior a la instantánea; el paso repite tipo, extremos, `created_seq` y clase; `get_relation` la devuelve con los mismos extremos y tipo y con la procedencia completa (origen y confianza, no solo la clase); la consulta la deja pasar (tipo, ventana, clase, actor, fuente); ninguna se repite;
- **el camino es un recorrido real**: parte de una semilla, cada salto sale de la unidad en que se está en una dirección que la consulta sigue, y la unidad de cada posición intermedia es un resultado de la misma respuesta con la distancia de esa posición. Esa última condición es lo que hace que «la revisión de cada unidad» del acta esté citada: cada unidad del camino es una semilla o un resultado con su propia revisión.

## 4. Pruebas

Todas corren sobre `Emulated<LogEngine>` y `KernoDb`, en las tres rutas (`reference_assoc_query`, `indexed_assoc_query`, `assoc_query` sin caída).

| Prueba | Qué demuestra |
|---|---|
| `every_cited_relation_and_revision_exists_in_the_snapshot` (I-40) | 6 semillas, 50 consultas generadas por instantánea (`old` y `new`), con cotas a menudo apretadas, todos los campos de la consulta y los tres fines. Las tres rutas dan el mismo resultado y cada una pasa `verify_trace`. Exige haber visto más de 500 resultados, respuestas vacías, más de 50 resultados cuya unidad tiene cabeza mayor que 1 y los tres fines |
| `an_old_snapshot_never_cites_what_came_after_it` (I-40) | Una instantánea anterior a 10 conjuntos de cambios más nunca cita una relación escrita después ni una revisión posterior; sobre la nueva, esas mismas relaciones sí aparecen (más de 50 pasos), así que la prueba no es vacía |
| `frozen_corpus_cites_only_what_the_snapshot_holds` (I-40) | Las 64 consultas del conjunto congelado, cada una en las tres rutas: igual al oráculo construido desde el relato (hits, caminos, `visited`) y con `verify_trace`; cada respuesta esperada del conjunto es un resultado; más de 500 pasos recorridos. Además cada consulta con tres cotas apretadas: las rutas coinciden y los rastros se verifican |
| `window_and_provenance_filters_equal_the_brute_force_oracle` | 4 semillas × 2 instantáneas. Ventana: todos los pares de una rejilla que incluye 0, 1, 2, 3, la mitad, la última secuencia menos uno, la última, una más y otra muy posterior. Clases: los 15 conjuntos no vacíos. Cada actor y cada fuente, y uno que nadie tiene. 40 combinaciones por instantánea con tipos, direcciones, varias semillas, ventana y actor siempre presentes. Las tres rutas igualan al oráculo. Exige más de 40 respuestas vacías y más de 200 no vacías, y que **quitar cada condición (ventana, clases, actor, fuente) cambie la respuesta** en más de 20 consultas: si no, la condición no decidía nada en la prueba |
| `frozen_corpus_filters_equal_the_oracle_one_condition_at_a_time` | Las 64 consultas del conjunto con ventana, clases, actor y fuente propios, tal cual y quitando una condición cada vez, contra el oráculo del relato. Cada condición cambia la respuesta en más de 3 consultas del conjunto |
| `inferred_is_never_declared_in_a_trace` (I-39) | Grafo a mano con cuatro relaciones paralelas `a→b` del mismo tipo (declarada, importada, algorítmica, inferida) y dos saltos más (uno inferido, otro declarado). Vectores exactos de unidad, relación y clase mostrada para sin filtro, `{Inferred}`, `{Declared}`, `{Imported}`, `{Algorithmic}`, `{Imported, Algorithmic}` y `{Imported, Inferred}`, en las tres rutas. El registro guardado conserva origen y confianza, y el rastro dice `Declared` solo para un registro declarado. Sobre mundos generados, ningún paso muestra otra clase que la del modelo, y las cuatro clases aparecen |
| `the_clock_orders_nothing` (I-26) | La misma historia (mismos identificadores y secuencias) bajo tres relojes: constante, **hacia atrás** con saltos grandes y creciente. Se comprueba que las marcas de tiempo de las revisiones y de los conjuntos de cambios difieren y que la del reloj hacia atrás decrece con la secuencia. 4 semillas × 40 consultas (la mitad con ventana): el texto `Debug` del resultado es idéntico en las tres rutas y los tres relojes, y las de cotas amplias igualan al oráculo de cada historia |

Ya cubierto antes y no repetido: `assoc_indexed::differential_seeded` pone todos los campos de `AssocQuery` y exige que las tres rutas coincidan (incluidos los errores y los fallos de índice), y `frozen_corpus_indexed_equals_reference` lo hace sobre el conjunto. **No se amplía**: ya cubre ventana, clases, actor y fuente. Lo que no cubría (cabezas mayores que 1) lo cubre `every_cited_*` con sus unidades actualizadas.

## 5. Decisiones y límites

- **La ventana se aplica a las relaciones**, a su `created_seq` (E7-01, §2). El acta (§2.1, punto 2) habla de «relaciones y revisiones creadas dentro de una ventana»; el rastro cita la revisión cabeza de cada unidad **en la instantánea**, con su `commit_seq`, pero no filtra las unidades por el `commit_seq` de su revisión. No se cambió: sería otra semántica (y otro campo de la consulta) que ni la fila de E7-03 ni el conjunto de evaluación piden, y E7-04 ya usa la recencia por `commit_seq` como señal de ranking, no como filtro. Queda registrado por si el propietario lo quiere distinto.
- **El rastro cita la revisión del resultado, no una por paso.** Las unidades intermedias son semillas o resultados de la misma respuesta (§3), cada una con su revisión; `verify_trace` lo exige.
- **I-40 se prueba, no se comprueba en consulta.** No hay un verificador público: quien llama puede comprobar un resultado con `get_relation`, `get_revision` y `change_set` sobre la misma instantánea, como hace `verify_trace`. La comprobación de cada respuesta del índice (E7-02, §4) sigue siendo la de siempre y no ve la completitud (H-1).
- **Relaciones inferidas.** Esta etapa no crea relaciones inferidas (Etapa 8); I-39 se prueba con relaciones escritas por la prueba con esa clase, que es lo que la capa de relaciones admite hoy. «Pendiente» en la fila de I-39 de `invariants.md` (relaciones inferidas por modelo) sigue siéndolo.
- **El reloj.** Ninguna línea de `assoc.rs` ni del camino del planificador lee un reloj. La prueba no demuestra su ausencia: demuestra que, con tres relojes distintos y la misma historia, nada cambia (§7, M15 y M16).
- **Copia.** `assoc_context.rs` no duplica el cargador del corpus: usa `load_corpus_with_relations`, que `load_corpus` ahora llama. `tests/assoc_reference.rs` conserva su propia copia (ver el límite «Copia» de [E7-02](etapa7-ejecucion-indexada.md#10-límites-y-riesgos)); no se tocó.

## 6. Dependencias

Ninguna nueva; `Cargo.toml` no cambia.

## 7. Mutaciones

Cada mutación se aplicó con `sed` a `crates/kerno-units/src/assoc.rs` sin mutar, se ejecutó `cargo test -p kerno-units --release --test assoc_context`, se anotaron las pruebas rojas y se restauró el fichero desde una copia (comprobado byte a byte con `cmp`; `git status` no muestra cambios en `src/`). Las 16 compilaron y las 16 fueron detectadas. «Todas» son las siete funciones (14 pruebas con los dos motores); cada nombre abreviado falla en sus dos motores.

| # | Mutación | Pruebas que fallaron |
|---|---|---|
| M01 | La ventana excluye su cota superior (`> w.hi` → `>= w.hi`) | `window_and_provenance_…`, `the_clock_orders_nothing` |
| M02 | La ventana excluye su cota inferior (`< w.lo` → `<= w.lo`) | `window_and_provenance_…`, `the_clock_orders_nothing`, `frozen_corpus_filters_…`, `frozen_corpus_cites_…` |
| M03 | El filtro de clases invertido | todas salvo `an_old_snapshot_…` |
| M04 | El filtro de actor siempre cierto | `every_cited_…`, `the_clock_…`, `window_and_provenance_…`, `frozen_corpus_filters_…`, `frozen_corpus_cites_…` |
| M05 | El filtro de fuente siempre cierto | las mismas cinco que M04 |
| M06 | El filtro de actor compara con la fuente | las mismas cinco que M04 |
| M07 | La revisión del resultado es la siguiente a la cabeza | todas salvo `inferred_is_never_declared_…` |
| M08 | El paso cita un identificador de relación que no existe | todas |
| M09 | Un paso de una relación inferida se muestra como `Declared` | `inferred_is_never_declared_…`, `every_cited_…`, `window_and_provenance_…`, `the_clock_…`, `frozen_corpus_cites_…`, `frozen_corpus_filters_…` |
| M10 | El `created_seq` del paso es uno más | `every_cited_…`, `the_clock_…`, `window_and_provenance_…`, `frozen_corpus_cites_…`, `frozen_corpus_filters_…` |
| M11 | La ruta por índice ignora la ventana | las mismas cinco que M04 |
| M12 | La ruta por índice ignora las clases | `inferred_is_never_declared_…`, y las mismas cinco que M04 |
| M13 | La ruta por índice ignora el actor | las mismas cinco que M04 |
| M14 | La ruta por índice ignora la fuente | las mismas cinco que M04 |
| M15 | La frontera de cada nivel se ordena por la marca de tiempo del conjunto de cambios | **solo** `the_clock_orders_nothing` |
| M16 | La ventana descarta además relaciones según la marca de tiempo del conjunto de cambios | **solo** `the_clock_orders_nothing` |

Lo que la tabla dice y lo que no:

- `the_clock_orders_nothing` es la única que mata M15 y M16: con un reloj constante la frontera ordenada por tiempo conserva el orden por identificador, y solo el reloj que retrocede lo cambia. Las demás pruebas no las ven.
- `inferred_is_never_declared_in_a_trace` mata M09 (la mutación propia de I-39), M03, M08 y M12.
- M01 (cota superior de la ventana) **no** la detectan las pruebas del corpus congelado: sus ventanas no tienen una relación justo en la cota superior. La detectan `window_and_provenance_…` (rejilla exhaustiva) y `the_clock_…`.
- `an_old_snapshot_never_cites_what_came_after_it` solo falla con M07 y M08, y no hay mutación del código de producción que la mate sola: el aislamiento de instantánea lo da el motor, y la prueba fija que el rastro no se aparta de él. Se deja como prueba de regresión de I-40, no como guardia de una línea concreta.
- Las pruebas de E7-01 y E7-02 siguen verdes bajo el código sin mutar; sus propias mutaciones están en [E7-01 §9.1](etapa7-consulta-asociativa.md#91-mutaciones) y [E7-02 §8](etapa7-ejecucion-indexada.md#8-mutaciones).

## 8. Verificación

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_context
```

Conteo de pruebas de `kerno-testkit` + `kerno-units` con `--release --all-features`: **625 aprobadas, 0 fallidas, 8 ignoradas** con este cambio; antes eran 611 (E7-02). Las 14 nuevas son las 7 funciones de `assoc_context` × 2 motores; el corredor las cuenta (`assoc_context`: `14 passed`, unos 11 s).

## 9. Límites y riesgos

- **Cobertura del oráculo.** El oráculo solo cubre consultas cuyo límite y presupuesto no se alcanzan. Con cotas apretadas lo que se comprueba es `verify_trace` y la igualdad entre rutas, no la respuesta exacta; el presupuesto y los fines de E7-01 siguen cubiertos por sus pruebas a mano.
- **Mundos pequeños.** Los mundos generados tienen de 9 a 13 unidades y unas 20 a 40 relaciones: densos para que haya relaciones paralelas, ciclos y varios caminos al mismo resultado. El corpus congelado aporta la escala de 2 000 unidades.
- **El modelo es de la prueba.** Si la prueba y la biblioteca coincidieran en un malentendido del orden documentado, el oráculo lo compartiría. Mitigación: el orden se escribió desde el texto de `assoc.rs` y los vectores a mano de E7-01 siguen siendo independientes de ambos.
- **Tiempo de las pruebas.** Las del corpus recorren la ruta de referencia, que es lineal en las relaciones por expansión; `assoc_context` tarda unos 11 s en `--release`. No se afirma nada de rendimiento (E7-05).

## 10. Documentos relacionados

- [Acta de la Etapa 7](etapa7-acta.md) §3 (E7-03), §4 (I-39, I-40)
- [Consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [ejecución indexada (E7-02)](etapa7-ejecucion-indexada.md)
- [Conjunto de evaluación](etapa7-conjunto-evaluacion.md) (corpus y consultas congelados, sin cambios)
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): relaciones y procedencia
- [Invariantes](invariants.md): I-26 (el reloj no ordena), I-39 (inferida no es declarada)
