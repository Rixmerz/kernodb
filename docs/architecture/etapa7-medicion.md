# Etapa 7 · E7-05 — Medición de la memoria asociativa

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Medido.** Calidad: §6.2 **no se cumple** (el umbral T4 es inalcanzable tal como está escrito, y el ranking queda por debajo del orden simple). Rendimiento: ver §4 |
| Acta | [Etapa 7](etapa7-acta.md) §6.2 (calidad), §6.3 (rendimiento), §7 punto 3 (ranking por defecto), fila E7-05 |
| Conjunto | `crates/kerno-testkit/data/assoc-eval/` (`MANIFEST` verificado antes de medir) · [conjunto de evaluación](etapa7-conjunto-evaluacion.md) |
| Código | `crates/kerno-units/tests/assoc_measure.rs` (calidad; se ejecuta con `cargo test`) · `crates/kerno-units/examples/assoc_bench.rs` (rendimiento; no se ejecuta con `cargo test`) · `crates/kerno-units/tests/harness/assoc.rs` (traducción de consultas, compartida con E7-02 y E7-03) |
| Salida cruda | [etapa7-medicion-salida.txt](etapa7-medicion-salida.txt) |

Este documento no cambia ningún umbral, ninguna respuesta esperada, ni ningún peso del ranking. Lo que aquí se afirma sale de la salida cruda; lo que no se afirma está en §5.

## Resultado en una línea

**El ranking no pasa a ser el orden por defecto.** Con los pesos por defecto, la consulta con ranking cumple recall@10 (0,939), nDCG@10 (0,912), al menos un 10 % sobre B1 y B1 en las cuatro clases, pero queda por debajo del orden simple «saltos, luego identificador» (nDCG@10 0,942). El umbral de un 10 % sobre el orden simple (T4) exige 1,036, y el nDCG no puede pasar de 1: no se puede cumplir. El orden por defecto sigue siendo «saltos, luego identificador».

## 1. Método

### 1.1 Regla de traducción (fijada antes de mirar resultados)

Cada consulta del fichero `queries.txt` se traduce a un `AssocQuery` con una sola regla, la misma para todas las consultas de todas las clases. La regla es la de `corpus_query` en `tests/harness/assoc.rs`, escrita en E7-02 y usada por las pruebas de E7-02 y E7-03; esta medición la reutiliza sin cambios. Solo usa lo que el fichero de consultas da:

| Campo de `queries.txt` | Campo de `AssocQuery` |
|---|---|
| `semilla` (índice de unidad) | `seeds = [id de esa unidad]` |
| `dirección` (`out`, `in`, `both`) | `direction` = `Out`, `In` o `Both` |
| `tipos` (lista) | `relation_types` = el conjunto de esos tipos |
| `profundidad` | `max_depth` |
| `ventana` (`lo-hi` o `-`) | `window` = `SeqWindow { lo, hi }`, o ninguna. Los pasos son `commit_seq`: el cargador comprueba que cada conjunto de cambios se confirma con su paso como secuencia |
| `clases` (lista o `-`) | `provenance.kinds`, o ninguna |
| `actor` (nombre o `-`) | `provenance.actor`, o ninguna |
| `fuente` (fuente o `-`) | `provenance.source`, o ninguna |
| `tipo` de la semilla | no entra en `AssocQuery`; lo usa solo B0 |
| `id`, `clase`, respuestas `E` | no entran en la consulta: son la evaluación |
| *(no está en el fichero)* `limit` | `MAX_ASSOC_LIMIT` = 1 000, para todas |
| *(no está en el fichero)* `visit_budget` | `MAX_ASSOC_VISITS` = 100 000, para todas |
| *(corte)* | 10, en las métricas, sobre el orden de cada brazo |

El `limit` se fija al máximo, y no al corte de 10, por una razón: el ranking solo reordena los resultados que la consulta devuelve (`rank_assoc`). Con un límite de 10, el ranking solo podría reordenar los diez primeros del orden simple, y no podría subir a la cima nada que el recorrido no haya devuelto. El corte de 10 se aplica después, en la evaluación. Ninguna consulta del conjunto llegó al límite: todas terminan con `Exhausted`, y ninguna agotó el presupuesto de visitas (§3, comprobaciones).

### 1.2 Brazos

