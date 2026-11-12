# Conjunto de evaluación de los derivados y de la búsqueda vectorial (E8-01)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-12 |
| Estado | **Congelado** en E8-01, desde el commit que añade este documento. Los ficheros de `data/derived-eval/` no cambian después de esa fecha. Un juicio nuevo exige volver a congelar el `MANIFEST` antes de medir (acta §6.1). El hash del commit está en `git log -- crates/kerno-testkit/data/derived-eval` |
| Acta | [Acta de la Etapa 8](etapa8-acta.md) §6.1 (conjunto), §6.2 (umbrales), §4 (criterios comunes) y decisión D5 |
| Código | `crates/kerno-testkit/src/derived_eval.rs` (generador y formatos), `examples/freeze_derived_eval.rs` (escribe los ficheros), `tests/derived_eval.rs` (pruebas). Usa `sha256.rs` y `assoc_eval::manifest_text`, que son de `kerno-testkit` |
| Datos | `crates/kerno-testkit/data/derived-eval/`: `corpus.txt`, `vectors.txt`, `queries.txt`, `MANIFEST` |
| Medición | Ninguna en este documento. E8-07 medirá los umbrales de §6.2 sobre este conjunto |

## 0. Qué mide y qué no

Este conjunto mide **la mecánica** de la búsqueda sobre vectores y del registro de derivados. No mide la calidad semántica de ningún modelo.

Los vectores son **deterministas y salen de una semilla** (acta, decisión D5). No vienen de ningún modelo ni de ningún proveedor, y ningún paso de la generación usa un modelo externo. Un tema tiene un centro, y sus decisiones avanzan desde ese centro a lo largo de su cadena (§2.6). Por construcción, la búsqueda vectorial encuentra la vecindad de la cadena: el conjunto no dice nada sobre cuánto se parece a la memoria real esa búsqueda. Eso es lo que la acta llama R-E8-5, y se declara aquí en §8.

Los grados de relevancia vienen del relato con el que se genera el corpus (§3.2), no del código de `kerno-units`. El generador no depende de `kerno-units`, y una prueba lo comprueba.

Este conjunto **no es** el de la Etapa 7 y no se usa para medir nada de la Etapa 8 que lo repita. Tiene otra semilla, otra estructura de relato y su propia permutación de identificadores (acta §6.1).

## 1. Qué se congela

| Elemento | Recuento |
|---|---|
| Unidades | 2 000: 800 `fact`, 500 `source`, 300 `task`, 300 `decision`, 100 `session` |
| Relaciones | 5 062: 1 900 `in_session`, 800 `cites`, 600 `same_source`, 495 `mentions`, 577 `related`, 400 `supports`, 290 `revises` |
| Relaciones por procedencia (ADR-0014) | 3 785 `declared`, 600 `algorithmic`, 577 `inferred`, 100 `imported` |
| Conjuntos de cambios | 300: 100 de un agente (`transcript/NNNN`), 100 del importador (`wiki-export/NNNN`), 100 del consolidador (`job/consolidate/NNNN`) |
| Vectores | 2 000, de dimensión 16, uno por unidad y en orden de identificador |
| Consultas | 100: 25 `vector_neighbourhood`, 25 `temporal`, 25 `provenance` (8 `PV-RE`, 8 `PV-SS`, 9 `PV-DE`) y 25 `combined` |
| Respuestas esperadas | 1 193: 265 de grado 3, 241 de grado 2, 87 de grado 1 y 600 de grado 0 |
| Relevantes por consulta | entre 1 y 13. Hay 16 consultas con más de 10 |
| Respuestas por consulta | entre 3 y 23 |
| Tamaño de los ficheros | `corpus.txt`: 7 365 líneas, 248 950 bytes. `vectors.txt`: 2 002 líneas, 153 200 bytes. `queries.txt`: 1 296 líneas, 18 406 bytes |

Los recuentos de la tabla se han contado sobre los ficheros congelados con `awk` (no se han tomado de los comentarios de cabecera). La prueba `quality_set_has_the_agreed_shape` comprueba una parte de ellos.

La escala de rendimiento (20 000 unidades, 3 000 conjuntos de cambios, 20 000 vectores, las mismas consultas en familias) se genera desde la misma semilla. **No se congela**: la prueba `performance_scale_is_deterministic` comprueba su tamaño y su determinismo.

