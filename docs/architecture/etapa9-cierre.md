# Cierre de la Etapa 9

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Paquete | E9-CL de [la acta de la Etapa 9](etapa9-acta.md) (§3) |
| Estado | **Cierre redactado, pendiente de aprobación del propietario.** La etapa queda cerrada como trabajo de ingeniería **con E9-02 hecho en el árbol el 2026-11-13, pendiente de revisión independiente y de fusión**. Las decisiones D1, D2, D3, D4, D12, D15 y D16, retenidas el 2026-11-12, quedan **tomadas el 2026-11-13 por delegación expresa del propietario**, revocables (§2). Lo que depende de ellas y no está hecho se declara «sin verificar» o «sin hacer» (§3), no hecho |
| Alcance de este documento | Documentación. No cambia código ni pruebas. Compara los documentos de cada paquete con el repositorio y deja escritos los límites que cada uno declara; **no repite** las mediciones ni la revisión de cada paquete |
| Lo que no se afirma | Que la revisión independiente de cada paquete (acta §4.10) se haya rehecho aquí: sus registros están en cada paquete. Que haya 1.0: no se declara; D12 (tomada el 2026-11-13) lo condiciona a D13 y a un uso real fuera del equipo |

## 1. Estado de los paquetes, comprobado en el repositorio

Se comprobó con `git log` y con los ficheros. Todos los paquetes están fusionados salvo E9-02, hecho en el árbol y sin fusionar.

| Paquete | Estado | Qué lo muestra en el repositorio |
|---|---|---|
| E9-00 Puerta de entrada | Fusionado | ADR-0012 y ADR-0014 alineados; decisiones en el roadmap |
| E9-01 Fixtures | Fusionado | [etapa9-fixtures.md](etapa9-fixtures.md); `crates/kerno-units/tests/layer_fixtures.rs`; `tests/vectors/derived-v2/` |
| **E9-02 Identificadores públicos** | **Hecho en el árbol el 2026-11-13 (D1), sin fusionar** | `crates/kerno-units/src/ids.rs` y `crates/kerno-units/tests/public_ids.rs` existen (49 pruebas listadas sin `test-support` y 79 con ella, en los dos motores; fixture `tests/vectors/public-ids-v3/`; [mutaciones](etapa9-mutaciones-e9-02.md)). Falta la revisión independiente y la fusión; I-27 queda **verificada en el árbol** |
| E9-03 Auditoría minimizada | Fusionado | [etapa9-auditoria.md](etapa9-auditoria.md); `tests/redaction_units.rs` |
| E9-04 a E9-06 Exportación | Fusionados | `src/export.rs`, `src/import.rs`, `tests/export_import.rs`, `tests/export_physical.rs`; [borrador](../file-format/export-v0-borrador.md) |
| E9-07 Reclamación pública | Fusionado | trait `Reclaim` en `kerno-core/src/contract.rs`; `src/reclaim.rs`; `tests/reclaim.rs` |
| E9-08a a E9-08g Envoltura | Fusionados | `grep -rn "pub fn engine" crates/kerno-units/src` sin resultados (T-EN1); [usos de `engine()`](etapa9-usos-de-engine.md) |
| E9-09 Lectura de derivados | Fusionado | `src/derived_query.rs`; `tests/derived_query.rs` |
| E9-10, E9-11 Vectores y errores | Fusionados | `tests/derived_replace.rs`; `tests/errors_api.rs` |
| E9-12 Estrés de derivados | Fusionado | `tests/derived_stress.rs` |
| E9-13 Decodificadores | Fusionado | `tests/decoders_robust.rs` |
| E9-14 Fuzzing | Fusionado | sección «Stage 9 run on the final code» de `crates/kerno-storage/fuzz/README.md` |
| E9-15 Mutaciones de E8 | Fusionado | [etapa9-mutaciones-e8.md](etapa9-mutaciones-e8.md) |
| E9-16 Revisión de K-E3-7 | Fusionado | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) (con el resultado de E9-22 en su §8) |
| E9-17, E9-18 SQLite | Fusionados | [informe E9](../testing/sqlite-report-e9.md); informe anterior sin nombres de terceros |
| E9-19, E9-20 Conjunto y medición | Fusionados | [conjunto](etapa9-conjunto-evaluacion.md); [medición](etapa9-medicion.md) y su [salida cruda](etapa9-medicion-salida.txt) |
| E9-21 Ejemplo | Fusionado | `examples/agent-memory/`; disposición de los quince puntos en `FRICTION.md` |
| E9-22 Hallazgos K1 y K2 | Fusionado (paquete abierto después de E9-16) | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) §8 |

