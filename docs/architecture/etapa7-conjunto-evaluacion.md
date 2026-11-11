# Conjunto de evaluación de la memoria asociativa (E7-00)

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Congelado** en E7-00. Los ficheros de `data/assoc-eval/` no cambian después de esta fecha (acta §6.1, §7 punto 2) |
| Acta | [Etapa 7](etapa7-acta.md) §6.1 (conjunto), §6.2 (calidad), §7 punto 2 |
| Decisión asociada | [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) (aceptado en E7-00). Este documento es el enlace de su sección de evidencia |
| Código | `crates/kerno-testkit/src/assoc_eval.rs` (generador y formatos), `assoc_metrics.rs` (métricas), `sha256.rs` (SHA-256 del manifiesto), `examples/freeze_assoc_eval.rs` (escritura de los ficheros) |
| Datos | `crates/kerno-testkit/data/assoc-eval/`: `corpus.txt`, `queries.txt`, `MANIFEST` |
| Medición | `crates/kerno-units/tests/assoc_baseline.rs`; salida cruda en [etapa7-e700-salida.txt](etapa7-e700-salida.txt) |

Este documento describe el conjunto tal como está en el código y en los ficheros. Cada cifra se ha obtenido de esos ficheros o de la salida de los comandos indicados. Ningún número se ha tomado de la intención del diseño.

## 1. Qué se congela

El conjunto tiene dos partes que se generan juntas desde una semilla:

- **Corpus**: 2 000 unidades, 3 958 relaciones y 300 conjuntos de cambios (escala de calidad).
- **Consultas**: 64 consultas, cada una con su semilla, sus restricciones y sus respuestas esperadas con grado de relevancia de 0 a 3. Son 527 respuestas esperadas en total: 140 de grado 3, 119 de grado 2, 57 de grado 1 y 211 de grado 0 (distractores con nombre).

Los recuentos de la tabla siguiente se han contado sobre los ficheros congelados con un script de lectura (no se han tomado de los comentarios de cabecera de los ficheros).

| Elemento | Recuento |
|---|---|
| Unidades | 2 000: 800 `fact`, 500 `source`, 300 `task`, 300 `decision`, 100 `session` |
| Relaciones | 3 958: 800 `cites`, 600 `same_source`, 573 `related`, 500 `about`, 300 `in_session`, 300 `produced_by`, 300 `supports`, 297 `mentions`, 288 `revises` |
| Relaciones por procedencia | 2 585 `declared`, 600 `algorithmic`, 573 `inferred`, 200 `imported` |
| Conjuntos de cambios | 300: 100 de un agente (fuente `transcript/NNNN`), 100 del importador (`wiki-export/NNNN`), 100 del consolidador (`job/consolidate/NNNN`) |
| Consultas por clase | 16 `multi_hop`, 16 `temporal`, 16 `provenance`, 16 `combined` |
| Semillas | 60 decisiones y 4 hechos (las cuatro consultas `PV-SS`) |
| Respuestas por consulta | entre 2 y 20; entre 2 y 9 relevantes (grado ≥ 1) en `multi_hop`, 5 a 9 en `temporal`, 4 a 7 en `combined` y 1 a 5 en `provenance` |

## 2. Cómo se construye el relato

El generador escribe, para cada sesión `s`, una memoria de agente como una historia. La historia tiene tres conjuntos de cambios por sesión, en este orden lógico.

### 2.1 Semilla y escalas

- Semilla: `SEED = 20_261_111` (la fecha del registro de entrada, 2026-11-11). Es un `u64`.
- Generador: SplitMix64 (`crates/kerno-testkit/src/rng.rs`), con la semilla mezclada con una constante fija. Las consultas usan otra corriente: la semilla con `0x0E70_0000_0000_0001` aplicado con XOR, para no compartir números con el corpus.
- Escala de calidad: 100 sesiones, 2 000 unidades, 300 conjuntos de cambios. Es el conjunto congelado.
- Escala de rendimiento: 1 000 sesiones, 20 000 unidades, 3 000 conjuntos de cambios y las mismas 64 consultas (la prueba `performance_scale_is_deterministic` comprueba la generación, el número de unidades y de conjuntos de cambios). Sus relaciones no se congelan.

### 2.2 Unidades de una sesión

Cada sesión crea 20 unidades en su primer conjunto de cambios, en este orden de índice local:

| Índice (sesión 0) | Tipo | Etiqueta | Papel |
|---|---|---|---|
| 0 | `session` | `s0000` | La sesión |
| 1–3 | `task` | `s0000.t0`–`t2` | Tres tareas de la sesión |
| 4–6 | `decision` | `s0000.d0`–`d2` | Tres decisiones de un mismo tema |
| 7–14 | `fact` | `s0000.f0`–`f7` | Ocho hechos (evidencias y menciones) |
| 15–19 | `source` | `s0000.src0`–`src4` | Cinco fuentes citadas por los hechos |