## 2. Cómo se construye el relato

### 2.1 Semilla y escalas

- Semilla: `SEED = 20_261_112` (la fecha de esta acta). Es distinta de la de la Etapa 7 (`20_261_111`).
- Generador: SplitMix64 (`crates/kerno-testkit/src/rng.rs`). Cada corriente tiene su propia constante, XOR con la semilla:
  - relato: la semilla;
  - permutación de identificadores: `0xD8E1_0000_0000_0004`;
  - vectores: `0xD8E1_0000_0000_0003`;
  - consultas: `0xD8E1_0000_0000_0002`.
- Escala de calidad: 100 sesiones, 2 000 unidades, 300 conjuntos de cambios. Es la que se congela.
- Escala de rendimiento: 1 000 sesiones. Se genera, no se congela.

### 2.2 Unidades de una sesión

Cada sesión `s` crea 20 unidades, en este orden de creación:

| Índice local | Tipo | Etiqueta | Papel |
|---|---|---|---|
| 0 | `session` | `s0000` | La sesión |
| 1–3 | `task` | `s0000.t0`–`t2` | Tres tareas (no tienen relaciones de grado) |
| 4–6 | `decision` | `s0000.d0`–`d2` | Tres decisiones de un mismo tema |
| 7–14 | `fact` | `s0000.f0`–`f7` | Ocho hechos |
| 15–19 | `source` | `s0000.src0`–`src4` | Cinco fuentes |

El tema de la sesión se elige al azar entre 10.

### 2.3 Cadenas, evidencia y distractores

- **Cadena de un tema.** Cada decisión `revises` la decisión anterior de su mismo tema, aunque esté en otra sesión. Un tema es una cadena lineal.
- **Evidencia.** Cada decisión `d_k` tiene un hecho `f_k` que la sustenta (`supports`). Cada hecho cita una fuente (`cites`). Las ocho citas se barajan en cada sesión, así que tres fuentes son citadas por dos hechos cada una.
- **Distractor de mención.** Los hechos `f0`..`f4` mencionan (`mentions`) una decisión de otro tema de una sesión anterior. Es un enlace falso.
- **Importación.** El importador añade un `supports` verdadero de `f7` a `d1`, con procedencia `imported wiki-export`.
- **Vínculos inferidos.** El consolidador añade, por cada decisión, hasta dos `related` con procedencia `inferred consolidator-v1`:
  - uno verdadero, a la decisión dos posiciones atrás en su cadena, con confianza uniforme entre 450 y 950 (solo si la decisión tiene al menos dos anteriores en su tema);
  - uno falso, a una decisión de otro tema de una sesión anterior, con confianza uniforme entre 300 y 800.
  Los rangos se solapan a propósito: la confianza es informativa, pero no basta.
- **Mismo origen.** El consolidador añade seis `same_source` algorítmicos (`dedup-v2`): tres verdaderos (los pares que citan la misma fuente) y tres falsos (pares elegidos al azar entre hechos con fuentes distintas). Los seis salen en orden y orientación aleatorios.

### 2.4 Pasos

La historia tiene pasos enteros, no un reloj. Para la sesión `s`:

- `3s+1`: el agente. Crea las unidades y las relaciones `in_session`, `revises`, `supports`, `cites` y `mentions`.
- `3s+2`: el importador. Crea el `supports` importado.
- `3s+3`: el consolidador. Crea los `related` inferidos y los `same_source` algorítmicos.

Cada enlace se crea en un paso no anterior al de sus extremos. El lector lo comprueba (§4). Las ventanas temporales de las consultas son intervalos de estos pasos.

### 2.5 Identificadores

Los identificadores son **una permutación aleatoria con semilla** de la fase de creación. El fichero lista las unidades en orden de creación, pero con su identificador permutado. Así el desempate por identificador no favorece a ninguna estructura, como pide §6.1. La prueba `identifiers_are_a_permutation_of_creation_order` comprueba que son una permutación de `0..2000` y que muy pocas caen en su posición de creación.

### 2.6 Vectores

Cada tema tiene un **centro**: 16 enteros uniformes entre −1 000 y 1 000.

