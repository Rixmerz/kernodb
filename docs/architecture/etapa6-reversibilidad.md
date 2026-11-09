# Etapa 6 · E6-03 — Reversibilidad y desactivación

Contrato de E6-03 ([acta de la Etapa 6](etapa6-acta.md) §3 y §4). Garantía: [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) G6 (la adaptación es reversible). Invariantes: I-34 (parte de optimizaciones) e I-36. Antecedentes: el almacén y sus dependencias en [E6-01](etapa6-dependencias.md); el uso en el planificador en [E6-02](etapa6-integracion.md). Las pruebas están en `crates/kerno-units/tests/cache_reversibility.rs`; las fixtures que comparten con E6-02, en `tests/harness/cache_script.rs`.

No cambia código de producción, ni el formato persistente, ni el motor, ni ningún otro crate. E6-03 solo añade pruebas y este documento. No mide rendimiento: eso es E6-04.

## 1. Qué se afirma

El estado aprendido (la caché de resultados) es un dato derivado, en memoria, fuera de toda transacción (ADR-0016, decisión). G6 exige que activarlo, desactivarlo, invalidarlo o borrarlo no cambie **nada** de lo que el usuario confirmó. «Nada» se comprueba con tres medidas a la vez, antes y después de cada operación:

| Medida | Cómo se toma | Qué cubre |
|---|---|---|
| Bytes del motor | `raw(db)`: todos los pares clave/valor del motor en una instantánea | datos confirmados, revisiones, relaciones, índices, metadatos del índice |
| `commit_seq` | `seq(db)`: la secuencia de la instantánea del motor | que ninguna operación confirma nada, ni siquiera una transacción vacía de efecto nulo |
| Estado lógico | API pública: respuestas de las cinco consultas de unidades y de las consultas de relaciones, historial de cada unidad, `change_sets_since(0)`, `index_status` | que las consultas siguen significando lo mismo (G1, G6) |

Además, `verify_units` (E3-07) no encuentra nada después de cada operación.

## 2. Las pruebas

Cada una corre sobre los dos motores (`Emulated<LogEngine>` y `KernoDb`): 10 pruebas en total.

| Prueba | Criterio | Qué hace |
|---|---|---|
| `every_operation_on_the_cache_leaves_the_data_as_it_was` | I-34, G6 | Sobre una base con historial, relaciones e índice construido, y con la caché caliente (entradas y aciertos > 0, para que no sea vacua): activar, desactivar, desactivar y reactivar una consulta, borrar una entrada, cambiar la clave de huella, reducir los límites y `clear()`. Compara bytes, `commit_seq` y estado lógico antes y después, comprueba que la operación sí actuó sobre el estado aprendido (entradas, interruptor) y que leer después tampoco escribe |
| `invalidating_by_each_dependency_leaves_the_data_as_it_was` | I-34, G4, G6 | Planta una entrada válida y consulta con versiones actuales que difieren en **una** de las cinco dependencias (esquema, planificador, estado del índice, watermark, `commit_seq`). La entrada se elimina, queda su `Invalidated { dependencia }` y los datos no cambian. La lista de dependencias está escrita en la prueba, no tomada de `Dependency::ALL` (ver M6) |
| `a_cache_in_use_never_changes_what_a_commit_writes` | I-34, G6 | Dos bases con el mismo guion de diez rondas de lecturas y escrituras; una con la caché encendida, cuyas entradas cada escritura invalida. Tras cada ronda, y tras un `clear()` a mitad, bytes y `commit_seq` son iguales a los de la base sin caché |
| `with_the_cache_off_no_optimization_takes_part` | I-36, G6 | Con la caché por defecto (apagada) corre el guion completo: ninguna lectura sale de `ResultCache`, ninguna tiene `QuerySource::Cache` ni `fallback`, y `stats() == CacheStats::default()` —el contador de usos (`reads`, `hits`, `writes`, …) está a cero— y el registro vacío. Un `lookup` directo devuelve `Disabled` y no mueve nada |
| `a_cache_switched_off_after_use_stops_taking_part` | I-36, G6 | Tras usar la caché, apagarla (global) deja las entradas en cero, los contadores y el registro quietos durante todo el guion, respuestas iguales a las de una base que nunca la tuvo y bytes iguales. Apagar una consulta: sigue sin acertar, ni leer ni escribir, un `lookup` directo devuelve `Disabled` (no `Miss`) y no deja registro; las demás consultas siguen sirviéndose |

### La suite completa con la adaptación desactivada