El tema de la sesión se elige al azar entre 12. Las decisiones de un mismo tema forman una **cadena**: cada decisión `revises` la anterior del tema. Así, una cadena cruza sesiones.

Los hechos `F0`, `F2` (y `F1`, que llega por importación) sustentan `D0`, `D1` y `D2` con una relación `supports`. Los hechos `F3`..`F7` son `about` de tareas. Cada hecho `cites` una de las cinco fuentes, según `j mod 5`.

### 2.3 Relaciones y procedencia

Las cuatro clases de procedencia de [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) aparecen así:

| Procedencia | Dónde se crea | Qué cubre |
|---|---|---|
| `declared` | Primer conjunto de cambios de la sesión | `in_session`, `produced_by`, `supports` (de `F0` y `F2`), `mentions`, `about`, `cites`, `revises` |
| `imported <fuente>` | Segundo conjunto (importador, `wiki-export`) | El `supports` de `F1` a `D1` y un `about` de `F6` a `T0` |
| `algorithmic <origen>` | Tercer conjunto (consolidador), origen `dedup-v2` | Seis `same_source` entre hechos: tres verdaderos y tres falsos |
| `inferred <origen> <confianza>` | Tercer conjunto, origen `consolidator-v1` | Dos `related` por decisión: uno verdadero y uno falso (ver 2.6) |

La relación `mentions` apunta de un hecho a una decisión de otro tema de una sesión anterior. Es el distractor de la cadena (ver 2.6).

### 2.4 Actores y fuentes

Hay dos usos distintos de la palabra «fuente» en el conjunto. No deben confundirse.

- **Fuente de un conjunto de cambios** (`C ... <fuente>`): el texto de donde viene el cambio. Para el agente, `transcript/NNNN`; para el importador, `wiki-export/NNNN`; para el consolidador, `job/consolidate/NNNN`. Las consultas `PV-SR` filtran por esta fuente.
- **Unidad de tipo `source`**: una de las cinco fuentes citadas por los hechos (`cites`). No es el origen de un cambio.

Los actores de los conjuntos de cambios son `agent-alpha`, `agent-beta` y `agent-gamma` (uno por sesión, elegido al azar: 32, 27 y 41 sesiones respectivamente), `importer` y `consolidator`. Las consultas `PV-AC` filtran por el actor de la sesión de la semilla, y las pares de la clase `combined` también.

### 2.5 Orden lógico de los cambios

La historia tiene pasos enteros, no un reloj. Para la sesión `s` los pasos son:

- `3s+1`: el agente escribe la transcripción. Crea la sesión, las tareas, las decisiones, los hechos, las fuentes y las relaciones `declared`.
- `3s+2`: el importador crea el `supports` y el `about` importados.
- `3s+3`: el consolidador añade los `related` inferidos y los `same_source` algorítmicos.

Cada relación se crea en un paso que no es anterior al de sus extremos. El analizador lo comprueba (ver 3.1). Las ventanas temporales de las consultas son intervalos de estos pasos.

### 2.6 Distractores

Un distractor es una unidad que una consulta alcanza dentro de sus cotas y que no es relevante. Se nombran en las respuestas esperadas con grado 0. Hay cuatro fuentes de distractores:

1. **Menciones entre temas**: el `mentions` de un hecho a una decisión de otro tema.
2. **Vínculos `related` falsos**: el consolidador apunta a una decisión de otro tema con confianza entre 302 y 600 por mil. Los verdaderos apuntan a la decisión dos posiciones atrás de la cadena con confianza entre 701 y 950 (recuento: 276 verdaderos y 297 falsos). Los dos rangos no se solapan: un umbral de confianza entre 600 y 701 separa los dos casos sin error.
3. **`same_source` falsos**: tres pares de hechos que no comparten fuente (`(0,3)`, `(1,4)`, `(2,3)`). Los verdaderos (`(0,5)`, `(1,6)`, `(2,7)`) sí comparten fuente, porque `j mod 5` coincide.
4. **Decisiones y evidencias a distancia 3 o más** en la cadena: aparecen con grado 0 cuando el recorrido las alcanza (ver 5.1).

## 3. Formatos de los ficheros

Los tres ficheros son texto con una línea por registro. Las líneas vacías y las que empiezan por `#` se ignoran al leer. Los separadores son espacios. Ningún campo puede contener espacios; las etiquetas, fuentes y orígenes son palabras sin espacio.

### 3.1 `corpus.txt`

```text
# kerno-testkit assoc-eval corpus v1
seed 20261111 scale quality
counts units 2000 relations 3958 changes 300
C 1 agent-alpha transcript/0000
U 0 1 session s0000
U 1 1 task s0000.t0
R 0 1 1 0 in_session declared
```

Campos, línea por línea:

| Línea | Forma | Significado |
|---|---|---|
| cabecera | `# kerno-testkit assoc-eval corpus v1` | Comentario de versión (no se lee) |
| semilla | `seed <u64> scale <quality\|performance>` | Semilla y escala con las que se generó |
| recuentos | `counts units <n> relations <n> changes <n>` | Totales. El lector los compara con los registros y rechaza el fichero si no cuadran |
| conjunto de cambios | `C <paso> <actor> <fuente>` | Abre un conjunto de cambios. El paso debe ser el siguiente al anterior (1, 2, 3, ...) |
| unidad | `U <índice> <paso> <tipo> <etiqueta>` | Crea una unidad del conjunto de cambios actual. El índice es el de su posición (0, 1, 2, ...) |
| relación | `R <índice> <paso> <desde> <hasta> <tipo> <procedencia>` | Crea una relación. Los índices de unidad son los de las líneas `U`. Ambos extremos deben existir y haber sido creados en un paso no posterior al de la relación |
| procedencia | `declared` \| `imported <fuente>` \| `algorithmic <origen>` \| `inferred <origen> <confianza>` | La clase de ADR-0014 con sus datos. La confianza va de 0 a 1000 (por mil) |

Orden de escritura: cada `C` va seguido de sus `U` y después de sus `R`. El lector rechaza una unidad o relación cuyo índice no sea el siguiente, o cuyo paso no sea el del conjunto de cambios que se está leyendo.

Ejemplos reales del fichero:

```text
C 2 importer wiki-export/0000
R 22 2 8 5 supports imported wiki-export
C 3 consolidator job/consolidate/0000
R 24 3 6 4 related inferred consolidator-v1 805
R 25 3 7 12 same_source algorithmic dedup-v2
```

### 3.2 `queries.txt`

```text
# kerno-testkit assoc-eval queries v1
seed 20261111 scale quality cut 10
counts queries 64
Q TM-01 temporal 466 decision both revises,supports,mentions 3 70-78 - - -
E 85 0
E 306 0
E 325 0
E 464 2
E 465 3
E 467 1
E 468 2
E 469 3
```

Es la consulta `TM-01` tal como está en el fichero: ocho respuestas esperadas, tres de grado 0, una de grado 1, dos de grado 2 y dos de grado 3.

| Línea | Forma | Significado |
|---|---|---|
| cabecera | `seed <u64> scale <escala> cut <n>` | El lector rechaza un corte distinto de 10 (`CUT`) |
| recuentos | `counts queries <n>` | Número de consultas. Debe coincidir con los `Q` leídos |
| consulta | `Q <id> <clase> <semilla> <tipo> <dirección> <tipos> <profundidad> <ventana> <clases> <actor> <fuente>` | Una consulta. Cada campo se describe abajo |
| respuesta | `E <índice> <grado>` | Una respuesta esperada. Va después de su `Q`. El grado debe ser de 0 a 3 |

Campos de `Q`, en orden:

| Campo | Valores | Significado |
|---|---|---|
| `id` | `MH-01`..`MH-16`, `TM-01`..`TM-16`, `CB-01`..`CB-16`, `PV-RE-1`..`PV-RE-4`, `PV-SS-1`..`PV-SS-4`, `PV-AC-1`..`PV-AC-4`, `PV-SR-1`..`PV-SR-4` | El prefijo nombra la familia |
| `clase` | `multi_hop`, `temporal`, `provenance`, `combined` | Clase de la consulta |
| `semilla` | índice de unidad | La unidad de partida |
| `tipo` | tipo de la semilla (`decision` o `fact`) | Lo usa B0 |
| `dirección` | `out`, `in`, `both` | Por dónde puede seguir el recorrido, relativo a la unidad en la que está |
| `tipos` | lista separada por comas de tipos de relación | Tipos que el recorrido puede seguir. Nunca vacía |
| `profundidad` | entero | Número máximo de saltos |
| `ventana` | `-` o `lo-hi` | Intervalo cerrado de pasos de creación de la relación |
| `clases` | `-` o lista de `declared`, `imported`, `algorithmic`, `inferred` | Clases de procedencia admitidas. `-` admite todas |
| `actor` | `-` o nombre | Actor del conjunto de cambios que creó la relación |
| `fuente` | `-` o fuente | Fuente del conjunto de cambios que creó la relación |

Los campos de `Q` se leen con este orden y sin campos opcionales: un `-` ocupa siempre su lugar. El `E` de una consulta va ordenado por índice, sin repetidos, y el lector lo comprueba (la prueba `quality_set_has_the_agreed_shape` lo hace).

Una consulta del conjunto, tal cual:

```text
Q PV-SR-1 provenance 146 decision both revises,supports,mentions 3 - - - transcript/0007
```

Es una consulta sin ventana y sin procedencia, que sigue la cadena de su tema y se limita a la fuente de transcripción de la sesión de la semilla.

### 3.3 `MANIFEST`

```text
8cd500f8ddb459fa80307bd9065335649cfe37d097c53dfbb16b10d430cc3354  corpus.txt
d141b05b4813f34ea3030bbfbedf8922e0fc8a2a9bc82dcefbf99083a546b4f3  queries.txt
```