- **Decisiones.** La posición de una decisión es el centro del tema más un **paseo**: cada decisión de la cadena suma un paso aleatorio, entre −40 y 40 por componente. Así las decisiones cercanas en la cadena quedan cerca en el espacio. El vector de la decisión es esa posición más un ruido de ±15 por componente.
- **Demás unidades.** Centro más un ruido de ±400 por componente.
- **Consultas vectoriales.** El vector de la consulta es la posición de la semilla más un ruido de ±15.

Todo el vector se calcula con enteros. La regla de juicio de §3.2 compara por coseno en coma flotante, pero solo para ordenar: la raíz cuadrada y las cuatro operaciones son IEEE 754 y dan el mismo resultado en cualquier plataforma, y los empates se resuelven por identificador.

## 3. Consultas y juicios

### 3.1 Las cuatro clases

| Clase | Identificador | Semilla | Recorrido | Ventana | Procedencia | Qué pone a prueba |
|---|---|---|---|---|---|---|
| `vector_neighbourhood` | `VN-01`..`25` | Decisión | Ambos sentidos, `revises`, profundidad 2 | Ninguna | Cualquiera | Vecindad de la cadena, con el vector de la consulta |
| `temporal` | `TE-01`..`25` | Decisión de sesión 2 o posterior | Ambos sentidos, `revises`, `supports`, `mentions`, profundidad 3 | Sesiones ±2 | Cualquiera | Contexto temporal |
| `provenance` | `PV-RE-01`..`08` | Decisión | Saliente, `related`, profundidad 1 | Ninguna | `inferred` | Vínculo inferido verdadero frente a falso |
| `provenance` | `PV-SS-01`..`08` | Hecho | Ambos sentidos, `same_source`, profundidad 1 | Ninguna | `algorithmic` | Igual, con `same_source` |
| `provenance` | `PV-DE-01`..`09` | Decisión de sesión 3 o posterior | Ambos sentidos, `revises`, `supports`, `mentions`, profundidad 3 | Ninguna | `declared` | Cadena declarada, sin importación ni inferencias |
| `combined` | `CB-01`..`25` | Decisión | Ambos sentidos, `revises`, `supports`, `related`, profundidad 3 | Sesiones ±1 | `declared`, `inferred` | Procedencia y tiempo a la vez |

Una ventana `lo-hi` es un intervalo cerrado de **pasos de creación de los enlaces**, no de unidades.

Un generador de consultas elige semillas al azar. Una semilla se acepta solo si la consulta tiene al menos una respuesta relevante y al menos un distractor distinto de la semilla. Si no, se vuelve a elegir.

### 3.2 Grados

El recorrido sigue **solo los vínculos verdaderos**. Un vínculo falso termina el recorrido: su extremo es un distractor hoja, nunca el camino hacia una unidad relevante.

- **Grado 3** a distancia 1 por vínculos verdaderos, **2** a distancia 2 y **1** a distancia 3. La profundidad máxima es 3.
- **Grado 0 nombrado**, en este orden de prioridad:
  1. la semilla, siempre, para que ninguna consulta la cuente como respuesta;
  2. el extremo de un vínculo falso que el recorrido encuentra;
  3. los diez vecinos más cercanos por coseno al vector de la consulta que no estén juzgados (solo `VN`);
  4. las diez primeras unidades del orden simple («saltos, luego identificador») que no estén juzgadas.

Las consultas `VN` tienen 10 vecinos por coseno, y el corte de §6.2 es 10. Por eso el top 10 exacto del vector queda juzgado por completo (§3.3).

Los grados 3, 2 y 1 son una convención de este documento, no juicios de personas. Ver §8.

### 3.3 Juicios sin juzgar

Lo que no aparece en `queries.txt` está **sin juzgar** (acta §6.1). Un brazo que devuelva una unidad sin juzgar en su top 10 no la cuenta como grado 0: la excluye del cálculo y su número se informa.

Por construcción, en este conjunto:

- el top 10 **exacto** del vector de cada consulta `VN` está juzgado. La razón: los diez vecinos del punto 3 de §3.2 son las diez unidades sin juzgar más cercanas al vector. Si una unidad del top 10 exacto no estuviera juzgada, sería una de las diez sin juzgar más cercanas, porque como mucho nueve unidades están más cerca que ella. Entonces habría recibido juicio, lo que contradice la hipótesis;
- las diez primeras del **orden simple** de cada consulta están juzgadas, porque el punto 4 de §3.2 las juzga todas.

Por eso el número de posiciones sin juzgar en el top 10 de esos dos brazos de referencia es **0**. Un brazo híbrido no tiene esa garantía: E8-07 contará sus posiciones sin juzgar y, si pasan del 10 %, el conjunto se juzgará más antes de medir (acta §6.1, Q5).

La garantía depende del **coseno**. Otra métrica, como la distancia euclídea, daría un top 10 distinto y no quedaría cubierto. El brazo vectorial de E8-07 debe usar coseno para que Q5 se cumpla.

## 4. Formatos de los ficheros

Los ficheros son texto con una línea por registro. Se ignoran las líneas vacías y las que empiezan por `#`. Los campos van separados por espacios y ninguno contiene espacios.

### 4.1 `corpus.txt`

```text
# kerno-testkit derived-eval corpus v1
seed 20261112 scale quality
counts units 2000 relations 5062 changes 300
C 1 agent-gamma transcript/0000
U 298 1 session s0000
U 547 1 task s0000.t0
U 1729 1 decision s0000.d0
```

| Línea | Forma | Significado |
|---|---|---|
| cabecera | `seed <u64> scale <quality\|performance>` | Semilla y escala |
| recuentos | `counts units <n> relations <n> changes <n>` | Totales. El lector los compara con los registros |
| conjunto de cambios | `C <paso> <actor> <fuente>` | Abre un conjunto de cambios. El paso es el siguiente al anterior |
| unidad | `U <identificador> <paso> <tipo> <etiqueta>` | Una unidad. El identificador está permutado |
| enlace | `R <paso> <desde> <hasta> <tipo> <procedencia>` | Un enlace. Los extremos deben existir y haber sido creados en un paso no posterior |
| procedencia | `declared` \| `imported <fuente>` \| `algorithmic <origen>` \| `inferred <origen> <confianza>` | La clase de ADR-0014 con sus datos. La confianza va de 0 a 1 000 |

### 4.2 `vectors.txt`

```text
# kerno-testkit derived-eval vectors v1
dim 16 counts vectors 2000
V 0 1262 363 -224 458 1029 -107 -871 -468 -285 -426 75 -522 -600 -353 -465 913
```

Una línea por unidad, en orden de identificador. El lector comprueba la dimensión de cada línea, el orden y el número total.

### 4.3 `queries.txt`

```text
# kerno-testkit derived-eval queries v1
seed 20261112 scale quality cut 10 dim 16
counts queries 100
Q VN-01 vector_neighbourhood 508 both revises 2 - - -796,-1008,566,103,-112,381,-198,57,-601,421,-133,-43,-88,437,716,884
E 157 0
E 230 0
E 232 0
E 508 0
```

`Q` lleva: identificador, clase, semilla, sentido, tipos separados por comas, profundidad, ventana (`-` o `lo-hi`), procedencias (`-` o lista) y vector (`-` o 16 enteros separados por comas). Cada `E` es una respuesta: unidad y grado de 0 a 3. El lector exige que las respuestas estén ordenadas por unidad, sin repetidos.

### 4.4 `MANIFEST`

Formato de `sha256sum`: un registro por fichero, en este orden: `corpus.txt`, `vectors.txt`, `queries.txt`.

```text
24cdbaa5d80d6e7a8377010769b70251b7ce6a9942cf129b6595585248d6dfe5  corpus.txt
7645119a705b3c154e44bcc6e914efe2b9a3712260645560d7a3ebbf1f55d538  vectors.txt
ef59c9019ef275c86ef4199757a7be327a9b075c91a21cacae5b8d864f8a24bd  queries.txt
```

## 5. Regeneración y verificación

Regenerar desde la semilla (escribe los tres ficheros y el `MANIFEST`; sobre el árbol congelado no cambia nada):

```text
cargo run --release -p kerno-testkit --example freeze_derived_eval
```

Verificar las pruebas, que comparan byte a byte con los ficheros congelados y comprueban el manifiesto:

```text
cargo test -p kerno-testkit --release --test derived_eval
```

Verificar el manifiesto con la herramienta estándar:

```text
$ cd crates/kerno-testkit/data/derived-eval && sha256sum -c MANIFEST
corpus.txt: OK
vectors.txt: OK
queries.txt: OK
```

Los tres `OK` salieron en la ejecución de este documento.

## 6. Comprobación de los umbrales de §6.2

Se ha comprobado que **cada umbral cae dentro de la cota de su métrica** sobre este conjunto, antes de congelarlo. Ningún umbral ha cambiado. La tabla usa valores de referencia medidos sobre los ficheros congelados con un programa temporal que no se conserva, como en el documento de la Etapa 7 (§9.2). Los que son cotas teóricas o pruebas están marcados como tales.

| Código | Umbral | Cota de la métrica en este conjunto | Referencia medida | Resultado |
|---|---|---|---|---|
| **Q1** | recall@10 = 1,000 frente a la referencia | El recall está en [0, 1] | Es la misma búsqueda; no depende del conjunto | **Dentro de la cota** |
| **Q2** | nDCG@10 con señal vectorial ≥ 0,80, en conjunto | El nDCG está en [0, 1] y un ranking perfecto da 1 | Brazo solo vectorial (coseno) sobre `VN`: media 0,708, mínimo 0,662 | **Dentro de la cota.** Un brazo solo vectorial no llega a 0,80 en esa clase; el umbral se medirá con un brazo híbrido en E8-07 |
| **Q3** | recall@10 con señal vectorial ≥ 0,90, en conjunto | Techo medio de un ranking perfecto: **0,977** (`thresholds_of_the_frozen_set_are_within_reach`). Hay 16 consultas con más de 10 relevantes, y ahí el techo es 10/R | Brazo vectorial sobre `VN`: recall@10 medio 1,000 | **Dentro de la cota**, con margen de 0,077 |
| **Q4** | Con señal vectorial, nDCG@10 ≥ sin ella − 0,01, en conjunto y en cada clase | Es no inferioridad: la cota es nDCG sin señal − 0,01 ≤ 1 | En `VN`: vector 0,708, sin señal (orden simple) 0,725; cota 0,715 | **Dentro de la cota.** Un brazo solo vectorial queda 0,007 por debajo de la cota en `VN`: es un resultado para E8-07, no una cota rota |
| **Q5** | Unidades sin juzgar en el top 10 ≤ 10 % de las posiciones, en cada brazo | Es una condición de validez: como mucho una posición de diez | 0 posiciones sin juzgar en el brazo vectorial y en el orden simple, por construcción (§3.3) | **Cumplida por construcción** en los dos brazos de referencia. Un brazo híbrido se cuenta en E8-07 |
| **Q6** | Solo si el ranking se activa por defecto en una etapa posterior: nDCG@10 ≥ orden simple + 0,02 | El orden simple debe quedar por debajo de 0,98 para que el umbral sea alcanzable | Orden simple, media por clase: `combined` 0,702; `provenance` 0,615; `temporal` 0,739; `vector_neighbourhood` 0,725 | **Dentro de la cota.** No es criterio de cierre de la Etapa 8 |

Nota sobre el orden simple. Es «saltos, luego identificador» sobre todos los enlaces admitidos, falsos incluidos, con el seed excluido. Es una referencia para el conjunto. El orden simple real del motor se medirá en E8-07, y si difiere de esta definición, su top 10 puede tener posiciones sin juzgar.

Conclusión: **ningún umbral queda fuera de la cota de su métrica.** Según la regla de la acta, no hay que parar ni cambiar umbrales. Hay dos observaciones para el propietario, que no son cotas:

1. Un brazo **solo vectorial** no cumple Q2 ni Q4 en la clase `vector_neighbourhood` (0,708 frente a 0,725 del orden simple, con 0,715 de cota). Eso es el resultado de la medición de E8-07, no del conjunto. Los umbrales siguen como están (decisión D4).
2. Q2 y Q3 «en conjunto» se refieren a consultas con señal vectorial, que en este conjunto son las 25 de `VN`. El conjunto no fija qué brazo híbrido se mide: lo fijará E8-07.

## 7. Pruebas y mutaciones

