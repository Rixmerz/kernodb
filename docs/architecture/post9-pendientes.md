# Pendientes posteriores a la Etapa 9

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Fuente | [Cierre de la Etapa 9](etapa9-cierre.md) §5, los doce puntos, en su orden |
| Qué es | La lista de los puntos abiertos del cierre con su estado actual. Se actualiza por paquete posterior; el cierre no se edita |
| Paquete que la abre | **P1** (K1). **P2** actualiza los puntos 3, 4, 5, 6, 10 y 11. **P3** cierra los tres trabajos abiertos de la revisión del ADR-0016 y el H-3 de la Etapa 6 (sección «Paquete P3»). **P4** (documentación) escribe los requisitos de 1.0, el borrador del CLA y el registro de la decisión sobre la aleatoriedad de los identificadores (secciones «Paquete P4» y «Decisión por delegación del 2026-11-13»). Los demás puntos están como los dejó el cierre: «sin cambio» quiere decir que ni P1 ni P2 los tocaron, no que se hayan revisado de nuevo |

| # | Punto del cierre | Estado | Nota |
|---|---|---|---|
| 1 | Roadmap, estado de la Fase 1 | Resuelto (2026-11-13, en el cierre) | Sin cambio |
| 2 | K-E3-7: K1, K2, K3, K4, K5 | **Parcial**: K1 cerrado en P1 (`0c2c3d5`); K2, K4 y K5 abiertos; K3 no se reproduce | Desglose siguiente |
| 3 | Mensaje de `NoValidSources` | **Resuelto en P2** (`2356ff3`) | El texto ahora nombra el dato: «no source of derived datum <id> is valid any more». Cambian `derived_stale.rs` (`Display`) y las dos pruebas que lo fijaban (`consolidation.rs`, `derived_stale.rs`). Mutación Q7 |
| 4 | E9-12, estrés de derivados (F-09) | **Resuelto en P2** (`2356ff3`) en lo que pedía el punto: el orden «bloqueo, commit» | `crates/kerno-units/tests/derived_lock_order.rs`, dos motores. Un motor envoltorio pregunta, desde dentro de su propio commit y en el hilo que confirma, si el bloqueo del registro está tomado; un bloqueo tardío da `false` (mutación Q5). Hace falta un gancho nuevo, `DerivedDb::hook_registry_locked`, solo con `test-support`: los ganchos que ya había (`hook_flush`, `hook_wait_idle`, `hook_compact_range` de `UnitDb`) actúan sobre el motor y ninguno ve el bloqueo del registro, que es de `DerivedDb`. Mutaciones Q5 y Q6. **No se prueba** que el bloqueo siga tomado durante el marcado: ninguna observación externa lo ve sin un lector que compita |
| 5 | E9-03, redacción | **Resuelto en P2** (`2356ff3`): ya hay mutación | El cierre decía que no había línea de producción que una mutación pudiera romper para filtrar contenido; sí las hay: en `verify.rs:643` el texto de un hallazgo de carga se forma con los bytes almacenados (`stored`) al alcance, y en `fingerprint.rs:92` el texto de forma de la observación se forma con el filtro de la consulta (`filter.unit_type`) al alcance. Lo que hoy entra en esos textos son claves, longitudes y nombres de condición, no contenido. Mutaciones Q8 (añade el tipo del filtro al texto de forma) y Q9 (añade los bytes almacenados al hallazgo): las dos en rojo en los dos motores. Además `redaction_units.rs` afirma ahora que corrió la ruta de carga de `verify` (`payload_findings >= 2`) y que se registró un texto de forma (`shape_texts >= 1`); mutaciones Q10 y Q11. Los fragmentos de menos de 8 bytes siguen sin buscarse |
| 6 | E9-08g | **Resuelto en P2** (`2356ff3`) | `compact()` tiene pruebas que fallan sin su comprobación de envenenamiento (`compact_refuses_a_poisoned_handle_{emulated_log,kerno_db}`, `reclaim.rs`; mutación Q1) y si su rango se ensancha (`compact_asks_for_the_revision_ranges`, `reclaim.rs`, y `compact_covers_exactly_the_revision_tags_kerno_db`, `db.rs`; mutaciones Q2 a Q4). `Emulated<LogEngine>` no implementa `Reclaim`: corre detrás del motor envoltorio de `reclaim.rs` |
| 7 | E9-14, fuzzing (H-7) | **Abierto por naturaleza** | `wal`, `manifest` y `open` llegaron solo a entre 4,5 y 6,7 millones de ejecuciones en 1 801 s por objetivo (`crates/kerno-storage/fuzz/README.md`, sección de E9-14), con la máquina no libre. Hace falta una corrida más larga de esos tres objetivos **en una máquina en reposo**. Esa corrida es aparte, todavía no existe, y su registro se añadirá aquí cuando exista |
| 8 | E9-21, ejemplo | Abierto, límite declarado | Sin cambio. `set_policy` solo recibe un `HistoryPolicy` (`None`, `Audit`, `Versioned`; `crates/kerno-units/src/types.rs:144`), sin `KeepLast(1)`. El ejemplo poda con `prune_history` (`examples/agent-memory/src/lib.rs:576`). Sus pruebas corren solo sobre `kerno_db`, por decisión delegada |
| 9 | E9-20, medición | **Abierto por naturaleza** | P1 y P2 se midieron **una vez**, con la máquina cargada (carga de 2,27 a 2,67 antes de la calidad; `etapa9-medicion.md` §1). Hace falta volver a medirlos **en reposo**, en una corrida aparte que todavía no existe. Mientras tanto, los umbrales de calidad se imprimen y no se afirman en la prueba (medición §5) |
| 10 | E9-15, mutaciones de E8 | **Resuelto en P2** (`2356ff3`): registradas | Q12 (`vector_store.rs:1230`, el vector reconstruido invertido) deja en rojo `quality_over_the_frozen_set`. Informe idéntico entre motores: Q13, una mutación **de la prueba** (una cifra del informe depende del motor). El código de `kerno-units` que recorre la medición no tiene ninguna rama que dependa del motor, así que una mutación suya no separa los dos informes; solo un motor que se aparte del contrato los separa. El candidato probado en `kerno-storage` (Q13b) no cambia la medición y lo ven las pruebas de transacciones del motor (Q13c). Ver [post9-mutaciones.md](post9-mutaciones.md), P2 |
| 11 | Prueba posiblemente intermitente (`space_is_reclaimed_after_range_deletes`) | **Resuelto en P2** (`2356ff3`): causa hallada, una suposición de tiempo de la prueba, compartida con `functional::space_is_reclaimed` | Ver la sección siguiente |
| 12 | Historia del repositorio | **Mancha aceptada** (cierre §5, punto 12) | Sin cambio: no se reescribe la historia. Los commits de `main` con trailers de autoría que las reglas del proyecto prohíben, entre ellos `c779b39`, siguen en ella |

