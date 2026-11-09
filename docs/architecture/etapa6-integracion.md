# Etapa 6 · E6-02 — La caché de resultados como candidato del planificador

Contrato de E6-02 ([acta de la Etapa 6](etapa6-acta.md) §3 y decisiones de §8). Garantías: [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) (G1 a G7). Modelo de dependencias y almacén: [E6-01](etapa6-dependencias.md). Huella y plan: [etapa5-huella-y-plan.md](etapa5-huella-y-plan.md). El código está en `crates/kerno-units/src/planner.rs` (`answer_units`, `answer_relations`, `store`), `cache.rs` (`lookup`, `CachedQuery`, `MissCause`) y `db.rs` (`UnitDb::result_cache`).

No cambia el formato persistente, ni el motor, ni ningún otro crate. La caché sigue **apagada por defecto** (acta §8.2) y E6-02 no mide nada: la medición es E6-03/E6-04.

## 1. Recorrido de `query` y `query_relations`

```
query(filter)
  └─ answer_units(filter)
       ├─ caché apagada ........................ plan_units(filter)      (el cuerpo de antes, una carga atómica de coste)
       ├─ sin clave de huella .................. plan_units(filter)
       ├─ index_stats() falla .................. Fallback{PreconditionFailed}, plan_units(filter)
       └─ lookup(huella, consulta exacta, Dependencies::current(seq, estado del índice))
            ├─ Hit ──► check_units(filter, filas)
            │            ├─ ok ........ devuelve las filas (path = ResultCache, source = Cache)
            │            └─ falla ..... Fallback{Discrepancy} + entrada eliminada,
            │                          responde la ruta de referencia, fallback = Some(ResultCache, Invalid(violación))
            ├─ Invalidated(dep) ........ Fallback{Invalidated}, planificador
            ├─ Disabled ................ Fallback{Disabled}, planificador
            └─ Miss / Inactive ......... (ya registrado como Missed{causa}), planificador
       ── tras cualquier camino que no sirvió la caché: store(huella, consulta, dependencias, filas)
```

`query_relations` es lo mismo con `check_relations` y sin estado de índice (I-23: las dependencias llevan `None`).

- **Interruptores.** Los dos —global y por consulta— se comprueban en `lookup` (E6-01) y el global, además, antes de tocar nada en `answer_*`. Encendidos ambos, se consulta; apagado cualquiera, el camino es el anterior.
- **Dependencias actuales.** Se construyen de la instantánea que lee: su `commit_seq`, y para unidades el `IndexStatus` leído de `IDX_META` en ese mismo uso. `lookup` compara las cinco dependencias de la entrada con ellas (`first_change`), en cada uso (G4). La entrada se sirve solo si todas coinciden; si no, se elimina y se nombra la primera que difiere. Una instantánea más antigua que la entrada también la invalida —nunca se sirve una `commit_seq` ajena—, a costa de que dos instantáneas de secuencias distintas se turnen la entrada.
- **Una entrada por forma de consulta, con sus literales.** La `Fingerprint` ignora los literales (ADR-0015): `unit_type = "a"` y `unit_type = "b"` comparten huella. Una entrada que sirviera a la otra devolvería filas ajenas, así que `CacheEntry` guarda la consulta exacta (`CachedQuery`, solo en memoria, nunca en el registro) y `lookup` la sirve únicamente a una igual; a otra de la misma forma le responde `Missed { OtherQuery }` sin tocar la entrada. Consecuencia: consultas que solo difieren en literales se turnan el hueco.
- **Clave de la huella.** La caché genera la suya la primera vez que se usa (`fingerprint_of`); no es comparable con la de otro almacén ni con la del `Observer`. Si el sistema no puede dar clave, `query` corre sin caché. `set_key` (para pruebas) borra el estado aprendido, porque las huellas de otra clave no son comparables.

## 2. Garantías, una a una