- **B1**: vecinos a un salto por `relations_of`, hacia fuera y luego hacia dentro, sin filtro, cortados en 10. Es el mismo código que `tests/assoc_baseline.rs`; el resultado se compara con la salida cruda de E7-00 (§3).
- **Orden simple**: `reference_assoc_query` con la regla de §1.1. Su orden es «distancia, luego identificador», por contrato de `AssocEnding::LimitReached`.
- **Ranking**: el mismo resultado ordenado con `RankMode::On`, con los pesos por defecto (`RankWeights::default()`: saltos 1 000, tipo 500, recencia 400, procedencia 250, similitud 1 500), sin tipos preferidos y **sin vectores**.

**Señal vectorial.** No se mide por calidad. El conjunto no contiene vectores (conjunto de evaluación, §8 punto 7): la similitud vale 0 para todos los resultados, y el brazo de calidad no la ejercita.

### 1.3 Comprobaciones previas a cualquier cifra

Todas se ejecutan dentro de la prueba y, si alguna falla, la prueba falla antes de imprimir las cifras:

| Comprobación | Resultado (salida cruda) |
|---|---|
| `MANIFEST`: SHA-256 de `corpus.txt` y `queries.txt` | 2 de 2 coinciden |
| I-32: índice = referencia para cada consulta | 64 de 64 |
| Planificador = referencia | 64 de 64, 0 caídas a la ruta de referencia |
| I-41: suma de las aportaciones = puntuación, en el modo diagnóstico | 757 aciertos (hits) de 757 |
| I-40: cada acierto cita relaciones y revisiones existentes, con sus campos, en un camino real desde una semilla, y cumple el filtro de la consulta | 757 de 757 aciertos (hits) |
| B1 reproducido desde la salida de E7-00 | 64 de 64 consultas, con los tres decimales impresos |
| Terminación de la consulta | 64 `Exhausted`; ninguna `LimitReached`; ninguna `BudgetExhausted` |

## 2. Resultados de calidad

Medias sobre las consultas, corte en 10. Fuente: la tabla `summary` de la salida cruda.

### 2.1 Resumen

| Ámbito | Consultas | B1 R@10 | B1 nDCG@10 | Orden simple R@10 | Orden simple nDCG@10 | Ranking R@10 | Ranking nDCG@10 |
|---|---|---|---|---|---|---|---|
| **overall** | 64 | 0,731 | 0,500 | **0,956** | **0,942** | **0,939** | **0,912** |
| `multi_hop` | 16 | 0,658 | 0,473 | 0,868 | 0,960 | 0,819 | 0,871 |
| `temporal` | 16 | 0,641 | 0,493 | 0,973 | 0,978 | 0,981 | 0,970 |
| `provenance` | 16 | 0,864 | 0,510 | 0,982 | 0,849 | 0,967 | 0,842 |
| `combined` | 16 | 0,761 | 0,523 | 1,000 | 0,981 | 0,991 | 0,965 |

Lectura de la tabla:

- B1 encuentra muchas de las relevantes (recall 0,731) pero las ordena mal (nDCG 0,500). Es la lectura que ya hacía E7-00.
- El orden simple ya supera a B1 en todas las clases, por mucho. Su nDCG es 0,942 en conjunto.
- El ranking con los pesos por defecto **baja** el nDCG respecto del orden simple en las cuatro clases (en `provenance` la diferencia es de 0,007). Baja también el recall en conjunto (0,956 → 0,939).

### 2.2 Por consulta

La tabla completa de las 64 consultas (B1, orden simple y ranking, con su terminación) está en la salida cruda, bajo `per query`. Las consultas en que el ranking pierde más frente al orden simple son las de `multi_hop` de profundidad 4 con dirección `out` y `in` (`MH-06`: nDCG 0,884 → 0,552; `MH-10`: 0,929 → 0,548) y `CB-12` (0,966 → 0,696). No se ha analizado qué señal causa cada pérdida: no era objeto de esta medición, y no se ajusta nada aquí.

## 3. Umbrales de §6.2

Cada umbral, con su número y su estado. Un umbral que no se cumple se registra como tal; no se modifica ni se recalcula con otros pesos.

| Umbral (acta §6.2) | Cota | Valor medido | Estado |
|---|---|---|---|
| **T1** recall@10 con ranking, en conjunto | ≥ 0,90 | **0,939** | **cumple** |
| **T2** nDCG@10 con ranking, en conjunto | ≥ 0,80 | **0,912** | **cumple** |
| **T3** nDCG@10 con ranking ≥ 1,10 × B1 | ≥ 0,550 (B1 = 0,500) | **0,912** | **cumple** |
| **T4** nDCG@10 con ranking ≥ 1,10 × orden simple | ≥ 1,036 (orden simple = 0,942) | **0,912** | **no cumple**: ver nota |
| **T5** nDCG@10 con ranking ≥ B1, en `multi_hop` | ≥ 0,473 | 0,871 | cumple |
| **T5** … en `temporal` | ≥ 0,493 | 0,970 | cumple |
| **T5** … en `provenance` | ≥ 0,510 | 0,842 | cumple (margen 0,332) |
| **T5** … en `combined` | ≥ 0,523 | 0,965 | cumple |
| **C1** resultados con rastro verificable (I-40; criterio de corrección, no umbral de calidad) | 100 % | **757 de 757** | **cumple** |