Formato de `sha256sum`: 64 dígitos hexadecimales en minúscula, dos espacios y el nombre del fichero, una línea por fichero, en este orden. Lista exactamente los dos ficheros congelados. Se comprueba con:

```text
$ cd crates/kerno-testkit/data/assoc-eval && sha256sum -c MANIFEST
corpus.txt: OK
queries.txt: OK
```

Los dos checks salieron en ese orden en la ejecución de este documento.

### 3.4 Regeneración y comprobación

```text
cargo run --release -p kerno-testkit --example freeze_assoc_eval
```

Este comando escribe los tres ficheros desde la semilla. Sobre el árbol de E7-00 reescribió los ficheros con el mismo contenido: después `git status` no mostró ningún cambio en `data/assoc-eval/`. La prueba `regenerated_set_is_byte_identical` comprueba lo mismo sin escribir nada.

## 4. Líneas base B0 y B1

Las dos líneas base están medidas sobre un `UnitDb` real (sobre `KernoDb`, en el sistema de ficheros simulado). Se cargan las 300 transacciones de cambios con su actor y su fuente. Antes de medir, la prueba comprueba que cada relación del corpus existe en la base con su tipo, su procedencia completa y sus extremos, y que hay 2 000 unidades activas.

### 4.1 Definiciones

- **B0, filtro por tipo**: la consulta existente por tipo de unidad (`UnitSnapshot::query` con el tipo de la semilla). Se queda con las diez primeras en el orden que devuelve la consulta, que es el orden por identificador. Los identificadores se asignan en orden de creación, así que el orden es el de índice del corpus. No usa ninguna restricción de la consulta.
- **B1, vecinos a un salto**: `relations_of` de la semilla hacia fuera (`Direction::Out`), luego hacia dentro (`Direction::In`), sin filtro de tipo. Cada grupo sale en el orden del índice de adyacencia (tipo de relación y después identificador de relación). Se eliminan repeticiones (la primera aparición gana) y se corta en diez. No aplica ninguna restricción de la consulta: ni tipo, ni profundidad, ni ventana, ni procedencia.

Ambas líneas base se evalúan con las métricas de la sección 6, con corte en 10.

### 4.2 Resultados

Copiados de la salida cruda, [etapa7-e700-salida.txt](etapa7-e700-salida.txt), con el punto decimal de la salida:

| Ámbito | Consultas | B0 R@10 | B0 nDCG@10 | B1 R@10 | B1 nDCG@10 |
|---|---|---|---|---|---|
| overall | 64 | 0.000 | 0.000 | 0.715 | 0.499 |
| multi_hop | 16 | 0.000 | 0.000 | 0.651 | 0.467 |
| temporal | 16 | 0.000 | 0.000 | 0.623 | 0.491 |
| provenance | 16 | 0.000 | 0.000 | 0.856 | 0.523 |
| combined | 16 | 0.000 | 0.000 | 0.730 | 0.515 |

Por consulta, B1 llega a recall@10 de 1.000 en doce de las 64 consultas: `MH-03`, `MH-07`, `MH-11`, `MH-15`, `PV-RE-1`..`PV-RE-4` y `PV-SS-1`..`PV-SS-4`. B0 tiene recall@10 de 0.000 en las 64.

### 4.3 Cómo se reproduce

```text
cargo test -p kerno-units --release --test assoc_baseline -- --nocapture --exact baselines_over_the_frozen_set
```

La salida de esta ejecución es idéntica, línea por línea, a [etapa7-e700-salida.txt](etapa7-e700-salida.txt) salvo las líneas de compilación y de tiempos.

### 4.4 Lectura honesta de los números

**B0 es cero en todas las consultas, y eso es estructural.** B0 no usa la semilla más que para leer su tipo. Devuelve siempre las mismas diez unidades del tipo: para las decisiones, los índices 4, 5, 6, 24, 25, 26, 44, 45, 46 y 64 (las decisiones de las sesiones 0 a 3); para los hechos de las cuatro consultas `PV-SS`, los índices 7 a 14, 27 y 28. Ninguna de esas unidades tiene grado mayor que 0 en ninguna respuesta esperada de su tipo. Eso no es un fallo de la medición: un filtro por tipo que no mira la semilla no puede acertar en un conjunto donde la relevancia depende del tema. Por eso B0 no es un suelo útil para comparar rankings. El criterio de §6.2 del acta compara con B1 y con el orden simple, no con B0.

**B1 sí alcanza la relevancia en algunas clases, pero la ordena mal.** Su recall@10 medio es 0.715, mientras que su nDCG@10 medio es 0.499. Encuentra muchas de las respuestas relevantes, pero las pone detrás de vecinos de otros tipos y de distractores. El nDCG es lo que separa B1 de un ranking con sentido. Para referencia, un 10 % mejor que B1 sobre la media sería 0.549, y el umbral absoluto de §6.2 es 0.80. Estos números no deciden nada en E7-00; son la base que E7-05 debe superar.