`tests/derived_eval.rs` tiene quince pruebas:

| Prueba | Qué comprueba |
|---|---|
| `regenerated_set_is_byte_identical` | Los tres ficheros congelados son exactamente lo que produce la semilla |
| `manifest_matches_committed_files` | El `MANIFEST` lista esos tres ficheros, en ese orden, con su SHA-256 |
| `committed_files_parse_back` | El texto congelado lee de vuelta el mismo relato y los mismos vectores y consultas |
| `parser_refuses_a_repeated_unit` | El lector rechaza un identificador repetido, y lo rechaza **por esa razón** |
| `parser_refuses_a_vector_of_the_wrong_size` | El lector rechaza un vector con una componente de más |
| `quality_set_has_the_agreed_shape` | Tamaños, clases de procedencia, cuatro clases de consulta, grados 0 a 3, y que cada consulta tiene relevantes y distractores |
| `vector_file_has_the_declared_dimension` | Cada línea de `vectors.txt` tiene la dimensión declarada |
| `expected_answers_are_reachable_within_the_bounds` | Cada respuesta relevante es alcanzable con las cotas de su consulta, y cada distractor es alcanzable o es un vecino del vector |
| `vector_top_ten_is_fully_judged` | El top 10 exacto del vector de cada consulta vectorial está juzgado |
| `identifiers_are_a_permutation_of_creation_order` | Los identificadores son una permutación y pocos caen en su posición de creación |
| `generation_is_deterministic` | La misma semilla da la misma historia |
| `performance_scale_is_deterministic` | La escala de 20 000 unidades tiene su tamaño y es determinista |
| `thresholds_of_the_frozen_set_are_within_reach` | El techo de recall@10 de un ranking perfecto (Q3) llega a 0,90 |
| `seed_is_not_the_stage_seven_seed` | La semilla es distinta de la de la Etapa 7 |
| `testkit_does_not_depend_on_kerno_units` | `Cargo.toml` de `kerno-testkit` no menciona `kerno-units` |

Las pruebas que solo leen los ficheros congelados no ven cambios del generador: por eso cada una se ha mutado en su propio código o en el del generador (criterio 4 de §4 de la acta). Cada mutación se revirtió con `sed` inverso y se comprobó la restauración.

