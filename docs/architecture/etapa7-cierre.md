# Cierre de la Etapa 7

| Campo | Valor |
|---|---|
| Estado | **Cerrada y aprobada por el propietario el 2026-11-12.** No hay bloqueantes abiertos (§3.2). Las decisiones sobre el arranque de la Etapa 8 y sobre T4 las tomó el propietario el mismo día (§8) |
| Fecha | 2026-11-12 (E7-06) |
| Acto | Revisión adversarial de E7-00 a E7-05, comprobación de los criterios de §4 de la acta, umbrales de §6, propuesta de promoción de I-40 e I-41 y actualización de roadmap, SAD e invariantes (acta §3, fila E7-06) |
| Base revisada | Árbol en `52f026f` (fusión tras el cierre de E7-05). Este cambio solo toca `docs/` y no añade pruebas ni código |
| Documentos | [Acta de la Etapa 7](etapa7-acta.md) · [conjunto de evaluación](etapa7-conjunto-evaluacion.md) · [consulta asociativa (E7-01)](etapa7-consulta-asociativa.md) · [ejecución indexada (E7-02)](etapa7-ejecucion-indexada.md) · [contexto y procedencia (E7-03)](etapa7-contexto-y-procedencia.md) · [ranking (E7-04)](etapa7-ranking.md) · [medición (E7-05)](etapa7-medicion.md) · [salida cruda](etapa7-medicion-salida.txt) · [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) · [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [invariantes](invariants.md) · [roadmap, Etapa 7](../roadmap/README.md#etapa-7--memoria-asociativa) · [SAD §9](SAD.md) · [cierre de la Etapa 6](etapa6-cierre.md) |

Este documento describe la etapa tal como queda en el repositorio. Cada cifra sale de la salida cruda comprometida ([etapa7-medicion-salida.txt](etapa7-medicion-salida.txt)) o de un comando ejecutado para este cierre (§5). Lo que no se ha podido comprobar se dice en §7.

**Ningún umbral, respuesta esperada, peso ni fichero del conjunto congelado se ha cambiado en E7-06.**

## 0. Resultado en breve

### 0.1 Calidad frente a §6.2 (conjunto congelado, 64 consultas, corte en 10)

| Umbral (acta §6.2) | Número medido | Estado |
|---|---|---|
| T1: recall@10 con ranking ≥ 0,90 | **0,939** | cumple |
| T2: nDCG@10 con ranking ≥ 0,80 | **0,912** | cumple |
| T3: nDCG@10 con ranking ≥ 1,10 × B1 (≥ 0,550; B1 = 0,500) | **0,912** | cumple |
| T4: nDCG@10 con ranking ≥ 1,10 × orden simple (≥ 1,036; orden simple = 0,942) | **0,912** | **no cumple**. El umbral es inalcanzable: exige un nDCG por encima de 1 (§3.2, R-7; [medición §3](etapa7-medicion.md#3-umbrales-de-62)) |
| T5: nDCG@10 con ranking ≥ B1 en cada clase | multi_hop 0,871 ≥ 0,473; temporal 0,970 ≥ 0,493; provenance 0,842 ≥ 0,510; combined 0,965 ≥ 0,523 | cumple en las cuatro clases |
| C1: resultados con rastro verificable (I-40; criterio de corrección) | **757 de 757** | cumple |

**Veredicto de §6.2:** no se cumple, por T4. T1 a T3 y T5 se cumplen, pero no bastan: §7 punto 3 del acta activa el ranking por defecto solo si se cumple §6.2 entero.

### 0.2 Rendimiento frente a §6.3

| Criterio (acta §6.3) | Número medido | Estado |
|---|---|---|
| Con 20 000 unidades y profundidad 2, el p50 del índice no es peor que el de la referencia | En **12 de 12** corridas. Índice p50 entre 0,147 y 0,159 ms; referencia p50 entre 103,9 y 110,5 ms; razón entre 0,138 % y 0,144 % | **cumple** |
| p50, p99 y entradas visitadas por profundidad (1 a 3) | Informativo, sin umbral (§4 de la medición) | informado |

### 0.3 Los resultados, en la forma en que deben leerse

1. **El orden simple «saltos, luego identificador» alcanza recall@10 0,956 y nDCG@10 0,942**, frente a 0,731 y 0,500 de la línea base B1.
2. **El ranking con los pesos por defecto alcanza recall@10 0,939 y nDCG@10 0,912**, por debajo del orden simple en nDCG@10 en las cuatro clases. En recall@10 queda por debajo en tres de las cuatro: en `temporal` el ranking tiene 0,981 frente a 0,973 del orden simple. Esta afirmación depende del desempate del orden simple (por identificador, que sigue el orden de creación): con otros desempates una clase cambia de signo (§3.2, R-6). En conjunto, el ranking queda por debajo con los tres desempates ensayados.
3. **El ranking no pasa a ser el orden por defecto.** Queda como opción apagada por defecto (`RankMode::Off`), y se considera experimental. El orden por defecto sigue siendo «saltos, luego identificador».
4. **T4 no es alcanzable tal como está escrito.** Con el orden simple en 0,942, la cota es 1,036, y el nDCG no puede pasar de 1. Es un defecto del umbral de la acta y **se registra como no cumplido, sin cambiar el umbral** (acta §7 y medición §3).
5. **La señal vectorial no se ha medido por calidad.** El conjunto congelado no contiene vectores. La similitud vale 0 en todas las consultas de la medición. Medirla exige un conjunto con vectores, que hoy no existe (ADR-0011: los vectores los entrega quien llama).
6. **El camino por índice cumple el criterio de rendimiento a 20 000 unidades y profundidad 2 en las doce corridas.** La razón, de unas 700 veces, se explica en la medición (§4) y no es la de un sistema en uso real: la referencia es el oráculo, no un camino de producción.

## 1. Paquetes de trabajo entregados

| WP | Entrega | Commits | Estado |
|---|---|---|---|
| E7-00 | Acta aprobada (2026-11-11), ADR-0011 aceptado, conjunto de evaluación congelado con `MANIFEST` SHA-256, métricas y líneas base B0 y B1 | `5395681` (implementación), `24df5c1` (documento) | hecho. **Una corrección posterior del conjunto** (`6b2a3cb`), antes de cualquier código de recuperación: §3.2, R-1 |
| E7-01 | Modelo `AssocQuery` con cotas duras, ruta de referencia en anchura sobre `reference_relations_of`, rastro por resultado | `60de6d8` | hecho |
| E7-02 | Ejecución por índice en el planificador, comprobación de cada respuesta, caída registrada a la referencia, explicación sin literales | `1c377a5` | hecho |
| E7-03 | Ventana de `commit_seq` y filtros de procedencia, actor y fuente en las dos rutas; rastro verificable (I-40) | `703e657` | hecho |
| E7-04 | Ranking con modo diagnóstico, vectores exactos en memoria (ADR-0011), `RankMode::Off` por defecto | `a2d7500`; marca en acta y roadmap `d13494f` | hecho |
| E7-05 | Medición de calidad sobre el conjunto congelado y de rendimiento; salida cruda comprometida | `9be5668` | hecho. El ranking no pasa a ser el orden por defecto |
| E7-06 | Este cierre: revisión adversarial (§3), comprobación de §4 y §6 (§0 y §2), promoción propuesta de I-40 e I-41 (§6), actualización de roadmap, SAD e invariantes | este cambio | hecho; **aprobado por el propietario el 2026-11-12** |

No se añadió ningún paquete fuera de la acta. No hay cambios de formato persistente (acta §2.3) ni dependencias nuevas (criterio de §4 de la acta sobre I-31).

## 2. Criterios de aceptación (acta §4)

| Criterio | Invariante | Evidencia | Estado |
|---|---|---|---|
| Calidad evaluable con un conjunto fijado antes de medir | — | `MANIFEST` comprobado en la prueba `quality_over_the_frozen_set` (2 de 2 ficheros); conjunto fijado en E7-00 y corregido una vez antes de todo código de recuperación (`6b2a3cb`, anterior a `60de6d8`). Ver R-1 | **cumplido**, con la excepción de R-1 frente a la fila de E7-05 |
| Toda respuesta rastreable a sus fuentes | **I-40** | `tests/assoc_context.rs`: `every_cited_relation_and_revision_exists_in_the_snapshot`, `an_old_snapshot_never_cites_what_came_after_it`, `frozen_corpus_cites_only_what_the_snapshot_holds` (dos motores); el verificador compara con un modelo independiente, no con el propio rastro. `tests/assoc_reference.rs`: `frozen_corpus_traces_are_verifiable`. Medición: 757 de 757 | **cumplido** |
| El ranking explica sus señales | **I-41** | `tests/assoc_rank.rs`: `contributions_add_up_to_the_score_and_match_an_oracle` (oráculo independiente, 8 semillas, 2 métricas); 42 mutaciones detectadas en E7-04. La comprobación de la medición no es independiente de `Contributions::sum`: R-3 | **cumplido** |
| Equivalencia con la ruta de referencia | I-32 | Prueba diferencial: `tests/assoc_indexed.rs`: `differential_seeded`, `frozen_corpus_indexed_equals_reference` (las 64 consultas congeladas, cada una también con tres cotas ajustadas: límite 3; presupuesto justo por debajo de las visitas completas; límite 2 y presupuesto 7; y una consulta de tres semillas con las cotas máximas; en los dos motores). Medición: 64 de 64 iguales a la referencia y al planificador, 0 caídas. **Limitación H-1 abierta**: con 64 relaciones o más y una entrada ausente en ambos extremos, la respuesta por índice difiere de la referencia sin fallback (R-2) | **cumplido con H-1**. I-32 **no afirmado** para ese caso |
| Una relación inferida no se presenta como declarada | I-39 | `tests/assoc_context.rs`: `inferred_is_never_declared_in_a_trace` | **cumplido** |
| Sin literales fuera de `diagnostic` | I-37 | `tests/sentinels.rs`: `no_sentinel_in_any_associative_plan_observation_or_error` (dos motores); `tests/assoc_rank.rs`: `sentinels_no_literal_outside_the_diagnostic_output`. La huella de consultas asociativas usa solo marcadores (`?unit_ids`, `?u32`, `?texts`…), comprobado en `tests/assoc_indexed.rs` | **cumplido** |
| El núcleo no necesita red ni modelo | I-31 | Revisión de dependencias: ningún commit desde antes de E7-00 toca los nueve `Cargo.toml` y el `Cargo.lock` del repositorio (`git log` vacío sobre esas rutas); `kerno-units` tiene las mismas dependencias que antes; ninguna referencia a red en `crates/kerno-units/src` ni `crates/kerno-testkit/src`. Los vectores los entrega quien llama, en memoria | **cumplido** |
| Sin ANN antes de un benchmark que lo justifique | — | Búsqueda vectorial solo exacta (`VectorSet::search`, fuerza bruta). Ningún índice aproximado en `src/` (búsqueda de `hnsw`, `ann`, `faiss`, `lsh`, `annoy` y `approximate` en `kerno-units/src` y `kerno-testkit/src`: sin resultados) | **cumplido por alcance** |

**Criterio de E7-05 que no se cumple literalmente:** «ningún umbral ni respuesta esperada cambia después de E7-00». La corrección de `6b2a3cb` cambió el contenido de los ficheros congelados y el número de respuestas esperadas (de 527 a 533). Ningún umbral de la acta cambió. Ver R-1. Este cierre lo registra como **no cumplido literalmente**; el propietario decide si lo acepta.

## 3. Revisión adversarial (E7-06)

### 3.1 Qué se revisó y cómo

**Código y pruebas, leídos:**

- `crates/kerno-units/src/assoc.rs` (520 líneas): modelo, validación de cotas, traversal, presupuesto, rastro, orden.
- `crates/kerno-units/src/assoc_rank.rs` (590 líneas): pesos, señales, determinismo, vectores, comprobaciones de rechazo.
- `crates/kerno-units/src/planner.rs`, funciones `decide_assoc`, `explain_assoc`, `assoc_query`, `plan_assoc`, `execute_assoc`, `check_adjacent`, `is_mirrored`, `adjacency_unusable`, `assoc_candidates`.
- `crates/kerno-units/src/fingerprint.rs`: `NormalizedQuery::of_assoc`.
- `crates/kerno-units/tests/`: `assoc_reference.rs`, `assoc_indexed.rs` (incluida `completeness_is_not_a_query_time_check`), `assoc_context.rs`, `assoc_rank.rs`, `assoc_measure.rs`, `assoc_baseline.rs`, `sentinels.rs` (sección asociativa) y `harness/assoc.rs`.
- `crates/kerno-testkit/src/assoc_eval.rs`: formato, lector, `expect_chain`, `walk_chain`, `expect_links`. El resto del generador no se leyó línea a línea.
- `crates/kerno-testkit/src/assoc_metrics.rs` y `data/assoc-eval/MANIFEST`, `corpus.txt`, `queries.txt`.
- Historia de git de la acta, de `data/assoc-eval`, de `assoc_rank.rs`, de `assoc.rs` y de `harness/assoc.rs`.
- Los documentos `etapa7-*.md` se contrastaron con el código en las afirmaciones citadas en §2 y §3.2; no se leyeron enteros línea a línea.

**Comprobaciones ejecutadas:**

- `cargo test -p kerno-testkit -p kerno-units --all-features --release`: exit 0 (§5).
- `cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set`: la salida reproduce, línea a línea, el bloque de calidad de [etapa7-medicion-salida.txt](etapa7-medicion-salida.txt) (desde `MANIFEST:` hasta el resumen de umbrales).
- Mutaciones en una **copia** del árbol en `HEAD`, con su propio directorio de compilación, fuera del repositorio: dos mutaciones de E7-05 y una de E7-04, más una mutación nueva (§3.3).
- Controles exploratorios en la misma copia, **no comprometidos en el repositorio** (§3.4).

**Qué se comprobó en el código y no es un hallazgo:**

- Ningún literal llega a un plan: la huella usa marcadores; `Plan::of` no recibe la consulta; `AssocError::kind` da solo la clase; los mensajes solo llevan identificadores y números.
- El orden es total y entero: `(distancia, identificador)` en el recorrido y `(puntuación, identificador)` en el ranking; no hay mapas hash en `assoc.rs` ni en `assoc_rank.rs`.
- El verificador de I-40 en `assoc_context.rs` compara con un modelo escrito aparte, no con el rastro.
- Una entrada que lleva a una unidad no activa cuenta como examinada y no entra en el resultado (`seen` marca cada unidad una vez, esté activa o no).
- Las respuestas esperadas se construyen con el libro de la historia (`Ledger`) y con los atributos de cada relación, no con la adyacencia del grafo generado; `corpus_query` no lee respuestas esperadas.

### 3.2 Hallazgos

Cada hallazgo indica su evidencia y si es bloqueante. **Bloqueantes: ninguno.**

| # | Hallazgo | Evidencia | Clase | Causa | Bloqueante | Estado |
|---|---|---|---|---|---|---|
| R-1 | El conjunto congelado cambió después de E7-00: `6b2a3cb` regeneró `corpus.txt`, `queries.txt` y `MANIFEST`, y las respuestas esperadas pasaron de 527 a 533. Ocurrió antes de cualquier código de recuperación (`60de6d8` es posterior) y está documentado en [conjunto §9](etapa7-conjunto-evaluacion.md#9-corrección-previa-al-uso-fugas-de-un-solo-campo). La corrección tomó como motivo fugas de un solo campo detectadas en los datos, no en el código | `git show 6b2a3cb --stat`; `git log --oneline -- crates/kerno-testkit/data/assoc-eval` (dos commits: `5395681` y `6b2a3cb`); conjunto §9.3 | determinista (historia) | introducido en E7-00, corrección | no | **registrado**: criterio de E7-05 no cumplido literalmente. El propietario acepta o rechaza la excepción |
| R-2 | **Límite H-1 del índice**: con 64 relaciones o más, una relación sin entrada de adyacencia en ninguno de sus dos extremos no se detecta en consulta. La respuesta por índice difiere de la referencia sin fallback. Solo `verify_units` (UNIT-36) lo ve | `tests/assoc_indexed.rs:637` (`completeness_is_not_a_query_time_check`): en la tienda grande, `out.result != want`, `fallback == None` y `UNIT-36` en el informe; ejecución de la prueba en verde | determinista (prueba que fija el límite) | preexistente (E7-02; H-1 de la acta §5) | no | **aceptado como riesgo** (acta §5); I-32 no se afirma en ese caso. Ver §7 |
| R-3 | **La comprobación de I-41 en la medición no es independiente de `Contributions::sum`.** Con `sum()` alterado en +1, `assoc_measure` sigue en verde; solo fallan 8 pruebas de `assoc_rank` (oráculo) | Replicado en la copia (§3.3, mutación M-R4): `test result: ok. 1 passed` en `assoc_measure`; `test result: FAILED. 11 passed; 8 failed` en `assoc_rank` | determinista | preexistente (E7-05) | no | **registrado**. La evidencia independiente de I-41 es el oráculo, no la cifra «757 de 757» |
| R-4 | **El contador de C1 es tautológico.** `traces.0` y `traces.1` suman el mismo número, `simple.hits.len()`. Un fallo hace `panic` antes de imprimir, así que la cifra 757/757 cuenta aciertos comprobados, no un recuento independiente | `crates/kerno-units/tests/assoc_measure.rs`, líneas que incrementan `traces` (justo después de `trace_failure`) | determinista (código) | preexistente (E7-05) | no | **registrado**; sin efecto en el veredicto |
| R-5 | **La convención de juicio puede favorecer al orden simple.** Toda unidad que el conjunto no nombra cuenta como grado 0. En las 64 consultas, el top 10 del orden simple tiene 36 de 476 posiciones sin nombrar (7,6 %); el del ranking, 53 de 476 (11,1 %). Ninguna unidad relevante nombrada queda sin alcanzar (322 de 322) | Control exploratorio (§3.4) sobre la salida de `reference_assoc_query` y `rank_assoc`. Determinista | determinista (recuento) | diseño del conjunto (§5.2: los distractores son «con nombre», solo los juzgados) | no | **registrado como sesgo posible en contra del ranking**. La diferencia de nDCG entre órdenes no es una medida limpia de relevancia |
| R-6 | **El resultado por clase depende del desempate del orden simple.** Con desempate invertido (identificador descendente dentro de cada distancia), el orden simple da nDCG@10 0,932 en conjunto; en `multi_hop` baja a 0,869, por debajo del ranking (0,871). Con desempate mezclado, 0,942 en conjunto. En `provenance` el orden simple sube a 0,897 y 0,886 (frente a 0,849). Por debajo en conjunto con los tres desempates | Controles exploratorios (§3.4) | determinista | diseño (desempate por identificador, que sigue el orden de creación) | no | **registrado**. La frase «por debajo del orden simple en las cuatro clases» vale solo para el desempate definido en la acta; en conjunto, el ranking queda por debajo con cualquiera de los tres |
| R-7 | **T4 es inalcanzable tal como está escrito** (acta §6.2): exige 1,036 con un nDCG acotado por 1. Cualquier ranking con el orden simple por encima de 0,909 falla T4 | Aritmética: 1,10 × 0,942 = 1,036; `assoc_measure.rs` imprime «T4 … ≥ 1,036 … 0,912 -> not met» | determinista | defecto del umbral (acta §6.2) | no | **registrado como no cumplido**. El umbral no se cambia (§8.2) |
| R-8 | **La configuración medida del ranking es parcial.** Con `RankOptions::default()`, la señal de tipo vale 0 (no hay tipos preferidos) y la similitud vale 0 (no hay vectores). Solo actúan saltos, procedencia y recencia. La recencia no se ha medido por separado; el conjunto no tiene actualizaciones de unidades, así que la recencia sigue el orden de creación | `assoc_measure.rs` (`RankOptions { mode: On, ..default() }`); [ranking §1](etapa7-ranking.md) | inferencial (código) | diseño de la medición | no | **registrado**: la conclusión sobre el ranking vale para esa configuración |
| R-9 | **La frase «pesos con un valor por defecto medido»** (acta §2.3) no describe lo que ocurrió: los pesos se justificaron antes de medir (E7-04) y no se ajustaron. «Medido» significa aquí evaluado una vez sobre el conjunto | `git log -- crates/kerno-units/src/assoc_rank.rs` (un solo commit: `a2d7500`, anterior a la medición); [ranking §4](etapa7-ranking.md) | inferencial (lectura) | redacción de la acta | no | **registrado**. La acta no se reescribe en este cierre |
| R-10 | **El recorrido de B1 no recibe las restricciones de la consulta** (tipo, dirección, ventana, procedencia): B1 es «sin filtro» por definición (acta §6.1). Así, la comparación con B1 favorece a las consultas asociativas por diseño. La comparación entre orden simple y ranking no se ve afectada, porque ambos ven el mismo resultado | `assoc_measure.rs`, `baseline_b1` | inferencial (código) | definición de B1 | no | **registrado** |
| R-11 | **Mutación 1 de la medición sin mensaje registrado.** El documento decía «falla (el mensaje exacto no se copió)» | Replicada (§3.3, M-R1): el mensaje es `MANIFEST: corpus.txt does not match the frozen file` | determinista | documental | no | **resuelto** en este cierre |
| R-12 | **Estabilidad de la señal vectorial entre plataformas**: garantizada por construcción (ADR-0011 y [ranking §6](etapa7-ranking.md)), no probada en otra plataforma | Lectura de `similarity` en `assoc_rank.rs` | inferencial | diseño | no | **registrado** |

**Sin hallazgo, pero registrado para que no se lea de otra forma:** el resultado de la medición de rendimiento se toma de la salida comprometida y no se repitió en E7-06 (§7).

### 3.3 Mutaciones replicadas en una copia de `HEAD` (`52f026f`)

Cada mutación se aplicó a una copia del árbol, con su propio directorio de compilación. El árbol de trabajo no se tocó. Comando: `cargo test -p kerno-units --release …`.

| # | Mutación | Qué debía fallar | Resultado real |
|---|---|---|---|
| M-R1 | `assoc_measure.rs:188`: `"corpus.txt" => QUERIES.as_bytes()` (de E7-05, mutación 1) | la comprobación del `MANIFEST` | **falla**: `assertion left == right failed: MANIFEST: corpus.txt does not match the frozen file` (`assoc_measure.rs:192`) |
| M-R2 | `assoc_measure.rs:132`: `if trace != kept` → `if trace == kept` (de E7-05, mutación 2, I-40) | el recorrido de I-40 | **falla**: `MH-01: a hit does not verify (I-40): relation …: the trace differs from the stored relation` |
| M-R3 | `assoc_rank.rs:528`: `score: Some(c.sum())` → `Some(c.sum() + 1)` (equivale a M13 de [ranking §8](etapa7-ranking.md#8-mutaciones)) | las pruebas de I-41 | **falla** en 8 pruebas de `assoc_rank` (cuatro en cada motor: `contributions_…`, `hand_vectors_with_…`, `hand_vectors_without_…`, `ties_…`) y en `assoc_measure`: `MH-01: the contributions do not add up to the score (I-41)` |
| M-R4 | `assoc_rank.rs:435` (cuerpo de `Contributions::sum`): `+ self.similarity` → `+ self.similarity + 1` (no es una mutación registrada; añadida en E7-06) | las pruebas de I-41 | `assoc_rank`: **falla** en 8 pruebas. `assoc_measure`: **pasa** (`test result: ok. 1 passed`). Ver R-3 |

Las copias de los ficheros mutados se restauraron desde `HEAD` antes de cada mutación nueva.

### 3.4 Controles exploratorios (no comprometidos)

Se ejecutaron en la misma copia, añadiendo pruebas auxiliares a `tests/assoc_measure.rs` que recorren las mismas 64 consultas con la misma traducción (`corpus_query`). No cambian ningún fichero del repositorio, no son parte de la medición comprometida y no se usan para cambiar ningún número de §0. Sirven para discutir R-5 y R-6.

| Control | Overall R@10 | Overall nDCG@10 | multi_hop nDCG | temporal nDCG | provenance nDCG | combined nDCG |
|---|---|---|---|---|---|---|
| Orden simple, tal como se mide (desempate por identificador) | 0,956 | 0,942 | 0,960 | 0,978 | 0,849 | 0,981 |
| Desempate invertido dentro de cada distancia | 0,949 | 0,932 | 0,869 | 0,983 | 0,897 | 0,979 |
| Desempate mezclado (orden determinista por hash del índice) | 0,947 | 0,942 | 0,928 | 0,971 | 0,886 | 0,982 |
| Ranking, tal como se mide (referencia) | 0,939 | 0,912 | 0,871 | 0,970 | 0,842 | 0,965 |

Unidades sin nombrar en el top 10 (R-5): orden simple 36 de 476; ranking 53 de 476. Unidades relevantes nombradas alcanzadas por el orden simple: 322 de 322.

Lectura: el resultado global del ranking por debajo del orden simple se mantiene con los tres desempates. Por clase, la comparación es sensible al desempate y al convenio de juicio. **Estas cifras no sustituyen a la medición de E7-05 y no se citan como resultado de la etapa.**

## 4. Medición (E7-05, salida comprometida)

Fuente: [etapa7-medicion-salida.txt](etapa7-medicion-salida.txt) y [medición §2 a §4](etapa7-medicion.md). Corte en 10; medias sobre las 64 consultas.

### 4.1 Calidad

| Ámbito | Consultas | B1 R@10 | B1 nDCG@10 | Orden simple R@10 | Orden simple nDCG@10 | Ranking R@10 | Ranking nDCG@10 |
|---|---|---|---|---|---|---|---|
| **overall** | 64 | 0,731 | 0,500 | **0,956** | **0,942** | **0,939** | **0,912** |
| `multi_hop` | 16 | 0,658 | 0,473 | 0,868 | 0,960 | 0,819 | 0,871 |
| `temporal` | 16 | 0,641 | 0,493 | 0,973 | 0,978 | 0,981 | 0,970 |
| `provenance` | 16 | 0,864 | 0,510 | 0,982 | 0,849 | 0,967 | 0,842 |
| `combined` | 16 | 0,761 | 0,523 | 1,000 | 0,981 | 0,991 | 0,965 |

Los endings de las 64 consultas del orden simple son todos `Exhausted`: ninguna llegó al límite de 1 000 resultados ni agotó el presupuesto de 100 000 visitas. El planificador no cayó a la ruta de referencia en ninguna consulta (`planner fallbacks: 0`).

### 4.2 Rendimiento (depth 2, las doce corridas)

| Invocación | Corrida | Referencia p50 (ms) | Índice p50 (ms) | Referencia p99 (ms) | Índice p99 (ms) |
|---|---|---|---|---|---|
| 1 | 1 a 4 | 104,70 a 109,26 | 0,148 a 0,156 | 187 a 202 | 0,393 a 0,441 |
| 2 | 1 a 4 | 103,85 a 108,28 | 0,147 a 0,155 | 193 a 205 | 0,414 a 0,432 |
| 3 | 1 a 4 | 107,65 a 110,51 | 0,149 a 0,159 | 187 a 209 | 0,410 a 0,435 |

Rango total: referencia p50 103,9–110,5 ms; índice p50 0,147–0,159 ms. Entradas visitadas por profundidad (media): 3,3 (profundidad 1), 11,1 (profundidad 2), 25,1 (profundidad 3). La tabla completa por corrida está en [medición §4](etapa7-medicion.md#4-rendimiento-63).

Desviaciones registradas en la medición: calentamiento de cuatro rondas de doce (un tercio, no el 10 % de la cabecera; la cabecera se corrigió después de medir, sin cambiar ningún cálculo).

## 5. Verificación de E7-06

| Comprobación | Comando | Resultado |
|---|---|---|
| Suite de `kerno-testkit` y `kerno-units` | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | exit 0; suma de las líneas `test result`: **645 aprobadas, 0 fallidas, 8 ignoradas**. Igual que en [medición §8](etapa7-medicion.md#8-verificación) (645). Ningún test nuevo en E7-06 |
| Medición de calidad, reproducción | `cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set` | `test result: ok. 1 passed`; el bloque desde `MANIFEST:` hasta el resumen de umbrales coincide línea a línea con `etapa7-medicion-salida.txt` |
| Formato | `cargo fmt --all --check` | exit 0 |
| Lint con avisos como error | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile [unoptimized + debuginfo] target(s) in 30.52s`; exit 0 |
| Pruebas citadas en §2 | búsqueda en la salida de la suite (`suite`, con `test … ok`) | aparecen como `ok`; en los ficheros con dos motores, en ambos (`emulated_log` y `kerno`, o `kerno_db` en `sentinels`) |
| Dependencias | `git log` sobre los nueve `Cargo.toml` y el `Cargo.lock` desde antes de E7-00 | sin commits |
| Red en el núcleo (I-31) | búsqueda de referencias a red en `kerno-units/src` y `kerno-testkit/src` | ninguna |

No se ha ejecutado el banco de rendimiento (`assoc_bench`) en E7-06: los números de §4.2 son los del commit (§7).

## 6. Promoción de I-40 e I-41

Ambas quedan **promovidas y aprobadas por el propietario el 2026-11-12**. La evidencia de cada una está en §2 y §3. Ninguna de las observaciones de §3.2 es bloqueante para ellas: R-2 (H-1) afecta a I-32, no a I-40 ni a I-41; R-3 y R-4 afectan a la forma de citar la medición, no a la existencia de la prueba independiente.

### 6.1 I-40

**Enunciado (acta §4):** cada resultado de una consulta asociativa cita las relaciones y revisiones que lo justifican, y todas existen en la instantánea consultada.

**Evidencia:** verificador independiente (modelo escrito aparte) en `tests/assoc_context.rs`, en los dos motores; prueba sobre el corpus congelado; medición con 757 de 757 aciertos. Mutaciones de E7-03 y la mutación M-R2 de §3.3, que hacen fallar la prueba.

**Límites:** I-40 cubre la integridad del rastro (existencia, campos, camino, revisión cabeza, filtros). No cubre que el conjunto de resultados sea completo: esa es la parte de H-1.

### 6.2 I-41

**Enunciado (acta §4):** la puntuación es determinista y el modo diagnóstico da la aportación de cada señal, cuya suma es la puntuación.

**Evidencia:** `contributions_add_up_to_the_score_and_match_an_oracle` (oráculo independiente sobre `get_relation` y `get_unit`, descrito en [ranking §7](etapa7-ranking.md#7-pruebas)); `hand_vectors_*` con valores calculados a mano; `same_state_and_query_give_the_same_order`; mutaciones de E7-04 (M01 a M42) y la mutación M-R3 de §3.3.

**Límites:** la comprobación de la medición (R-3) no detecta un fallo dentro de `sum`. El oráculo sí. El determinismo de la similitud vectorial entre plataformas no está probado (R-12).

## 7. Riesgos abiertos y límites

1. **H-1 del índice (R-2).** Con 64 relaciones o más, una relación sin entrada de adyacencia en ninguno de sus extremos no se detecta en consulta. La respuesta por índice puede ser incompleta sin fallback. La prueba `completeness_is_not_a_query_time_check` fija el límite: si alguien lo cierra, la prueba debe cambiar. Lo cubren `verify_units` (UNIT-36) y la prueba diferencial sobre tiendas sanas.
2. **El conjunto es sintético y de un solo autor** (conjunto §8, puntos 1 y 2). El relato, las consultas y las reglas de grado las escribió el mismo autor, y las respuestas esperadas recorren la cadena de decisiones que el grafo también contiene. Por eso el orden simple, que recorre el grafo en anchura, puede puntuar alto por esa coincidencia de origen. **No se puede afirmar que el conjunto sea fácil ni difícil**: no hay otra medida de dificultad independiente.
3. **Estructura regular** (conjunto §8, punto 5): tres decisiones por tema y sesión, cadenas lineales, doce temas. La memoria real es más irregular.
4. **Convención de juicio (R-5)**: unidades sin nombrar valen 0. Penaliza más al brazo que lleva más unidades sin nombrar al top 10; hoy, el ranking.
5. **Sensibilidad al desempate (R-6)**: las comparaciones por clase cambian con el desempate. El global no.
6. **Configuración del ranking (R-8)**: sin señal de tipo ni vectores. La recencia no se ha medido aparte.
7. **Señal vectorial no medida por calidad**: el conjunto no tiene vectores.
8. **Sesenta y cuatro consultas**, con 1 a 9 relevantes cada una. Una diferencia pequeña entre dos métodos no es significativa (conjunto §8, punto 8).
9. **Rendimiento**: una máquina de 4 CPU con carga media de 1 a 9 durante la medición; las cifras absolutas no son comparables entre máquinas. Las doce corridas son de la salida comprometida; **no se repitieron en E7-06**.
10. **Escalas no medidas**: calidad a 2 000 unidades solamente; rendimiento a 20 000 solamente; profundidad 4 en rendimiento no medida; concurrencia entre lectores no medida.
11. **Estabilidad entre plataformas de la similitud vectorial** (R-12): no probada fuera de esta máquina.
12. **Los pesos por defecto no están ajustados** (R-9): se fijaron por justificación. Cambiarlos exige otra medición con otro conjunto, porque ajustar sobre este lo contaminaría.
13. **El ranking no ve unidades que el corte deja fuera** (ranking §10): el orden de (saltos, identificador) limita el resultado antes de ordenar por puntuación. Si lo relevante está más allá del límite, el ranking no lo recupera.
14. **Recencia sin medir por separado**, y el conjunto no tiene actualizaciones de unidades (R-8).

## 8. Decisiones del propietario

El propietario tomó estas dos decisiones el 2026-11-12. Se registran aquí, con la recomendación que sostuvo la evidencia en el momento del cierre.

### 8.1 ¿Arranca la Etapa 8?

**Por qué hay decisión pendiente:** el [roadmap](../roadmap/README.md#decisiones-del-propietario-de-2026-11-11), punto 1, dice que si el ranking no cumple los umbrales de calidad de §6.2, la Etapa 8 no empieza hasta que el propietario decida cómo seguir. El ranking no cumple §6.2 (T4, y por debajo del orden simple en nDCG@10).

**Recomendación que sostiene la evidencia:** aceptar este cierre con el orden simple «saltos, luego identificador» como orden por defecto y el ranking como opción apagada (medición §6, primera opción). El [roadmap](../roadmap/README.md#etapa-8--consolidación-y-embeddings) no pone el ranking entre los entregables ni las aceptaciones de la Etapa 8, así que no hay una dependencia que obligue a medirlo antes. Hay una advertencia: la Etapa 8 producirá los embeddings que la señal vectorial del ranking consumiría, y la calidad de esa señal no se ha medido (§7, punto 7). Si el propietario quiere medirla, necesita un conjunto con vectores, que es un conjunto nuevo (§8.2). No condicionar la Etapa 8 a esa medición.

**Decidido por el propietario el 2026-11-12:** la Etapa 8 arranca. El ranking de resultados asociativos queda **desactivado por defecto y marcado como experimental**. Con el ranking desactivado, el orden de los resultados es el de «saltos, luego identificador».

### 8.2 ¿Se reescribe T4 antes de cualquier medición nueva?

**Por qué hay decisión pendiente:** T4 (acta §6.2) no se puede cumplir tal como está escrito (R-7). Cambiarlo ahora, con el conjunto actual, sería ajustar el umbral a los resultados.

**Recomendación que sostiene la evidencia:** no cambiar T4 sobre este conjunto. Si el propietario quiere volver a medir el ranking, reescribir T4 como umbral alcanzable (por ejemplo, no inferior al orden simple, o una mejora en otra métrica) **antes** de medir, dejarlo congelado con fecha, y construir un conjunto nuevo (otra semilla y, si es posible, otro autor para las consultas). No usar este conjunto para ajustar pesos ni umbrales. Medir de nuevo con este conjunto no aporta evidencia nueva, porque ya ha visto los resultados (medición §6).

**Decidido por el propietario el 2026-11-12:** T4 no se reescribe ahora. El texto de T4 en la [acta de la Etapa 7](etapa7-acta.md) (§6.2) queda como está, y no se usa para decidir nada sobre este conjunto. Si el ranking se vuelve a medir, será con un conjunto de evaluación nuevo y un umbral definido en ese momento, antes de medir. No se ajusta ningún peso ni umbral sobre el conjunto congelado.

## 9. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-12 | **Cerrada y aprobada por el propietario** | Sin bloqueantes abiertos (§3.2). Criterios de §2 cumplidos, con H-1 (§2, I-32) y con la excepción de R-1 frente a la fila de E7-05. Umbrales de §0.1 registrados tal como se midieron; T4 no cumplido y no reescrito (§8.2). I-40 e I-41 promovidas y aprobadas (§6). Decisiones de §8: la Etapa 8 arranca con el ranking desactivado por defecto y marcado como experimental |