**Clases en las que un salto ya llega a todo lo relevante.** Son las doce consultas con recall@10 de 1.000:

- **`PV-RE-1`..`PV-RE-4` y `PV-SS-1`..`PV-SS-4`**: estas consultas son de un salto por construcción (profundidad 1, sobre `related` o `same_source`). B1 alcanza exactamente las mismas unidades relevantes. Lo que B1 pierde es el orden: los distractores (dos `related` falsos en `PV-RE`, un `same_source` falso en `PV-SS`) aparecen junto a las relevantes, y el nDCG queda entre 0.500 y 0.624. Estas consultas miden el filtro por procedencia, no el recorrido.
- **`MH-03`, `MH-07`, `MH-11` y `MH-15`**: son las consultas de dirección `out` solo por `revises`, profundidad 4. Sus respuestas relevantes son la decisión anterior (grado 3) y la de dos posiciones atrás (grado 2). La primera es un `revises` saliente. La segunda es el destino del `related` verdadero del consolidador, que sale de la decisión. Por eso un salto las alcanza sin recorrer la cadena. Ejemplo: en `MH-03`, la semilla `505` tiene el `revises` a `504` (grado 3) y el `related` verdadero a `186` (confianza 910, grado 2); B1 los recupera a los dos. El nDCG es 0.552 porque la tarea de la semilla y los `related` falsos ocupan posiciones delante.

En las clases `temporal` y `combined` ningún salto llega al 100 %: el máximo de recall@10 de B1 es 0.667 en `temporal` y 0.800 en `combined`. Ahí la ventana temporal, el filtro de procedencia o la profundidad hacen falta para separar lo relevante de lo alcanzable.

**Límite de lectura.** Las dos clases en las que un salto basta son las que más ayudan a B1 por construcción: una consulta de un salto no mide recorridos a varios saltos. Por eso el resultado de B1 en `PV-RE` y `PV-SS` no debe leerse como una señal de calidad del método.

## 5. Respuestas esperadas y grados

### 5.1 De dónde salen

Las respuestas esperadas **no** salen de recorrer el grafo generado. Salen de la historia, con dos piezas de contabilidad que el generador mantiene aparte del corpus:

- La **cadena de cada tema**: la lista de sus decisiones en orden de confirmación, y para cada decisión su `revises` y su `supports` (el hecho que la sustenta). El recorrido de `walk_chain` sigue esta lista, no las relaciones del corpus.
- Las **restricciones** de la consulta, aplicadas a la relación que el recorrido quiere seguir: tipo, dirección, clase de procedencia, ventana, actor y fuente.

El procedimiento de `expect_chain` para una consulta de cadena desde la semilla `s`:

1. Recorre la cadena hacia atrás y hacia delante, un `revises` por paso, mientras la relación del paso sea admitida por las restricciones y la profundidad no llegue al máximo.
2. Cada decisión alcanzada a profundidad `d` tiene grado `3` si `d = 1`, `2` si `d = 2` y `0` en otro caso (distractor con nombre).
3. Para cada decisión alcanzada (y para la semilla, con `d = 0`) el `supports` que la sustenta cuenta si es admitido y si cabe en la profundidad. Su hecho tiene grado `3` si `d = 0`, `2` si `d = 1`, `1` si `d = 2` y `0` en otro caso.
4. El hecho de esa evidencia tiene una mención (`mentions`) a una decisión de otro tema. Si la mención es admitida y cabe en la profundidad, la decisión mencionada entra con grado 0.

Las consultas de procedencia por tipo de relación (`PV-RE`, `PV-SS`) no usan la cadena: cada respuesta es un vecino de la semilla por la relación pedida (`expect_links`). Un vecino por una relación verdadera recibe grado 2 en `PV-RE` (`related`) y grado 3 en `PV-SS` (`same_source`); por una relación falsa, grado 0.

La prueba `expected_answers_are_reachable_within_the_bounds` comprueba que cada respuesta del fichero es alcanzable con las cotas de su consulta, con una búsqueda en anchura sobre el corpus. Esa búsqueda **comprueba** las respuestas congeladas; no las produce. La prueba `every_query_has_a_reachable_distractor` comprueba que cada consulta nombra al menos un distractor alcanzable, para que las cotas no decidan la consulta por sí solas.

### 5.2 Grados

| Grado | Significado | Ganancia `2^g − 1` |
|---|---|---|
| 3 | Decisión a una revisión (`d = 1`) y evidencia de la semilla (`d = 0`); `same_source` verdadero (consultas `PV-SS`) | 7 |
| 2 | Decisión a dos revisiones; evidencia a una revisión; `related` verdadero (consultas `PV-RE`) | 3 |
| 1 | Evidencia a dos revisiones | 1 |
| 0 | Distractor con nombre (decisión a tres o más revisiones, evidencia a tres o más, mención, `related` falso, `same_source` falso) | 0 |

