# Etapa 6 · E6-01 — Modelo de dependencias y almacén del estado aprendido

Contrato de E6-01 ([acta de la Etapa 6](etapa6-acta.md) §3 y decisiones de §8). El código está en `crates/kerno-units/src/cache.rs`. Garantías: [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) (G4 dependencias explícitas y versionadas, G7 registro de transiciones); privacidad: [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md). Campos de partida: [ejecución adaptativa](adaptive-execution.md), [huella y plan](etapa5-huella-y-plan.md), [observación](etapa5-observacion.md).

E6-01 **no conecta** la caché a `query()` ni al planificador: eso es E6-02. Nada del crate lee `ResultCache` salvo que un llamante lo pida. No cambia el formato persistente ni ningún otro crate.

## 1. Modelo de dependencias (G4)

Una `CacheEntry` está clavada por la `Fingerprint` de la consulta (la huella con clave de `fingerprint.rs`; no se define otro hash) y lleva un `CachedResult` (las filas tal como las devolvió la consulta) con todas las dependencias bajo las que se calculó.

| `Dependency` | Versión que compara | Cambia cuando |
|---|---|---|
| `SchemaVersion` | `LAYER_VERSION` | cambian los formatos de registro de la capa |
| `PlannerVersion` | `PLANNER_VERSION` | cambia el comportamiento del planificador |
| `IndexState` | `Ready`=0, `Building`=1, `Stale`=2, `Failed`=3 | el índice que leyó cambia de frescura |
| `IndexWatermark` | la `commit_seq` que el índice refleja | el índice se reconstruye |
| `CommitSeq` | la `commit_seq` de la instantánea | cualquier commit |

- **`CommitSeq` exige coincidencia exacta.** Una entrada nunca sirve a otra `commit_seq`, ni mayor ni menor; no hay excepción del tipo «lo que leyó no cambió» (acta §2.3).
- **Ninguna entrada existe sin todas sus dependencias.** `CacheEntry::new` recibe un `Dependencies` cuyos campos no son opcionales y rechaza lo que no podría explicarse con verdad: resultado de otro tipo de consulta que el de la huella (`KindMismatch`), estado de índice que no corresponde al tipo (una consulta de unidades lo registra, una de relaciones no, como en `Plan::new`; `IndexStatus`) y ausencia de evidencia (`NoEvidence`).
- **La comprobación va antes de cada uso.** `ResultCache::lookup(fingerprint, &Dependencies::current(...))` compara el conjunto de la entrada con el vigente (`Dependencies::first_change`) en el orden de la tabla. La primera que difiere se devuelve en `Lookup::Invalidated(dependency)` y la entrada se elimina (I-35). Un estado de índice ausente en un lado y presente en el otro cuenta como cambio de `IndexState`.
- `version_of` da el número de cada dependencia. Una consulta de relaciones no tiene índice (I-23) y un índice que nunca se construyó no tiene `watermark`: en ambos casos la versión es `None` y se compara como tal.

Resultados posibles de `lookup`: `Hit` (entrada activa, dentro de su edad y con todas las dependencias iguales), `Miss`, `Inactive` (creada y no activada), `Invalidated(dependency)` y `Disabled`. Un `Hit` no sustituye la validación del planificador (G5): eso es E6-02.

## 2. Almacén

`ResultCache` está solo en memoria y separado de los datos del usuario: no tiene manejador de base, no abre transacciones ni escribe en el motor, de modo que no puede formar parte de una transacción de usuario. Sus entradas contienen contenido de usuario (un resultado son filas), por eso nada se persiste ni se registra.

Acotado por tamaño y edad (`CacheConfig`, mismo estilo que la retención de `observation.rs`): `max_entries` (256), `max_bytes` (16 MiB, estimado con `CachedResult::approx_bytes`), `max_age` (1 h desde la creación) y `max_log_records` (1024; los registros caducan también a `max_age`). Se expulsa primero lo más antiguo. Un resultado mayor que `max_bytes` se rechaza (`TooLarge`). `set_config` valida (ningún límite puede ser 0) y aplica los límites al momento. `clear()` borra todo el estado aprendido —entradas, registro y contadores— y deja un único `Cleared { entries }`; los interruptores se conservan, como `Observer::clear` conserva el nivel.

## 3. Interruptores

