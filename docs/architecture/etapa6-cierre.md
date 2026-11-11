# Cierre de la Etapa 6

| Campo | Valor |
|---|---|
| Estado | **Pendiente de aprobación del propietario.** No es efectivo hasta que el propietario lo apruebe (§7). |
| Fecha | 2026-11-11 (E6-05) |
| Acto | Revisión adversarial, promoción de I-33…I-36 y cierre de la Etapa 6 (acta §3, fila E6-05) |
| Alcance | Paquetes E6-00 a E6-05: caché de resultados como única optimización, en `kerno-units` (`cache.rs`, `planner.rs`, `tests/cache*.rs`) |
| Documentos | [Acta de la Etapa 6](etapa6-acta.md) · [medición y centinelas](etapa6-medicion.md) (§8, veredicto final) · [E6-00](etapa6-e600-remedicion.md) · [ejecución adaptativa](adaptive-execution.md) · [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [invariantes §2.5](invariants.md) · [roadmap](../roadmap/README.md#etapa-6--primera-plasticidad-operacional) · [cierre de la Etapa 5](etapa5-cierre.md) |

Este documento describe la etapa tal como queda en la rama tras E6-05. **La caché de resultados queda desactivada por defecto** (acta §8, decisión 2). La mejora neta de §6 del acta **no queda afirmada**: el p99 de W1 no se puede certificar. Ningún umbral se ha modificado.

## 0. Resultado en breve

| Umbral (acta §6) | Resultado en la medición final (§4) | Pasa |
|---|---|---|
| W1, p50 de la caché encendida ≥ 20 % mejor | −97,8 % a −97,9 % en las cuatro corridas | **Sí** |
| W1, p99 ≤ +5 % | +0,5 %, +7,8 %, +4,7 %, +7,5 % en cuatro corridas; coste real emparejado de la lectura que no se sirve de +3,4 % a +4,9 % | **No certificado** (dos de cuatro corridas por encima de +5 %; el coste real medido está por debajo, pero dentro del ruido) |
| Peor caso sin repeticiones, p99 ≤ +5 % (20 000 y 100 000 unidades) | Coste real emparejado de −0,6 % a +0,5 % (media); en las ocho corridas de p99 mediana, dos lecturas quedan en +5,3 % (una a 20 000, otra a 100 000) | **Sí en la medición emparejada; no en todas las corridas** |
| Criterio conjunto de mejora neta (§6) | No se cumple de forma estable: el p99 de W1 no se certifica | **No afirmado** |

**Consecuencia (acta §8.2):** la caché de resultados sigue **desactivada por defecto** y disponible como opción (`ResultCache::set_enabled`). Activarla por defecto exigiría una decisión del propietario sobre un criterio que la medición no permite afirmar.

## 1. Paquetes de trabajo entregados

| WP | Entrega | Commits (principales) | Estado |
|---|---|---|---|
| E6-00 | Acta aprobada (2026-11-09) con las decisiones de §7 y §8; re-medición del planificador sobre máquina sin carga | `558880f`, `ac1a000`, `38e2a5d`, `fccd7d2` | hecho |
| E6-01 | Modelo de dependencias versionadas (`Dependency`, `Dependencies`), almacén en memoria con cota de tamaño y edad, registro de transiciones (G7) | `fbe1e95`, `3759479` | hecho |
| E6-02 | Caché como primer candidato del planificador en `query` y `query_relations`; fallback con causa registrada; comprobación de filas en cada acierto | `44f076a`, `f17cb20` | hecho |
| E6-03 | Reversibilidad y desactivación: activar, desactivar, invalidar y borrar no tocan datos ni `commit_seq` | `b08f1cb`, `e93208c`, `3985822` | hecho |
| E6-04 | Medición con la carga de §6 y centinelas de I-37 | `f0d5cca`, `ca28c7f` | hecho; el veredicto de su primer intento se revisó en E6-04b |
| E6-04b | Admisión (no guardar un resultado que nadie repetirá), huella memorizada por forma, banco de 20 rondas y medición emparejada; veredicto final de §8 de la medición | `b7dd18f`, `2a1c433`, `c5ab3cb`, `9ea9363`, `15b2814` | hecho; veredicto de §8 |
| E6-05 | Este cierre: revisión adversarial (§3), promoción de I-33…I-36 en `invariants.md` §1 y §2.5, actualización del roadmap, de SAD §9, de `adaptive-execution.md` y de la tabla de §4 del acta, correcciones de comentarios de `cache.rs` | este cambio | hecho, **pendiente de aprobación** |

## 2. Criterios de aceptación (acta §4)

| Criterio | Invariante / garantía | Evidencia | Estado |
|---|---|---|---|
| Mejora neta medida frente al modo desactivado, con umbrales fijados antes | — | §4: p50 de W1 cumple; p99 de W1 no se certifica; peor caso cumple en la medición emparejada | **no afirmado**: la caché queda desactivada por defecto |
| Equivalencia de resultados | I-32, G1 | `tests/cache_planner.rs`, `cache_on_off_and_invalidated_give_identical_results` (`emulated_log` y `kerno_db`): caché encendida y apagada, con escrituras entre rondas y una base retenida que lee una instantánea antigua. Limitación H-1 (§3) | **cumplido** |
| Fallback a un plan válido, registrado, ante precondición, invalidación, desactivación o discrepancia | I-33, G2, G3 | `tests/cache_planner.rs`: `a_failed_check_discards_the_entry_and_the_reference_path_answers`, `observations_never_act_as_authority`, `every_use_of_the_cache_is_a_transition`. La causa queda en el registro de la caché, no en la observación (H-6) | **cumplido** (con H-6) |
| Invalidación probada por cada dependencia | I-35, G4 | `cache.rs`, `each_declared_dependency_invalidates_the_entry_and_is_named` (un caso por `Dependency`, con `match` exhaustivo); `tests/cache_reversibility.rs`, `invalidating_by_each_dependency_leaves_the_data_as_it_was` | **cumplido** |
| Invalidar, desactivar o borrar no altera datos | I-34, G6 | `tests/cache_reversibility.rs`, `every_operation_on_the_cache_leaves_the_data_as_it_was`, `a_cache_in_use_never_changes_what_a_commit_writes` (estado lógico, bytes del motor y `commit_seq` iguales) | **cumplido** |
| Desactivación verificada | I-36, G6 | `tests/cache_reversibility.rs`, `with_the_cache_off_no_optimization_takes_part`, `a_cache_switched_off_after_use_stops_taking_part`. La mutación de §3 (H-7) sobrevive | **cumplido con H-7** |
| La ruta aprendida pasa las mismas validaciones | G5 | Mutación de `check_units` en el acierto: muere (§3.1) | **cumplido** (con H-1) |
| Trazabilidad sin literales | G7, I-37 | `cache.rs`, `every_transition_leaves_a_record`; `tests/cache_sentinels.rs`, `every_g7_transition_leaves_its_record` y `no_literal_in_anything_the_cache_path_reports` (cuatro niveles, dos motores); mutaciones S-M1 y S-M2 de [E6-04b](etapa6-medicion.md) §4 | **cumplido** |

## 3. Revisión adversarial (E6-05)

### 3.1 Qué se revisó

- `crates/kerno-units/src/cache.rs` (1 675 líneas): modelo de dependencias, `CacheEntry`, almacén `ResultCache`, admisión (`worth_storing`), registro de transiciones y huella memorizada por forma.
- `crates/kerno-units/src/planner.rs`, funciones `answer_units`, `answer_relations`, `store`, `execute_units` y `execute_relations`: la única entrada de la caché al camino de consulta.
- `crates/kerno-units/tests/cache.rs`, `cache_planner.rs`, `cache_reversibility.rs`, `cache_admission.rs` y `cache_sentinels.rs`.
- Frente a ADR-0016 (G1 a G7) e I-37, según la lectura del código, de las pruebas y de las mutaciones de §3.3.

Cada garantía se leyó en el código que la hace cumplir y se buscó la prueba que falla si se rompe. Resultado por garantía:

| Garantía | Qué la hace cumplir en el código | Prueba que la defiende | Estado |
|---|---|---|---|
| G1 equivalencia | Cada acierto pasa por `check_units`/`check_relations` y cada entrada guarda la respuesta de la ruta planificada o de la de referencia sobre la misma `commit_seq` | `cache_on_off_and_invalidated_give_identical_results`; mutación de G4 (§3.3) | cumple; completitud no comprobada en consulta (H-1) |
| G2 referencia siempre disponible | Cada fallo, invalidación, desactivación o discrepancia en `answer_units`/`answer_relations` ejecuta el camino de referencia o el planificado sin estado aprendido | `a_failed_check_discards_the_entry_and_the_reference_path_answers` | cumple |
| G3 observaciones no son autoridad | `answer_units` y `answer_relations` no leen el observador; la caché solo usa `index_stats` y `fingerprint_of` | `observations_never_act_as_authority` | cumple (lectura de código confirmada por búsqueda en `planner.rs`) |
| G4 invalidación explícita | `Dependencies::first_change` compara las cinco dependencias antes de cada uso; `commit_seq` es coincidencia exacta; no hay excepción «salvo prueba» | mutación de `commit_seq` (§3.3): muere | cumple |
| G5 sin saltarse validaciones | Acierto → `check_units`; un fallo borra la entrada y responde la referencia | mutación de `check_units` (§3.3): muere | cumple (H-1) |
| G6 reversibilidad | Todas las operaciones de la caché operan sobre `ResultCache`, que no tiene manejador del motor ni transacción | `cache_reversibility.rs` (estado lógico, bytes, `commit_seq`) | cumple (no se mutó) |
| G7 trazabilidad | Cada transición llama a `State::record`, con `OPTIMIZATION_VERSION` y `PLANNER_VERSION`; `Transition` no tiene campo para literales | `every_transition_leaves_a_record` y `every_g7_transition_leaves_its_record` | cumple (H-6 en la observación) |
| I-37 | `TransitionRecord`, `CacheError`, `CacheStats` y `Transition` no tienen campo de filtro, unidad ni fila; `UnitError::InvalidArgument` lleva solo el nombre del límite | `no_literal_in_anything_the_cache_path_reports` (cuatro niveles, dos motores, con control); mutaciones S-M1 y S-M2 | cumple |

### 3.2 Hallazgos

Cada hallazgo indica la evidencia y si se corrigió en este cambio. **Bloqueantes: ninguno.** No hay hallazgo que contradiga G1 a G7 ni I-37; los que quedan son documentales, de cobertura o de diseño, y se listan como decisiones en §6.

| # | Hallazgo | Evidencia | Clase | Causa | Estado |
|---|---|---|---|---|---|
| H-1 | `check_units` y `check_relations` comprueban que la respuesta de la caché sea un subconjunto correcto (filtro, `Active`, orden), no que sea completa. Es el mismo límite de H-1 de [cierre de la Etapa 5](etapa5-cierre.md) §3 | Lectura de `planner.rs` (`answer_units`, `check_units`). Inferencial | inferencial | heredado | **aceptado como limitación**; lo cubre la prueba diferencial, no la consulta |
| H-2 | El fallback con índice `Building` no se ejerce a través de `query` (H-2 de la Etapa 5) | Heredado | inferencial | heredado | **abierto**, sin cambio en esta etapa |
| H-3 | La evidencia de creación de la entrada es constante: `Evidence { observations: 1, window: Duration::ZERO }` en `planner.rs` (`store`). No sale de `ExecutionObservation`. G3 lo permite (la observación no es autoridad), pero spec §9 pide «evidencia observada (número de observaciones y ventana)», y aquí el campo no describe observación alguna | `planner.rs`, `store()`. Determinista | determinista | introducido en E6-02 | **abierto**: el propietario decide si la evidencia debe venir del observador (con G3) o si basta con documentar que es la ejecución que produjo la respuesta |
| H-4 | Comentario de cabecera de `cache.rs` obsoleto: decía que la caché «no está conectada a `query` ni al planificador (E6-02)» | `grep` de `cache.rs` y `planner.rs:354` | determinista | pre-existente (E6-01) | **corregido** en E6-05 (solo comentario) |
| H-5 | Una entrada más vieja que `max_age` no se sirve nunca (la comprobación está en `lookup`), pero sigue en memoria mientras la caché no cree una entrada, cambie sus cotas o consulte esa misma huella. `prune_entries` solo lo llaman `create` y `set_config`. El comentario de cabecera decía «none older than `max_age`», que no es cierto en reposo. La acta §5 pide «cota por tamaño y edad» | `grep -n prune_entries crates/kerno-units/src/cache.rs` → solo las llamadas dentro de `set_config` y `create`. Determinista | determinista | pre-existente (E6-01) | **comentario corregido** en E6-05. **El código no cambia**: la memoria sigue acotada por `max_entries` y `max_bytes`. Corrección posible de una línea (llamar a `prune_entries` en `stats()` y `records()`), que el propietario decide |
| H-6 | Las causas de la caché (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) no llegan a `ExecutionObservation`. Su campo `fallback` lleva `FallbackCause` (`Corrupt` o `Invalid`); un `discrepancy` aparece como `Invalid` con el camino `ResultCache` abandonado, y un `invalidated` o `disabled` no deja rastro en la observación (la consulta se registra con su camino planificado y sin `fallback`). Están en el registro de transiciones (G7), y `adaptive-execution.md` §2 lo decía de otra manera | `grep` de `observation.rs` y `planner.rs` (`CacheFallbackCause` solo se usa en `cache.fallback`). Determinista | determinista | introducido en E6-02 frente al diseño de [ejecución adaptativa](adaptive-execution.md) | **documentado** en `adaptive-execution.md` §2 y en I-33. Llevarlo a la observación es un cambio de tipo público (`FallbackCause`), que no se hace en esta etapa |
| H-7 | La mutación «con la caché apagada se sigue el camino de la caché» (`if !self.cache.enabled()` → `if false`, `planner.rs:354`) **sobrevive** a toda la suite de caché. Con la mutación, con la caché apagada se hace una lectura del índice más por consulta; ni las respuestas ni los contadores cambian. Ninguna prueba de I-36 observa que el camino de la caché no se ejecute | §3.3, fila I-36. Determinista | determinista | pre-existente (E6-03) | **abierto**: I-36 está verificada en su parte de resultados y contadores, no en la de coste. Una prueba fiable exige observar la lectura del índice, que hoy no tiene punto de inyección |

### 3.3 Mutaciones de la revisión

Cada mutación se aplicó con `sed` a una línea de producción, se corrió la suite con `cargo test -p kerno-units --all-features --release --no-fail-fast --lib --test cache --test cache_planner --test cache_reversibility --test cache_admission --test cache_sentinels` (sin filtro posicional, que filtra todos los binarios a la vez) y se restauró con `git checkout`. El árbol quedó limpio tras cada una.

| Mutación | Garantía / invariante | Resultado real |
|---|---|---|
| `cache.rs`, `Dependency::CommitSeq => Some(0)` | G4, I-35 | **muere**: `an_entry_never_serves_another_commit_seq`, `each_declared_dependency_invalidates_the_entry_and_is_named`, `first_change_vectors`, `dependency_vectors`, `every_transition_leaves_a_record` (lib); `a_commit_invalidates_the_cached_result_of_an_earlier_snapshot` y `cache_on_off_and_invalidated_give_identical_results` en `emulated_log` y `kerno_db` |
| `planner.rs`, `CachedResult::Units(units) => Ok::<_, Violation>(units)` | G5, I-33 | **muere**: `a_failed_check_discards_the_entry_and_the_reference_path_answers` en `emulated_log` y `kerno_db`; `no_literal_in_anything_the_cache_path_reports` en ambos motores (`test result: FAILED. 8 passed; 2 failed` y `2 passed; 2 failed`) |
| `planner.rs:354`, `if !self.cache.enabled()` → `if false` | I-36, G6 | **sobrevive** (H-7): ninguna de las cinco binarias de prueba falla |

Lo que no se mutó: el registro de transiciones (G7) y los centinelas de I-37 se revisaron por lectura y por las mutaciones S-M1 y S-M2 de [E6-04b](etapa6-medicion.md) §4, ya registradas.

### 3.4 Qué queda sin revisar

- Concurrencia entre lectores que comparten la caché: la medición y las pruebas son de un hilo.
- Otras escalas de W1 (solo 20 000 unidades) y otras proporciones de repetición.

## 4. Medición (E6-04 y E6-04b, veredicto final de la medición §8)

Porcentajes de la caché encendida frente a la apagada (negativo = más rápida), **mediana de rondas**, p99 salvo que se indique. Los datos son la tabla de [medición §8](etapa6-medicion.md#8-e6-04b-veredicto-final-cuatro-corridas-2026-11-11), sin cambios.

| Carga | 5 rondas, corrida 1 | 5 rondas, corrida 2 | 20 rondas, corrida A | 20 rondas, corrida B | Umbral |
|---|---|---|---|---|---|
| Sin repeticiones, 20 000 unidades (p99) | +4,0 % | −3,8 % | +2,9 % | **+5,3 %** | ≤ +5 % |
| Sin repeticiones, 100 000 unidades (p99) | **+5,3 %** | +2,6 % | −4,5 % | +1,4 % | ≤ +5 % |
| W1 `w1-index-ready`, 20 000 unidades (p99) | +0,5 % | **+7,8 %** | +4,7 % | **+7,5 %** | ≤ +5 % |
| W1 `w1-index-ready`, 20 000 unidades (p50) | −97,9 % | −97,8 % | −97,8 % | −97,9 % | ≤ −20 % |

Medición emparejada (sonda temporal, ya retirada, de [medición §8](etapa6-medicion.md#8-e6-04b-veredicto-final-cuatro-corridas-2026-11-11)): la caché se enciende y apaga cada 10 lecturas sobre la misma instantánea, y se comparan medias.

| Qué | Apagada | Encendida | Diferencia |
|---|---|---|---|
| Lectura sin repetición, 20 000 (media) | 819 – 1 213 µs | 823 – 1 216 µs | −0,6 % a +0,5 % |
| Primera lectura tras una escritura en W1 (media, la que fija el p99) | 3 588 – 3 667 µs | 3 720 – 3 834 µs | **+3,4 % a +4,9 %** (≈ 125 – 180 µs) |

### 4.1 Lectura honesta

- **p50 de W1: cumple** y no es frágil. Una lectura servida desde la caché cuesta ≈ 0,03 ms frente a ≈ 1,3 ms.
- **p99 de W1: no se puede certificar.** La lectura que fija el p99 es la primera tras cada escritura, la que la caché no puede servir (G4). Su coste real emparejado es +3,4 % a +4,9 %, por debajo de 5 %, pero la dispersión entre rondas lo lleva por encima en dos de cuatro corridas. Reducirlo exige no copiar el resultado al guardarlo, lo que cambia el tipo público `CachedResult`/`QueryOutcome` y las pruebas existentes; queda fuera de esta etapa ([medición §8](etapa6-medicion.md#8-e6-04b-veredicto-final-cuatro-corridas-2026-11-11)).
- **Peor caso sin repeticiones: el coste real cumple** en la medición emparejada (−0,6 % a +0,5 %). Pero el p99 de una corrida de rondas sí supera +5 % en dos de ocho lecturas (+5,3 % en cada escala). **Se registra como incumplimiento puntual de una corrida, no como sobrecoste real.**
- **Criterio conjunto de §6 (mejora neta): no afirmado.** Hace falta que p50 y p99 de W1 y el peor caso cumplan a la vez de forma estable, y el p99 de W1 no lo hace.

### 4.2 Límites de la medición

- Una sola máquina (4 CPU, carga de fondo ≈ 1). W1 solo a 20 000 unidades. Sin concurrencia. Sin variación del tamaño de la caché (`max_entries`, `max_bytes`).
- El comando de la corrida final es `index_bench -- --cache`; no se repitió el banco largo en esta etapa, y los números se toman del committed (`etapa6-medicion-salida.txt`).
- Ningún umbral ni parámetro de la medición se ha cambiado en E6-05: solo se han tocado documentos y comentarios.

## 5. Verificación de E6-05

| Comprobación | Comando | Resultado |
|---|---|---|
| Suite de `kerno-units` en el árbol final (tras los cambios de comentario) | `cargo test -p kerno-units --all-features --release` | `exit=0`; **320 pasadas, 1 ignorada**; cada binaria `test result: ok` |
| Formato | `cargo fmt --all --check` | `fmt_exit=0` |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile` en 20,43 s; `clippy_exit=0` |

Las pruebas de `kerno-units` estaban en verde antes de E6-05 (320 pasadas), y siguen estándolo tras los cambios de documentación y de comentario de `cache.rs`.

## 6. Decisiones abiertas para el propietario

1. **Caché desactivada por defecto** (acta §8.2). Se mantiene: la mejora neta de §6 no se afirma.
2. **H-3**: ¿la evidencia de creación viene del observador o basta con documentar que es la ejecución que produjo la respuesta?
3. **H-5**: ¿se llama a `prune_entries` también en `stats()` y `records()`, para cumplir la cota por edad en reposo? Es una línea por función y necesita una prueba.
4. **H-6**: ¿se lleva la causa de la caché a `ExecutionObservation` (cambio de `FallbackCause`) o se queda en el registro de transiciones?
5. **H-7**: ¿se acepta la prueba de I-36 en su parte de resultados y contadores, o se pide un punto de inyección para observar la lectura del índice?
6. **Qué hacer con p99 de W1**: aceptar el cierre con la caché apagada y la medición documentada, o repetir con más rondas o con la copia evitada (cambio de tipo público).

## 7. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| — | **Pendiente** | El propietario aprueba o rechaza el cierre y decide §6 |