| | Cómo se cumple en este paquete |
|---|---|
| G1 equivalencia | El resultado servido es el de una ejecución anterior a la misma `commit_seq` y estado de índice, con las mismas validaciones. La prueba diferencial lo contrasta con la caché apagada. |
| G2 fallback | Todo fallo (precondición, invalidación, desactivación, discrepancia) ejecuta una ruta sin estado aprendido y deja `Fallback{causa}`. Tras una discrepancia responde la **ruta de referencia**, no el planificador. |
| G3 la observación no es autoridad | `answer_*` no lee el `Observer`. Crear una entrada necesita una ejecución real cuyas filas se guardan; el recuento de evidencia lo lleva la caché (una ejecución, `Evidence { observations: 1 }`), no el observador. El nivel del observador ni activa ni desactiva ni conserva ni elimina entradas. |
| G4 dependencias | `Dependencies::current` en cada uso; un commit mueve `commit_seq` y deja inservibles las entradas anteriores. |
| G5 mismas validaciones | `check_units` / `check_relations` sobre las filas cacheadas antes de devolverlas. |
| G6 reversible | Apagar elimina todas las entradas (`Deleted{Disabled}`), vuelve a `plan_*` y no toca datos, `commit_seq` ni índices. |
| G7 registro | Cada acierto (`Used`), fallo (`Missed{NoEntry, OtherQuery, NotActive, Expired}`), almacenamiento (`Created`, `Activated`, `Deleted{Replaced}`), invalidación (`Invalidated{dependencia}`) y fallback (`Fallback{causa}`) es una `Transition` con huella y versiones; ningún campo lleva filtro, unidad ni fila. |

**Qué decide crear una entrada.** El acta (§3) dice «cuando una observación lo justifique»; el paquete lo reduce a lo mínimo que cumple G3: una consulta que no se sirvió de caché y terminó bien guarda su respuesta. No hay umbral de repetición, porque cualquiera habría necesitado leer el observador. Un resultado mayor que `max_bytes` no se guarda y la consulta no falla (la caché es consultiva).

## 3. Cambios de superficie pública

Adiciones, ninguna eliminación: `PlanPath::ResultCache` (fuera de `candidates_of`: se elige por coincidencia exacta de dependencias, no por coste, y un `Plan` nunca lo lista), `QuerySource::Cache { index }`, `PlanKind::Cache` (la observación de una ejecución servida de caché lleva ese camino), `Transition::Missed { cause }` con `MissCause`, `CachedQuery`, `UnitDb::result_cache()`, `ResultCache::fingerprint_of` / `set_key`. Cambian de firma `CacheEntry::new` (recibe además la `CachedQuery`) y `ResultCache::lookup` (recibe la consulta). Con la caché apagada la salida de `query` y `query_relations` es la de antes: mismo `QueryOutcome` / `RelationsOutcome`, byte a byte (lo comprueba `turning_the_cache_off_restores_the_reference_behaviour`).

## 4. Pruebas

Conteo de pruebas (suma de las líneas `test result` de `cargo test -p kerno-units --all-features --release`): 285 tras E6-01 → 295 tras E6-02. Las 10 nuevas son `tests/cache_planner.rs`, 5 pruebas × 2 motores (`Emulated<LogEngine>` y `KernoDb`).