Relevante es grado ≥ 1. Distribución en el conjunto: 140 de grado 3, 119 de grado 2, 57 de grado 1 y 211 de grado 0.

### 5.3 Las clases de consulta

| Clase | Consultas | Semilla | Restricciones | Qué pone a prueba |
|---|---|---|---|---|
| `multi_hop` (`MH-01`..`MH-16`) | 16 | decisiones de la sesión 5 o posterior, con al menos 4 decisiones antes y 4 después en su tema, y una mención en la evidencia | Cuatro formas por `n mod 4`: `both` con tres tipos y profundidad 3 (0); `both` con profundidad 4 (1); `out` solo `revises`, profundidad 4 (2); `in` con `revises` y `supports`, profundidad 4 (3). Sin ventana ni procedencia | Recorrido de la cadena y de su evidencia |
| `temporal` (`TM-01`..`TM-16`) | 16 | las mismas | `both`, tres tipos, profundidad 3 o 4 según `n` impar, y una ventana de pasos alrededor de la sesión de la semilla | Contexto temporal |
| `combined` (`CB-01`..`CB-16`) | 16 | decisiones con evidencia `declared` | Ventana, procedencia `declared` solo, profundidad 3 o 4, y el actor de la sesión en las consultas pares | Procedencia y tiempo juntos |
| `provenance` (`PV-RE`, `PV-SS`, `PV-AC`, `PV-SR`) | 16 | decisiones (`PV-RE`, `PV-AC`, `PV-SR`) o hechos de las tres primeras posiciones de cada sesión (`PV-SS`) | `PV-RE`: `related` de un salto con procedencia `inferred`; `PV-SS`: `same_source` de un salto con procedencia `algorithmic`; `PV-AC`: cadena con el actor de la sesión; `PV-SR`: cadena con la fuente de transcripción de la sesión | Procedencia, actor y fuente |

Dos detalles de estas definiciones que afectan a la lectura:

- Una ventana `lo-hi` es un intervalo cerrado de pasos de **creación de la relación**, no de unidades. Siempre incluye los tres pasos de la sesión de la semilla y hasta dos sesiones a cada lado.
- Las consultas `multi_hop` no tienen ventana, así que su conjunto de respuestas es el de la cadena completa dentro de la profundidad.

## 6. Métricas

Implementadas en `crates/kerno-testkit/src/assoc_metrics.rs`. Una lista de resultados es el orden de índices de unidad que devuelve la consulta, el mejor primero.

Convenciones comunes:

- Una unidad repetida en la lista cuenta **una vez**, en su primera posición. La repetición **ocupa** su posición: las posiciones siguen contando desde la lista original.
- Una unidad que no aparece en las respuestas esperadas tiene grado 0.
- Una lista más corta que `k` deja las posiciones que faltan vacías. No se acorta a `k`.
- Con `k = 0` las tres métricas valen 0.

Sea `R` el conjunto de respuestas con grado ≥ 1, `r_p` el grado en la posición `p` (1 es la primera), `k` el corte y `gain(g) = 2^g − 1`:

- **recall@k** = |{unidades relevantes entre las primeras `k` posiciones}| / |R|. Si `R` está vacío, vale 0.
- **precision@k** = |{unidades relevantes entre las primeras `k` posiciones}| / `k`.
- **DCG@k** = Σ sobre las primeras `k` posiciones (sin repetidos) de `gain(r_p) / log2(p + 1)`.
- **IDCG@k** = Σ para `i = 1..k` de `gain(g_i) / log2(i + 1)`, donde `g_1 ≥ g_2 ≥ ...` son todos los grados de las respuestas esperadas ordenados de mayor a menor (incluidos los 0, que no suman).
- **nDCG@k** = DCG@k / IDCG@k. Vale 0 si `k = 0` o si IDCG vale 0 (no hay nada relevante).

Casos límite:

- **Resultados vacíos**: DCG = 0 y recall = precision = nDCG = 0.
- **`k` mayor que la lista**: recall y DCG cuentan solo las posiciones que existen. La precisión sigue dividiendo entre `k`: con una sola unidad relevante y `k = 10`, la precisión es 0.1. El IDCG usa los `k` mejores grados esperados, no la longitud de la lista.
- **Precisión y recall con pocas relevantes**: con más de 10 relevantes la precisión@10 nunca llega a 1. En este conjunto ninguna consulta tiene más de 9 relevantes, así que el recall@10 sí puede llegar a 1.
- **Grado fuera de la escala**: `gain` entra en pánico con un grado mayor que 3. El lector del fichero también lo rechaza.

Ejemplo de la prueba `hand_computed_example`: resultados `[1, 9, 2]`, con 1 de grado 3, 2 de grado 2 y 3 de grado 1, con `k = 3`. DCG = 7/log2(2) + 0 + 3/log2(4) = 8.5. Recall = 2/3 y precisión = 2/3.