## Punto 2, desglose

| Hallazgo | Estado | Qué lo muestra |
|---|---|---|
| **K1** | **Cerrado en P1 desde el extremo que perdió su entrada**, para relaciones con registro de conteo (capa 4). **Abierto, declarado**: relaciones de una base de antes de la capa 4 hasta que se crea o borra una relación de su `(unidad, dirección, tipo)`, y el doble daño (entrada y conteo perdidos juntos) | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) §9; [units-v4.md](../file-format/units-v4.md) §7; `crates/kerno-units/tests/rel_counts.rs`; [post9-mutaciones.md](post9-mutaciones.md) |
| K2 | Abierto, sin cambio: corregido solo para `IDX_PRIMARY` con el conteo intacto; abierto con el conteo rebajado y para `IDX_TYPE` | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) §8 |
| K3 | No se reproduce, sin cambio | `get_unit_checks_oldest_rev` |
| K4 | Abierto, menor, **sin cambio**. P1 no lo cierra | `change_set_info` sigue sin cruzar los miembros con su `REV_META` |
| K5 | Abierto, menor, **sin cambio**. P1 no lo cierra | `get_relation` sigue sin comprobar `created_seq` ni los extremos |

La revisión independiente de E9-16 sigue pendiente, y la de P1 también: el autor de P1 no es su revisor (acta §4.10).