**Nota sobre T4.** El umbral pide que el ranking supere en un 10 % el nDCG del orden simple. Con el orden simple en 0,942, la cota es 1,036. El nDCG está acotado por 1, así que **ningún** ranking puede cumplir T4 mientras el orden simple tenga un nDCG superior a 0,909. Esto es un defecto del umbral tal como está escrito en §6.2, no un resultado de la medición, y no se corrige aquí: el umbral no cambia en esta etapa. Hace falta que el propietario decida si §6.2 se reescribe antes de medir de nuevo. Con el umbral tal como está, no hay ranking posible que lo cumpla.

**Veredicto de §6.2 en conjunto: no se cumple.** T4 no se cumple, y además el ranking queda por debajo del orden simple (0,912 frente a 0,942), que es lo que §7 punto 3 del acta mira para decidir el orden por defecto.

## 4. Rendimiento (§6.3)

Escala: 20 000 unidades, 39 958 relaciones, 3 000 conjuntos de cambios, `KernoDb` en disco (sistema de ficheros real, sin el simulador), las 64 consultas del conjunto de rendimiento con la regla de §1.1 y la profundidad sustituida por 1, 2 o 3. Cada corrida mide 512 pares (64 consultas × 8 rondas contadas) por ruta y profundidad. Las dos rutas corren sobre la misma instantánea, una después de la otra, con el orden alternado par a par.

**Protocolo** (fijado antes de la primera corrida; `crates/kerno-units/examples/assoc_bench.rs`):

- tres invocaciones del comando, en procesos separados, cada una con cuatro corridas: **doce corridas**, todas publicadas;
- cada corrida recorre las rondas a una profundidad; las cuatro primeras rondas de cada corrida son calentamiento y no cuentan (ver la nota de abajo);
- p50 y p99 por ruta, con el método de rango más próximo;
- la carga (media de 1, 5 y 15 minutos de `/proc/loadavg`) se registra antes y después de cada corrida; la máquina tiene 4 CPU;
- las invocaciones corrieron una a una, nunca en paralelo.

**Veredicto de §6.3 (profundidad 2, p50):** el p50 del recorrido por índice **no es peor** que el de la referencia, en las **doce** corridas. Índice: entre 0,147 y 0,159 ms; referencia: entre 103,9 y 110,5 ms. La razón índice/referencia está entre 0,138 % y 0,144 %. **Cumple.**

Tabla por corrida, profundidad 2 (tiempos en milisegundos):

| Invocación | Corrida | Referencia p50 | Índice p50 | Referencia p99 | Índice p99 | Carga media 1 min (antes → después) |
|---|---|---|---|---|---|---|
| 1 | 1 | 104,9543 | 0,1481 | 186,7349 | 0,4213 | 6,82 → 4,87 |
| 1 | 2 | 109,2580 | 0,1562 | 202,2549 | 0,4412 | 2,77 → 4,62 |
| 1 | 3 | 106,9097 | 0,1491 | 200,4527 | 0,4123 | 3,83 → 3,69 |
| 1 | 4 | 104,6953 | 0,1500 | 187,5749 | 0,3930 | 2,70 → 1,48 |
| 2 | 1 | 103,8506 | 0,1470 | 193,3364 | 0,4322 | 0,98 → 1,00 |
| 2 | 2 | 106,7527 | 0,1501 | 194,6790 | 0,4264 | 1,03 → 1,01 |
| 2 | 3 | 108,2784 | 0,1547 | 204,9643 | 0,4138 | 1,09 → 1,02 |
| 2 | 4 | 108,2131 | 0,1549 | 195,6016 | 0,4219 | 1,00 → 1,00 |
| 3 | 1 | 110,5134 | 0,1588 | 200,5446 | 0,4349 | 0,96 → 0,99 |
| 3 | 2 | 108,2287 | 0,1493 | 208,5447 | 0,4095 | 1,00 → 1,00 |
| 3 | 3 | 107,6515 | 0,1523 | 187,3676 | 0,4103 | 1,02 → 1,00 |
| 3 | 4 | 108,4628 | 0,1527 | 196,9757 | 0,4251 | 1,06 → 1,02 |