## 7. Pruebas nuevas y mutaciones

Este WP añade 20 pruebas. Los conteos después de E7-00, con las nuevas marcadas, son estos:

| Conjunto | Después | Nuevas | Antes (derivado) |
|---|---|---|---|
| `kerno-testkit` lib (`assoc_metrics` 9, `sha256` 2) | 90 aprobadas | 11 | 79 |
| `kerno-testkit` `tests/assoc_eval.rs` | 8 aprobadas | 8 | 0 |
| `kerno-units` `tests/assoc_baseline.rs` | 1 aprobada | 1 | 0 |

El conteo «antes» es el de después menos las pruebas nuevas, porque el commit de E7-00 solo añade pruebas nuevas. No se ha ejecutado en el árbol anterior.

### 7.1 Mutaciones

Cada prueba nueva se ha ejecutado sobre una versión del código con una mutación. Después se ha restaurado el fichero con `git checkout -- <fichero>`. Tras la última restauración, ningún fichero de código aparecía modificado: `git status` solo listaba los documentos de este WP y `experiments/narrative-threads/.venv`, que no es de este trabajo. Las líneas son las del código sin mutar.

| Prueba | Mutación (fichero:línea) | Pruebas que fallaron |
|---|---|---|
| `regenerated_set_is_byte_identical` | `src/assoc_eval.rs:844`: cabecera del corpus `v1` → `v2` | `regenerated_set_is_byte_identical` (7 aprobadas) |
| `manifest_matches_committed_files` | `src/sha256.rs:6`: constante `K[0]` `0x428a2f98` → `0x428a2f99` | `manifest_matches_committed_files` (7 aprobadas) |
| `committed_files_parse_back` | `src/assoc_eval.rs:1063`: origen de `algorithmic` con sufijo `x` | `committed_files_parse_back` (7 aprobadas) |
| `parser_refuses_a_unit_out_of_order` | `src/assoc_eval.rs:1011`: el lector deja de comprobar el índice de unidad | `parser_refuses_a_unit_out_of_order` (7 aprobadas) |
| `quality_set_has_the_agreed_shape` | `src/assoc_eval.rs:239`: `temporal` se lee como `multi_hop` | `committed_files_parse_back`, `quality_set_has_the_agreed_shape` (6 aprobadas) |
| `expected_answers_are_reachable_within_the_bounds` | `src/assoc_eval.rs:216`: `out` y `in` intercambiados al leer la dirección | `expected_answers_are_reachable_within_the_bounds`, `every_query_has_a_reachable_distractor`, `committed_files_parse_back` (5 aprobadas) |
| `every_query_has_a_reachable_distractor` | `src/assoc_eval.rs:1147`: actor `-` leído como el filtro literal `-` | `every_query_has_a_reachable_distractor`, `expected_answers_are_reachable_within_the_bounds`, `committed_files_parse_back` (5 aprobadas) |
| `performance_scale_is_deterministic` | `src/assoc_eval.rs:101`: escala de rendimiento de 1000 a 999 sesiones | `performance_scale_is_deterministic` (7 aprobadas) |
| `gains_follow_two_to_the_grade_minus_one` | `src/assoc_metrics.rs:29`: ganancia `2^g − 1` → `g` | `gains_follow_two_to_the_grade_minus_one`, `k_larger_than_the_result_list`, `hand_computed_example`, `repeats_count_once` (5 aprobadas) |
| `a_grade_outside_the_scale_is_refused` | `src/assoc_metrics.rs:28`: el límite del pánico sube a 100 | `a_grade_outside_the_scale_is_refused` (8 aprobadas) |
| `hand_computed_example` | `src/assoc_metrics.rs:75`: descuento `log2(p + 1)` → `log2(p + 2)` | `hand_computed_example`, `ties_do_not_depend_on_their_order`, `repeats_count_once`, `k_larger_than_the_result_list` (5 aprobadas) |
| `empty_results_score_zero` | `src/assoc_metrics.rs:79`: `nDCG` vale 1 con lista vacía | `empty_results_score_zero` (8 aprobadas) |
| `no_distractor_is_relevant_and_zero_cut_scores_zero` | `src/assoc_metrics.rs:66`: `precision` sin la guarda de `k = 0` | `no_distractor_is_relevant_and_zero_cut_scores_zero` (8 aprobadas) |
| `expected_without_relevant_units_scores_zero` | `src/assoc_metrics.rs:57`: `recall` sin la guarda de «no hay relevantes» | `expected_without_relevant_units_scores_zero` (8 aprobadas) |
| `ties_do_not_depend_on_their_order` | `src/assoc_metrics.rs:84`: orden ideal ascendente en vez de descendente | `hand_computed_example`, `k_larger_than_the_result_list`, `ties_do_not_depend_on_their_order` (6 aprobadas) |
| `k_larger_than_the_result_list` | `src/assoc_metrics.rs:70`: la precisión divide entre `min(k, longitud)` | `empty_results_score_zero`, `k_larger_than_the_result_list` (7 aprobadas) |
| `repeats_count_once` | `src/assoc_metrics.rs:42`: se desactiva la eliminación de repetidos | `repeats_count_once` (8 aprobadas) |
| `standard_vectors` | `src/sha256.rs:6`: la misma constante `K[0]` que arriba | `standard_vectors`, `padding_boundaries` (lib: 88 aprobadas, 2 fallan). La de integración, `manifest_matches_committed_files`, también falla |
| `padding_boundaries` | `src/sha256.rs:32`: umbral de relleno `< 56` → `< 55` | `padding_boundaries` (solo esta; `standard_vectors` sigue aprobada) |
| `baselines_over_the_frozen_set` | `kerno-units/src/db.rs:749`: `relations_of` deja solo la primera arista de cada rango (cambio en la ruta de referencia que usa B1) | `baselines_over_the_frozen_set`, en `tests/assoc_baseline.rs:130` (la aserción de «relación ausente o alterada») |