## Efecto de P1 sobre un límite de la Etapa 7

El límite H-1 de [etapa7-cierre.md](etapa7-cierre.md) (R-2): con 64 relaciones o más, una relación creada después y sin ninguna de sus dos entradas de adyacencia no se detectaba en consulta. Con los conteos de P1 **se detecta** (`Fallback` `Corrupt` en el salto) en una base con conteos. La prueba que fijaba el límite, `completeness_is_not_a_query_time_check`, pasa a llamarse `a_relation_lost_with_both_entries_is_caught_by_its_counts` en `crates/kerno-units/tests/assoc_indexed.rs` y afirma lo contrario; el cierre de la Etapa 7 no se edita y su cita queda como el estado de entonces. El mismo límite persiste para relaciones de una base de antes de la capa 4 sin conteo.

## Punto 11, la prueba intermitente

**Qué se hizo.** Se leyó la prueba (`space_is_reclaimed_after_range_deletes`, `crates/kerno-testkit/src/harness/range.rs`) y el perfil que la falla (`M0` de `crates/kerno-storage/src/testing.rs`: `CompactionConfig::m0`, con hilos de fondo para volcar y compactar, una sola compactación a la vez y un freno de escritura que solo duerme 1 ms por escritura). El binario `shared_ranges` (release, `-p kerno-storage --all-features`) se ejecutó con `--exact m0_profile::shared_range::space_is_reclaimed_after_range_deletes`, una ejecución por proceso. «Con carga» quiere decir seis bucles de 100 ejecuciones en paralelo mientras corría a la vez `cargo test --release -p kerno-storage --all-features` entero, en una máquina de 4 CPU (carga media de 1 minuto entre 3,2 y 3,6 al terminar cada corrida).

| Prueba | Condición | Ejecuciones | Fallos |
|---|---|---|---|
| `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` | Sola, árbol base | 200 | 0 |
| la misma | Con carga, árbol base | 600 | **4** (1, 0, 0, 1, 2, 0 por bucle) |
| la misma | Sola, con el cambio | 200 | 0 |
| la misma | Con carga, con el cambio | 600 | 0 |
| `serial::shared_functional::space_is_reclaimed` (binario `shared`) | Con carga, árbol base (tres bucles de 100 junto a tres del cambio y la suite entera) | 300 | **1** |
| la misma | Con la misma carga, con el cambio | 300 | 0 |

**Los fallos.** Los cuatro de la prueba del punto 11 son la misma aserción, la del tamaño tras recargar, nunca la de claves: «cycle 7: 4843423 bytes on disk after refill, first fill used 910690 (bound 4691336, …)», «cycle 5: 3318243 … (bound 3070324, …)», «cycle 6: 4238167 … (bound 3459008, …)» y «cycle 4: 3716764 … (bound 3082648, …)». Los datos no cambian de una ejecución a otra (semilla fija, `SimVfs`, mismas escrituras); cambia cuánto han avanzado los hilos de fondo cuando la prueba mide. Incluso el primer llenado, que fija la cota, sale distinto con carga (de 505 437 a 910 690 bytes).

**Causa: una suposición de tiempo de la prueba, no un defecto del motor.** La prueba medía el directorio justo después de la última escritura de cada recarga y suponía que la compactación ya había alcanzado al escritor. Con `M0` nada lo garantiza: la escritura vuelve antes de que el fondo vuelque y compacte, y con la máquina cargada el único hilo de compactación puede quedar un ciclo entero detrás; la muestra ve entonces la basura de los ciclos anteriores que ya está en cola para irse. Lo confirma que, con el mismo motor y sin escribir nada más, esperar a que el tamaño baje de la cota basta en las 600 ejecuciones con carga: el motor sí reclama, solo que más tarde. La prueba hermana `functional::space_is_reclaimed` tiene la misma suposición y falló igual con carga (en una corrida de la suite entera, perfil `serial`: «cycle 4: 3293880 bytes on disk after refill, first fill used 497008»; y una vez en las 300 de la tabla); se corrige con ella, en el mismo sitio.