**Dependencias (I-31).** `git log 52f026f..HEAD -- Cargo.toml Cargo.lock` no devuelve commits. Ninguna dependencia nueva en la etapa.

## 2. Decisiones

**Tomadas por delegación del propietario (2026-11-12). Todas revocables por él:** D5, D6, D7, D8, D9, D10 (en lo que es el borrador), D11, D13 (como criterio), D14, D17, D18, D19, D20, D21, D22 y D23. La tabla con la opción de cada una está en [acta §8.1](etapa9-acta.md). Si el propietario revoca una, lo que dependa de ella se revisa; ninguna cambia el formato en disco.

**Retenidas el 2026-11-12 y tomadas el 2026-11-13 por delegación expresa del propietario, revocables** (el propietario, en persona, pidió al delegado tomar la mejor decisión en cada punto pendiente). Cada una se toma con la opción que recomendaba la acta. Lo que queda sin verificar o sin hacer:

| Decisión | Opción tomada | Qué queda **sin verificar** o sin hacer |
|---|---|---|
| D1 | (a): token aleatorio opaco con mapeo persistido (opción 1 de ADR-0005) | E9-02 hecho en el árbol, sin fusionar. ADR-0005 queda **Aceptado** con esa opción |
| D2 | (a): AEAD por bloque, como diseño a usar si se construye cifrado | Nada se implementa. ADR-0006 sigue **Pendiente** |
| D3 | (b): sin cifrado en 1.0; 1.0 no promete confidencialidad en reposo ni gestión de claves, y lo declara | K-1 a K-4 de ADR-0009 sin medir, porque no hay cifrado |
| D4 | (a): sin integridad autenticada en 1.0; el límite queda en el threat model §6.6 y `verify()` con `deep` es el recurso | H-2 queda cerrado como límite aceptado y documentado (§5 de este documento, §6) |
| D12 | (b): 1.0 se declara solo después de D13 y de un uso real fuera del equipo | **1.0 no se declara**. No se congela el formato de exportación ni se promete semver |
| D15 | (b) como dirección: licencia amplia no exclusiva con derecho a relicenciar | El texto legal del CLA **no está escrito** y necesita la aprobación del propietario antes de aceptar cualquier contribución externa. No se acepta ninguna |
| D16 | (b): ADR-0016 queda **pendiente de confirmación** hasta revisar G1 a G7. **Cerrada el 2026-11-13:** revisión hecha, ADR-0016 **aceptado y confirmado** con enmiendas | La revisión se hizo como tarea aparte ([ADR-0016](../adr/0016-adaptacion-e-invalidacion.md), «Revisión de 2026-11-13»). Nada de la etapa depende de ADR-0016. Trabajo abierto que deja: la explicación del plan no lista la caché, la evidencia de creación es constante (H-3) y `set_key` no deja registro |

## 3. Criterios comunes (acta §4) y umbrales (acta §5)

Las cifras de esta tabla se **citan** de los documentos de cada paquete. Solo se reejecutaron las pruebas de §4 (invariantes).

| Código | Resultado | Fuente y salvedad |
|---|---|---|
| T-EX1, T-EX2 | Cumplidos según E9-05 y E9-06 | `export_import.rs`, `export_physical.rs`. El formato es un **borrador sin congelar** (D10, D12) |
| T-RE1, T-RE2 | Cumplidos según E9-07 | `tests/reclaim.rs`. No es una promesa de borrado físico (ADR-0009, opción 3) |
| T-FZ1 | Cumplido con salvedades | 15 objetivos, 1 801 s cada uno, sin fallos. Salvedades en §5, punto 7 |
| T-PR1 | Cumplido según E9-13 | `decoders_robust.rs` |
| T-ST1 | Cumplido con un límite | Salvedad en §5, punto 4 |
| T-FR1 | 15 de 15 con disposición | `examples/agent-memory/FRICTION.md`. Punto 6: E9-02 existe en el árbol (`PublicId`), sin fusionar; la parte de identidad del punto 3 es un cambio de formato que D12 no decide |
| T-EN1 | Cumplido | `grep -rn "pub fn engine" crates/kerno-units/src`: sin resultados |
| T-MU1 | Cumplido con salvedades | E9-15 (19 de 19 pruebas), E9-03, E9-20. Salvedades en §5, puntos 5, 6, 9 y 10 |
| T-SQ1, T-NM | Cumplidos para el informe nuevo | Precondición de carga anotada en el informe; sin nombres de terceros distintos de SQLite. La carga de las mediciones de E9-14 y E9-20 no fue la de una máquina libre (§5) |
| T-CNT | **No reejecutado aquí** | Cada paquete anotó su conteo; este cierre no añade pruebas |
| T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5 | Cumplidos en la única medición | [etapa9-medicion.md](etapa9-medicion.md) §2. Salvedades en §5, punto 9 |