Comandos usados para cada grupo:

```text
cargo test -p kerno-testkit --all-features --release --test assoc_eval
cargo test -p kerno-testkit --all-features --release --lib assoc_metrics
cargo test -p kerno-testkit --all-features --release --lib sha256
cargo test -p kerno-units --release --test assoc_baseline
```

Dos observaciones sobre las mutaciones:

- Las pruebas que solo leen los ficheros congelados no ven los cambios del generador. Por eso la mutación de `build_queries` no se usó: las pruebas de forma habrían seguido en verde. Las mutaciones se hicieron sobre el lector, que sí usan esas pruebas.
- La mutación de la línea base cambia código de `kerno-units`, que es la ruta de referencia de `relations_of`. Es intencionada: B1 depende de ella, y la prueba debe fallar si esa ruta devuelve menos vecinos de los que hay.

### 7.2 Puerta de verificación

La puerta del encargo, `cargo fmt --all --check && cargo test -p kerno-testkit --all-features --release`, pasa sobre este árbol. La salida se recoge en el informe de cierre de esta tarea.

## 8. Límites de un conjunto sintético

Este conjunto mide la implementación sobre un relato generado. No mide la calidad de la memoria en uso real. Los límites que el lector debe tener presentes:

1. **Mismo autor para el relato y para las consultas.** El generador, las familias de consultas y las reglas de grado las escribió la misma persona, antes de escribir el código de recuperación. El congelado protege contra ajustar las respuestas al código, pero no contra un supuesto del generador que el código comparta con él.
2. **Los grados son una elección.** Los grados 3, 2 y 1 por distancia y los distractores de grado 0 son convenciones de este documento, no juicios de relevancia de personas.
3. **La confianza de `related` separa los casos sin error.** Los verdaderos tienen confianza 701–950 y los falsos 302–600. Un ranking que use la confianza como señal acierta todos los `related` del conjunto. Esto favorece a `PV-RE` y debe tenerse en cuenta al leer cualquier medida de esa clase.
4. **La fuente de `same_source` coincide con `cites`.** Los pares verdaderos y falsos de `same_source` son consistentes con las fuentes citadas, así que el conjunto no tiene un caso en el que la relación algorítmica y la historia discrepen de forma sutil.
5. **Estructura regular.** Cada sesión tiene exactamente tres decisiones de un tema, ocho hechos y cinco fuentes; cada tema usa una cadena lineal. La memoria real tiene estructuras irregulares, ramas y decisiones que se contradicen.
6. **Vocabulario pequeño.** Doce temas, tres agentes, tres fuentes de transcripción, un importador y un consolidador. Las etiquetas son cadenas como `s0000.d0`, sin texto natural.
7. **Sin vectores.** El conjunto no contiene vectores. ADR-0011 dice que los vectores los entrega quien llama, así que la señal vectorial no se mide con este conjunto por sí sola: E7-04 tendrá que aportar vectores para medirla.
8. **Sesenta y cuatro consultas.** Con 16 por clase y 1 a 9 relevantes por consulta, la varianza de una media es alta. Una diferencia pequeña entre dos métodos no es significativa en este conjunto.
9. **Rendimiento no medido.** La escala de 20 000 unidades está generada y probada en forma, pero E7-00 no mide tiempos. Las mediciones de §6.3 del acta son de E7-05.

Lo que sí garantiza el conjunto, y para lo que está pensado: que cualquier cambio en el generador, en el formato o en la lectura de los ficheros se detecta, que las respuestas esperadas son alcanzables con sus cotas y que la base de referencia B1 se puede medir sobre ellas.

## 9. Documentos relacionados

- [Acta de la Etapa 7](etapa7-acta.md) §6 (conjunto, umbrales) y §7 (decisiones)
- [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md): vectores exactos primero, decidido en E7-00
- [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md): las cuatro clases de procedencia
- [Salida cruda de E7-00](etapa7-e700-salida.txt)
