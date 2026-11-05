# Cierre de la Etapa 4

| Campo | Valor |
|---|---|
| Estado | **Aprobado por el propietario (2026-11-05)** |
| Fecha | 2026-11-05 (borrador de E4-05) |
| Acto | Aprobación del cierre de la Etapa 4 por el propietario del proyecto |
| Alcance | Paquetes E4-00 a E4-05: relaciones declaradas, ruta de referencia, índice primario y el índice secundario `IDX_TYPE`, en el crate `kerno-units` |
| Documentos | [Acta de la Etapa 4](etapa4-acta.md) · [workloads](etapa4-workloads.md) · [invariantes §2, §2.2, §2.3](invariants.md) · [matriz de fallos](failure-matrix.md) · [`units-v1.md`](../file-format/units-v1.md) · [ADR-0007](../adr/0007-consistencia-de-indices.md) · [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) · [roadmap](../roadmap/README.md#etapa-4--relaciones-e-índices) · [cierre de la Etapa 3](etapa3-cierre.md) |

Este documento describe la etapa tal como quedó en `main` después de E4-04 (`e2d16f6`, PR #20), más las correcciones de la revisión adversarial de E4-05. **El cierre no es efectivo hasta que el propietario lo apruebe** (§7).

## 1. Paquetes de trabajo entregados

| WP | Entrega | PR | Estado |
|---|---|---|---|
| E4-00 | Acta aprobada; ADR-0007 aceptado; I-32, I-23 e I-24 asignadas a E4-03; rechazo de archivar o borrar unidades con relaciones | #16 | hecho |
| E4-01 | Ruta de referencia sin índices sobre `UnitSnapshot` (`reference_filter`, `reference_relations*`) | #17 | hecho |
| E4-02 | Relaciones declaradas (ADR-0014, opción 2: `REL` y adyacencias `REL_OUT`/`REL_IN` en el mismo `ChangeSet`), procedencia obligatoria, UNIT-36, barridos de caída, U2 y U6 re-medidos | #18 | hecho (ADR-0014 aceptado, opción 2) |
| E4-03 | Índice primario `IDX_PRIMARY` con `IDX_META` (estado y watermark), `rebuild_index`, `indexed_scan`, `query`, UNIT-37 | #19 | hecho |
| E4-04 | Workload W1 escrito antes de medir; justifica `IDX_TYPE` (UNIT-38) | #20 | hecho |
| E4-05 | Revisión adversarial, vectores de relaciones, promoción de invariantes, este documento | #21 | hecho, aprobado |

## 2. Criterios de aceptación (acta §4)

| Criterio | Evidencia | Estado |
|---|---|---|
| Los resultados por índice coinciden con la ruta de referencia (I-32 aplicado a índices) | `tests/index.rs`: `differential_seeded` (6 semillas × 2 motores × 40 pasos; desde E4-05 el workload incluye `restore` y `run_retention`), `differential_across_scan_pages`; `check` compara `IndexPath::Primary` y `IndexPath::ByType` con `reference_filter` y con un modelo independiente; `tests/relations.rs`: `differential_*` (adyacencia frente a referencia). Mutaciones en [invariantes §2.2](invariants.md) | **cumplido** |
| Un índice reconstruido es equivalente a uno nuevo (I-24) | `tests/index.rs`: `rebuilt_equals_fresh` (entradas byte a byte, `IDX_PRIMARY` e `IDX_TYPE`) | **cumplido** |
| El estado de frescura está expuesto (I-23) | `tests/index.rs`: `states_and_watermark`, `building_is_reported_while_a_build_waits`, `snapshots_keep_their_own_status`, `failed_build_is_recorded_and_repaired`, `status_survives_reopen`, `index_every_point_{power_loss,process_kill}`; UNIT-37/38; [matriz de fallos](failure-matrix.md) §9 | **cumplido** |
| Cada índice secundario tiene workload documentado y equivalencia con la ruta de referencia; ningún índice sin workload | [etapa4-workloads.md](etapa4-workloads.md) (W1 → `IDX_TYPE`); pruebas diferenciales por `IndexPath::ByType` | **cumplido** |
| Una relación declarada nunca se confunde con una inferida (I-39) | `tests/relations.rs`: `provenance_is_stored_and_checked`, `declared_is_never_confused_with_inferred`, `stored_provenance_corruption_is_detected` (nueva en E4-05); vectores `rel_*` en `tests/struct_vectors.rs`; mutaciones en [invariantes §2.3](invariants.md) | **cumplido** (parte declarada; la inferida es de la Etapa 7) |
| Relaciones e índices en la frontera atómica (I-21, acta E4-02) | `tests/crash.rs`: `relations_every_point_{power_loss,process_kill}`; `tests/relations.rs`: `abort_leaves_no_relation`, `relation_only_changeset_commits`; filas de relaciones de la [matriz de fallos](failure-matrix.md) | **cumplido** |
| U2 y U6 re-medidos sin cambiar umbrales (acta E4-02) | ADR-0014, «Evidencia», criterio 3 | **cumplido con incumplimientos heredados** (K-E3-8 y K-E3-9 siguen abiertos; ver §5) |

Puerta de corrección: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo test --workspace` en verde al cerrar E4-05 (1 212 pruebas antes, 1 214 después).

## 3. Revisión adversarial (E4-05)

Método: el mismo que en la Etapa 3. Para cada invariante de la etapa se localizó el código que la hace cumplir y una prueba que falla si se rompe, y se aplicó una mutación para comprobarlo (tablas de [invariantes §2.2 y §2.3](invariants.md)). La revisión **no encontró bloqueantes**. Encontró estos huecos, corregidos en E4-05:

| # | Hueco | Corrección |
|---|---|---|
| H-1 | El decodificador independiente (`tests/spec_decoder`) trataba `REL` (`0x40`) y `REL_OUT`/`REL_IN` (`0x41`/`0x42`) como UNIT-2, y no había vectores comprometidos de relaciones: el formato de E4-02 no tenía la doble implementación que sí tienen los demás registros | `spec_decoder` decodifica los tres según `units-v1.md` §2, §3.7 y §3.8; 10 vectores positivos y 25 negativos `rel_*` en `struct_vectors.rs` (UNIT-3, 6, 9, 10, 11, 12, 13, 23), comprobados contra el decodificador de producción, también por truncamiento y por bytes invertidos |
| H-2 | `units-v1.md` §6 no decía que la coherencia de la procedencia de `REL` (UNIT-13) se comprueba tras UNIT-10, como hace `codec.rs`; tampoco que un `REL_OUT`/`REL_IN` sin el separador NUL es UNIT-3 | §6, punto 2, y reglas de clave de §2 |
| H-3 | I-39: ninguna prueba dañaba la procedencia ya guardada y la leía por la API | `relations::stored_provenance_corruption_is_detected` (`get_relation`, `relations_of`, `reference_relations` y `verify_units` rechazan con UNIT-13) |
| H-4 | El workload diferencial de índices no tenía `restore` (que cambia el `schema_version` de la cabeza) ni `run_retention` (un commit que no cambia cabezas) | `random_step` de `tests/index.rs` los incluye, con el modelo de revisiones retenidas |
| H-5 | I-21 (relaciones), I-23, I-24, I-32 e I-39 sin la promoción formal ni las pruebas de E4-02/E4-04 citadas | Filas actualizadas en `invariants.md` §2; tabla de mutaciones §2.3 |

## 4. Defectos menores conocidos y aceptados

Ninguno afecta a la corrección de los resultados entregados; quedan registrados para que no se pierdan.

| # | Asunto | Por qué se acepta |
|---|---|---|
| M-1 | `created_seq` de un `REL` no se contrasta con el `CHANGESET` de esa secuencia (UNIT-36 solo exige que no sea posterior a la instantánea) | La relación no tiene revisiones; un `created_seq` falso no cambia ninguna respuesta. Revisar con la CLI de verificación (Etapa 9) |
| M-2 | Un `IDX_META` con watermark posterior a la instantánea dispara también los demás hallazgos de UNIT-37 en cascada | Es corrupción y se informa; solo sobra ruido en el informe |
| M-3 | `active_entries` (construcción del índice) lee solo las cabezas | Por diseño (invariantes §2.2, «Limitación conocida») |
| M-4 | Los índices construidos por E4-03 no llevan `IDX_TYPE`, y `IDX_META` no registra la generación | El formato de la Etapa 4 no se ha publicado; E4-03 y E4-04 son un solo formato (`units-v1.md` §3.11). Si se publica, `IDX_META` necesitará una versión de generación |
| M-5 | Una entrada `IDX_TYPE` ausente no se ve al consultar (solo `verify_units`, UNIT-38) | Igual que M-3: una consulta verifica lo que devuelve, no lo que descarta |
| M-6 | El `Ord` de `Key` reserva memoria al comparar | Coste, no corrección |
| M-7 | La consulta solo por tipo tiene p95 de 17 ms con 100 000 unidades (W1), por encima de los 10 ms del criterio de W1 | La cota ideal (abrir los 999 resultados) ya cuesta 12 ms; ningún índice baja de ahí ([etapa4-workloads.md](etapa4-workloads.md)) |
| M-8 | El documento del workload W1 se integró en el mismo cambio que la implementación de `IDX_TYPE` | Los umbrales de W1 se fijaron antes de medir y las mediciones «antes» están comprometidas por separado; queda dicho para la trazabilidad |
| M-9 | **No hay salida cruda del banco de E4-02 comprometida** (`relation_bench`, `unit_bench --relations`): las cifras solo están en ADR-0014, «Evidencia». **U6 (K-E3-9) no se re-midió con índices activos**: E4-03 y E4-04 no ejecutaron `unit_bench`; el índice es derivado y ningún `ChangeSet` de aplicación lo escribe, pero la medición que pedía el acta (§5) no existe | Aceptado como hueco de evidencia, no de corrección. Se recomienda re-medir U6 con índices y relaciones y comprometer la salida antes o al inicio de la Etapa 5 |
| M-10 | `rebuild_index` concurrente: un intento que falla puede escribir `Failed` sobre un `Ready` que otro intento acababa de escribir | Conservador: el índice se declara no utilizable cuando lo era, nunca al revés (I-23 se cumple); una reconstrucción lo repara |
| M-11 | Prueba intermitente `kerno::retention_every_point_paced_power_loss_sync` (anterior a la Etapa 4) | Reproducida en E4-05: 1 fallo en 6 ejecuciones de `cargo test -p kerno-units --test retention`, en la aserción final (`tests/retention.rs:570`, «no power loss took back a deferred retention; the sweep proved nothing»). Es la comprobación de que el barrido es significativo, no una pérdida de datos: en esa ejecución ningún corte de energía devolvió una retención diferida, lo que depende del momento de las rotaciones y vaciados de fondo de `KernoDb`. No hay una corrección pequeña y claramente correcta; queda registrada |

## 5. Riesgos

| ID | Riesgo | Estado al cierre |
|---|---|---|
| K-E3-8 | U2 incumple en `history` | **Abierto**, sin cambios: re-medido en E4-02 en el mismo rango (ADR-0014, «Evidencia»); las relaciones no están en el camino de `history` |
| K-E3-9 | U6 incumple con retención concurrente | **Abierto**: re-medido en E4-02 con relaciones (1 pasada por modo), dentro del rango documentado; **no re-medido con índices** (M-9) |
| K-E3-2 | Ranura de escritura única | Abierto; cada relación añade tres escrituras dentro de la ranura |

## 6. Decisión abierta del propietario

- **ADR-0007: si los índices se mantienen dentro de cada `ChangeSet`.** Hoy el índice primario y `IDX_TYPE` son derivados: solo `rebuild_index` los escribe y cualquier commit posterior los deja `Stale`, con `query` cayendo a la ruta de referencia. W1 midió consultas sobre un índice recién construido; **no aporta evidencia** sobre el coste de mantenerlos en cada `ChangeSet` ni sobre cuánto tiempo pasan `Stale` en una carga mixta. La decisión requiere un workload de lectura y escritura mezcladas, que no existe todavía. No bloquea el cierre.

## 7. Cierre

| # | Condición de cierre | Estado |
|---|---|---|
| 1 | Criterios de la acta §4 cumplidos, con evidencia (§2) | hecho |
| 2 | Revisión adversarial sin bloqueantes abiertos (§3) | hecho |
| 3 | Promoción de I-21 (relaciones), I-23, I-24, I-32 (índices) e I-39 (declaradas) en [invariants.md](invariants.md) | hecho, **aprobado por el propietario el 2026-11-05** |
| 4 | Roadmap, SAD §9 y acta actualizados | hecho en el cambio que incorpora este documento |
| 5 | Aprobación del propietario del cierre, de la promoción y de los menores aceptados (§4) | **pendiente** |

## 8. Siguiente etapa

La Etapa 5 (planificador y observabilidad) empieza solo cuando el propietario apruebe este cierre. Hereda K-E3-8, K-E3-9, la decisión abierta de §6 y la recomendación de M-9.