**El cambio.** `settled_size` (`crates/kerno-testkit/src/harness/mod.rs`), que usan las dos pruebas de espacio para cada recarga salvo la primera: si el tamaño supera la cota, se vuelve a medir cada 5 ms, hasta 30 s, mientras siga por encima. Un motor que no reclama se queda por encima toda la espera y falla con el mismo mensaje y el tamaño que conserva; la cota (`4 * primer llenado + 1 MiB`), el número de ciclos y la comprobación de que los puntos solo los borran los rangos no cambian. Un motor que no publica estadísticas (`Engine::stats` vacío: el motor de referencia y los motores con fugas de las pruebas propias del arnés) no tiene trabajo de fondo que esperar y se mide una sola vez, como antes, así que esas pruebas no tardan más. La espera sondea una condición porque el contrato `RangeDeleteEngine` no tiene un «esperar a reposo» (`TestHooks::wait_idle` es de `KernoDb`, no del contrato que usa el arnés).

**Mutaciones** ([post9-mutaciones.md](post9-mutaciones.md), P2): Q14 (`compaction.rs`, la compactación deja de quitar los puntos que cubre un rango) deja en rojo la aserción de claves; Q15 (`deleter.rs`, los archivos obsoletos nunca se borran) deja en rojo la de tamaño de las dos pruebas, en todos los perfiles, después de agotar la espera; Q16 (`settled_size` devuelve 0 sin medir) deja en rojo la prueba del arnés que exige que un motor que no reclama falle.

**Límites.** (1) El primer llenado se sigue midiendo una vez, sin esperar: con carga sale mayor y eso solo afloja la cota en esa ejecución, como antes. (2) Con un motor que no reclama, la prueba tarda 30 s más en fallar. (3) Que no haya fallado en 600 ejecuciones con carga no prueba que no pueda fallar: un fondo parado más de 30 s la haría fallar.

## Paquete P3: trabajo abierto del ADR-0016 y H-3

Origen: «Revisión de 2026-11-13» del [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md), punto 4, y H-3 del [cierre de la Etapa 6](etapa6-cierre.md). Mutaciones: [post9-mutaciones.md](post9-mutaciones.md), P3.

| # | Punto | Estado | Qué lo muestra |
|---|---|---|---|
| P3-1 | La explicación del plan (`Plan`, `explain_*`) no decía si se consultó la caché ni si se usó estado aprendido | **Resuelto en P3** | `Plan::cache()` (`CacheView`) y `Plan::answering_path()`; `explain_filter` y `explain_relations` lo calculan con `ResultCache::peek` sin tocar la caché. Prueba en los dos motores: `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` y `the_plan_does_not_announce_an_entry_that_fails_its_check` (`cache_planner.rs`), que comparan el plan con el `path` y el `source` de la lectura siguiente. `explain_assoc` devuelve `NotApplicable`: la consulta asociativa no consulta la caché |
| P3-2 | Evidencia de creación constante `{1, 0}` | **Resuelto en P3**, con un cambio de comportamiento | La evidencia es la del observador (`Observer::evidence_of`); sin ella no se crea la entrada, así que no se activa. Con el observador por debajo de `Fingerprint` (su valor por defecto es `Off`) la caché encendida no guarda nada. `observations_never_act_as_authority` y `relation_evidence_comes_from_the_observer` (`cache_planner.rs`) |
| P3-3 | Las entradas no llevan coste | **Resuelto en P3** | `CacheEntry::cost_ns` (nanosegundos medidos de la ejecución que sustituye), `Transition::Created { evidence, cost_ns }`, `CacheView::Hit { cost_ns }`; `NoCost` si falta. Límite: una sola medida por entrada, no un modelo de coste |
| P3-4 | `set_key` borra sin registro | **Resuelto en P3** | `Transition::KeyReplaced { entries }` en el registro nuevo. `replacing_the_key_leaves_a_record_of_the_wipe` (`cache_planner.rs`, dos motores) y `set_key_leaves_one_record_of_the_wipe_in_the_new_log` (`cache.rs`) |