Rango entre corridas (doce): referencia p50 103,85–110,51 ms, índice p50 0,147–0,159 ms; referencia p99 186,7–208,5 ms, índice p99 0,393–0,441 ms.

**Profundidades 1 y 3 (informativas; sin umbral).** En las doce corridas:

| Profundidad | Entradas visitadas (media) | Aciertos (media) | Referencia p50 (rango) | Índice p50 (rango) | Referencia p99 (rango) | Índice p99 (rango) |
|---|---|---|---|---|---|---|
| 1 | 3,3 | 2,7 | 26,5–27,9 ms | 0,059–0,066 ms | 32,0–42,6 ms | 0,127–0,175 ms |
| 2 | 11,1 | 6,3 | 103,9–110,5 ms | 0,147–0,159 ms | 186,7–208,5 ms | 0,393–0,441 ms |
| 3 | 25,1 | 12,9 | 165,1–172,9 ms | 0,241–0,253 ms | 616,5–654,9 ms | 1,220–1,461 ms |

Las entradas visitadas y los aciertos son los mismos en las dos rutas; cada par se compara antes de contar (los resultados son iguales, ver §1.3).

**Nota sobre el calentamiento.** La cabecera de `assoc_bench.rs` dice que el calentamiento es el 10 % de las rondas. Las corridas comprometidas se hicieron con `--reps 12` y cuatro rondas de calentamiento, es decir, un tercio de las rondas, no un décimo: la cota de 10 % corresponde al valor por defecto (`--reps 40`) y no a este comando. Con más calentamiento se descartan más muestras, no menos, y el veredicto no depende de ello. Queda registrado como desviación de la cabecera, no del protocolo medido.

**Lectura honesta.**

- El efecto es grande y no depende de la carga: el recorrido por índice tarda ≈ 0,15 ms, la referencia ≈ 105 ms, a profundidad 2. La referencia recorre las relaciones en cada salto (`reference_relations_of`), y el índice de adyacencia lee solo los rangos de la unidad. La razón es de unas 700 veces, y se mantiene en las doce corridas, incluidas las de la primera invocación, que empezó con carga ≈ 8,7 sobre 4 CPU.
- **Esta razón no es la de un sistema en uso real.** La referencia es el oráculo de E7-01 y no un camino de producción. Lo que sí se afirma es que el camino por índice no es peor que el de referencia, que es lo que pide §6.3.
- Como el umbral de §6.3 es de comparación entre rutas, la medición no fija un tiempo absoluto que cumplir: no hay umbral absoluto de latencia en el acta.

## 4.1 Resultado de rendimiento frente a §6.3

| Criterio | Número | Estado |
|---|---|---|
| Con 20 000 unidades y profundidad 2, el p50 del índice no es peor que el de la referencia (doce corridas) | índice p50 ≤ referencia p50 en 12 de 12 corridas; razón 0,138 %–0,144 % | **cumple** |
| p50, p99 y entradas visitadas por profundidad (1 a 3) | informado (§4) | informativo |

## 5. Lo que no se afirma, y por qué

1. **La señal vectorial no se mide por calidad.** El conjunto no tiene vectores. ADR-0011 dice que los vectores los entrega quien llama, así que medirla exige un conjunto con vectores, que no existe en esta etapa.
2. **El orden simple puede beneficiarse de la estructura del conjunto.** Los identificadores se asignan en orden de creación, y el orden simple desempata por identificador. El conjunto corrige las fugas de un solo campo que se conocían (conjunto de evaluación, §9), pero deja una señal estructural residual: la distancia entre índices de origen y destino de un `related` verdadero acierta 0,729 en el corpus (conjunto de evaluación, §8 punto 3). Un control que desempate al revés o al azar no se ha hecho, así que no se sabe cuánto del 0,942 viene de la relevancia y cuánto de esa estructura. Las cifras absolutas del orden simple no deben leerse como calidad del método.
3. **Conjunto sintético, mismo autor para el relato y las consultas** (conjunto de evaluación, §8 puntos 1 y 2). Las diferencias pequeñas no son significativas: con 64 consultas y entre 1 y 9 relevantes por consulta, la varianza de la media es alta.
4. **Un solo juego de pesos, sin ajuste.** Se mide el ranking con los pesos por defecto, como se acordó. Un ranking con otros pesos sería otro método: necesitaría un conjunto de medición distinto, porque ajustar pesos sobre este conjunto lo contaminaría.
5. **T4 no se puede cumplir** (§3). Con este umbral, ningún ranking pasa §6.2 aunque supere al orden simple en todo lo demás. La medición no dice nada sobre un ranking con otro umbral.
6. **Rendimiento en una máquina compartida**: la carga media era ≈ 8,7 sobre 4 CPU al empezar la primera invocación, y ≈ 1 en las otras dos. El veredicto de §6.3 se cumplió en las tres, y la diferencia es de casi tres órdenes de magnitud, así que la carga no lo decide; sí afecta a los tiempos absolutos, que no se afirman más allá de lo registrado en §4.
7. **No medido**: escala de calidad distinta de 2 000 unidades, escala de 100 000, profundidad 4 en rendimiento, concurrencia entre lecturas, vectores.