| Requisito | Prueba |
|---|---|
| G1: resultados idénticos con la caché apagada, encendida e invalidada a mitad por escrituras intercaladas | `cache_on_off_and_invalidated_give_identical_results`. Tres bases idénticas, diez rondas de lecturas con una escritura entre ronda y ronda (crea, actualiza, relaciona, reconstruye el índice); una instantánea vieja se relee tras cada escritura. No es vacía: exige aciertos entre escrituras e `Invalidated{CommitSeq}` después, y que la apagada no registre nada |
| G2 y G5: un fallo de comprobación descarta la entrada y responde la referencia | `a_failed_check_discards_the_entry_and_the_reference_path_answers`. Entradas falsificadas, válidas en todas sus dependencias y con filas de otra consulta, desordenadas, archivadas (`NotMatching`, `OutOfOrder`, `NotActive`) y una de relaciones: no se devuelven, `fallback = Invalid(violación)`, `Fallback{Discrepancy}` y `Deleted{Discrepancy}` en el registro |
| G6: apagar restaura la referencia | `turning_the_cache_off_restores_the_reference_behaviour`. Apagada por defecto: salida completa igual a una base sin caché y registro vacío; encendida, sirve; apagada de nuevo, vuelve a la salida base, sin entradas, sin escribir `commit_seq` y sin `Used`; el interruptor por consulta |
| G3: las observaciones no son autoridad | `observations_never_act_as_authority`. Veinte observaciones de la consulta no crean ni sirven una entrada; en los cuatro niveles del observador, y vaciándolo, la caché se comporta igual; lo observado no mantiene viva una entrada tras una escritura |
| G7: cada uso es una transición, sin literales | `every_use_of_the_cache_is_a_transition`, incluido el fallo `OtherQuery` y la búsqueda de `doc`, `img`, `cites` en todos los registros |

## 5. Mutaciones

Método de las etapas anteriores, con un cambio: todos los archivos están ya en git, así que la restauración se comprobó con `git diff`. Cada mutación es una edición del código de producción, se ejecutó `cargo test -p kerno-units --all-features --release --test cache_planner` (10 pruebas) y se revirtió. Salida real:

| Mutación | Resultado |
|---|---|
| `check_units` no se llama en un acierto (`planner.rs`) | `a_failed_check_...` `FAILED` en los dos motores; `8 passed; 2 failed` |
| `check_relations` no se llama en un acierto | ídem; `8 passed; 2 failed` |
| `lookup` ignora los cambios de dependencia (`first_change(now).filter(\|_\| false)`) | `cache_on_off_and_invalidated_...`, `every_use_of_the_cache_...`, `observations_never_...` `FAILED` en los dos motores; `4 passed; 6 failed` |
| `lookup` ignora la igualdad de la consulta (`if !same_query && false`) | `cache_on_off_and_invalidated_...` y `every_use_of_the_cache_...` `FAILED`; `6 passed; 4 failed` |
| `answer_units` exige además `self.observer.stats().retained != 0` (el observador pasa a ser autoridad) | `observations_never_act_as_authority` `FAILED` (en esta mutación fallan las 10: con el observador en `Off` la caché deja de usarse; que no sea selectiva no importa, lo que prueba es que depender del observador se detecta) |
| Se quita el `Fallback{Discrepancy}` del registro | `a_failed_check_...` `FAILED`; `8 passed; 2 failed` |
| Se quita `store` tras la consulta de unidades | las 10 `FAILED` (ninguna entrada llega a existir) |
| Se quita `drop_all_entries` de `set_enabled` | `turning_the_cache_off_...` `FAILED` en los dos motores; `8 passed; 2 failed` |

La primera ejecución de la suite al reanudar la tarea encontró `lookup` con el `.filter(|_| false)` de la tercera fila ya puesto —una mutación que el intento anterior dejó sin revertir—: fallaban `an_entry_never_serves_another_commit_seq`, `each_declared_dependency_invalidates_the_entry_and_is_named` y `every_transition_leaves_a_record`. Se revirtió y la suite quedó en verde.

## 6. Qué queda

- E6-03: reversibilidad completa (estado lógico, bytes del motor y `commit_seq` antes/después) y suite con la adaptación apagada.
- E6-04: medición con los umbrales de §6 del acta. Un criterio de creación basado en repetición, si la medición lo pidiera, tendría que seguir sin leer el observador (G3).
- Queda abierto el coste de que dos instantáneas de secuencias distintas se turnen una entrada; la medición dirá si importa.
