# Cierre de la Etapa 5

| Campo | Valor |
|---|---|
| Estado | **Pendiente de aprobación del propietario** |
| Fecha | 2026-11-06 (E5-05) |
| Acto | Revisión adversarial y cierre de la Etapa 5 (acta §3, fila E5-05) |
| Alcance | Paquetes E5-00 a E5-05: puertas de entrada, huella y plan explicable, planificador determinista, `ExecutionObservation`, centinelas y cierre, en el crate `kerno-units` |
| Documentos | [Acta de la Etapa 5](etapa5-acta.md) · [huella y plan](etapa5-huella-y-plan.md) · [planificador](etapa5-planificador.md) · [observación](etapa5-observacion.md) · [centinelas](etapa5-centinelas.md) · [re-medición de U6](etapa5-u6-remedicion.md) · [ejecución adaptativa](adaptive-execution.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [invariantes §2.2, §2.4](invariants.md) · [roadmap](../roadmap/README.md#etapa-5--planificador-y-observabilidad) · [cierre de la Etapa 4](etapa4-cierre.md) |

Este documento describe la etapa tal como queda en la rama tras E5-05. **El cierre no es efectivo hasta que el propietario lo apruebe** (§7). La promoción de I-32 (parte del planificador) e I-37 también queda pendiente de esa aprobación.

## 1. Paquetes de trabajo entregados

| WP | Entrega | Commit | Estado |
|---|---|---|---|
| E5-00 | Acta aprobada (2026-11-05); decisiones de §7 registradas; U6 re-medido con índices y relaciones, salida cruda comprometida (cierra M-9) | `6c4cf0f`, `29744e1`, `d456cb0` | hecho |
| E5-01 | Huella con clave de la forma de la consulta (`fingerprint.rs`) y plan explicable (`plan.rs`) | `7bdf9b8` | hecho |
| E5-02 | Planificador determinista de los siete pasos de spec §9 sobre los caminos existentes; `query` y `query_relations` lo usan; fallback registrado | `88a55ca` | hecho |
| E5-03 | `ExecutionObservation` con niveles `off`, `aggregate`, `fingerprint`, `diagnostic` y retención acotada en memoria | `878f541` | hecho |
| E5-04 | Centinelas en observaciones, explicaciones y errores, en los cuatro niveles | `cf7bf14` | hecho |
| E5-05 | Revisión adversarial (§3), criterios (§2), promoción de I-32 (planificador) e I-37, roadmap, SAD §9 y `adaptive-execution.md` | este cambio | hecho, **pendiente de aprobación** |

ADR-0015 se aceptó el 2026-11-06 con la evidencia de E5-04 (acta §8, decisión 1). ADR-0016 y ADR-0018 siguen *Propuestos*: no se usan en esta etapa.

## 2. Criterios de aceptación (acta §4)

| Criterio | Evidencia | Estado |
|---|---|---|
| El plan puede inspeccionarse | `UnitSnapshot::explain_filter` y `explain_relations` devuelven el `Plan`: candidatos con coste y utilizabilidad, elegido, motivo y estado de frescura (`tests/plan.rs`, 10 pruebas; `planner.rs`, `candidates_carry_the_usability_the_state_gives`, `costs_follow_the_documented_model`). Véase [planificador](etapa5-planificador.md) §1 y [huella y plan](etapa5-huella-y-plan.md) §2 | **cumplido** |
| El plan elegido devuelve lo mismo que la ruta de referencia (I-32, parte del planificador) | `tests/index.rs` (`check`, semillas 1–6, `differential_across_scan_pages`) compara `query` con `reference_filter` con y sin índice listo y exige que el camino ejecutado sea el que `explain_filter` anuncia y `fallback == None` en un almacén sano. `tests/relations.rs` (`assert_equivalent`) hace lo mismo con `query_relations`. Mutaciones de §2.4 | **cumplido** |
| Fallback a la ruta de referencia registrado cuando el índice no es utilizable (I-23, I-32) | `Stale` y `Failed`: `planner_falls_back_when_the_index_is_not_ready`, con todos los filtros. Respuesta incorrecta o fallo de un camino de índice: `a_failing_or_wrong_index_path_falls_back_to_the_reference` y `a_failing_or_wrong_adjacency_falls_back_to_the_reference`, con un caso por `Violation` y por `Corrupt`. `Building`: ver H-2 (§3) | **cumplido con una salvedad** (H-2) |
| Ninguna observación, métrica o explicación contiene literales ni contenido fuera de `diagnostic` (I-37, I-29, IU-10) | `tests/sentinels.rs`: `no_sentinel_in_any_observability_explanation_or_error_output` y `levels_below_diagnostic_carry_no_shape_text`, en `Emulated<LogEngine>` y `KernoDb`. Mutaciones de [centinelas](etapa5-centinelas.md) §4 y de §2.4 | **cumplido** (verificado en E5-04) |
| Observar no altera datos confirmados ni la secuencia de commit (I-34, solo observaciones) | `tests/observation.rs`: `observing_does_not_change_the_data`, con `Off` y `Diagnostic`, en dos motores; iguala respuestas, secuencias de commit y bytes del motor ([observación](etapa5-observacion.md) §5) | **cumplido** (verificado en E5-03; la parte de optimizaciones es de la Etapa 6) |

Criterio de rendimiento de E5-02 y E5-03 (acta §8, decisión 5): **no es un criterio de aceptación**, se registra como riesgo (§5).

## 3. Revisión adversarial (E5-05)

Método: el mismo que en las Etapas 3 y 4. Para cada invariante de la etapa se leyó el código que la hace cumplir, se localizó la prueba que falla si se rompe y se aplicó una mutación. Las mutaciones se hicieron sobre el código de producción del árbol de trabajo, cada una con una inversa exacta de `sed` que se comprobó con `git diff` (árbol limpio tras cada una). E5-05 no añade pruebas nuevas: las mutaciones son sobre pruebas existentes.

**Resultado: no hay bloqueantes abiertos.** Hay dos hallazgos que el propietario debe conocer (H-1 y H-2) y dos menores (H-3 y H-4).

| # | Hallazgo | Evidencia | Estado |
|---|---|---|---|
| H-1 | `check_units` y `check_relations` comprueban que la respuesta de un camino de índice sea un subconjunto correcto (lo que el filtro selecciona, solo `Active`, orden estrictamente ascendente), pero **no que sea completa**. Un camino de índice que descartara una unidad que el filtro sí selecciona devolvería una respuesta incorrecta sin fallback en tiempo de consulta. Lo detectan la prueba diferencial en CI y `verify_units` (UNIT-37), no la consulta. Es la misma limitación que ya registraba [invariantes §2.2](invariants.md) («Limitación conocida») | Lectura de `planner.rs` (`execute_units`, `check_units`, `execute_relations`, `check_relations`). Inferencial | **Aceptado como limitación conocida.** Comprobar la completitud en tiempo de consulta exige la ruta de referencia completa y anula el índice. La Etapa 6 no debe contar con esa comprobación para su equivalencia de resultados (I-32 se apoya en la prueba diferencial) |
| H-2 | El fallback con índice `Building` no se ejerce a través de `query`. Una instantánea no ve `Building` (solo `UnitDb::index_status` lo informa, [planificador](etapa5-planificador.md) §4), así que la aceptación de E5-02 («fallback probado con `Stale`, `Building` y `Failed`») se cumple en `Building` solo a nivel de candidatos (`candidates_carry_the_usability_the_state_gives`) | Lectura de `index.rs` (`UnitSnapshot::index_status`) y de `planner.rs`. Inferencial | **Abierto, menor.** Se registra como salvedad de §2. No bloquea: el comportamiento de una reconstrucción en curso lo cubren las pruebas de `tests/index.rs` (`building_is_reported_while_a_build_waits`) |
| H-3 | Un registro de observación se pierde y se cuenta (`dropped`) cuando el cerrojo está ocupado; no se espera | [observación](etapa5-observacion.md) §3 y §9 | Aceptado: es el diseño |
| H-4 | La retención por edad comprueba solo el frente de la cola: con un reloj que retrocede, un registro viejo puede quedar detrás de otros más nuevos hasta que la cota de tamaño lo expulse | `observation.rs`, nota `ponytail:` de la retención; [observación](etapa5-observacion.md) §4 | Aceptado, con su ruta de mejora documentada (recorrer la cola al leer) |

### 3.1 Mutaciones de la revisión

Comando de cada mutación: `sed -i` sobre el archivo indicado; las pruebas: `cargo test -p kerno-units --all-features --release --no-fail-fast` con el filtro indicado. Resultado real de cada una, copiado de la salida:

| Mutación (fila de la tabla de §2.4) | Pruebas | Resultado real |
|---|---|---|
| `planner.rs`: `execute_units` acepta el camino de índice sin `check_units` (`match Ok::<(), Violation>(())`) | `--lib planner` | `test planner::tests::a_failing_or_wrong_index_path_falls_back_to_the_reference ... FAILED`; `test result: FAILED. 6 passed; 1 failed` |
| `planner.rs`: el camino `IndexPrimary` nunca se ejecuta (`PlanPath::IndexPrimary => None`) | `--test index` | `test result: FAILED. 12 passed; 14 failed` (`differential_seeded`, `differential_across_scan_pages`, `rebuilt_equals_fresh`, `index_path_refuses_what_it_can_see`, entre otras) |
| `observation.rs`: el texto de la forma sale desde `Fingerprint` (`fingerprint_text: (level >= ObservationLevel::Fingerprint)`) | `--test sentinels --test observation` y `--lib observation` | `queries_report_what_ran` (×2, `observation.rs:173`) y `levels_below_diagnostic_carry_no_shape_text` (×2, `sentinels.rs:317`) FAILED; en `--lib`, `fingerprint_records_the_hash_and_diagnostic_adds_the_text` FAILED |

Las tres se restauraron con la inversa de `sed`, y `git status` quedó solo con los documentos de este cambio.

Lectura de la tercera: la búsqueda de centinelas (`no_sentinel_in_any_observability_explanation_or_error_output`) sigue en verde con esa mutación, porque el texto de la forma son marcadores (`units[unit_type=?text]`) y no contiene literales. Lo caza la prueba estructural `levels_below_diagnostic_carry_no_shape_text`. Por eso E5-04 tiene las dos.

## 4. Verificación de la puerta de cierre

Comandos y resultado real, en la rama, tras E5-05:

```text
$ cargo fmt --all --check
(sin salida; código 0)

$ cargo clippy --workspace --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.48s
(código 0)

$ cargo test --workspace --exclude kerno-storage --all-features --release
passed=615 failed=0 ignored=8

$ cargo test -p kerno-storage --all-features --release
passed=649 failed=0 ignored=4
```

Conteo de pruebas (suma de las líneas `test result`, agregada):

| Suite | Antes (E5-04, `cf7bf14`) | Después (E5-05) |
|---|---|---|
| `cargo test --workspace --exclude kerno-storage --all-features --release` | 615 pasan, 8 ignoradas | 615 pasan, 8 ignoradas |
| `cargo test -p kerno-storage --all-features --release` | 649 pasan, 4 ignoradas | 649 pasan, 4 ignoradas |
| Suite completa | **1264** | **1264** |

E5-05 no añade pruebas; el conteo no cambia. `kerno-units` solo: 259 pruebas en E5-03; no se volvió a contar por separado.

## 5. Riesgos y mediciones

| ID | Riesgo | Estado al cierre |
|---|---|---|
| K-E3-8 | U2 incumple en `history` | **Abierto**, sin cambios: el planificador no toca `history` |
| K-E3-9 | U6 incumple con retención concurrente | **Abierto**: re-medido con índices y relaciones en E5-00 (`etapa5-u6-remedicion.md`): p99 de escritura 1,05–1,18×, rendimiento 0,89–0,94×. Sigue incumpliendo y **no bloquea** (acta §5). Las observaciones no escriben en el motor |
| M-11 | Prueba intermitente `retention_every_point_paced_power_loss_sync` | Abierta, sin cambios desde E4-05 (ver [cierre de la Etapa 4](etapa4-cierre.md) §4). No se reprodujo en las ejecuciones de E5-05 |
| M-9 | U6 sin índices | **Cerrado** en E5-00 |
| Coste del planificador (acta §8.5, ≤ 10 % sobre W1) | Medido en [planificador](etapa5-planificador.md) §7: p99 +8,5 % a 100 000 unidades; en la medición de E5-03 (ejecución 1), +14,1 % a 20 000 y +2,8 % a 100 000; la ejecución 2 no reproduce el +14,1 % (+0,3 %) | **Incumplimiento puntual registrado como riesgo, no bloqueante** (acta §5, §8). Ruido de máquina compartida más probable que sobrecoste real; se propone repetir sobre máquina sin carga antes de la Etapa 6 |
| Coste de observar (acta §8.5, ≤ 5 % de la p99 sin observar) | `diagnostic` a 20 000 unidades, ejecución 1: +5,9 %; las otras siete mediciones cumplen ([observación](etapa5-observacion.md) §7) | **Incumplimiento puntual registrado como riesgo, no bloqueante** |
| Huella con diccionario (ADR-0015) | Clave local no exportada, decisión 4 de la acta | **Mitigado** |

## 6. Decisiones que el propietario toma

- **Aprobación de este cierre**, de la promoción de I-32 (parte del planificador) e I-37 en [invariantes.md](invariants.md) §2 y §2.4, y de la aceptación de las limitaciones H-1 y H-2 (§3).
- **Repetir la medición del planificador y de `diagnostic`** sobre máquina sin carga, si el propietario quiere cerrar los dos riesgos de rendimiento de §5 antes de la Etapa 6. No se cambia ningún umbral.

## 7. Cierre

| # | Condición de cierre | Estado |
|---|---|---|
| 1 | Criterios de la acta §4 cumplidos, con evidencia (§2) | hecho (un criterio con salvedad, H-2) |
| 2 | Revisión adversarial sin bloqueantes abiertos (§3) | hecho |
| 3 | Promoción de I-32 (planificador) e I-37 en [invariants.md](invariants.md) | hecho, **pendiente de aprobación del propietario** |
| 4 | Roadmap, SAD §9 y `adaptive-execution.md` actualizados | hecho en el cambio que incorpora este documento |
| 5 | Aprobación del propietario de este cierre, de la promoción y de las limitaciones (§6) | **pendiente** |

## 8. Siguiente etapa

La Etapa 6 (primera plasticidad operacional) empieza solo cuando el propietario apruebe este cierre. Hereda:

- K-E3-8 y K-E3-9, y el riesgo de coste del planificador y de `diagnostic` (§5).
- H-1: la Etapa 6 no puede apoyar la equivalencia de una optimización aprendida en una comprobación de completitud en tiempo de consulta; debe usar la prueba diferencial.
- I-33…I-36 y I-34 (parte de optimizaciones), que siguen en la Etapa 6.
- Los planes `learned`, `cache` y las causas de fallback `precondition_failed`, `invalidated`, `disabled` y `discrepancy` de `ExecutionObservation`, que no se implementaron en esta etapa (acta §2.3; [ejecución adaptativa](adaptive-execution.md)).