**Efecto sobre otras piezas.** El banco `index_bench` sube el observador a `Fingerprint` en el brazo con caché (sin eso la caché no guardaría nada) y por tanto su medida de ese brazo incluye el coste de observar: no es comparable con la de la Etapa 6. Las pruebas `cache_admission`, `cache_forget`, `cache_reversibility` y `cache_sentinels` suben el observador por la misma razón.

## Paquete P4: requisitos de 1.0, borrador del CLA y registro de identificadores

P4 es documentación. No cambia código, pruebas, `Cargo.toml`, `Cargo.lock` ni `LICENSE`.

| Entregable | Estado |
|---|---|
| [requisitos-1.0.md](requisitos-1.0.md): auditoría de los siete requisitos de D13 | Escrita. Seis requisitos están en «Cumplido con límites declarados» y uno en «Cumplido». **1.0 no está declarado** |
| [cla-propuesta.md](../legal/cla-propuesta.md): borrador del CLA en español y en inglés | Escrito como **borrador sin vigor**. Espera la revisión de un abogado y la aprobación del propietario. Hasta entonces no se acepta ninguna contribución externa |
| Decisión sobre la aleatoriedad de los identificadores (sección siguiente) | Registrada como límite aceptado, en esta página y en las notas de [ADR-0005](../adr/0005-identificadores-publicos.md) |
| Nota en [ADR-0019](../adr/0019-licencia-y-contribuciones.md) | Una línea: el borrador existe y espera al propietario. El texto de ADR-0019 no se reescribe |
| Enlace desde la Etapa 9 de [docs/roadmap/README.md](../roadmap/README.md) | Añadido. El README raíz no se toca, y `CONTRIBUTING.md` no se crea |

## Decisión por delegación del 2026-11-13: aleatoriedad de los identificadores públicos

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Quién | Tomada por delegación expresa del propietario. Revocable por él |
| Decisión | Los identificadores públicos (`PublicId`, opción 1 de ADR-0005) leen su aleatoriedad del origen del sistema operativo a través de `std`: `/dev/urandom`. En una plataforma sin ese origen, crear un identificador público falla con un error tipado y no crea nada |
| Dependencias | Ninguna nueva. El `Cargo.toml` y el `Cargo.lock` del repositorio no han cambiado desde `52f026f` |
| Código | `crates/kerno-units/src/ids.rs` (`os_random` y `mint`). El error es `UnitError::Storage(kerno_core::Error::Io)` |
| Límite aceptado | En Windows, WASI y otras plataformas sin `/dev/urandom`, `create_public` devuelve ese error. Resolver e identificar tokens ya emitidos sigue funcionando. Una lectura con error o corta, o un token cuyos 16 bytes son iguales, también falla y no se reintenta |
| Qué lo levanta | (a) Una API de aleatoriedad en la biblioteca estándar para las plataformas que el proyecto soporta. (b) La decisión de aceptar una dependencia, `getrandom`, que `ids.rs` nombra como vía de ampliación. Esa decisión pasa por una resolución expresa y por I-31 |
| Dónde se registra | Aquí, y en la sección «Notas» de [ADR-0005](../adr/0005-identificadores-publicos.md) |

**Nota sobre la regla de dependencias.** La frase «ningún crate del motor tiene dependencias de terceros» no se cumple en este repositorio tal cual. `kerno-core` no tiene dependencias, y `CONTRIBUTING.md` lo guarda con `cargo tree`. Pero `kerno-storage` depende de `crc32c`, `parking_lot`, `quick_cache` y `crossbeam-skiplist`, y `kerno-units` de `crc32c` y `blake3`. La regla que de verdad se aplica a esta decisión es I-31 (acta §4.8): ningún `Cargo.toml` ni `Cargo.lock` cambia sin una decisión que lo pida. Por esta decisión no se añade ninguna dependencia.