La caché está apagada por defecto (acta §8.2) y el resto de la suite de `kerno-units` no la enciende, de modo que `cargo test -p kerno-units --all-features --release` **es** la suite completa con la adaptación desactivada; no hay un modo aparte que activar. Lo que la hace una afirmación y no una coincidencia es `with_the_cache_off_no_optimization_takes_part`, que fija el valor por defecto y el contador a cero. Resultado de esta etapa: 305 pruebas pasan (295 antes de E6-03 + 10 nuevas), 1 ignorada ya existente (`struct_vectors`).

## 3. Mutaciones probadas

Cada mutación se aplica al código de producción, se corre `cargo test -p kerno-units --all-features --release --test cache_reversibility`, se anota qué pruebas mueren y se restaura el archivo (`git checkout`). Todas mueren.

| ID | Mutación | Archivo | Mueren |
|---|---|---|---|
| M1 | Desactivar deja la caché consultada: `set_enabled` guarda siempre `true` | `cache.rs` | `a_cache_switched_off_after_use_stops_taking_part`, `every_operation_…` |
| M2 | Desactivar no elimina las entradas | `cache.rs` | las mismas |
| M3 | `clear()` no borra las entradas | `cache.rs` | `every_operation_…` |
| M4 | `delete()` no elimina nada | `cache.rs` | `every_operation_…` |
| M5 | `lookup` ignora el interruptor por consulta | `cache.rs` | `a_cache_switched_off_after_use_stops_taking_part` |
| M6 | `CommitSeq` fuera de las dependencias comprobadas (`Dependency::ALL`) | `cache.rs` | `invalidating_by_each_dependency_…` |
| M7 | Caché encendida por defecto | `cache.rs` | `with_the_cache_off_…`, `invalidating_by_each_dependency_…` |
| M8 | `UnitDb::result_cache()` confirma una transacción del motor (poner y borrar una clave: mueve `commit_seq`, no deja bytes) | `db.rs` | `a_cache_in_use_…`, `a_cache_switched_off_…`, `every_operation_…`, `invalidating_by_each_dependency_…` |
| M9 | `UnitDb::result_cache()` crea una unidad (mueve bytes y `commit_seq`) | `db.rs` | las mismas |
| M10 | Un `lookup` con la caché apagada cuenta una lectura | `cache.rs` | `with_the_cache_off_…` |

Notas de lo que se aprendió probando:

- **«`clear()` que sube `commit_seq`» no se puede escribir tal cual.** `ResultCache` no tiene ningún manejador del motor: el tipo ya impide que una operación del almacén escriba. Lo más cercano que sí compila es hacer que el único camino hacia el almacén, `UnitDb::result_cache()`, escriba (M8, M9); cada prueba que llama a una operación lo atraviesa.
- **Sobrevivieron cuatro mutaciones en la primera versión de las pruebas, y se corrigieron las pruebas:** M5 (la caché ya impedía crear entradas de una consulta apagada, así que el `Disabled` y el `Miss` eran indistinguibles por los contadores; ahora se llama a `lookup` directo y se exige `Disabled` sin registro), M6 (la prueba recorría `Dependency::ALL`, la misma lista que la mutación recortaba), M10 (el planificador nunca llama a `lookup` con la caché apagada, así que hay que llamarlo directo) y M8 en su forma de transacción vacía (una transacción vacía del motor no consume secuencia: equivalente; se sustituyó por poner y borrar una clave).
- **El orden importa en las pruebas.** `freeze` lee por el planificador; con la caché encendida eso la rellena. Las estadísticas de la operación se toman antes de congelar el «después», y en la prueba de dependencias el «antes» se congela con la caché apagada.

## 4. Estado de los criterios del acta (§4)

| Criterio | Evidencia en E6-03 |
|---|---|
| Invalidar, desactivar o borrar no altera datos (I-34, G6) | las tres primeras pruebas de §2, con M1 a M9 |
| Desactivación verificada (I-36, G6) | las dos últimas pruebas de §2, con M1, M5, M7, M10 y la suite completa |

El estado de la tabla del acta no se cambia aquí: la promoción de I-34 completa e I-36 y la actualización de §4 corresponden a E6-05, como con E6-01 y E6-02.

## 5. Lo que E6-03 no cubre

- **Rendimiento.** Ni el coste de la caché apagada ni la mejora neta: E6-04.
- **Persistencia del estado aprendido.** No existe: la caché vive en memoria y desaparece al cerrar la base, de modo que «borrar el estado» tras un reinicio es trivial. Si una etapa posterior lo persistiera, esta prueba tendría que ampliarse a los bytes de ese almacén.
- **Concurrencia entre el apagado y una lectura en curso.** `lookup`, `create` y `fallback` vuelven a comprobar el interruptor bajo el cerrojo (E6-01); no hay prueba de carrera específica aquí.
- **El observador de Etapa 5.** Su nivel y sus registros son independientes de la caché (G3, probado en E6-02); aquí solo se comprueba que apagar la caché no depende de él.