- **Global**, **apagado por defecto** (acta §8.2). `set_enabled(false)` elimina todas las entradas.
- **Por consulta**: `disable_query(fingerprint)` / `enable_query`; apagar elimina la entrada de esa huella.
- Con la adaptación apagada, global o para esa consulta, ninguna entrada se lee ni se escribe: `create`, `activate` y `lookup` devuelven `Disabled` sin tocar el almacén, y `fallback` no registra. Un `lookup` con el global apagado cuesta una carga atómica. El estado se vuelve a comprobar bajo el cerrojo.

## 4. Registro de transiciones (G7)

Cada decisión sobre una optimización deja un `TransitionRecord`:

| `Transition` | Cuándo |
|---|---|
| `Created { evidence }` | `create`; la `Evidence` son `observations` (≥ 1) y la ventana que cubren. La entrada aún no sirve |
| `Activated` | `activate`; activar una entrada ya activa no registra |
| `Used` | un `lookup` sirvió la entrada |
| `Fallback { cause }` | `fallback`, con `PreconditionFailed`, `Invalidated`, `Disabled` o `Discrepancy` (esta última también elimina la entrada) |
| `Invalidated { dependency }` | `lookup` encontró que cambió esa dependencia |
| `Enabled` / `Disabled { switch }` | interruptor `Global` o `Query` |
| `Deleted { reason }` | `Requested`, `Replaced`, `EvictedBySize`, `EvictedByAge`, `Disabled`, `Discrepancy` |
| `Cleared { entries }` | `clear` |

Todo registro lleva `optimization_version` (`OPTIMIZATION_VERSION` = 1) y `planner_version` (el de la entrada; `PLANNER_VERSION` si el registro no es de una entrada). Siguiendo ADR-0015 los campos son: `id` local creciente, la huella (`None` para el interruptor global y `clear`), la transición, las dos versiones, la `commit_seq` de la entrada y `recorded_at`. No hay campo para un filtro, una unidad, una fila ni un mensaje: los errores (`CacheError`) son clases sin contenido.

## 5. Pruebas

Conteo de pruebas (suma de las líneas `test result` de `cargo test -p kerno-units --all-features --release`):

| | Pruebas |
|---|---|
| Antes de E6-01 (árbol base `666e4c3`) | 263 |
| Después de E6-01 | 285 |

Se añaden 22: 18 en `cache::tests` (`src/cache.rs`) y 4 en `tests/cache.rs` (2 pruebas × 2 motores). Antes se contó con la misma orden sobre el árbol base en un `git worktree` aparte.

| Requisito | Prueba |
|---|---|
| Una prueba por dependencia que cambia, entrada invalidada y dependencia nombrada (I-35) | `each_declared_dependency_invalidates_the_entry_and_is_named` (el `match` es exhaustivo: una dependencia nueva no compila sin su caso) |
| Nunca sirve otra `commit_seq` | `an_entry_never_serves_another_commit_seq`; con instantáneas reales y un commit y una reconstrucción reales, `a_commit_invalidates_the_cached_result_of_an_earlier_snapshot` |
| Vectores fijos del conjunto | `dependency_vectors`, `first_change_vectors` |
| Cotas de tamaño y edad | `retention_is_bounded_by_the_number_of_entries`, `..._by_bytes`, `..._by_age`, `the_log_is_bounded_by_size_and_age` |
| Apagada no guarda nada | `disabled_stores_nothing_and_reads_nothing`, `the_per_query_switch_stops_one_query_and_drops_its_entry` |
| Cada transición tiene su registro | `every_transition_leaves_a_record` |
| Ningún literal en ningún registro, en ningún nivel | `no_filter_literal_in_any_transition_record_at_any_level` (centinelas de `sentinels.rs` sembrados en datos y filtros; control de que las filas guardadas sí los contienen; los cuatro niveles de observación; comprueba que cada tipo de transición aparece, para no ser vacía) |

## 6. Mutaciones

Método de las etapas anteriores, con una salvedad: `cache.rs` es un archivo nuevo sin seguimiento en git, así que la restauración se comprobó con `cmp` contra una copia, no con `git diff`. Cada mutación es un `sed -i` sobre el código de producción (la última, sobre la prueba), se ejecutó `cargo test -p kerno-units --all-features --release --no-fail-fast --lib cache` y `... --test cache`, y se restauró el archivo. Salida real:

| Mutación | Resultado |
|---|---|
| `CommitSeq => Some(self.commit_seq.0)` → `Some(0)` | `--lib`: `FAILED` `an_entry_never_serves_another_commit_seq`, `dependency_vectors`, `each_declared_dependency_invalidates_the_entry_and_is_named`, `every_transition_leaves_a_record`, `first_change_vectors`; `13 passed; 5 failed`. `--test cache`: `emulated_log::a_commit_invalidates_...` y `kerno_db::a_commit_invalidates_...` `FAILED`; `2 passed; 2 failed` |
| `SchemaVersion => Some(1)` | `each_declared_...` y `first_change_vectors` `FAILED`; `16 passed; 2 failed` (`dependency_vectors` sigue en verde: `LAYER_VERSION` vale 1 y el vector no distingue) |
| `PlannerVersion => Some(1)` | `each_declared_...` y `first_change_vectors` `FAILED`; `16 passed; 2 failed` (mismo motivo) |
| `IndexWatermark => self.index.map(\|_\| 0)` | `dependency_vectors`, `each_declared_...`, `first_change_vectors` `FAILED`; `15 passed; 3 failed`. Es el defecto que traía el árbol al reanudar la tarea: la prueba lo detectó antes de la mutación |
| `IndexState` siempre `Ready` | `dependency_vectors`, `each_declared_...`, `first_change_vectors` `FAILED`; `15 passed; 3 failed` |
| Cota de tamaño (`while ... max_entries \|\| ... max_bytes` → `while false`) | `retention_is_bounded_by_bytes`, `retention_is_bounded_by_the_number_of_entries` `FAILED`; `16 passed; 2 failed` |
| Edad en `lookup` (`if created_at.0 < oldest_kept` → `if false`) | `retention_is_bounded_by_age` `FAILED`; `17 passed; 1 failed` |
| Edad al crear (filtro de `prune_entries` → `&& false`) | `retention_is_bounded_by_age` `FAILED`; `17 passed; 1 failed` |
| Caché encendida por defecto (`AtomicBool::new(true)`) | `disabled_stores_nothing_and_reads_nothing`, `every_transition_leaves_a_record`, `records_carry_versions_...`, `the_log_is_bounded_by_size_and_age` `FAILED`; `14 passed; 4 failed` |
| `create` ignora los dos interruptores | `disabled_stores_nothing_and_reads_nothing` `FAILED`; `17 passed; 1 failed` |
| `lookup` ignora el interruptor global | `disabled_stores_nothing_and_reads_nothing` `FAILED`; `17 passed; 1 failed` |
| `lookup` ignora el interruptor por consulta | `the_per_query_switch_stops_one_query_and_drops_its_entry` `FAILED`; `17 passed; 1 failed` |
| Se quita el registro `Used` | `--lib`: `every_transition_leaves_a_record`, `records_carry_versions_...`, `the_log_is_bounded_by_size_and_age` `FAILED`; `15 passed; 3 failed`. `--test cache`: `no_filter_literal_in_any_transition_record_at_any_level` en los dos motores `FAILED` (falta el tipo de transición); `2 passed; 2 failed` |
| El registro `Invalidated` nombra siempre `CommitSeq` | `each_declared_dependency_invalidates_the_entry_and_is_named` `FAILED`; `17 passed; 1 failed` |
| (sobre la prueba) el informe de `Lookup::Hit` incluye las filas (`format!("{r:?}")`) | `unit type sentinel "SNT-TYPE-7f3a91c2" leaked at level Off:`, en los dos motores; `2 passed; 2 failed` |

La última mutación simula una fuga que ningún código de producción actual tiene: no hay campo de `TransitionRecord` ni variante de `CacheError` que contenga contenido, así que no hay una mutación de producción pequeña que filtre un literal. Lo que demuestra es que la búsqueda de centinelas puede fallar con las filas reales en juego.

Dos mutaciones más, sobre el borrado:

| Mutación | Resultado |
|---|---|
| Se comenta `st.drop_all_entries(DeleteReason::Disabled, now);` de `set_enabled` | `disabled_stores_nothing_and_reads_nothing` y `every_transition_leaves_a_record` `FAILED`; `16 passed; 2 failed` |
| Se comenta `st.entries.clear();` de `clear` | `clear_deletes_all_state_and_keeps_the_switches` `FAILED`; `17 passed; 1 failed` |

## 7. Qué queda para E6-02

Conectar `lookup`/`create`/`activate`/`fallback` al camino de `query`, validar las filas servidas antes de devolverlas (G5), y decidir cuándo una observación justifica crear la entrada. Hasta entonces `ResultCache` no la usa nadie en el crate.