**Latencia.** Sin umbral absoluto (D7). Los cocientes P1 y P2 se midieron con la máquina cargada.

## 4. Invariantes

Se ejecutó, en release, `cargo test -p kerno-units --all-features --release --test derived_suppression --test consolidation --test reclaim --test redaction_units`:

```text
consolidation:      test result: ok. 30 passed; 0 failed
derived_suppression: test result: ok. 19 passed; 0 failed
reclaim:             test result: ok. 10 passed; 0 failed
redaction_units:     test result: ok. 3 passed; 0 failed
```

| Invariante | Resultado del cierre |
|---|---|
| **I-29** para `kerno-units` | **Promovida** (D19, E9-03 pasa). Con el límite de §5, punto 5 |
| **I-42** | **Promovida** (D19): verificada de nuevo con `reclaim`, que no escribe en la capa (`reclaim_changes_no_visible_state`) |
| **I-43** | **Promovida** (D19): verificada de nuevo con `the_original_stays_readable_through_every_summary_operation` y `reads_of_a_source_are_the_same_with_and_without_summaries` |
| **I-28** | **No aplica en 1.0** (D19; D3 tomada el 2026-11-13: sin cifrado en 1.0). Lo que se puede decir hoy: `verify()` no encuentra archivos desconocidos en el directorio (`lifecycle_soak.rs`) |
| **I-30** | Verificada **para el borrador** y para la reimportación. La congelación del formato no se hace: D12 (tomada el 2026-11-13) la deja para después de D13 y de un uso real |
| **I-27** | **Verificada en el árbol** (2026-11-13) por `crates/kerno-units/tests/public_ids.rs` (`tampered_text_is_refused_with_its_own_error`, `tampered_but_well_formed_tokens_name_nothing`); sin revisión independiente ni fusión |
| **I-31** | Verificada por revisión de dependencias (§1) |
| I-N1 (reclamación) | Registrada como **I-44** en [invariantes](invariants.md) §2.7 |
| I-N2 (envoltura) | Registrada como **I-45** en [invariantes](invariants.md) §2.7 |

La promoción de I-29, I-42, I-43, I-44 e I-45 es una elección **por delegación y revocable** (D19). Las dos nuevas (I-44, I-45) son propuestas del acta que este cierre numera; no están aprobadas por el propietario.

## 5. Límites conocidos y puntos abiertos

Cada punto es un límite o un pendiente. El punto 1 está resuelto; los demás siguen abiertos.

