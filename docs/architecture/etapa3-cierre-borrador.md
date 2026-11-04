# Cierre de la Etapa 3: borrador (E3-10)

Estado: **borrador** para revisión del propietario. Faltan los resultados de E3-09 (marcados como *pendiente*). · Relacionado con: [diseño de la Etapa 3](etapa3-design.md) §11–§13, [acta E3-00](etapa3-acta-e3-00.md), [invariantes §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3), [matriz de fallos §7](failure-matrix.md), [roadmap](../roadmap/README.md#etapa-3--unidades-revisiones-y-procedencia)

Este texto está pensado para incorporarse como sección de cierre del diseño o del informe de la etapa una vez integrado E3-09. Describe la etapa tal como quedó en `main` después de E3-08 (`3a8b794`), más las correcciones de la revisión adversarial de E3-10.

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
| E3-09 | `unit_bench`, objetivos U1–U6, ritmo de retención, `docs/testing/etapa3-report.md` | — | *pendiente: en curso en otra rama; resultados por incorporar* |
| E3-10 | Revisión adversarial, promoción de IU-1…IU-10, este cierre | — | borrador (rama `e3-10-prep`) |

### Criterios de aceptación del roadmap

| Criterio | Evidencia | Estado |
|---|---|---|
| Se reconstruye la evolución de una unidad | `history::model_as_of`, `policies::policy_model` (historial, as-of y `state_as_of` comparados con `UnitModel`) | cumplido |
| Se restaura sin destruir revisiones posteriores (I-22) | `history::restore_keeps_later` | cumplido; falta actualizar la fila I-22 de `invariants.md` |
| El historial no depende del WAL (I-20) | `crash::wal_independence_{deleted,recycled}_wals` | cumplido (E3-08) |
| Contenido y revisión en la misma frontera atómica, también en la matriz de fallos (I-21) | barridos de `crash.rs`, `process_kill.rs`; matriz de fallos U1–U10 | cumplido (E3-08) |
| Bancos y métricas U1–U6 frente a la referencia; T1–T10 sin tocar | *pendiente de E3-09* | *pendiente* |

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
- ADR-0004, ADR-0008 (mínimo) y ADR-0010: aceptados.
- Ambigüedades resueltas en cada WP con la opción más conservadora (notas de E3-03…E3-07 del diseño):
  - una revisión por unidad y `ChangeSet`;
  - `audit` conserva el hash y la longitud;
  - la retención hace un `ChangeSet` `Retention` por llamada;
  - los miembros de `Retention` nombran la primera revisión retenida;
  - `verify_units` solo afirma IU-9 después del último `Maintenance`.
- **Siguen abiertas** las decisiones 9–13 de §12:
  - poda de `CHANGESET`;
  - `unit_id` y su relación con ADR-0005;
  - troceado de cargas útiles;
  - umbrales U1–U6 (*los cierra E3-09*);
  - orden de ADR-0009 frente a la IA.

## 4. Resultados de rendimiento (E3-09)

*Pendiente: se incorporará desde `docs/testing/etapa3-report.md` cuando E3-09 se integre. Incluirá U1–U6 frente a la referencia, el coste de la retención y la confirmación de que T1–T10 no cambian.*

## 5. Riesgos abiertos

| ID | Riesgo | Estado al cierre |
|---|---|---|
| K-E3-1 | Falla la predicción `commit_seq = read_seq + 1` | Mitigado: comprobación tras el commit con envenenamiento (`seq_mismatch_poisons_the_handle`). Sigue vigente si el motor llega a consumir secuencia por su cuenta |
| K-E3-2 | La ranura de escritura es única: un `ChangeSet` largo bloquea a todos | Abierto. Se acota con `max_changeset_ops` y con la retención por lotes. El multiescritor está fuera de alcance |
| K-E3-3 | Amplificación por escribir varios registros por revisión | *Pendiente de las métricas U4/U5 (E3-09)* |
| K-E3-4 | Las instantáneas largas anulan la retención | Abierto. El informe separa «liberado lógicamente» de «reclamado». Ver el ritmo de retención en E3-09 |
| K-E3-5 | Una capa construida sobre ADR pendientes cuesta revertirla | Mitigado por bytes opacos, `alg`/`kind` reservados y `layer_version` |
| K-E3-6 | El borrado lógico se confunde con la supresión | Abierto hasta ADR-0009 (decisión 13) |
| K-E3-7 (nuevo) | Las lecturas comprueban menos de lo que comprueba `verify_units`. Por ejemplo, un `REV_META` intermedio dañado solo se ve cuando una lectura pasa por él | Aceptado por diseño: las lecturas comprueban lo que tocan y `verify_units` lo recorre todo. D-1 muestra que cada comprobación de lectura conviene que reutilice la regla de `verify`. Se recomienda revisarlo en la Etapa 9 junto con la CLI de verificación |

## 6. Pendiente para cerrar la etapa

1. Integrar E3-09 y completar §1 (fila E3-09), §4 y los riesgos K-E3-3 y K-E3-4.
2. Que el propietario apruebe la promoción de IU-1…IU-10 ([invariants.md §2.1](invariants.md#21-invariantes-de-la-capa-de-unidades-etapa-3)) y la actualización de las filas I-22 e I-26.
3. Actualizar el roadmap (Etapa 3 con su estado real) y SAD §8 con lo construido.
