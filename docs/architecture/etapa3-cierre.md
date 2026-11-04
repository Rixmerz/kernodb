# Cierre de la Etapa 3

| Campo | Valor |
|---|---|
| Estado | **Cerrada** |
| Fecha de aprobación | 2026-11-04 |
| Acto | Aprobación del cierre de la Etapa 3 por el propietario del proyecto |
| Alcance | Paquetes E3-00 a E3-10: unidades, revisiones y procedencia, en el crate `kerno-units` |
| Documentos | [Diseño de la Etapa 3](etapa3-design.md) · [acta E3-00](etapa3-acta-e3-00.md) · [invariantes §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3) · [matriz de fallos §7](failure-matrix.md) · [informe de la Etapa 3](../testing/etapa3-report.md) · [roadmap](../roadmap/README.md#etapa-3--unidades-revisiones-y-procedencia) |

Este documento describe la etapa tal como quedó en `main` después de E3-09 (`b2ed16c`, PR #13), más las correcciones de la revisión adversarial de E3-10 (PR #14). El propietario lo aprobó el 2026-11-04.

## 0. Aprobación del propietario (2026-11-04)

El propietario aprobó el cierre de la Etapa 3 con estas decisiones:

- **Etapa 3 cerrada.**
- **IU-1…IU-10 promovidas y aprobadas** ([invariants.md §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3)).
- **I-22 e I-26 verificadas** (E3-10).
- **U2 y U6 incumplen.** Se integran con el incumplimiento documentado como riesgos abiertos **K-E3-8** y **K-E3-9** (§5). No se rebajó ningún umbral y T1–T10 no se han tocado.
- **K-E3-7 aceptado por diseño**, con la recomendación de revisarlo en la Etapa 9 junto con la CLI de verificación.

## 1. Paquetes de trabajo entregados

| WP | Entrega | PR | Estado |
|---|---|---|---|
| E3-00/E3-01 | Acta de entrada (formato v1 congelado, ADR-0013 aceptado, decisiones 1–8). ADR-0004 y ADR-0010 aceptados. `units-v1.md` con codificaciones, reglas `UNIT-n` y vectores. Crate `kerno-units` | #4 | hecho |
| E3-02 | Integridad de `REV_PAYLOAD`: longitud y BLAKE3-256 (UNIT-26…28). ADR-0008 mínimo aceptado | #5 | hecho |
| E3-02b | Codecs de todos los registros, modelo de errores, `Clock`/`UnitIdSource`, `UnitModel` y decodificador independiente de la especificación | #6 | hecho |
| E3-03 | `UnitDb`, `ChangeSet`, CRUD y archivado, límites, predicción de secuencia con envenenamiento | #7 | hecho |
| E3-04 | `history`, `get_revision`, `unit_as_of`, `state_as_of`, `change_sets_since`, `compare`, `restore` | #8 | hecho |
| E3-05 | Políticas `none`/`audit`/`versioned` por tipo, `set_policy`, `HistoryUnavailable` | #9 | hecho |
| E3-06 | `RetentionPolicy`, `run_retention` por lotes, `compact`, `RetentionReport` | #10 | hecho |
| E3-07 | `verify_units`, `verify_engine`, `verify_immutable` (UNIT-29…35), detección de daños | #11 | hecho |
| E3-08 | Barridos de caída de la capa (`changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure`, `enospc`, `torn_write`, `wal_independence`, `kill -9` real). I-20 e I-21 verificadas. Filas U1–U10 de la matriz de fallos | #12 | hecho |
| E3-09 | `unit_bench`, objetivos U1–U6, ritmo de retención, `docs/testing/etapa3-report.md` | #13 | hecho (U2 y U6 incumplen; integrado con el incumplimiento documentado, ver §4) |
| E3-10 | Revisión adversarial, promoción de IU-1…IU-10, cierre de la etapa | #14 | hecho (aprobado el 2026-11-04) |

### Criterios de aceptación del roadmap

| Criterio | Evidencia | Estado |
|---|---|---|
| Se reconstruye la evolución de una unidad | `history::model_as_of`, `policies::policy_model` (historial, as-of y `state_as_of` comparados con `UnitModel`) | cumplido |
| Se restaura sin destruir revisiones posteriores (I-22) | `history::restore_keeps_later` | cumplido; fila I-22 de `invariants.md` verificada |
| El historial no depende del WAL (I-20) | `crash::wal_independence_{deleted,recycled}_wals` | cumplido (E3-08) |
| Contenido y revisión en la misma frontera atómica, también en la matriz de fallos (I-21) | barridos de `crash.rs`, `process_kill.rs`; matriz de fallos U1–U10 | cumplido (E3-08) |
| Bancos y métricas U1–U6 frente a la referencia; T1–T10 sin tocar | `unit_bench` (3 pasadas, [informe §8–§9](../testing/etapa3-report.md)); T1–T10 antes/después en `spikes/storage/results/2026-11-04-e3/t1-t10-e3-09c/` | cumplido con incumplimientos aceptados: U1 cumple; U2 y U6 incumplen (riesgos K-E3-8 y K-E3-9); T1–T10 sin tocar y sin regresión |

## 2. Revisión adversarial (E3-10)

Método: para cada invariante IU-1…IU-10 se localizó el código que la hace cumplir y una prueba que falla si se rompe, y se aplicó una mutación del código para comprobarlo. El detalle está en [invariants.md §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3). Las mutaciones se aplicaron una a una sobre `main` y se revirtieron; ninguna queda en el árbol.

### 2.1 Defectos encontrados

| # | Defecto | Dónde | Prueba que lo reproduce | Estado |
|---|---|---|---|---|
| D-1 | **IU-2 incompleta en la ruta de lectura y escritura.** `load_unit` solo comparaba «borrado o no» entre la cabeza y su `REV_META`. Una cabeza `Archived` sobre una revisión `Create`/`Update`, o una `Active` sobre `Archive`, se aceptaba. `get_unit` devolvía un ciclo de vida que no se había confirmado, y `unarchive`/`update` actuaban sobre él. `verify_units` sí lo detectaba (UNIT-31). | `crates/kerno-units/src/db.rs`, `load_unit` (comparación de `lifecycle`) | `unit_db::corruption_detected`, caso «lifecycle» | **corregido**: se compara `lifecycle_after(kind)`, la misma regla que usa `verify` |

No se encontraron otros defectos en el código. Las demás diferencias son **huecos de cobertura** (la invariante se cumplía, pero ninguna prueba fallaba al romperla):

| # | Hueco | Corrección |
|---|---|---|
| H-1 | IU-1: solo se comprobaba en el escenario fijo de `verify.rs`. Una reescritura `Audit` que cambiara un campo además de `payload_present` sobrevivía a toda la suite. | `policy_model` llama a `verify_immutable` en cada ronda y tras cada retención |
| H-2 | IU-2: ninguna prueba dañaba los metadatos de la cabeza, y la comparación correspondiente de `load_unit` se podía quitar sin que nada fallara. | Casos «metadata» y «lifecycle» en `corruption_detected` |
| H-3 | IU-10: la única prueba era la del mensaje de un decodificador. Meter contenido en un error de la API o en un hallazgo de `verify` no lo detectaba nada. Las pruebas `sentinel_no_content`, `immutability_sweep` y `payload_hash_checked` que cita el diseño no existían con ese nombre. | `verify::sentinel_no_content` |

## 3. Decisiones de la etapa

- Decisiones 1–8 de §12 del diseño: decididas el 2026-10-31 ([acta E3-00](etapa3-acta-e3-00.md)). Se usa un crate aparte (`kerno-units`), una marca de capa sin bit `incompat`, copia completa por revisión, una sola copia de la carga útil vigente, metadatos versionados, BLAKE3-256 (ADR-0008) y `AsOf::Time` queda fuera.
- Decisión 12 de §12 (umbrales U1–U6): decidida el 2026-11-01 ([diseño, §12](etapa3-design.md#12-decisiones-del-propietario)). Sus resultados están en §4.
- ADR-0004, ADR-0008 (mínimo) y ADR-0010: aceptados.
- Ambigüedades resueltas en cada WP con la opción más conservadora (notas de E3-03…E3-07 del diseño):
  - una revisión por unidad y `ChangeSet`;
  - `audit` conserva el hash y la longitud;
  - la retención hace un `ChangeSet` `Retention` por llamada;
  - los miembros de `Retention` nombran la primera revisión retenida;
  - `verify_units` solo afirma IU-9 después del último `Maintenance`.
- **Siguen abiertas** las decisiones 9, 10, 11 y 13 de §12 del diseño. Ninguna bloquea el cierre:
  - poda de `CHANGESET` (decisión 9);
  - `unit_id` y su relación con ADR-0005 (decisión 10);
  - troceado de cargas útiles (decisión 11);
  - orden de ADR-0009 frente a la IA (decisión 13).
- Si U2 se juzga sobre `history_page` en lugar de sobre `history` sigue siendo decisión del propietario. Está ligado a K-E3-8.

## 4. Resultados de rendimiento (E3-09)

Fuente: [`docs/testing/etapa3-report.md`](../testing/etapa3-report.md) §8–§9 y `spikes/storage/results/2026-11-04-e3/` (`unit_bench-full-e3-09c-run{1,2,3}.txt`). Tres pasadas completas en máquina quieta; ningún umbral se rebajó y la metodología de `unit_bench` no cambió.

| Id | Resultado (pasadas 1 / 2 / 3) | Veredicto |
|---|---|---|
| U1 (≤ 1,5×) | 1,05× / 1,09× / 1,10× | **cumple** 3 de 3 |
| U2 sobre `history` (≤ 1,5×) | 1,50× (1,503) / 1,27× / 0,95× | **incumple** (la pasada 1 supera el límite) |
| `history_page` | 0,90× / 0,88× / 0,58× | informativo, sin veredicto |
| U3 (aviso > 5×) | 5,03× / 4,66× / 4,78× | informativo (aviso en la pasada 1) |
| U4 | +339 B de WAL por `update`; amplificación 2,67 (1 KiB) y 3,22 (64 KiB) frente a 1,10 y 1,78 de put | informativo: coste estructural de la copia completa (decisión 4) |
| U5 | 1 257,7 B por revisión frente a 1 068,0 B (1,178×, +189,7 B) | informativo |
| U6 | p99 de escritura 1,05–1,18×, rendimiento del escritor 0,89–0,94× (1,06× una vez) | **incumple** 3 de 3 |

- **U2.** `history` oscila alrededor de 1,5× por materializar 100 `Revision` y por la variación del p99 de la referencia; la vista ligera `history_page` ya no añade coste propio. Juzgar U2 sobre `history_page` cambiaría la definición de la decisión 12 y queda para el propietario.
- **U6.** Tras E3-09c la retención no hace `fsync` propio (21 800–27 700 rev/s). Lo que queda es el trabajo de fondo que la poda genera: rotaciones, vaciados y compactaciones.
- **T1–T10.** Sin tocar y sin regresión por encima del ruido entre repeticiones (informe §8.2 y §9.2).
- **Decisión del propietario (2026-11-04).** Integrar con el incumplimiento de U2 y U6 documentado, sin rebajar umbrales y sin tocar T1–T10. Ambos quedan como riesgos abiertos K-E3-8 y K-E3-9 (§5).

### 4.1 Defectos corregidos en la revisión del PR #13

- **Durabilidad de un `ChangeSet` vacío.** `ChangeSet::commit` sin operaciones devolvía `durable = (durability == Sync)`, aunque una retención diferida (decisión 12c) podía dejar la secuencia actual sin sincronizar. Ahora solo es duradero si el `durable_seq` del motor lo cubre (los motores que no informan `durable_seq` sincronizan cada commit). Con prueba nueva.
- **Desplazamientos de `history_page`.** `history_page` y `MetaFixed` guardan los desplazamientos dentro del búfer de página como `usize` en vez de `u32`, así que una página de más de 4 GiB ya no los trunca.
- **Prueba de retención y rotación.** Una retención pausada en modo `Sync` llena el memtable y lo rota, y la rotación sincroniza el WAL (un `wal_syncs`), lo que hace duraderos los conjuntos diferidos previos, como permite la decisión 12c. La prueba espera ahora exactamente una sincronización por rotación en `Sync` y ninguna en los demás modos.

## 5. Riesgos abiertos

| ID | Riesgo | Estado al cierre |
|---|---|---|
| K-E3-1 | Falla la predicción `commit_seq = read_seq + 1` | Mitigado: comprobación tras el commit con envenenamiento (`seq_mismatch_poisons_the_handle`). Sigue vigente si el motor llega a consumir secuencia por su cuenta |
| K-E3-2 | La ranura de escritura es única: un `ChangeSet` largo bloquea a todos | Abierto. Se acota con `max_changeset_ops` y con la retención por lotes. El multiescritor está fuera de alcance |
| K-E3-3 | Amplificación por escribir varios registros por revisión | Medido (U4/U5): +339 B de WAL por `update` y 1,178× de espacio por revisión. Aceptado como coste de la copia completa (decisión 4); sin defecto |
| K-E3-4 | Las instantáneas largas anulan la retención | Abierto. El informe separa «liberado lógicamente» de «reclamado». La retención por lotes con durabilidad diferida (E3-09c) va a 21 800–27 700 rev/s, pero no reclama espacio mientras haya instantáneas antiguas |
| K-E3-5 | Una capa construida sobre ADR pendientes cuesta revertirla | Mitigado por bytes opacos, `alg`/`kind` reservados y `layer_version` |
| K-E3-6 | El borrado lógico se confunde con la supresión | Abierto hasta ADR-0009 (decisión 13) |
| K-E3-7 | Las lecturas comprueban menos de lo que comprueba `verify_units`. Por ejemplo, un `REV_META` intermedio dañado solo se ve cuando una lectura pasa por él | **Aceptado por diseño** (aprobado por el propietario el 2026-11-04). Las lecturas comprueban lo que tocan y `verify_units` lo recorre todo. Tras D-1, la comprobación de lectura de la cabeza reutiliza `lifecycle_after(kind)` y no hay otra comprobación de lectura con regla propia; la evidencia es `unit_db::corruption_detected` (casos «metadata» y «lifecycle»). **Recomendación: revisarlo en la Etapa 9** junto con la CLI de verificación |
| K-E3-8 | U2 incumple: `history` llega a 1,50× (1,503) de la referencia en 1 de 3 pasadas | **Abierto.** Integrado con el incumplimiento documentado (decisión del propietario, 2026-11-04). Umbral sin cambiar. Opciones: juzgar U2 sobre `history_page` (0,58–0,90×) o abaratar la materialización de `Revision`. Llega a la Etapa 4 como riesgo heredado (ver [acta de la Etapa 4](etapa4-acta.md)) |
| K-E3-9 | U6 incumple: con retención concurrente, p99 de escritura 1,05–1,18× y rendimiento 0,89–0,94× (1,06× una vez) | **Abierto.** Integrado con el incumplimiento documentado (decisión del propietario, 2026-11-04). La causa son las rotaciones, vaciados y compactaciones de fondo que genera la poda. Se recomienda limitar su ritmo en el motor. Llega a la Etapa 4 como riesgo heredado (ver [acta de la Etapa 4](etapa4-acta.md)) |

## 6. Cierre

| # | Condición de cierre | Estado |
|---|---|---|
| 1 | Integrar E3-09 y completar §1, §4 y los riesgos K-E3-3 y K-E3-4 | hecho (PR #13) |
| 2 | Aprobación del propietario de la promoción de IU-1…IU-10 ([invariants.md §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3)) y de las filas I-22 e I-26 | **hecho el 2026-11-04** |
| 3 | Roadmap (Etapa 3 con su estado real) y SAD §8 actualizados, con K-E3-8 y K-E3-9 | hecho en el cambio que incorpora este documento |

## 7. Siguiente etapa

La Etapa 4 (relaciones e índices) empieza solo cuando el propietario apruebe su acta: [etapa4-acta.md](etapa4-acta.md). Hereda K-E3-8 y K-E3-9, que siguen abiertos.