1. **Roadmap, estado de la Fase 1. Resuelto el 2026-11-13** por delegación expresa del propietario: la tabla de fases y la tabla de tareas de SAD §20 de `docs/roadmap/README.md` dicen «Cerrada» para la Fase 1 con la fecha 2026-11-11, que es la que da el texto de la sección «Fase 1: plan de trabajo». El informe de la Fase 1 (`docs/testing/f1-report.md`) es de 2026-10-29 y describe el estado de entonces, anterior al cierre.
2. **K-E3-7 (E9-16 y E9-22).** *K1*: corregido desde el extremo que conserva su entrada de adyacencia; **sigue abierto** desde el extremo que la perdió, donde `relations_of` y `query_relations` devuelven una respuesta corta sin `Fallback`; `verify_units` (UNIT-36) la reporta. *K2*: corregido para `IDX_PRIMARY` con el conteo intacto; abierto con el conteo rebajado y para `IDX_TYPE`. *K3*: no se reproduce. *K4* (`change_set_info` no cruza los miembros con su `REV_META`) y *K5* (`get_relation` no comprueba `created_seq` ni los extremos): **siguen abiertos, menores y sin cambio**. La revisión independiente de E9-16 figura como pendiente en su registro.
3. **Mensaje de `NoValidSources`.** El texto «no source of the derived datum is valid any more» no se cambió porque `crates/kerno-units/tests/consolidation.rs` (línea 496) fija ese texto. Mejorarlo exigiría cambiar esa prueba, que es de otro paquete.
4. **E9-12, estrés de derivados (F-09).** La prueba comprueba que ningún derivado queda `ready` después de que un cambio de su fuente ha vuelto. **Un bloqueo tardío posterior al commit sigue en verde**: el orden exacto de bloqueo del commit guardado no tiene prueba determinista. F-09 queda cubierto solo en parte.
5. **E9-03, redacción.** No hay evidencia de mutación para las rutas del informe de `verify` ni de las observaciones: el texto de `VerifyFinding` solo lleva claves de registro y no hay línea de producción que la prueba pueda ver fallar sin inventar una fuga. La prueba comprueba que esas rutas se ejercitaron y que ningún texto lleva un centinela. Los fragmentos de menos de 8 bytes no se buscan.
6. **E9-08g.** Ninguna prueba falla si `compact()` pierde su comprobación de envenenamiento o si amplía su rango (hallazgo F3 de su revisión).
7. **E9-14, fuzzing (H-7).** La máquina **no** estaba libre (carga media de unos 4 en 4 CPU). `wal`, `manifest` y `open` alcanzaron solo entre 4,5 y 6,7 millones de ejecuciones, dos órdenes de magnitud menos que los decodificadores puros; 30 minutos es poco para ellos. **H-7 se marca verificada con esos límites**: 15 objetivos, 1 801 s cada uno, sin fallos, sin entradas nuevas en el corpus. Una corrida más larga de esos tres es el paso natural.
8. **E9-21, ejemplo.** (a) Por decisión delegada, sus pruebas corren solo sobre `kerno_db`, porque el ejemplo no depende de `kerno-testkit`. (b) La fila del punto 15 de la acta pide `KeepLast(1)` mediante `set_policy`, pero la API pública no lo permite (`set_policy` solo recibe una `HistoryPolicy`); el ejemplo poda de forma explícita (`prune_history`) y su catálogo gana una revisión por escritura hasta que se pode.
9. **E9-20, medición.** Se midió **una vez**, con otra carga activa en la máquina: P1 y P2 **no se midieron en reposo**. El banco de tiempo no corre sobre el conjunto congelado, sino sobre una historia generada de la misma semilla; P2 cronometra solo 25 consultas. Los umbrales de calidad **se imprimen, no se afirman** en la prueba. F-08 sigue: P2 no mide la lectura de vectores persistidos. El conjunto es sintético y de 2 000 unidades.
10. **E9-15, mutaciones de E8.** Dos mutaciones (D3 y D4) hacen fallar **una** prueba y no dos, como explica el documento: `quality_over_the_frozen_set` no puede verlas. No hay mutación aparte para P4 ni para el informe idéntico entre motores.
11. **Prueba posiblemente intermitente.** `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` (storage) falló una vez y pasó al repetirla durante la etapa. **Sospecha de prueba intermitente, por investigar**; no se investigó aquí ni se cambió.
12. **Historia del repositorio.** Algunos commits de `main` (entre ellos `c779b39`) llevan trailers de autoría que las reglas del proyecto prohíben. No se pueden quitar sin reescribir la historia, que no está permitido. **Mancha conocida y aceptada.**

## 6. Threat model y DESIGN

- [Threat model](../security/threat-model.md): T-03 y T-04 siguen **abiertas** (sin cifrado ni autenticación en 1.0; D2, D3 y D4 tomadas el 2026-11-13). **H-2**: el motor no protege frente a una reescritura deliberada con CRC válido; la defensa disponible es `verify()` con `deep`. H-2 queda cerrado como límite aceptado y documentado (D4). T-09 sigue **abierta**; la reclamación pública es mecanismo, no borrado físico. T-10: `ci.yml` no ejecuta fuzzing ni auditoría de dependencias; sigue abierta.
- [`DESIGN.md`](../../crates/kerno-storage/DESIGN.md): H-9 precisada en la sección «Qué puede sobrevivir».
- **H-5 (lo que no se mide)**: C-5 (bloques liberados del medio), memoria, cachés, lápidas, hilo de borrado y handle. Sin medir.