## 6. Recomendación

**El orden por defecto sigue siendo «saltos, luego identificador».** El ranking queda como opción, apagada por defecto, como lo dejó E7-04.

Razones, por el orden de peso:

1. §7 punto 3 del acta: el ranking se activa por defecto solo si cumple §6.2. No cumple (T4), y el propio umbral T4 no se puede cumplir tal como está escrito.
2. Aun sin mirar T4, el ranking con pesos por defecto empeora el orden simple en nDCG en las cuatro clases (conjunto: 0,912 frente a 0,942), y en recall (0,939 frente a 0,956).
3. Lo que el ranking sí hace bien (superar B1 con holgura, cumplir T1 a T3) lo hace el orden simple también, y mejor.

**Decisiones que quedan para el propietario** (la hoja de ruta, Etapa 7, «Punto de corte en E7-05», dice que la Etapa 8 no empieza hasta que el propietario decida cómo seguir):

- Aceptar el resultado negativo, cerrar la etapa con el ranking como opción (acta §5, riesgo «El ranking no mejora al orden simple»), o
- revisar §6.2 (en particular T4) y repetir la medición con un conjunto nuevo, no con este, porque el conjunto ya ha visto los resultados.

## 7. Reproducción

```text
# calidad (cada vez que se ejecuta cargo test; también en CI)
cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set

# rendimiento (no se ejecuta con cargo test; tres invocaciones de cuatro corridas)
cargo run --release -p kerno-units --example assoc_bench -- --runs 4 --reps 12 --dir <directorio temporal>
```

La salida cruda [etapa7-medicion-salida.txt](etapa7-medicion-salida.txt) es la de estas órdenes, en este orden: la calidad, y luego las tres invocaciones de rendimiento, con la carga de la máquina en cada corrida.

## 8. Verificación

| Comprobación | Orden | Resultado |
|---|---|---|
| Formato | `cargo fmt --all --check` | 0 (tras aplicar `cargo fmt --all` a los dos ficheros nuevos) |
| Lints, con avisos como error | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| Suite de `kerno-testkit` y `kerno-units` | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | 0; **645 aprobadas, 0 fallidas** |
| Conteo de pruebas | antes → después | 644 → 645 (`assoc_measure::quality_over_the_frozen_set`). El cambio no toca ningún otro fichero de pruebas |
| Mutación 1 de la prueba: comparar el fichero equivocado en el `MANIFEST` | cambiar `CORPUS.as_bytes()` por `QUERIES.as_bytes()` | **falla** (la prueba termina en rojo; el mensaje exacto no se copió) |
| Mutación 2: invertir la comparación de rastro (I-40) | `trace != kept` → `trace == kept` | **falla**: «MH-01: a hit does not verify (I-40): … the trace differs from the stored relation» |
| Mutación 3: invertir la comprobación de aportaciones (I-41) | `assert_eq!` → `assert_ne!` | **falla**: «MH-01: the contributions do not add up to the score (I-41)» |

Cada mutación se hizo sobre el fichero de la prueba, se ejecutó la prueba en rojo y se restauró el texto original (comprobado con `grep` antes de seguir). No se ha tocado código de producción: `assoc.rs`, `assoc_rank.rs`, el planificador, el conjunto congelado, el formato persistente ni ningún umbral.

**Desviaciones y lo que se corrigió después de medir.**

- La cabecera de `assoc_bench.rs` describía el calentamiento como el 10 % de las rondas; las corridas comprometidas usaron cuatro rondas de doce (ver §4). La cabecera se corrigió después de medir; no cambia ningún cálculo.
- Después de medir, `cargo fmt` reformateó los dos ficheros de ejemplo y de prueba. No cambia ninguna instrucción, pero el binario de rendimiento de la salida cruda se compiló antes de ese cambio y de la corrección de la cabecera.
- La salida de calidad comprometida es de una segunda ejecución de la misma prueba, sin cambios en el código de medición; reproduce la primera ejecución (mismas cifras, mismos veredictos).