| Mutación | Dónde | Pruebas que fallaron |
|---|---|---|
| `DECISION_NOISE` 15 → 16 | `src/derived_eval.rs:47` | `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Semilla 20 261 111 | `src/derived_eval.rs:27` | `seed_is_not_the_stage_seven_seed`, `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Identificadores sin permutar | `src/derived_eval.rs:767` | `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Vector sin una componente | `src/derived_eval.rs:860` | `regenerated_set_is_byte_identical` |
| El lector acepta identificadores repetidos | `src/derived_eval.rs:1019` | `parser_refuses_a_repeated_unit` (ver abajo) |
| El lector de vectores acepta una componente de más | `src/derived_eval.rs:1075` | `parser_refuses_a_vector_of_the_wrong_size` |
| Recorrido de profundidad 0 | `tests/derived_eval.rs`, `reach` | `expected_answers_are_reachable_within_the_bounds` |
| Top 15 en lugar de top 10 | `tests/derived_eval.rs` | `vector_top_ten_is_fully_judged` |
| Grado exigido 4 | `tests/derived_eval.rs` | `quality_set_has_the_agreed_shape` |
| `vectors.txt` con los bytes de `queries.txt` | `tests/derived_eval.rs` | `manifest_matches_committed_files` |
| 20 001 unidades | `tests/derived_eval.rs` | `performance_scale_is_deterministic` |
| Dependencia de `kerno-units` invertida | `tests/derived_eval.rs` | `testkit_does_not_depend_on_kerno_units` |
| Umbral Q3 de 0,99 | `tests/derived_eval.rs` | `thresholds_of_the_frozen_set_are_within_reach` |
| Comparación con otra semilla | `tests/derived_eval.rs` | `generation_is_deterministic` |
| Identificador fijo en 0 (`fixed < 0`) | `tests/derived_eval.rs` | `identifiers_are_a_permutation_of_creation_order` |
| Dimensión de línea `DIM + 1` | `tests/derived_eval.rs` | `vector_file_has_the_declared_dimension` |

**Una mutación que quedó verde.** La primera versión de `parser_refuses_a_repeated_unit` pasaba con el lector mutado: el lector sin comprobación de repetidos rechazaba el mismo fichero más adelante, porque un enlace apuntaba a un identificador no creado. La prueba acertaba por la razón equivocada. Se corrigió para exigir el mensaje de identificador repetido, y la mutación pasa a fallar.

Cuenta de pruebas: el módulo `derived_eval.rs` no tiene pruebas propias. Las quince de `tests/derived_eval.rs` son nuevas y aprobadas en la última ejecución de ese binario. El conteo total de `kerno-testkit` y `kerno-units` no está en este documento.

## 8. Límites

1. **Vectores de función, no de modelo.** La búsqueda vectorial encuentra la vecindad porque el generador la pone ahí (§2.6). Un resultado alto en `VN` mide que la búsqueda y el registro funcionan, no que un modelo real separe los temas o las decisiones. Es el límite R-E8-5 de la acta.
2. **Mismo autor para el relato, las consultas y los grados.** El generador, las familias de consultas y las reglas de grado los escribió el mismo equipo antes de escribir el código de la Etapa 8. El congelado protege contra ajustar las respuestas al código, pero no contra un supuesto del generador que el código comparta con él (R-E8-2).
3. **Juicios sin independencia.** No hay ningún juicio externo. Los grados 3, 2 y 1 son una convención de distancia, no de relevancia humana.
4. **Estructura regular.** Cada sesión tiene exactamente tres decisiones de un tema, ocho hechos y cinco fuentes. Cada tema tiene una sola cadena lineal, y hay diez temas. La memoria real tiene ramas, decisiones que se contradicen y estructuras irregulares.
5. **Vocabulario pequeño.** Diez temas, tres agentes, un importador y un consolidador. Las etiquetas son cadenas como `s0000.d0`, sin texto natural.
6. **Cien consultas.** Con 25 por clase y entre 1 y 13 relevantes, la varianza de una media es alta. Una diferencia pequeña entre dos brazos no es significativa en este conjunto.
7. **Distractores de vector elegidos por el conjunto.** Los diez vecinos por coseno se fijan al generar el conjunto, antes de que ningún brazo corra, para que ninguno elija la lista. Q5 queda cumplida para el brazo vectorial y para el orden simple, pero no para un brazo que use otra métrica o que mezcle señales.
8. **Métrica fijada.** La garantía de Q5 depende del coseno (§3.3).
9. **Rendimiento no medido.** La escala de 20 000 unidades está generada y probada en forma. Los tiempos son de E8-07.
10. **Lo que sí garantiza.** Que cualquier cambio del generador, del formato o de la lectura se detecta; que las respuestas esperadas son alcanzables con sus cotas; que cada consulta tiene relevantes y distractores; que los dos brazos de referencia no tienen posiciones sin juzgar en el top 10; y que cada umbral de §6.2 cae dentro de la cota de su métrica.

## 9. Estado de las puertas de E8-00

Este conjunto no depende del contenido de ADR-0018, pero E8-01 es un paquete posterior a E8-00 (acta §3), y en el repositorio, al escribir este documento:

- La acta de la Etapa 8 dice en su cabecera y en §10 que está «propuesta, pendiente de aprobación del propietario». La tarea de E8-01 indica que el propietario la aprobó el 2026-11-12, pero el texto de la acta no lo recoge.
- ADR-0018 figura como **Propuesto** en `docs/adr/0018-*.md`.
- ADR-0009 figura como **Aceptado** (2026-11-12, opción 3).

Este documento no cierra ninguna de esas puertas.

## 10. Documentos relacionados

- [Acta de la Etapa 8](etapa8-acta.md) §3 (E8-01), §4 (criterios comunes), §6.1 (conjunto), §6.2 (umbrales), §8 (D4, D5) y §9 (R-E8-1 a R-E8-6)
- [Conjunto de evaluación de la Etapa 7](etapa7-conjunto-evaluacion.md): el conjunto de la memoria asociativa, con otra semilla y otra estructura
- [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md): vectores exactos en memoria
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): las cuatro clases de procedencia
- [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md): derivados y su registro