## 7. Erratas

- **Acta §2.3 c.** `DerivedDb::close` devuelve `Result<(), UnitError>`, no `()`; descarta el `CloseReport` porque la capa no depende de `kerno-storage` (D9). Dispuesto en FRICTION 11 como límite declarado.
- **Acta §2.3 f.** La cita de `compact_range` en ADR-0009 (C-5) es `db.rs:2009`; la línea correcta cuando se escribió el acta era `db.rs:2011`. No se edita el ADR (aceptado); **se corrige la cita aquí**. Si el código cambia, la línea se vuelve a buscar con `grep -n "pub fn compact_range" crates/kerno-storage/src/db.rs`.
- **ADR-0020, fixture.** La cita `derived-v2/db-with-vector` existe ahora, reconstruida en E9-01; sus bytes no se reproducen entre compilaciones (la integridad la da `MANIFEST`). La comprobación T5 sobre `d01f646` se hizo una vez y el árbol de archivos no queda idéntico (la apertura del motor crea `LOCK` y un registro de sesión).
- **H-12.** La cita `sst.rs:332` del [acta del adelanto](etapa9-adelanto-acta.md) pasa a `sst.rs:335`.

## 8. Hallazgos heredados

| Hallazgo | Estado al cierre |
|---|---|
| H-2 | Cerrado como límite aceptado y documentado (D4, tomada el 2026-11-13) |
| H-3 | Aceptado a posteriori (D20 i) |
| H-4 | Se mantiene contradicha, sin reescribir (D17) |
| H-5 | Sin medir; declarado (§6) |
| H-6, H-8 | Cerrados por E9-03 |
| H-7 | **Verificado con los límites del punto 7** |
| H-9, H-12 | Cerrados en este paquete |
| H-13 | Revisado (E9-16); K1, K2, K4 y K5 con el estado del punto 2 |
| H-14 | I-27: verificada en el árbol por E9-02 (D1); falta fusionar. I-28: no aplica. I-30: verificada para el borrador |
| F-02, F-03 | E9-11 |
| F-04 | E9-10 |
| F-05 | Abierto: no hay formato en disco para resúmenes (D8) |
| F-06, F-07 | Medidos en E9-20, con las salvedades del punto 9 |
| F-08 | Abierto, límite declarado |
| F-09 | Parcial (punto 4) |
| F-10 | Evidencia en E9-15, con las salvedades del punto 10 |
| F-11 | Comprobado, con la salvedad de §7 |
| F-13 | Quince puntos con disposición (T-FR1) |
| F-12 | Ver abajo |

**F-12, rondas de revisión de E8-03 y E8-04.** Lo que se puede reconstruir hoy son los commits de cada paquete, no las rondas: `6bbadbd` y `23af819` (E8-03, fusionado en `d01f646`) y `ea533ca` (E8-04, fusionado después). Las rondas de revisión no tienen registro en el repositorio; no se inventan. **Sigue abierto como carencia de registro.**

## 9. Cambios de superficie pública (R-E9-17)

Pre-1.0, sin promesa que romper (D9): `Reclaim` en `kerno-core`; `UnitDb::reclaim`, `UnitDb::compact` y `DerivedDb::reclaim`; `UnitDb::engine` retirada; accesores `raw_*` y ganchos bajo la característica `test-support` (D23); `DerivedDb::snapshot`, `query`, `history`, `derived_of`; sustitución pública de vectores; `RevisionId::first`; `UnitError::is_locked`.

## 10. Qué sigue

revisión independiente y fusión de E9-02, hecho en el árbol (I-27 verificada allí); aprobación del propietario del texto del CLA antes de aceptar cualquier contribución externa (D15); revisión de G1 a G7 de ADR-0016 hecha y D16 cerrada (2026-11-13; su trabajo abierto está en el ADR); declaración de 1.0 solo después de D13 y de un uso real fuera del equipo (D12); investigar la prueba intermitente (punto 11); corrida más larga de fuzzing de `wal`, `manifest` y `open`; K1/K2/K4/K5 abiertos (punto 2). El estado de la Fase 1 en el roadmap ya está resuelto (punto 1).
