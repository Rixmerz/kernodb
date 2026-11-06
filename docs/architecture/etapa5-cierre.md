# Cierre de la Etapa 5

| Campo | Valor |
|---|---|
| Estado | **Pendiente de aprobación del propietario** |
| Fecha | 2026-11-06 (E5-05) |
| Acto | Revisión adversarial y cierre de la Etapa 5 por el equipo; la aprobación la da el propietario del proyecto (§7) |
| Alcance | Paquetes E5-00 a E5-05: puertas de entrada, huella y plan explicable, planificador, `ExecutionObservation`, centinelas y cierre, en el crate `kerno-units` |
| Documentos | [Acta de la Etapa 5](etapa5-acta.md) · [cierre de la Etapa 4](etapa4-cierre.md) · [ejecución adaptativa](adaptive-execution.md) · [invariantes §2.4](invariants.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) · [roadmap](../roadmap/README.md#etapa-5--planificador-y-observabilidad) · [SAD §9](SAD.md) |

Este documento describe la etapa tal como queda en la rama tras E5-04 (`cf7bf14`), más lo que encontró la revisión de E5-05. **El cierre no es efectivo hasta que el propietario lo apruebe** (§7). La revisión no encontró bloqueantes; quedan dos decisiones del propietario (§3, H-2 y H-5).

## 1. Paquetes de trabajo entregados

| WP | Entrega | Commit | Estado |
|---|---|---|---|
| E5-00 | Acta aprobada el 2026-11-05; re-medición de U6 con índices y relaciones, con la salida cruda comprometida (M-9) | `6c4cf0f`, `29744e1`, `d456cb0` | hecho. U6 sigue incumpliendo (K-E3-9) |
| E5-01 | Huella con clave de la forma de la consulta y plan explicable (`fingerprint.rs`, `plan.rs`) | `7bdf9b8` | hecho |
| E5-02 | Planificador de los siete pasos de spec §9 sobre los caminos existentes (`planner.rs`); `query` y `query_relations` pasan por él; prueba diferencial en CI | `88a55ca` | hecho |
| E5-03 | `ExecutionObservation` con niveles `off`, `aggregate`, `fingerprint` y `diagnostic`; retención por tamaño y edad, en memoria (`observation.rs`); coste medido | `878f541` | hecho. Un umbral de coste fuera en una medición (R-E5-2) |
| E5-04 | Centinelas en todo nivel y toda salida de observabilidad, explicación y error (`tests/sentinels.rs`); revisión de T-11; ADR-0015 aceptado con esa evidencia | `cf7bf14` | hecho |
| E5-05 | Revisión adversarial (§3), promoción de I-32 (planificador) e I-37, este documento, roadmap, SAD §9 y ejecución adaptativa | este commit | hecho, pendiente de aprobación |

## 2. Criterios de aceptación (acta §4)

| Criterio | Invariante | Evidencia | Estado |
|---|---|---|---|
| El plan puede inspeccionarse | — (spec §9) | `explain_filter` y `explain_relations` (`planner.rs`) devuelven el `Plan` que `query` y `query_relations` siguen en esa instantánea. `tests/plan.rs` (10 pruebas: candidatos completos, desempate, rechazos de `Plan::new`, fallback por estado, base real). `tests/index.rs` y `tests/relations.rs` exigen que el plan sea el mismo en dos llamadas y que el camino ejecutado sea el que anuncia | **cumplido** |
| El plan elegido devuelve lo mismo que la referencia | I-32 (planificador) | `tests/index.rs`, `check`: semillas 1–6, barridos por páginas y rondas de reconstrucción, `query` frente a `reference_filter`. `tests/relations.rs`, `assert_equivalent`: `query_relations` frente a `reference_relations_of`, con y sin índice listo, y `fallback == None` en un almacén sano. Mutaciones en [invariantes §2.4](invariants.md) | **cumplido**. La validación de respuestas se prueba solo con respuestas inyectadas (H-3) |
| Fallback a la referencia registrado cuando el índice no es utilizable | I-23, I-32 | `Stale` y `Failed`: `tests/index.rs::planner_falls_back_when_the_index_is_not_ready` (con todos los filtros) y `planner::a_failing_or_wrong_index_path_falls_back_to_the_reference` (un caso por `Violation` y por `Corrupt`). `Building`: no llega a una instantánea; se prueba a nivel de plan y de manejador (H-2) | **cumplido con matiz**: `Building` solo a nivel de plan (decisión §7, punto 2) |
| Ninguna observación, métrica o explicación con literales ni contenido fuera de `diagnostic` | I-37, I-29, IU-10 | `tests/sentinels.rs`: `no_sentinel_in_any_observability_explanation_or_error_output` y `levels_below_diagnostic_carry_no_shape_text`, en `Emulated<LogEngine>` y `KernoDb` (4 pruebas) | **cumplido** (verificado en E5-04, [informe](etapa5-centinelas.md)) |
| Observar no altera datos confirmados ni la secuencia de commit | I-34 (solo observaciones) | `tests/observation.rs::observing_does_not_change_the_data`, en dos motores: respuestas, caminos, relaciones, secuencias de commit y bytes del motor iguales con `Off` y con `Diagnostic`; comprueba que la ejecución observada sí registró | **cumplido** (verificado en E5-03, [informe](etapa5-observacion.md)) |

Puerta de corrección de la etapa, en la misma rama: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` y `cargo test` en verde (§6).

## 3. Revisión adversarial (E5-05)

Método: el de las Etapas 3 y 4, aplicado a la etapa completa. Para cada invariante promovida se localizó el código que la hace cumplir y las pruebas que deben fallar si se rompe; se comprobaron las citas de pruebas de los informes de E5-02 a E5-04 (todas existen) y las cifras de los informes contra la salida cruda que ellos mismos comprometen; y se aplicaron dos mutaciones nuevas (§6). Hallazgos:

| # | Hallazgo | Evidencia | Tratamiento |
|---|---|---|---|
| H-1 | El informe de E5-03 contaba «7 de 8» mediciones de coste de observar y «una de ocho» para `diagnostic`. La tabla tiene 12 valores (3 niveles en 4 ejecuciones) y `diagnostic` se midió 4 veces | Tabla de [etapa5-observacion.md](etapa5-observacion.md) §7 | **Corregido**: 11 de 12 y una de cuatro, en §7 y §9. El dato no cambia |
| H-2 | Criterio 3 (fallback con `Building`). Una instantánea nunca ve `Building`: lo informa solo el manejador (I-23). Ninguna consulta puede producir un fallback con esa causa. `tests/index.rs` prueba que una instantánea durante una reconstrucción responde igual que la referencia, y `planner.rs` prueba la usabilidad de los candidatos con el estado inyectado. El acta pedía «fallback probado con índice `Stale`, `Building` y `Failed`» | `tests/index.rs` ~l. 606–620; `planner.rs` ~l. 519 | **No corregido: decisión del propietario** (§7, punto 2). No es bloqueante: los tres estados producen el mismo `Unusable(IndexNotReady(state))` y el mismo fallback, y la separación entre manejador y snapshot es I-23 |
| H-3 | La prueba diferencial de CI no mata la eliminación de la validación de respuestas. Con `check_units` sin comprobar el filtro, `tests/index.rs` (26 pruebas) y `tests/relations.rs` (24) siguen en verde; la matan dos pruebas unitarias de `planner.rs` | §6, mutación 1 | **Sin acción**: correcto por diseño, porque un almacén sano no produce respuestas erróneas. Registrado en [invariantes §2.4](invariants.md) |
| H-4 | [ejecución adaptativa](adaptive-execution.md) §2 describía campos que `ExecutionObservation` no tiene (`plan_id`, `plan_kind` con `learned`, `rows_examined`, `bytes_read`, `cache`, `outcome = cancelled`, causas de fallback de la Etapa 6) y no decía cuáles existen; §4 no decía cómo se probaban I-32 e I-37 | `observation.rs`: `ExecutionObservation`, `Outcome`, `PlanKind` | **Corregido**: §2 sigue el `struct` y lista lo no implementado con su motivo; §4 cita las pruebas |
| H-5 | I-34 dice «Etapa 6» en su fila, aunque su parte de observaciones se verificó en E5-03 (acta §7, punto 6) | [invariantes](invariants.md) §2, fila I-34 | **No promovida**: fuera del alcance de E5-05. Decisión del propietario (§7, punto 4) |
| H-6 | Dos mediciones fuera de umbral en una sola ejecución: planificador +14,1 % en p99 (20 000 unidades, ejecución 1) y `diagnostic` +5,9 % (misma escala y ejecución). No se reproducen en la ejecución 2 | §5, R-E5-1 y R-E5-2 | **Registradas como riesgos**; el acta (§8, punto 5) dice que no bloquean. No se re-midieron en E5-05 |
| H-7 | `FingerprintKey::generate()` lee `/dev/urandom` y no está condicionada por plataforma. Sin ese fichero, `set_level` a `fingerprint` o más devuelve el error y el nivel no cambia (comportamiento de `set_level`). No se ha comprobado fuera de Unix | `fingerprint.rs` l. 28 y 192; `observation.rs`, `set_level` | **Registrado** como M-12 |

La revisión **no encontró bloqueantes**. Las decisiones H-2 y H-5 son del propietario; ninguna impide aprobar el resto.

## 4. Defectos menores conocidos y aceptados

Ninguno cambia una respuesta de `query` ni de `query_relations`.

| # | Asunto | Por qué se acepta |
|---|---|---|
| M-1 … M-8 | Ver [cierre de la Etapa 4](etapa4-cierre.md) §4 | Sin cambios en E5 |
| M-9 | **Cerrado como hueco de evidencia (E5-00)**: U6 se re-midió con índices y relaciones, y la salida cruda está en [etapa5-u6-remedicion.md](etapa5-u6-remedicion.md). U6 sigue incumpliendo (K-E3-9, §5) | La medición se hizo; el resultado no cambia el riesgo |
| M-10 | `rebuild_index` concurrente | Sin cambios en E5 |
| M-11 | Prueba intermitente `retention_every_point_paced_power_loss_sync` | No se observó en la pasada de E5-05. Sigue abierta (§5) |
| M-12 | `FingerprintKey::generate()` depende de `/dev/urandom` (H-7) | En otras plataformas el nivel no se puede subir, y lo dice `set_level`. No se ha probado |
| M-13 | El planificador usa una prior fija (`N/2` con condiciones) y un sondeo de 64 entradas (`PROBE`). El conteo por tipo en `IDX_META` es la mejora si una carga lo pide | [etapa5-planificador.md](etapa5-planificador.md) §3, ponytail |
| M-14 | La edad de retención se comprueba solo en el frente de la cola: un reloj que retrocede puede dejar un registro viejo tras otros más nuevos hasta que la cota de tamaño lo expulse | [etapa5-observacion.md](etapa5-observacion.md) §4, ponytail |
| M-15 | Con contención, las observaciones se pierden y se cuentan (`dropped`) en vez de esperar | [etapa5-observacion.md](etapa5-observacion.md) §3 y §9 |

## 5. Riesgos

| ID | Riesgo | Estado al cierre |
|---|---|---|
| K-E3-8 | U2 incumple en `history` | **Abierto**, sin cambios. El planificador no toca `history` |
| K-E3-9 | U6 incumple con retención concurrente | **Abierto**. Re-medido con índices y relaciones en E5-00: FAIL en los dos presupuestos (p99 de escritura 1,06× y 1,18×; rendimiento 0,93× y 1,01×; lectura 1,13× y 0,22×). Las observaciones no escriben en el motor (I-34) |
| K-E3-2 | Ranura de escritura única | Abierto, sin cambios |
| M-11 | Prueba intermitente de retención | Abierto. Un fallo de esa prueba no se atribuye a un WP sin reproducirlo |
| R-E5-1 | El planificador añade coste a cada `query` (acta §5). W1, umbral ≤ 10 % sobre p99 (acta §8, punto 5) | **Registrado, no bloqueante**. Seis mediciones de p99: +0,6 %, +8,5 %, **+14,1 %** (20 000, ejecución 1), +0,3 %, +2,8 % y −6,0 %. Una fuera de umbral, no reproducida. Se re-mediría con el mismo protocolo si el propietario lo pide |
| R-E5-2 | Coste de registrar una observación (ADR-0015, criterio 4): ≤ 5 % sobre p99 (acta §8, punto 5) | **Registrado, no bloqueante**. `diagnostic` +5,9 % en 20 000, ejecución 1. Los otros 11 de 12 valores están dentro, incluidos los otros tres de `diagnostic` (+3,6 %, −2,6 %, −3,4 %). El coste real está acotado por la huella y, en `diagnostic`, por copiar el texto |
| R-E5-3 | Ataque de diccionario sobre la huella (acta §5) | **Mitigado**: clave local no exportada (ADR-0015). Residuo: quien lee la memoria del proceso lee la clave (T-11) |

## 6. Verificación

Comandos ejecutados en la rama al cerrar E5-05, con su resultado real:

```text
$ cargo fmt --all --check
(sin salida; código 0)

$ cargo clippy --workspace --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 39.80s
(código 0)

$ cargo test -p kerno-units --all-features --release
suma de las líneas «test result» de las 19 suites: 263 passed; 0 failed; 1 ignored
(lib 23, codecs 21, crash 31, fingerprint 7, history 12, index 26, observation 8,
payload_integrity 16, plan 10, policies 6, process_kill 2, reference 6, relations 24,
retention 20, sentinels 4, struct_vectors 9 (1 ignored), unit_db 26, verify 12)

$ cargo test --workspace --exclude kerno-storage --all-features --release
passed=615 failed=0 ignored=8
```

Conteo de pruebas: **615 antes (E5-04) y 615 después (E5-05)**. E5-05 no añade pruebas: no hay prueba nueva que ver fallar. `kerno-storage` no se ha vuelto a ejecutar: esta etapa no lo toca.

Mutaciones de la revisión. Cada una se aplicó al código de producción, se ejecutaron las pruebas y se restauró con `git checkout -- <archivo>`; el árbol quedó limpio antes de escribir este documento:

| Mutación | Invariante | Resultado |
|---|---|---|
| `planner.rs`, `check_units`: se quita la comprobación de `filter.matches` | I-32 | `cargo test -p kerno-units --lib planner`: **2 FAILED** (`units_are_checked_against_the_filter_state_and_order`, `a_failing_or_wrong_index_path_falls_back_to_the_reference`), 5 passed. `cargo test -p kerno-units --test index --test relations`: **26 y 24 passed** (H-3) |
| `fingerprint.rs`, `NormalizedQuery::of_filter`: el literal de tipo entra en la forma | I-37 | `cargo test -p kerno-units --test sentinels`: **4 FAILED** (`fingerprint units/blake3-keyed/v1/2591dc7d… at level Off depends on a literal`). Tras restaurar: 4 passed |

## 7. Decisiones que el propietario debe tomar

1. **Aprobar este cierre**, la promoción de I-32 (parte del planificador) e I-37 en [invariantes](invariants.md), y los menores aceptados (§4). Las dos promociones quedan pendientes hasta entonces.
2. **Criterio 3 con `Building` (H-2).** Recomendación: aceptar la redacción actual, con el matiz de que un snapshot nunca ve `Building`. La alternativa es exigir, antes de la Etapa 6, un fallback registrado a nivel de manejador, con una prueba propia.
3. **Riesgos R-E5-1 y R-E5-2.** Recomendación: aceptarlos como riesgos, tal como prevé el acta. Si el propietario quiere evidencia más firme, repetir la medición con el protocolo de E5-02 y E5-03 antes de aprobar.
4. **I-34 (parte de observaciones, H-5).** ¿Anotar ya en su fila la verificación de E5-03, o dejarla para la Etapa 6? E5-05 no la ha promovido.

## 8. Cierre

| # | Condición de cierre | Estado |
|---|---|---|
| 1 | Criterios de la acta §4 cumplidos, con evidencia (§2) | hecho, con el matiz del criterio 3 (§7, punto 2) |
| 2 | Revisión adversarial sin bloqueantes abiertos (§3) | hecho: sin bloqueantes; H-2 y H-5 quedan para el propietario |
| 3 | Promoción de I-32 (planificador) e I-37 en [invariantes](invariants.md) | hecha, **pendiente de aprobación del propietario** |
| 4 | Roadmap, SAD §9 y [ejecución adaptativa](adaptive-execution.md) actualizados | hecho en el cambio que incorpora este documento |
| 5 | Aprobación del propietario del cierre, de la promoción y de los menores aceptados (§4) | **pendiente** |

## 9. Siguiente etapa

La Etapa 6 (primera plasticidad operacional) empieza solo cuando el propietario apruebe este cierre. Hereda:

- I-32 para los planes `learned`, y I-33, I-35 y I-36, que siguen en Etapa 6. La parte de I-34 correspondiente a optimizaciones también.
- La decisión de §7, punto 4 sobre I-34.
- El `plan_kind` `learned`, `cache` y las causas de fallback de la Etapa 6 listadas en [ejecución adaptativa](adaptive-execution.md) §2.
- K-E3-8, K-E3-9, M-11, R-E5-1 y R-E5-2. Cualquier optimización aprendida se mide frente al modo desactivado con los umbrales fijados antes de medir (ADR-0012).
- Las mejoras marcadas como ponytail en M-13 y M-14, con su disparador: un conteo por tipo en `IDX_META` si la prior elige el camino lento, y recorrer la cola de retención al leer si el reloj retrocede en la práctica.
