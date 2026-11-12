# Cierre del adelanto de la Etapa 9 (E9A-06)

| Campo | Valor |
|---|---|
| Estado | **Cerrado el 2026-11-12. No queda ningún hallazgo bloqueante abierto** (§3). La promoción de I-29 para `kerno-storage` se registra como **aprobada por delegación del propietario el 2026-11-12, revocable** (§5) |
| Fecha | 2026-11-12 (E9A-06) |
| Acto | Revisión adversarial de E9A-01 a E9A-05 leyendo su código y sus pruebas, comprobación de sus criterios de §3.2 de la acta, promoción de I-29 y actualización de [invariantes](invariants.md), [threat model](../security/threat-model.md) (T-06, T-08, T-09), [DESIGN.md](../../crates/kerno-storage/DESIGN.md) y [roadmap](../roadmap/README.md) |
| Base revisada | Árbol en `76ff9a4` (fusión tras el registro de los 30 minutos de fuzzing). Este cambio solo toca `docs/` y `crates/kerno-storage/DESIGN.md`; no añade ni cambia pruebas ni código |
| Documentos | [Acta del adelanto](etapa9-adelanto-acta.md) · [cierre de la Etapa 7](etapa7-cierre.md) · [invariantes](invariants.md) · [threat model](../security/threat-model.md) · [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) · [DESIGN de `kerno-storage`](../../crates/kerno-storage/DESIGN.md) · [fuzzing (E9A-04)](../../crates/kerno-storage/fuzz/README.md) · [roadmap, Etapa 9](../roadmap/README.md#etapa-9--endurecimiento) |

Este documento describe el adelanto tal como queda en el repositorio. Cada cifra sale de un comando ejecutado para esta revisión (§4) o de un registro comprometido que se cita como tal. Lo que no se ha podido comprobar se marca en §7 como **no verificado**.

## 0. Resultado en breve

### 0.1 Criterios de cierre (acta §3.2, E9A-06)

| Criterio | Resultado | Evidencia |
|---|---|---|
| Ningún hallazgo bloqueante abierto | **Cumplido** | §3: los catorce hallazgos tienen estado; ninguno queda bloqueante abierto. H-2 es el candidato más serio y se explica en §3 |
| El informe lleva los comandos y su salida real | **Cumplido** | §2 y §4: comandos literales y líneas de salida copiadas de la ejecución |
| Suite de `kerno-storage` en verde con `--all-features --release` | **Cumplido** | `passed=672 failed=0 ignored=4` (§4.1) |
| Cada prueba de mutación de la acta falla con la mutación aplicada | **Cumplido** en las cuatro mutaciones que pide la acta (E9A-01, E9A-02, E9A-03 y E9A-05), más las dos de la corrección F-E9A-03-1; todas reproducidas en una copia (§4.3) | La de E9A-03 (CRC de bloques SST) y la de E9A-05 (checkpoint `CLEAN`) hacen fallar sus pruebas, como la de E9A-02 y la de E9A-01 |

### 0.2 Qué se promueve y qué no

- **I-29 se promueve para `kerno-storage`** (E9A-02 pasa y cubre lo que su línea de aceptación lista; §5). Queda en Fase 4 para la auditoría y los logs de capas superiores.
- **I-27, I-28 e I-30 no cambian.** Siguen en Fase 4, fuera de esta acta (acta §2.3).
- **T-06** queda *mitigado, no cerrado*. **T-08** queda medido para `kerno-storage`. **T-09** sigue *abierto*: se mide lo que sobrevive en el escenario de la prueba, sin garantía de borrado físico (ADR-0009, opción 3).

### 0.3 Qué no se ha comprobado

- Los 30 minutos por objetivo de fuzzing **no se han repetido** (se pidió no hacerlo). Se cita el registro de `fuzz/README.md` y solo se ha comprobado que el corpus comprometido reproduce sin fallos (§4.4).
- La delegación del 2026-11-12 no está registrada por escrito en el repositorio que he leído (§5.4).

## 1. Paquetes revisados

| Paquete | Qué pedía la acta | Qué se entregó (commits) | Comprobado ahora | Estado |
|---|---|---|---|---|
| E9A-01 | Medir qué sobrevive a un borrado, con cuatro predicciones fijadas antes de ejecutar (P1 a P4). Sin política ni API | `ae94bae`, `2bab299` · `tests/suppression_survival.rs` (552 líneas, observación) | Tres pruebas en verde; P1 a P4 se cumplen; mutación M-D reproducida (§2.1) | **Hecho**, con hallazgos H-4 y H-5 |
| E9A-02 | I-29: las siete variantes de `Error`, sin centinelas en ningún texto de errores, `verify()`, `inspect()` ni métricas | `a0c349c`, `2bab299` · `tests/redaction.rs` (584 líneas) | Seis pruebas en verde; mutación M-A reproducida (§2.2) | **Hecho**, con hallazgo H-6 |
| E9A-03 | Propiedades de apertura sobre WAL, SST y MANIFEST: cero pánicos, solo `Corruption` o `Incompatible`, nunca datos distintos del modelo, 2 000 casos por tipo y modo | `adaadd`, `9f41531`, `98d6b98`, `5207c32` · `tests/open_properties.rs`. Corrección de `src/` del hallazgo F-E9A-03-1: `d89c728` | Trece pruebas en verde; mutaciones M-B, M-E y M-E2 reproducidas (§2.3) | **Hecho**, con el hallazgo F-E9A-03-1 resuelto (H-1), su límite abierto (H-2) y un hallazgo de proceso (H-3) |
| E9A-04 | Fuzzing con cobertura: un objetivo por decodificador (14) y uno para la apertura, fuera del espacio de trabajo, corpus comprometido, no bloqueante en CI | `4b51114`, `53a16cb`, `43538fa`, `9a50a69` · `crates/kerno-storage/fuzz/` (15 objetivos, corpus comprometido, `README.md`) | Los 15 objetivos reproducen su corpus comprometido sin fallos; ningún paso de CI ejecuta el fuzzing. Los 30 minutos **no verificados** aquí (§7) | **Hecho** según su registro |
| E9A-05 | Estrés del ciclo de vida: 200 ciclos, cuatro escritores, cierres limpios y caídas simuladas, comparación con el modelo en cada reapertura | `153198c`, `f5fd147` · `tests/lifecycle_soak.rs` (416 líneas) | Una prueba en verde (200 ciclos); mutación M-C reproducida en el ciclo 0 (§2.5) | **Hecho** |

## 2. Revisión por paquete

Cada subsección dice qué pedía la acta, qué se entregó, qué leí en el código y las pruebas, y qué comandos ejecuté. Las líneas de salida son copias literales.

### 2.1 E9A-01: qué sobrevive a un borrado

**Pedido.** Cuatro predicciones (acta §3.2) fijadas antes de la primera ejecución, sin política ni API nueva. La prueba se etiqueta como observación.

**Entregado.** `tests/suppression_survival.rs` con tres pruebas: la configuración por defecto (`observation_of_current_behaviour_default_log`), `wal_preallocate` (`observation_of_current_behaviour_preallocated_log`) y una instantánea abierta antes del borrado (`observation_of_current_behaviour_snapshot_pins_tables`). El encabezado dice que las predicciones no cambiaron después de la primera ejecución. Comprobé con `git diff ae94bae HEAD -- crates/kerno-storage/tests/suppression_survival.rs` que el archivo es idéntico a su primera versión comprometida, aunque no puedo probar desde git que esa versión se escribió antes de ejecutar.

**Lectura.** Las tres pruebas comparan el directorio (`SimVfs::dump`) en seis o siete etapas por ejecución. Las aserciones de visibilidad (`assert_visible_state`) comparan `get` y `scan` con el modelo después de cada borrado, de cada volcado, de la compactación y de la reapertura.

**Comando y salida.**

```
$ cargo test -p kerno-storage --all-features --release --test suppression_survival -- --nocapture --test-threads=1
[survival] P1 (delete: value still in the log, no flush): HOLDS (delete value in ["000004.wal"])
[survival] P1 (delete_range: value still in the log, no flush): HOLDS (range value in ["000004.wal"])
[survival] P2 (value in no live file after flush, compact_range, clean close): HOLDS (delete value in [], range value in [])
[survival] P3 (tombstone keeps the key in a live file until compaction discards it): HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
[survival] C-2: MANIFEST names a key before compact_range: ["MANIFEST"], after: []
[survival] C-3: range fragments 1 -> 0
[survival] P1 with wal_preallocate: HOLDS (delete value in ["000004.wal"], range value in ["000004.wal"])
[survival] P4 (flushed log is zeros and holds no value): HOLDS (spare is zeros: Some(true); delete value in ["000007.sst"]; range value in ["000007.sst"])
[survival] P2 with wal_preallocate: HOLDS (delete value in [], range value in [])
[survival] P3 with wal_preallocate: HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

Con la instantánea abierta, la misma ejecución da esta línea, y en la etapa `compact_range` el valor y la clave solo están en `000007.sst`:

```
[survival]   compact_range: rewritten=4 pinned_files=3 pinned_bytes=30990 kept_tables=["000003.sst", "000005.sst", "000007.sst"] retained_bytes=31702
[survival] stage: snapshot dropped
[survival]   files: 000008.wal=96 000010.sst=17062 000011.sst=10423 LOCK=0 MANIFEST=242
```

**Mutación (acta §3.2: «P2 debe fallar»).** Reproducida en una copia del árbol (§4.3, M-D). Con las dos ediciones de la mutación (las tablas de compactación no descartan puntos cubiertos ni fragmentos), P2 sale `CONTRADICTED` con el valor de rango todavía en un archivo vivo, y las tres pruebas fallan.

**Hallazgos.** H-4 (la cláusula de conteo de C-4 no se sostiene tal como la escribe ADR-0009), H-5 (lo que no se mide).

### 2.2 E9A-02: I-29 en `kerno-storage`

**Pedido.** Producir las siete variantes de `kerno_core::Error`; ningún centinela, ni en bytes ni en su forma ASCII, en `Display` ni `Debug` de errores, `Issue`, `VerifyReport`, `inspect()` ni `metrics()`. Mutación: añadir la clave con `{:?}` al mensaje de `sst.rs:332` con la tabla ausente, y la prueba debe fallar.

**Entregado.** `tests/redaction.rs`: una prueba por configuración (por defecto, `Buffered`, `inline`, `m0`, `serial`) más la prueba del detector. La prueba de cada configuración comprueba antes que la escena produce las siete variantes (`VARIANTS`) y al menos un `Issue`, y comprueba que el propio escenario guarda los centinelas en el disco («otherwise "no leak" proves nothing»).

**Lectura.** El detector busca cada centinela en ocho formas: texto, texto en depuración, hexadecimal, hexadecimal con prefijo `0x`, escapes `\x`, escapes ASCII, lista decimal y lista hexadecimal (en mayúsculas o minúsculas, según la forma). `detector_finds_every_form_of_every_sentinel` prueba que el detector encuentra cada forma y no da falsos positivos en un texto limpio.

**Comando y salida.**

```
$ cargo test -p kerno-storage --all-features --release --test redaction
test detector_finds_every_form_of_every_sentinel ... ok
test buffered_durability_leaks_no_content ... ok
test inline_profile_leaks_no_content ... ok
test m0_profile_leaks_no_content ... ok
test default_config_leaks_no_content ... ok
test serial_profile_leaks_no_content ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.31s
```

**Mutación.** Reproducida en una copia (§4.3, M-A): la clave se añade al mensaje de `Table::open` en `src/sst.rs:335`. Ahora la línea está en el 335, no en el 332 de la acta, porque `d89c728` añadió imports arriba (H-12). Con la mutación, las cinco pruebas de configuración fallan con «N of M texts carry a sentinel (I-29)» y el detector sigue en verde.

**Hallazgo.** H-6 (el detector solo busca centinelas completos).

### 2.3 E9A-03: propiedades de apertura y F-E9A-03-1

**Pedido.** `proptest` sobre una base con tablas, WAL vivo y sin cierre limpio. Mutaciones de bits, truncamientos y reescrituras con CRC recalculado en WAL, SST y MANIFEST. Abrir en lectura-escritura y en solo lectura, y llamar a `verify()`. Cero pánicos, errores solo `Corruption` o `Incompatible`, cero datos distintos del modelo, 2 000 casos por tipo y modo.

**Entregado.** `tests/open_properties.rs`: seis propiedades (tres tipos de archivo por dos modos) con 2 000 casos cada una por defecto (`proptest_config`), semilla fija (`DEFAULT_BASE_SEED`) y `failure_persistence` desactivada. Siete pruebas de regresión y de control acompañan a las propiedades, entre ellas las dos de F-E9A-03-1.

**Lectura del código y del cambio de `src/`.** Leí `d89c728` completo. La comprobación nueva es `format::sst::check_filter_holds_index_keys`: la última clave de cada bloque de datos, que el índice ya tiene en memoria, debe pasar el filtro. La llaman `Table::open` (en `src/sst.rs`) y `verify()` (en `src/verify.rs`), no el códec `open_sst`, para no alterar el orden de reglas de v1.md §13.1. El cambio no escribe nada: en el diff no encontré ninguna escritura nueva, y su mensaje y `DESIGN.md` dicen que no cambia el formato. La comprobación cuesta un hash por bloque, no lectura de datos.

**Comando y salida.**

```
$ cargo test -p kerno-storage --all-features --release --test open_properties
test default_run_draws_the_same_case_seeds_every_time ... ok
test control_unmutated_directory_opens_to_the_final_model_in_both_modes ... ok
test known_limit_get_trusts_a_resealed_filter_missing_interior_keys ... ok
test resealed_wal_payload_rewrite_is_another_valid_commit_not_damage ... ok
test resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify ... ok
test manifest_read_write ... ok
test mutations_reach_both_outcomes_and_change_the_file ... ok
test manifest_read_only ... ok
test sst_read_only ... ok
test wal_rewrite_targets_are_the_frame_header_only ... ok
test wal_read_only ... ok
test sst_read_write ... ok
test wal_read_write ... ok
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.28s
```

**Mutaciones.** Reproducidas en una copia (§4.3):
- M-B (acta): sin la comprobación de CRC de bloques SST en `open_block`. Fallan `sst_read_write`, `sst_read_only` y `mutations_reach_both_outcomes_and_change_the_file`.
- M-E (F-E9A-03-1, apertura): sin `check_filter_holds_index_keys` en `Table::open`. Falla `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` con «a table whose filter lost every bit opened».
- M-E2 (F-E9A-03-1, `verify()`): sin la comprobación en `verify()`. Falla la misma prueba con «deep=true: one SST-V3 per table, at its filter block».

**Hallazgos.** H-1 (resuelto), H-2 (límite abierto), H-3 (proceso).

### 2.4 E9A-04: fuzzing con cobertura

**Pedido.** Un objetivo por decodificador (14) y uno para la apertura, fuera del espacio de trabajo, con corpus comprometido, no bloqueante en CI. Criterio aprobado: 30 minutos por objetivo, cero fallos, cero entradas que tarden más de 1 s, corpus y entradas que fallen comprometidos.

**Entregado.** `crates/kerno-storage/fuzz/`: 15 objetivos en `fuzz_targets/`, su propio espacio de trabajo (`[workspace]` en su `Cargo.toml`), 31 semillas de la apertura y 14 directorios de semillas de decodificadores en `corpus/`, `seed_open.py` que reconstruye `corpus/open` desde `tests/vectors/v1/dirs`, y `README.md`.

**Lectura.** El número de decodificadores lo fija `struct_vectors.rs:2367` (`assert_eq!(kinds.len(), 14)`). La lista de nombres de los objetivos es la del README; no la comparé uno a uno con los puntos de entrada de esa prueba. Comprobé que ningún workflow de CI menciona el fuzzing (`git grep -i fuzz -- .github`: sin resultados). El objetivo `open` rechaza cualquier error distinto de `Corruption` o `Incompatible`, salvo la apertura de solo lectura sin `MANIFEST`, que el README documenta como rechazo esperado (REC-13, DIR-2).

**Comando y salida (replay del corpus comprometido, sin fuzzing).** Ver §4.4. Los 15 objetivos salen con código 0.

**Registro comprometido (no reejecutado).** `README.md` tabla «Acceptance of the acta: met on 2026-11-12»: 15 objetivos, 30 minutos cada uno (1 801 segundos), cero fallos; la ejecución de cada uno se hizo en 4 CPU locales con la misma configuración del comando del README. Ejecuciones de 60 segundos del 2026-11-10 también registradas. Esta revisión **no** reejecutó los 30 minutos (instrucción del encargo).

**Hallazgos.** Ninguno propio. El límite de cobertura del fuzzing (solo los lectores de `kerno-storage`, solo lo que la cobertura alcanzó) se dice en `threat-model.md` §6.2.

### 2.5 E9A-05: estrés del ciclo de vida

**Pedido.** 200 ciclos por defecto, cuatro escritores, cero commits confirmados perdidos, `verify()` sin hallazgos y contenido igual al modelo en cada reapertura. Mutación: no escribir el checkpoint `CLEAN` en el cierre; la prueba debe fallar.

**Entregado.** `tests/lifecycle_soak.rs`: una prueba con 200 ciclos (`KERNO_SOAK_CYCLES` la cambia) y semilla fija (`DEFAULT_SEED`). Cada décimo ciclo termina en caída simulada en vez de cierre limpio. Un vigilante detiene la prueba si un ciclo se bloquea; es una guarda contra bloqueos, no un umbral de rendimiento.

**Lectura.** `verify_closed` se ejecuta sobre cada estado en disco antes de la reapertura siguiente, de modo que lo que se reabre es lo que se verificó. Compara `snap.seq() >= confirmed` (ningún commit confirmado perdido) y el contenido con el modelo. Tras cierre limpio exige `CLEAN`, cero WAL, cero huérfanos y cero archivos desconocidos; tras caída exige que haya WAL que reproducir.

**Comando y salida.**

```
$ cargo test -p kerno-storage --all-features --release --test lifecycle_soak -- --nocapture
lifecycle_soak: 200 cycles, seed 0x50ace9a5, work [("flushes", 1323), ("compactions", 2168), ("tombstones_dropped", 4615), ("range_tombstones_dropped", 5212), ("covered_points_dropped", 14868)]
test lifecycle_soak_keeps_every_confirmed_commit_and_verifies_clean ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.57s
```

**Mutación.** Reproducida en una copia (§4.3, M-C): `close_clean` llama a `checkpoint_state(..., false)`. La prueba falla en el ciclo 0 con la regla REC-9; la semilla que imprime es `0x50ace9a5`, la de la cabecera. El número del WAL que aparece en el mensaje (`000046.wal`) es distinto del que cita la cabecera (`000040.wal`): depende de la historia de la ejecución, y la regla y el ciclo coinciden.

**Hallazgo.** H-8 (comentario de `group_commit.rs`, previo a este paquete).

## 3. Hallazgos

Estado: **resuelto** (corregido y comprobado), **abierto y bloqueante**, o **abierto y no bloqueante** (con la razón). Evidencia: *determinista* cuando un comando o una prueba lo muestra; *inferencial* cuando sale de leer el código; *no verificado* cuando no pude comprobarlo.

| # | Paquete | Hallazgo | Evidencia | Clase | Causa | Bloqueante | Estado |
|---|---|---|---|---|---|---|---|
| H-1 | E9A-03 | F-E9A-03-1, caso grueso: un filtro de SST a cero o reescrito con CRC válido se abría y `get` ocultaba claves sin error. Sin `deep`, `verify()` no lo informaba | `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` pasa en HEAD; M-E y M-E2 la hacen fallar | determinista | preexistente: el bug estaba en `src/` antes de E9A | no | **Resuelto** en `d89c728` (apertura y `verify()`) |
| H-2 | E9A-03 | Límite que queda: un filtro al que solo le faltan claves **interiores** de un bloque se abre, y `get` responde «no existe» sin error mientras `scan` las devuelve. Solo `verify()` con `deep` lo informa. Lo mismo vale para una reescritura de valores o del payload del WAL, que un CRC no distingue | `known_limit_get_trusts_a_resealed_filter_missing_interior_keys` pasa y fija el límite; cabecera de `open_properties.rs` | determinista (fijado por prueba) | preexistente: formato sin autenticación (ADR-0006 pendiente) | **no** (ver nota) | **Abierto y no bloqueante**: comprobarlo en la lectura exige leer el bloque de datos en cada «no», lo que anula el filtro; cerrarlo frente a una reescritura deliberada exige AEAD (ADR-0006, fuera de esta acta, §2.3). El motor no promete protección frente a esa reescritura (threat model §6.6, T-04 abierto). La prueba impide que el límite se olvide |
| H-3 | E9A-03 | Proceso: la corrección de `src/` de H-1 se hizo dentro del paquete E9A-03 y entró en el mismo squash que E9A-05 (`f5fd147`, #45). La acta (§3.1) pide que una corrección de `src/` sea un paquete aparte aceptado por separado. No encontré registro de esa aceptación | `git show --stat d89c728`: `src/format/sst.rs`, `src/sst.rs`, `src/verify.rs`; `git log` del squash | determinista (historia) | introducido en el trabajo de E9A-03 | no | **Abierto y no bloqueante**: el cambio tiene su prueba de regresión, dos mutaciones que la hacen fallar y documentación. Falta que el propietario lo acepte a posteriori o pida separarlo. No es un fallo técnico |
| H-4 | E9A-01 | La cláusula de conteo de C-4 (ADR-0009) no se sostiene: `pinned_files = 3` mientras solo `000007.sst` contiene el valor y la clave | Salida de §2.1: `pinned_files=3`, `kept_tables` con tres tablas; en la etapa `compact_range`, el valor y la clave solo en `000007.sst` | determinista | preexistente (la medición lo muestra; no es un defecto del motor) | no | **Abierto y no bloqueante**: ADR-0009 ya lo registra en su tabla de criterios, y la aceptación tomó solo el plazo de C-4. No cambio ADR-0009 en este cierre |
| H-5 | E9A-01 | No medido: C-5 (bloques liberados del medio), memoria y cachés, lápidas de punto que un movimiento trivial lleva al último nivel, borrado por hilo de fondo y handle de larga vida | Encabezado de `suppression_survival.rs` («Not exercised»); ADR-0009, C-5 y K-1 a K-4 | determinista (ausencia) | alcance de la observación | no | **Abierto y no bloqueante**: no hay contrato medido que esté roto; la prueba es observación; T-09 sigue abierto |
| H-6 | E9A-02 | El detector busca centinelas completos. Un fragmento parcial de una clave o valor en un mensaje no lo detectaría | `find_sentinels` busca `s.bytes` completo en cada forma; ningún escenario produce fragmentos | inferencial | diseño de la prueba | no | **Abierto y no bloqueante**: la acta (§2.2) dice que los mensajes con datos interpolados llevan nombres de archivo, reglas y números; no comprobé que ningún mensaje lleve un fragmento de clave |
| H-7 | E9A-04 | Los 30 minutos por objetivo no se han reejecutado en esta revisión | Encargo: no repetir el fuzzing de 30 minutos | no verificado | — | no | **No verificado** (se cita el registro, §2.4). El replay del corpus sí se hizo y sale sin fallos |
| H-8 | E9A-05 | Comentario de `tests/group_commit.rs:538` dice «10^4 commits»; el código hace 4 × 1 500 y 4 × 1 000 (línea 548). Lo señaló la acta en §2.4(3) | `git grep` en `group_commit.rs` | determinista | preexistente | no | **Abierto y no bloqueante**: la acta lo asignó a E9A-05 solo si ese paquete tocaba el archivo, y no lo tocó; este cierre no cambia pruebas. Corrección: un cambio de comentario en un paquete propio |
| H-9 | Doc (DESIGN.md) | La frase «el borrado tampoco sobrescribe» (sección «Qué puede sobrevivir») vale para la configuración por defecto. Con `wal_preallocate` el WAL volcado se sobrescribe con ceros, como dice la sección de WAL preasignado de la misma guía y mide P4 | `DESIGN.md` (sección de WAL preasignado); salida de P4 | determinista (lectura) | preexistente | no | **Abierto y no bloqueante**: lo que DESIGN declara del WAL no cambió en este cierre, así que no lo reescribo; precisarlo es un cambio posterior |
| H-10 | Doc (DESIGN.md) | DESIGN.md decía «ADR-0009 pendiente» y «queda para la Fase 4» desde que ADR-0009 se aceptó (2026-11-12) | `grep -n ADR-0009` antes del cambio | determinista | preexistente desde E9A-00b | no | **Resuelto** en este cierre (DESIGN.md, dos frases) |
| H-11 | Doc (threat model) | La frase «fuzzing del formato (Fase 6)» de T-06 y «sigue pendiente» de §6.2 quedaban desfasadas | Lectura de `threat-model.md` | determinista | preexistente desde E9A-04 | no | **Resuelto** en este cierre (T-06, §6.2, §6.7) |
| H-12 | Doc (acta) | La acta cita la mutación en `sst.rs:332`; el mensaje está en `sst.rs:335` desde `d89c728` | `sed -n 335p crates/kerno-storage/src/sst.rs` muestra el mensaje «referenced by the MANIFEST but missing»; `grep -n "sst.rs:332" docs/architecture/etapa9-adelanto-acta.md` da las líneas 38 y 103 | determinista | documental (imports añadidos por el cambio de H-1) | no | **Abierto y no bloqueante**: la mutación M-A se reprodujo en la línea 335, pero la cita de la acta sigue en 332 y este cierre no cambia el texto aprobado de la acta. Corregirla es un cambio posterior de documentación |
| H-13 | Acta | K-E3-7 (lecturas de `kerno-units` que comprueban menos que `verify_units`) no se ha revisado | Acta §2.3 y §7, punto 4: fuera de este adelanto, recomendado para la Etapa 9 después de E7-06 | — | decisión de alcance | no | **Abierto y no bloqueante**: deliberadamente fuera de E9A |
| H-14 | Acta | I-27, I-28 e I-30 siguen sin verificar | Acta §2.3 y §4 | — | fuera de alcance (Fase 4) | no | **Abierto y no bloqueante**: el adelanto no las incluye |

**Ningún hallazgo bloqueante queda abierto.** H-2 es el candidato más serio: un filtro reescrito con CRC válido puede hacer que `get` no encuentre claves que la tabla contiene, sin error. No lo clasifico como bloqueante por dos razones: el motor no declara protección frente a reescrituras deliberadas (T-04 sigue abierto y así lo dice el threat model), y su solución depende de una decisión que el propietario no ha tomado (ADR-0006). **Si el propietario lo quiere tratar como bloqueante**, la consecuencia es que la Etapa 8 no escriba derivados hasta que ADR-0006 esté aceptado. Esa decisión es suya.

## 4. Comandos y salida

Todo se ejecutó en la raíz del árbol revisado (`76ff9a4`) salvo las mutaciones, que se hicieron en una copia exportada de ese mismo commit fuera del árbol de trabajo.

### 4.1 Suite completa de `kerno-storage`

```
$ cargo test -p kerno-storage --all-features --release > suite.txt 2>&1; echo "exit=$?"
exit=0
$ grep -c "^test result: ok" suite.txt
41
$ grep -cE "FAILED|panicked" suite.txt
0
$ grep "^test result" suite.txt | awk '{p+=$4; f+=$6; i+=$8} END{print "passed=" p, "failed=" f, "ignored=" i}'
passed=672 failed=0 ignored=4
```

`suite.txt` está fuera del repositorio (carpeta de trabajo de la revisión). Se ejecutó el 2026-11-12 sobre el árbol de este cierre, que no cambia código ni pruebas respecto a `76ff9a4` (`git diff --stat 76ff9a4`: solo `docs/` y `crates/kerno-storage/DESIGN.md`). El conteo coincide con la línea base registrada.

### 4.2 Pruebas de los cinco paquetes

Las salidas literales están en §2.1 a §2.5. Resumen: `suppression_survival` 3/3, `redaction` 6/6, `open_properties` 13/13, `lifecycle_soak` 1/1, todas en verde.

### 4.3 Mutaciones (copia del árbol, no el árbol de trabajo)

Se exportó el árbol de `76ff9a4` (`git archive`) a un directorio aparte, con su propio directorio de compilación. Cada mutación se aplicó con edición de archivo, se ejecutó la prueba indicada y se revirtió. La línea de base de la copia pasa (`test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.65s`, `redaction`).

| # | Mutación | Prueba | Resultado real |
|---|---|---|---|
| M-A | `src/sst.rs:335`: añadir `(key {:?})` con `meta.smallest` al mensaje «referenced by the MANIFEST but missing» (acta E9A-02) | `redaction` | `test result: FAILED. 1 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.93s`. Fallan las cinco pruebas de configuración: `inline_profile_leaks_no_content`, `m0_profile_leaks_no_content`, `buffered_durability_leaks_no_content`, `default_config_leaks_no_content`, `serial_profile_leaks_no_content`. El detector pasa |
| M-B | `src/format/block.rs:62`: `if false && crc_ctx(...) != le_u32(...)` (acta E9A-03) | `open_properties` | `test result: FAILED. 10 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.94s`. Fallan `sst_read_only`, `sst_read_write`, `mutations_reach_both_outcomes_and_change_the_file` |
| M-C | `src/db.rs:1453`: `checkpoint_state(..., true)` → `false` en `close_clean` (acta E9A-05) | `lifecycle_soak` | `seed 0x50ace9a5 cycle 0: verify() found [Issue { file: "000046.wal", offset: None, rule: "REC-9", detail: "required WAL missing (the MANIFEST says it is the oldest live log)" }]`; `test result: FAILED. 0 passed; 1 failed` |
| M-D1 | `src/compaction.rs:799`: `if false && cover.covering(...) > it.seq()` (solo esta edición) | `suppression_survival` | Las tres pruebas fallan; en la de compactación por defecto, el mensaje es `after compact_range: unexpected key zz-r00-KSENT-RNG`, una lectura que no coincide con el modelo |
| M-D | M-D1 más `src/compaction.rs`, `keep_fragments`: se conservan todos los fragmentos (acta E9A-01) | `suppression_survival` | `[survival] P2 (value in no live file after flush, compact_range, clean close): CONTRADICTED (delete value in [], range value in ["000016.sst"])`; `test result: FAILED. 0 passed; 3 failed` |
| M-E | `src/sst.rs:353`: se anula `check_filter_holds_index_keys` en `Table::open` | `open_properties` | `ReadWrite: a table whose filter lost every bit opened`; `test result: FAILED. 12 passed; 1 failed` |
| M-E2 | `src/verify.rs:340`: se anula la misma comprobación en `verify()` | `open_properties` | `assertion \`left == right\` failed: deep=true: one SST-V3 per table, at its filter block`; `test result: FAILED. 12 passed; 1 failed` |

Mutaciones de la cabecera de `open_properties.rs` que **no** reproduje: la que anula el filtro en `Table::get`, la que fija `tolerate_get = false` en el caso de filtro reescrito, y la que desactiva la comprobación dentro de `SstVerifier::observe`. Quedan **no verificadas** en esta revisión.

### 4.4 Replay del corpus comprometido (sin fuzzing)

Un replay ejecuta cada entrada del corpus una vez (`-runs=0`): no genera entradas nuevas y no escribe en el corpus comprometido. Orden de la ejecución: un objetivo por línea.

```
$ cargo +nightly fuzz run --fuzz-dir crates/kerno-storage/fuzz <objetivo> <directorio-vacio> crates/kerno-storage/fuzz/corpus/<objetivo> -- -runs=0 -max_len=65536 -timeout=1; echo "exit=$?"
varint exit=0 Done 4 runs in 0 second(s)
header exit=0 Done 27 runs in 0 second(s)
commit exit=0 Done 3 runs in 0 second(s)
frame exit=0 Done 5 runs in 0 second(s)
data_block exit=0 Done 3 runs in 0 second(s)
filter_block exit=0 Done 2 runs in 0 second(s)
rangedel_block exit=0 Done 2 runs in 0 second(s)
props_block exit=0 Done 3 runs in 0 second(s)
index_block exit=0 Done 2 runs in 0 second(s)
footer exit=0 Done 2 runs in 0 second(s)
record exit=0 Done 9 runs in 0 second(s)
wal exit=0 Done 7 runs in 0 second(s)
sst exit=0 Done 4 runs in 0 second(s)
manifest exit=0 Done 6 runs in 0 second(s)
open exit=0 Done 33 runs in 0 second(s)
```

Cada línea es el código de salida del comando de un objetivo y la última línea de su registro. Se lanzó un bucle de shell sobre los quince objetivos, con un directorio de trabajo vacío por objetivo (`<directorio-vacio>`), fuera del repositorio. El comando nombra el toolchain `nightly` y el subcomando `fuzz` de `cargo-fuzz` porque forman parte de lo que hay que ejecutar. El directorio `crates/kerno-storage/fuzz/target/` lo genera la compilación y está ignorado por git.

### 4.5 Formato

`cargo fmt --all --check` sobre el árbol de este cierre: sin salida, código de salida 0 (`fmt exit=0`, §8).

## 5. Promoción de I-29 para `kerno-storage`

### 5.1 Enunciado

I-29 (SAD §11.4): la auditoría y los logs no registran contenido completo de documentos, secretos ni tokens.

### 5.2 Qué cubre la prueba de E9A-02 frente a su línea de aceptación

| Lo que lista la línea de E9A-02 (acta §3.2) | Lo que comprueba `tests/redaction.rs` | Cubierto |
|---|---|---|
| Producir las siete variantes de `kerno_core::Error` (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`, `Incompatible`, `ReadOnly`); si falta una, la prueba falla | `check_case` exige que `VARIANTS` esté completo y que no aparezca ninguna variante desconocida | Sí |
| Ningún centinela, en bytes ni en forma ASCII, en `Display` ni `Debug` | `Seen::error` guarda `to_string()` y `{:?}` / `{:#?}` de cada error; `leaks()` busca los centinelas en cada texto | Sí |
| `Issue` | `seen.debug(issue)` y `seen.display(issue.detail)` | Sí |
| `VerifyReport` | `seen.outcome(&report)` con su `Debug` | Sí |
| `inspect()` | `seen.outcome(inspect(...))` | Sí |
| `metrics()` | `observe` con `db.metrics()` y `db.stats()` | Sí |
| Mutación: clave con `{:?}` en el mensaje de `sst.rs`, con la tabla ausente, la prueba debe fallar | M-A (§4.3): cinco de cinco pruebas de configuración fallan | Sí |

Las cinco configuraciones son las que recoge el encabezado de la prueba (por defecto, `Buffered`, `inline`, `m0`, `serial`). `verify()` se ejecuta sobre copias con daño de bit, truncamiento y archivo ausente, además de la versión sana.

### 5.3 Límites

- `kerno-storage` no tiene capa de logs ni de auditoría. Lo que se verifica son los errores, los informes y las métricas. La parte de SAD §11.4 que cubre logs y auditoría sigue en Fase 4 y se repite con cada capa que emita texto (R-E9A-3 de la acta; H-6 sobre el detector).
- El detector busca centinelas completos (H-6).

### 5.4 Aprobación

La tarea que dispara este cierre dice que la promoción fue **aprobada por delegación del propietario el 2026-11-12 y es revocable**. La acta (§3.2) decía «pendiente de aprobación del propietario». No encontré en el repositorio un registro escrito de esa delegación fechado el 2026-11-12: las decisiones de esa fecha que sí constan son E9A-00b, el arranque de la Etapa 8 y el cierre de la Etapa 7. Si la delegación no existe, I-29 queda como **propuesta** y el propietario la confirma o la retira en [invariantes](invariants.md).

## 6. Documentos actualizados en este cierre

| Documento | Cambio |
|---|---|
| [`invariants.md`](invariants.md) | I-29 pasa a «verificada para `kerno-storage`» con el límite de logs en Fase 4; encabezado con la promoción. I-27, I-28 e I-30 sin cambios |
| [`threat-model.md`](../security/threat-model.md) | T-06, T-08 y T-09 en §3, §6.2 y §6.7: lo medido o probado, sin ampliar. Sin cambio en §6.6 (T-04) |
| [`DESIGN.md`](../../crates/kerno-storage/DESIGN.md) | Dos frases sobre ADR-0009 (H-10). Lo demás declarado no cambió; H-9 queda como está |
| [`etapa9-adelanto-acta.md`](etapa9-adelanto-acta.md) | E9A-06 y las filas de §4 que este cierre cierra, marcadas; I-29 en §6; entrada en §10 |
| [`roadmap/README.md`](../roadmap/README.md) | Adelanto listado en la Etapa 9, con el cierre |

No se modificó ningún archivo de `src/`, `tests/`, `fuzz/` ni ningún ADR.

## 7. Riesgos abiertos y límites

1. **H-2**: el filtro reescrito con CRC válido, con las claves interiores de un bloque faltantes, sigue sin detección en `get`. Cerrarlo necesita ADR-0006.
2. **H-3**: el cambio de `src/` de H-1 no fue un paquete aparte aceptado por separado, como pide la acta. Pide decisión del propietario.
3. **H-4 y H-5**: ADR-0009 sigue con C-4 contradicho en su cláusula de conteo y con C-5 sin medir. T-09 sigue abierto.
4. **No verificado:** los 30 minutos por objetivo de fuzzing (H-7). Las mutaciones de la cabecera de `open_properties.rs` que no reproduje (§4.3).
5. **No verificado:** las cifras 357 claves y 307 ocultas de F-E9A-03-1 (threat model §6.6 y cabecera de `open_properties.rs`). No las reprodujo esta revisión, que solo comprobó el resultado de la prueba.
6. **Nombres de herramientas en el texto:** la acta (§2.2, §3.2, §7) y `fuzz/README.md` nombran herramientas de fuzzing y dependencias, y ese texto aprobado no se reescribió en este cierre. Este cierre nombra solo lo que sus comandos y archivos necesitan: `proptest`, la biblioteca de las propiedades de `tests/open_properties.rs` (§2.3), y `cargo +nightly fuzz` (comando de §4.4). No nombra modelos, asistentes ni la herramienta que produjo el trabajo.
7. **Alcance del fuzzing:** cubre los lectores de `kerno-storage` y solo las entradas que la cobertura alcanzó. Esta revisión no amplía esa cobertura.
8. **Medición de estrés:** el soak y las propiedades son de semilla fija; las ejecuciones se repiten con la misma semilla. Para explorar otras semillas, el encabezado de cada prueba dice cómo (`KERNO_SOAK_SEED`, `OPEN_PROPERTIES_BASE_SEED`).

## 8. Comprobaciones de cierre y aprobación

| Comprobación | Comando | Resultado |
|---|---|---|
| Formato | `cargo fmt --all --check; echo "fmt exit=$?"` | Sin salida; `fmt exit=0` (ejecutado el 2026-11-12, §4.5) |
| Suite de `kerno-storage` | `cargo test -p kerno-storage --all-features --release` | `passed=672 failed=0 ignored=4` (§4.1) |
| Clippy | No ejecutado: este cierre no cambia código ni pruebas | — |

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-12 | **Cierre del adelanto (E9A-06) hecho** | Ningún hallazgo bloqueante abierto (§3). I-29 promovida para `kerno-storage` por delegación del propietario, revocable (§5.4) |
