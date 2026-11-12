# Acta de entrada de la Etapa 9: endurecimiento

| Campo | Valor |
|---|---|
| Fecha | 2026-11-12 |
| Estado | **Aprobada por delegación del propietario el 2026-11-12, revocable por él** (§8.1). Las decisiones retenidas para el propietario en persona (D1 a D4, D12, D15 y D16) no se toman por delegación y bloquean los paquetes que indica §8.1 |
| Acto | Alcance, estado de partida comprobado en código, paquetes E9-00 a E9-CL, umbrales, invariantes, riesgos y decisiones D1 a D23 |
| Documentos | [Roadmap](../roadmap/README.md) · [acta de la Etapa 8](etapa8-acta.md) · [cierre de la Etapa 8](etapa8-cierre.md) · [acta del adelanto de la Etapa 9](etapa9-adelanto-acta.md) · [cierre del adelanto](etapa9-adelanto-cierre.md) · [SAD](SAD.md) (§11, §12, §13, §16, Fases 4 y 6) · [invariantes](invariants.md) · [threat model](../security/threat-model.md) · [ADR-0005](../adr/0005-identificadores-publicos.md) · [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md) · [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) · [ADR-0010](../adr/0010-api-publica-y-crates.md) · [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) · [ADR-0019](../adr/0019-licencia-y-contribuciones.md) · [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md) · [informe SQLite](../testing/sqlite-report.md) · [lista de fricción](../../examples/agent-memory/FRICTION.md) |

Esta acta describe el repositorio tal como está el 2026-11-12. Cada afirmación sobre el código se comprobó leyéndolo; las citas de cifras de pruebas (856 en `kerno-units` y `kerno-testkit`, 672 en `kerno-storage`) vienen de cierres anteriores y **no se han reejecutado** aquí. Además de leer el código, lo único ejecutado para esta acta es `cargo fmt --all --check` (salida vacía, `exit=0`). Esta acta no cambia código ni pruebas.

## 0. Estado

Esta acta está **aprobada por delegación del propietario el 2026-11-12, revocable**. Fija el alcance de la Etapa 9, sus paquetes con los ficheros que puede escribir cada uno, los umbrales antes de medir y las decisiones de §8, cada una con su recomendación. Cada decisión se registra en §8.1 como tomada por delegación, salvo las que cambian formato, una promesa a usuarios o un texto legal: esas quedan **retenidas para el propietario en persona** y §8.1 dice qué queda bloqueado mientras tanto.

Ningún código de la Etapa 9 se escribe antes de E9-00. Los paquetes que escriben código respetan §4 y §3.2.

## 1. Condiciones de inicio

| Condición | Estado leído el 2026-11-12 | Qué falta |
|---|---|---|
| Etapa 8 cerrada como trabajo de ingeniería | **Cumplida**. [Cierre](etapa8-cierre.md) §0 y §8. Q4 no demostrada; sin hallazgo bloqueante | E8-10 (follow-up antes de 1.0), tratado en E9-19 y E9-20 |
| ADR-0009 aceptado y escrito en su archivo | **Cumplida**: opción 3, en `0009-supresion-retencion-y-backups.md` y en el índice | C-4 contradicha en su cláusula de conteo; A-1 y B-1 no cumplidos (§2.3, D17 y D18) |
| Cierre del adelanto (E9A-06) | **Cumplido** el 2026-11-12, sin hallazgo bloqueante abierto. I-29 promovida para `kerno-storage` por delegación, revocable | Hallazgos H-2 a H-14 del cierre, asignados en §9 |
| ADR-0018 y ADR-0020 aceptados | **Cumplido** (2026-11-12) | Ninguna para la Etapa 9 |
| Decisión 1 del roadmap (orden seguridad frente a IA) | Resuelta el 2026-11-11 en el [acta del adelanto](etapa9-adelanto-acta.md) §10 | Ninguna |
| Decisión 3 del roadmap (umbrales frente a SQLite) | Pendiente en el roadmap. Recomendación de §8: informe, no criterio (D6) | Decisión del propietario, tomada por delegación en §8.1 |
| ADR-0019, texto del CLA | ADR aceptado el 2026-11-11, pero **el texto del CLA no está escrito** (ADR-0019, Seguimiento) | Redacción por el propietario (D15, retenida) |
| ADR-0005, ADR-0006, ADR-0012 | **Propuesto**, **Pendiente** y **Propuesto** en sus archivos | ADR-0005 y ADR-0006 se deciden en persona (D1 a D4). ADR-0012 se acepta por delegación en E9-00 (D5) |

## 2. Alcance

### 2.1 Qué entrega la Etapa 9

Los entregables son los del [roadmap](../roadmap/README.md) (Etapa 9), más la reclamación física desde la API pública que pide el cierre de la Etapa 8 (§6, grupo F):

| Entregable | Qué entrega esta etapa | Paquete |
|---|---|---|
| Identificadores públicos (ADR-0005, I-27) | **Retenido** (D1). Si el propietario elige, el codec y su prueba de tokens manipulados | E9-02 (retenido) |
| Cifrado y gestión de claves (ADR-0006, I-28) | **Retenido** (D2 a D4). Sin código de cifrado en 1.0 salvo que el propietario lo elija. I-28 se declara no aplicable (D19) | Ninguno sin decisión |
| Auditoría minimizada | Verificación de I-29 para `kerno-units` (errores, informes, observaciones) y ampliación de la prueba de redacción de `kerno-storage` a fragmentos. Ningún registro persistido de auditoría (D8) | E9-03 |
| Exportación e importación (I-30) | Formato de exportación **en borrador**, versionado y sin congelar (D10); exportación de unidades, revisiones, relaciones y políticas; importación en un motor nuevo; independencia del formato físico | E9-04, E9-05, E9-06 |
| Migraciones con fixtures de versiones previas | Fixtures de v1 comprobados, y el fixture de ADR-0020 reconstruido desde su commit base o declarado ausente (D11). El registro `MIGRATION` queda reservado sin escritor | E9-01 |
| Estrés y fuzzing sobre el motor de almacenamiento | Corrida completa de fuzzing de 30 minutos por objetivo sobre el código final de `kerno-storage` | E9-14 |
| Estrés y fuzzing por encima del motor | Estrés de derivados y pruebas de propiedades sobre los decodificadores de `kerno-units` (D21: sin herramienta nueva) | E9-12, E9-13 |
| Benchmarks comparativos | Nueva pasada del informe SQLite, sin umbrales (D6), en calma; corrección de nombres de terceros del informe actual | E9-17, E9-18 |
| API pública que pide la lista de fricción | Lectura de derivados, sustitución de vectores, errores, envoltura del motor y reclamación física (§2.4) | E9-07 a E9-11 y E9-08a a E9-08e |
| Conjunto de evaluación E8-10 | Conjunto congelado por separado, con vectores de consulta en cada clase, y una sola medición | E9-19, E9-20 |
| Reclamación física desde la API pública (ADR-0009 opción 3) | `Reclaim` público y `reclaim()` en `UnitDb` y `DerivedDb`, con el texto de ADR-0009. Sin promesa de borrado físico | E9-07 |
| Cierre | Revisión adversarial, invariantes, threat model, SAD como versión nueva, roadmap | E9-CL |

**Ya hecho por el adelanto y no se repite** ([cierre del adelanto](etapa9-adelanto-cierre.md)): medición de supervivencia tras borrado (`suppression_survival.rs`); I-29 para `kerno-storage` (`redaction.rs`); propiedades de apertura con 2 000 casos por tipo y modo (`open_properties.rs`); los 15 objetivos de fuzzing de `kerno-storage` con su corpus comprometido (replay sin fallos); estrés del ciclo de vida de 200 ciclos (`lifecycle_soak.rs`); la corrección F-E9A-03-1 de la apertura de SST (H-1). Esta etapa no repite ninguno de ellos; E9-14 solo repite la corrida de 30 minutos sobre el código final.

**Fuera de alcance, explícitamente** (§2.5): cifrado en reposo y borrado criptográfico (salvo que D3 se decida a favor), copias de seguridad como función del motor, índice vectorial aproximado, búsqueda textual, fusión híbrida, resumidor real, relaciones inferidas, cambios de `Config::default`, y cualquier promesa de borrado físico.

### 2.2 Punto de partida (comprobado en el código)

| Tema | Lo que dicen los documentos | Lo que muestra el código | Veredicto |
|---|---|---|---|
| Identificadores públicos | ADR-0005: codec por defecto con token aleatorio y mapeo persistido | No hay `IdentifierCodec`, ni token, ni mapeo en `crates/`. `UnitId` es `[u8; 16]` opaco (`crates/kerno-units/src/types.rs:9`) | **No existe** |
| Cifrado | ADR-0006 pendiente; I-28 en Fase 4 | El bit `AEAD` se rechaza al abrir: `Incompatible("encryption not supported")` (`crates/kerno-storage/src/format/header.rs:197-198`). La ranura de 40 bytes debe ser cero (`format/block.rs:66`; `format/features.rs:5`) | **No existe**; reserva de formato |
| Claves | `KeyProvider` en ADR-0006 | Ninguna API de claves | **No existe** |
| Auditoría | I-29 en Fase 4; SAD §11.4 | `kerno-storage` no tiene capa de logs (advance §2.2). Errores e informes: `tests/redaction.rs` (storage) y `IU-10` con `tests/sentinels.rs` (units) | I-29 verificada en storage; **no en units** |
| Exportación | I-30 en Fase 4; `KRNO_EXP` reservado | `MAGIC_EXPORT` está declarado (`format/header.rs:23-24`) y solo aparece en la lista de magias válidas (`header.rs:32`). Ningún código lo escribe | **No existe** |
| Migraciones | Registro `MIGRATION` reservado | `KIND_MIGRATION_RESERVED = 5` (`format/manifest.rs:26-28`). Ningún escritor | **Reservado** |
| Fixtures de versiones previas | ADR-0020, T1, T5 y T6, citan `tests/vectors/derived-v2/` y `derived-v2/db-with-vector` | Existen `crates/kerno-storage/tests/vectors/v1/{dirs,structs}` y `crates/kerno-units/tests/vectors/v1/structs`. `derived-v2` no aparece en el árbol (búsqueda en todo el repositorio, sin resultados). El commit base `d01f646` existe | **Falta el fixture de ADR-0020** (E9-01) |
| Fuzzing | 15 objetivos en `crates/kerno-storage/fuzz` (advance §2.4) | 15 ficheros en `fuzz/fuzz_targets/`. `ci.yml` no ejecuta fuzzing ni `cargo audit` o `cargo deny` (`grep` sobre `.github/workflows`: sin resultados de fuzz ni de audit). **No hay objetivo para `kerno-units`** | Storage: hecho según su registro. Units: no existe |
| Estrés | Tres pruebas en storage (advance §2.2) | `lifecycle_soak.rs`, `group_commit.rs` y `process_kill.rs` existen. En `kerno-units` solo hay `crash.rs` y `process_kill.rs`. **No hay prueba de concurrencia de derivados**, y el orden de bloqueo del commit guardado no tiene prueba (F-09 de E8) | Parcial |
| Comparación con SQLite | Informe sin umbrales (E8-S0) | `sqlite` solo aparece en `spikes/storage/spike-bench/Cargo.toml` (búsqueda en todos los `Cargo.toml`). Ningún crate de `crates/` lo usa | El informe existe; **sin umbrales** (D6) |
| Reclamación física | ADR-0009, punto 2; FRICTION 4 | `UnitDb::compact` existe solo con `E: TestHooks` (`crates/kerno-units/src/retention.rs:662-669`). `KernoDb::compact_range` es `pub` (`crates/kerno-storage/src/db.rs:2011`) dentro de `impl KernoDb` (`db.rs:1769`), que no lleva `cfg(test)`. `DerivedDb` guarda su `UnitDb` como `pub(crate)` (`crates/kerno-units/src/derived_stale.rs:177`), así que la capa no lo expone | **Parcial**: ver §2.3, punto a |
| Envoltura del motor | Regla de la envoltura de §4 (lección de E8) | `UnitDb::engine(&self) -> &E` es `pub` y devuelve el motor interno. Su documentación dice: «Writing to it directly bypasses the layer» (`crates/kerno-units/src/db.rs:488-491`). Lo usan **20 ficheros**, con 133 llamadas (`grep -rln '\.engine()'`; reparto en E9-08b a E9-08d) | **Incumple la regla** |
| Capa de derivados: lectura | FRICTION 1 y 2 | `DerivedDb` tiene `get_unit`, `close`, `begin_changeset`, `state`, `descriptor`, `value`, `build`, `rebuild`, `supersede` y `vector` (`derived_stale.rs`, `vector_store.rs:1166`). No tiene `snapshot`, `query` ni historial. `dependents_of_unit` está en `DerivedRegistry` (`derived.rs:293`), no en `DerivedDb` | **Confirmado** |
| Sustitución de vectores | FRICTION 3 y 8 | `replace_vector` es `pub(crate)` (`vector_store.rs:1083`). `supersede_vector` es `pub` (`vector_store.rs:1060`) | **Confirmado** |
| Resúmenes | Solo en memoria (E8-06, cierre §5.1) | Citado del cierre de E8; no reverificado en esta acta | Citado |
| Opciones por defecto | `wal_preallocate` falso por defecto | `crates/kerno-storage/src/config.rs:543` | **Confirmado** |
| Dependencias | Sin cambios de `Cargo.toml` ni `Cargo.lock` desde E7-00 (I-31) | `git log 52f026f..HEAD -- Cargo.toml Cargo.lock` no devuelve commits. `kerno-units` solo depende de `kerno-core` y de dos bibliotecas de cálculo, y tiene `proptest`, `kerno-testkit` y `kerno-storage` como dependencias de desarrollo (`crates/kerno-units/Cargo.toml`) | **Confirmado** |
| CI | No dice nada de fuzzing ni de auditoría de dependencias | `.github/workflows/ci.yml`: `cargo fmt --all --check`, `cargo clippy ... -D warnings` (línea 26), comprobaciones de `cargo tree` (líneas 28–34), tests de `kerno-storage` (línea 75) y de los demás crates (línea 57). Sin fuzz ni audit | **Confirmado**; T-10 sigue pendiente |
| Estados de ADR en el índice | `docs/adr/README.md` | 0005 Propuesto; 0006 Pendiente; 0009 Aceptado; 0010 Aceptado (fachada y semver «se deciden cuando exista una API pública estable»); 0012 Propuesto (aplicado en M0); 0014 Aceptado; 0015 Aceptado; 0016 Aceptado (2026-11-09); 0017 Propuesto; 0019 Aceptado; 0020 Aceptado | Ver §2.3, puntos e, f, g |
| Invariantes | I-27, I-28, I-30 en Fase 4; I-42 e I-43 verificadas en pruebas y pendientes de aprobación | [invariantes](invariants.md) líneas 50–53, 162–163 | **Confirmado** |

### 2.3 Discrepancias entre documento y código (el código gana)

a. **`KernoDb::compact_range` no es un método de prueba.** El [cierre de la Etapa 8](etapa8-cierre.md) §6, grupo F, dice «`KernoDb::compact_range` es un método de prueba». El código dice otra cosa: `pub fn compact_range` está en `crates/kerno-storage/src/db.rs:2011`, dentro de `impl KernoDb` (`db.rs:1769`), que no lleva `cfg(test)`; el único bloque `#[cfg(test)]` de ese tipo empieza en `db.rs:1664`. Lo que sí es cierto: `DerivedDb` no lo expone, pero `UnitDb::engine()` (`crates/kerno-units/src/db.rs:489`) sí da un camino público con `E = KernoDb`. La lista de fricción (punto 4) dice «`KernoDb::compact_range` is an inherent method», que es exacto. **Consecuencia:** la reclamación pública ya es alcanzable, pero solo rodeando la capa de derivados. E9-08 retira ese camino y E9-07 da el camino correcto.

b. **Friction 14 no es correcto.** La lista dice que una segunda apertura da «ningún modo de distinguir “bloqueada” de “corrupta” salvo por el texto». El código distingue las dos: el would-block se convierte en `Error::Locked` (`crates/kerno-storage/src/db.rs:1804`), que es una variante propia (`crates/kerno-core/src/error.rs:17`), y `UnitError::Storage` la conserva (`crates/kerno-units/src/error.rs:167`). Un llamador puede hacer `match` sobre `UnitError::Storage(kerno_core::Error::Locked)`. Lo que falta es un atajo legible, y E9-11 lo añade.

c. **Friction 11 es casi exacto.** `DerivedDb::close` devuelve `Result<(), UnitError>` (`derived_stale.rs:236`), no `()`. Lo que sí es cierto es que descarta el `CloseReport` de `KernoDb::close` (`db.rs:2029`), porque la capa no depende de `kerno-storage` (D9).

d. **ADR-0020 cita un fixture que no existe.** T1, T5 y T6 (`docs/adr/0020-persistencia-de-vectores-como-derivados.md`, líneas 297–302) citan `crates/kerno-units/tests/vectors/derived-v2/` y `derived-v2/db-with-vector`. No hay ningún fichero con ese nombre. El ADR está aceptado, así que no se edita (regla del índice): E9-01 reconstruye el fixture desde `d01f646` o deja constancia de que falta.

e. **ADR-0014 se contradice a sí mismo.** El encabezado dice «Aceptado (opción 2; evidencia de E4-02)», y en el cuerpo, línea 113, dice «Sigue *Propuesto*». E9-00 corrige el cuerpo; no cambia la decisión.

f. **Línea de ADR-0009.** El punto C-5 cita `crates/kerno-storage/src/db.rs:2009` para `compact_range`. Hoy está en la línea 2011. El texto de ADR-0009 no se edita para esto; se corrige la cita en el cierre de la etapa (E9-CL) como errata, no como cambio de decisión.

g. **Numeración de la adaptación.** En este repositorio, ADR-0012 es la metodología de benchmarks (`docs/adr/0012-metodologia-de-benchmarks.md`) y la adaptación operacional e invalidación es ADR-0016 (roadmap, Etapa 6). Si hubo en algún momento otra numeración para la adaptación, no está en `docs/adr/`. ADR-0016 figura como «Aceptado (2026-11-09)», con «Decisores: Propietario del proyecto» pero sin referencia a un acta de aprobación; el propietario pidió, en la sesión que envió esta tarea, revisar sus garantías G1 a G7 antes de aprobarlo; ver D16.

h. **Estado de la capa de unidades.** El [cierre del adelanto](etapa9-adelanto-cierre.md) §3, H-13, dice que K-E3-7 no se ha revisado. Esta acta lo confirma: no hay revisión escrita de las lecturas de `kerno-units` frente a `verify_units` (E9-16).

### 2.4 Fricción de E8-09: disposición de los quince puntos

El ejemplo de [`examples/agent-memory/FRICTION.md`](../../examples/agent-memory/FRICTION.md) es el caso de uso real que pide el roadmap antes de 1.0. Cada punto recibe una disposición: **en alcance** (un paquete lo resuelve y E9-21 lo vuelve a probar), **límite declarado** (se documenta y no cambia la API en esta etapa) o **retenido** (depende de una decisión de §8 que el propietario toma en persona).

| # | Qué dice la lista | Qué muestra el código | Disposición |
|---|---|---|---|
| 1 | `DerivedDb` no lista, consulta ni lee historial | Confirmado (§2.2) | **En alcance**: E9-09 (`snapshot`, `query` y `history` delegan en `UnitSnapshot`) |
| 2 | No hay forma de hallar los derivados de una unidad | `dependents_of_unit` existe en `DerivedRegistry` (`derived.rs:293`) | **En alcance**: E9-09 (`derived_of(unit)`) |
| 3 | El id derivado cambia en cada reconstrucción; la ruta atómica deja basura | La identidad incluye la revisión fuente (I-38, por diseño de ADR-0020). `replace_vector` es `pub(crate)` (`vector_store.rs:1083`) | **Parcial**: E9-10 hace pública la sustitución atómica. Cambiar la identidad es un cambio de formato, retenido con D12 |
| 4 | **(promesa)** El olvido es solo lógico; no hay vía pública de compactación | §2.2 y §2.3 a | **En alcance**: E9-07 (`Reclaim` y `reclaim()`), con el texto de ADR-0009 (D14) |
| 5 | Vectores persistidos y resúmenes no se abren juntos | Los resúmenes son solo en memoria (cierre E8, §5.1) | **Límite declarado** (D8). La aplicación guarda sus definiciones |
| 6 | Ids de 16 bytes opacos; fuente de ids antes de abrir | `UnitId` opaco (`types.rs:9`). El ADR-0005 es el que decide la forma del identificador | **Retenido** con D1 |
| 7 | `create` devuelve `UnitId`; `put_vector` pide `RevisionId` | `create` devuelve `UnitId` (`db.rs:1133`); unas 97 llamadas en las pruebas de `kerno-units` | **En alcance**: E9-10 añade `RevisionId::first(unit)` (sin cambiar `create`) |
| 8 | Sustituir un vector obsoleto con uno calculado fuera pasa por «recalcular» | `replace_vector` no es pública (`vector_store.rs:1083`) | **En alcance**: E9-10 |
| 9 | Dos mundos de vectores: la búsqueda copia todos los vectores en cada llamada | Confirmado en el ejemplo; la búsqueda usa `VectorSet` en memoria | **En alcance**: E9-09 (el conjunto se construye una vez) |
| 10 | La receta no liga `dim` con el vector producido | Sin validación en `put_vector` | **En alcance**: E9-10 (rechazo si `dim` no coincide con la longitud) |
| 11 | `DerivedDb::close` descarta el informe de cierre | Devuelve `Result<(), UnitError>` (`derived_stale.rs:236`); el informe es de `kerno-storage` | **Límite declarado** (D9). La capa no depende de `kerno-storage` |
| 12 | Tres tipos de error sin un tipo común; mensajes sin ids | Tres tipos (`UnitError`, `DerivedStaleError`, `ConsolidationError`). `DerivedStaleError` no nombra la unidad | **En alcance** parcial: E9-11 mejora el mensaje de `DerivedStaleError` (F-02 y F-03 de E8). El tipo común es **límite declarado** (D9) |
| 13 | Abrir requiere tres crates y un objeto de rasgo; no hay `open_dir` | `kerno-units` no depende de `kerno-storage` (`Cargo.toml`, con la nota de que la capa «never depends on it») | **Límite declarado** (D9) |
| 14 | Segunda apertura: «bloqueada» no se distingue de «corrupta» | **Falso en el código** (§2.3 b): `Error::Locked` existe y se conserva | **En alcance**: E9-11 añade `UnitError::is_locked()`; FRICTION se corrige en E9-21 |
| 15 | El tipo de unidad del catálogo comparte espacio con la aplicación; el historial crece | `UnitDb::set_policy(unit_type, p, meta)` existe (`db.rs:619`) y `run_retention` está en `retention.rs:391` | **Resuelto por la API existente**: la aplicación fija `KeepLast(1)` en su tipo. Se documenta en E9-21 |

Además, la regla de envoltura (§2.2, «Envoltura del motor») no aparece en la lista de fricción: sale del estudio de la Etapa 8 y se trata en E9-08.

### 2.5 Fuera de alcance

- **Cifrado en reposo, claves y borrado criptográfico.** Retenido (D2 a D4). Mientras no se decidan, K-1 a K-4 de ADR-0009 no se pueden medir.
- **Identificadores públicos con token.** Retenido (D1). Sin el codec, I-27 no se verifica.
- **Función de copia de seguridad.** No la hay y no se crea (D18). El plazo de las copias no se promete.
- **Índice vectorial aproximado, búsqueda textual, fusión híbrida, resumidor real, relaciones inferidas.** Son de otras etapas o no tienen benchmark que los justifique (ADR-0011; E8 §2.3, D8 y D12 de E8).
- **Cambios de `Config::default`** de `kerno-storage` y de `Config::limits`.
- **Ranking por defecto.** Sigue apagado y experimental (D3 de E8 y decisión del 2026-11-12).
- **Garantía de borrado físico** en cualquier forma. ADR-0009 opción 3 no la contiene.
- **Crate de fachada `kerno`.** ADR-0010 deja la fachada para cuando haya API estable (D9).
- **Revisión de la lógica de K-E3-7 con cambio de código.** Esta etapa solo la revisa (E9-16). Si la revisión encuentra un defecto, se abre un paquete nuevo con su propio criterio, no se corrige dentro de E9-16.

## 3. Paquetes de trabajo

Cada paquete con código cumple §4. Cada uno escribe **solo** en los ficheros que indica. Donde dos paquetes deben añadir una línea al mismo `lib.rs`, el orden de fusión está en §3.1. Los paquetes de documentos no tocan código.

| WP | Contenido | Depende de | Escribe (exacto) | Puede correr junto a |
|---|---|---|---|---|
| **E9-00** Puerta de entrada | Aprobación de esta acta por delegación; ADR-0012 aceptado en su archivo (D5); cuerpo de ADR-0014 alineado con su encabezado (§2.3 e); decisiones de §8 registradas en el índice | Esta acta | `docs/adr/0012-metodologia-de-benchmarks.md` (solo estado y decisión), `docs/adr/0014-representacion-fisica-de-relaciones.md` (solo la línea 113), `docs/adr/README.md` (filas 0012 y 0014), `docs/roadmap/README.md` (sección de la Etapa 9; único escritor de esa sección hasta E9-CL) | Ninguno: es la puerta |
| **E9-01** Fixtures de versiones previas (ADR-0020 T1, T5, T6; F-11) | Comprobar que los fixtures v1 de storage y units abren y dan el modelo. Reconstruir `derived-v2/` desde `d01f646` en una copia fuera del árbol y comprobar T5 (`IncompatibleLayer`, UNIT-16, `found_layer_version = 2`); si no se puede construir, declararlo | E9-00 | `crates/kerno-units/tests/layer_fixtures.rs` (nuevo), `crates/kerno-units/tests/vectors/derived-v2/` (nuevo, solo si se reconstruye), `docs/architecture/etapa9-fixtures.md` (nuevo) | E9-03, E9-04, E9-07, E9-08a, E9-10 a E9-13, E9-15 a E9-19 |
| **E9-02** Identificadores públicos (ADR-0005, I-27) | **Retenido (D1).** Si el propietario elige (a): codec y mapeo, pruebas de tokens manipulados y de estabilidad. No empieza | E9-00 y **D1 en persona** | (cuando se libere) `crates/kerno-units/src/ids.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea, último en la fusión), `crates/kerno-units/tests/public_ids.rs` (nuevo) | — |
| **E9-03** Auditoría minimizada (I-29 en units; H-6 de storage; H-8) | Prueba de centinelas de texto para errores, informes e informes de `verify` de `kerno-units`, con la misma forma de detector que storage. Fragmentos de clave y valor en `kerno-storage` (H-6). Corregir el comentario «10^4 commits» de `group_commit.rs:538` (H-8) | E9-00 | `crates/kerno-units/tests/redaction_units.rs` (nuevo), `crates/kerno-storage/tests/redaction.rs` (solo añadir la búsqueda de fragmentos), `crates/kerno-storage/tests/group_commit.rs` (solo el comentario), `docs/architecture/etapa9-auditoria.md` (nuevo) | E9-01, E9-04, E9-07, E9-08a, E9-10 a E9-13 |
| **E9-04** Exportación lógica, borrador (I-30; D10) | Formato de exportación con cabecera propia (`KRNO_EXP` queda reservado sin uso, lo que se declara en el borrador). Contenido: unidades, revisiones, relaciones y políticas. No incluye derivados (ADR-0018: se reconstruyen) ni tokens (espera D1) | E9-00 | `crates/kerno-units/src/export.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea, primero en la fusión), `docs/file-format/export-v0-borrador.md` (nuevo) | E9-01, E9-03, E9-07, E9-08a, E9-10 a E9-13, E9-15 a E9-19 |
| **E9-05** Importación en motor nuevo | Importar el borrador en un directorio nuevo con `emulated_log` y con `kerno_db`; el estado lógico es el mismo. Errores de formato con la clase de `UnitError` | E9-04 | `crates/kerno-units/src/import.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea, segundo en la fusión), `crates/kerno-units/tests/export_import.rs` (nuevo) | E9-03, E9-07, E9-10 a E9-13 |
| **E9-06** Independencia del formato físico (I-30) | Exportar antes y después de flush, de `compact_range` público y de cierre; los bytes del borrador son iguales | E9-05, E9-07 | `crates/kerno-units/tests/export_physical.rs` (nuevo) | E9-12, E9-13, E9-15 a E9-19 |
| **E9-07** Reclamación física pública (ADR-0009 opción 3; FRICTION 4; D14) | Trait público `Reclaim` en `kerno-core` con `compact_range`, que `TestHooks` deja como estaba. `impl Reclaim for KernoDb` delega en el método existente. `UnitDb<E: Reclaim>::reclaim()` y `DerivedDb<E: Reclaim>::reclaim()` con el texto de ADR-0009 | E9-00 | `crates/kerno-core/src/contract.rs` (solo el trait nuevo), `crates/kerno-storage/src/db.rs` (solo el `impl Reclaim for KernoDb`; el método inherente no se toca), `crates/kerno-units/src/reclaim.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea, tercero en la fusión), `crates/kerno-units/tests/reclaim.rs` (nuevo) | E9-01, E9-03, E9-04, E9-08a, E9-10 a E9-13, E9-15 a E9-19 |
| **E9-08a** Envoltura: accesores de `UnitDb` (lección de E8) | Clasificar los usos de `engine()` por método (tabla en el documento) y añadir los accesores que los usos necesitan. `engine()` no cambia en este paquete | E9-00 | `crates/kerno-units/src/db.rs` (solo accesores nuevos), `docs/architecture/etapa9-usos-de-engine.md` (nuevo) | E9-01, E9-03, E9-04, E9-07, E9-10 a E9-13, E9-15 a E9-19 |
| **E9-08b** Migrar pruebas, grupo 1 | Sustituir `engine()` por los accesores, en 49 usos | E9-08a | `crates/kerno-units/tests/index.rs`, `crates/kerno-units/tests/relations.rs`, `crates/kerno-units/tests/assoc_indexed.rs`, `crates/kerno-units/tests/assoc_reference.rs`, `crates/kerno-units/tests/harness/units.rs` | E9-08c |
| **E9-08c** Migrar pruebas, grupo 2 | Lo mismo, en 33 usos | E9-08a | `crates/kerno-units/tests/history.rs`, `tests/verify.rs`, `tests/unit_db.rs`, `tests/policies.rs`, `tests/observation.rs`, `tests/sentinels.rs`, `tests/cache_sentinels.rs`, `tests/retention.rs` | E9-08b |
| **E9-08d** Migrar pruebas, grupo 3 y ejemplos | Lo mismo, en 51 usos, incluidos los ejemplos de medición. Un ejemplo de medición no cambia lo que mide | E9-08a | `crates/kerno-units/tests/derived_suppression.rs`, `tests/derived_vectors.rs`, `tests/derived_measure.rs`, `crates/kerno-units/examples/index_bench.rs`, `examples/relation_bench.rs`, `examples/unit_bench.rs`, `crates/kerno-units/src/retention.rs` (campo interno en lugar de `engine()`) | E9-08b y E9-08c |
| **E9-08e** Retirar `engine()` | `UnitDb::engine` deja de ser `pub`; `grep -rn "pub fn engine" crates/kerno-units/src` sin resultados | E9-08b, E9-08c, E9-08d | `crates/kerno-units/src/db.rs` (solo la retirada) | — |
| **E9-08f** Revisión de E9-08a: escritura cruda y ganchos bajo `test-support` (D23) | `raw_commit` y los ganchos `hook_flush`, `hook_wait_idle` y `hook_compact_range` pasan a existir solo con la característica `test-support`, apagada por defecto, y en las pruebas unitarias del crate; las pruebas de integración y los ejemplos que los llaman declaran `required-features`. Las lecturas crudas comprueban el envenenamiento. Pruebas de los accesores más estrictas | E9-08e | `crates/kerno-units/Cargo.toml` (la característica y los `required-features`), `crates/kerno-units/src/db.rs` (accesores y `accessor_tests`), `crates/kerno-units/src/reclaim.rs` y `crates/kerno-units/src/retention.rs` (acceso interno al motor), `crates/kerno-units/examples/index_bench.rs`, `relation_bench.rs` y `unit_bench.rs` (la orden de ejecución lleva `--features test-support`), `docs/architecture/etapa9-usos-de-engine.md`, `docs/architecture/etapa9-acta.md` (D23) | — |
| **E9-08g** Hallazgos menores de la revisión de E9-08f | `UnitDb::compact` pasa de `E: TestHooks` al rasgo público `Reclaim`, el mismo de `reclaim`, con los mismos límites y la misma comprobación de envenenamiento. Los comentarios y el documento nombran las dos llamadas públicas de reclamación (`reclaim`, todo el espacio de claves; `compact`, los rangos de revisiones). En `accessor_tests`: la secuencia de un `raw_commit` de varias entradas avanza lo de un commit; `hook_compact_range` depende de sus límites; el bucle sobre `memtable_rotations` está acotado y un contador ausente falla | E9-08f | `crates/kerno-units/src/retention.rs` (solo `compact`), `crates/kerno-units/src/db.rs` (comentario de los ganchos y `accessor_tests`), `crates/kerno-units/Cargo.toml` (solo el comentario de la característica), `docs/architecture/etapa9-usos-de-engine.md`, `docs/architecture/etapa9-acta.md` (§3, §3.1 y D23) | — |
| **E9-09** Lectura de derivados (FRICTION 1, 2, 9) | `DerivedDb::snapshot()`, `query(&UnitFilter)`, `history(unit)`, `derived_of(unit)` y búsqueda sobre vectores persistidos que construye el `VectorSet` una vez por lote de consultas | E9-08e | `crates/kerno-units/src/derived_query.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea, cuarto en la fusión), `crates/kerno-units/tests/derived_query.rs` (nuevo) | E9-10 a E9-13 |
| **E9-10** Sustitución y receta de vectores (FRICTION 3, 7, 8, 10; F-04 de E8) | Hacer pública la sustitución atómica (`replace_vector`) con el nombre que resulte del paquete. `put_vector` rechaza `dim` distinto de la longitud del vector. `RevisionId::first(unit)`. Prueba de F-04: cambiar solo `profile` cambia la identidad | E9-00 | `crates/kerno-units/src/vector_store.rs`, `crates/kerno-units/src/types.rs`, `crates/kerno-units/tests/derived_replace.rs` (nuevo) | E9-01, E9-03 a E9-07, E9-12, E9-13 |
| **E9-11** Errores (FRICTION 12, 14; F-02 y F-03 de E8) | `UnitError::is_locked()`. Mensajes de `DerivedStaleError` con el id de la unidad. `valid_inputs` (`read_input` que devuelve `None`) pasa a error explícito, no a omisión silenciosa | E9-00 | `crates/kerno-units/src/error.rs`, `crates/kerno-units/src/derived_stale.rs` (solo `Display` y `valid_inputs`), `crates/kerno-units/tests/errors_api.rs` (nuevo) | E9-01, E9-03 a E9-07, E9-12, E9-13 |
| **E9-12** Estrés de derivados (F-09, parcial) | 4 hilos escriben cambios derivados y otros leen; invariante: ningún derivado `ready` con una fuente cambiada. Sin umbral de tiempo; el sondeo espera una condición, sin `sleep` | E9-00 | `crates/kerno-units/tests/derived_stress.rs` (nuevo) | E9-01, E9-03 a E9-07, E9-10, E9-11, E9-13 |
| **E9-13** Propiedades de los decodificadores de `kerno-units` (D21) | Mutación de bytes de los registros de unidades, relaciones, descriptores y vectores, con `proptest` (ya es dependencia de desarrollo). 2 000 casos por tipo y modo; ningún pánico; errores solo de clase corrupción o incompatibilidad | E9-00 | `crates/kerno-units/tests/decoders_robust.rs` (nuevo) | E9-01, E9-03 a E9-07, E9-10, E9-11 |
| **E9-14** Fuzzing de `kerno-storage` sobre el código final (H-7) | Corrida de 30 minutos por objetivo de los 15, sin escribir en el corpus salvo entradas que fallen. Corre con la máquina libre (§3.2) | E9-07 | `crates/kerno-storage/fuzz/README.md` (sección nueva con la corrida), `crates/kerno-storage/fuzz/corpus/` (solo entradas nuevas que fallen) | Ninguno: uso exclusivo de la máquina |
| **E9-15** Evidencia de mutación de E8-06 y E8-07 (F-10; D20) | Mutar cada prueba nueva de `consolidation.rs` y de `derived_measure.rs` en una copia, registrar línea y salida | E9-00 | `docs/architecture/etapa9-mutaciones-e8.md` (nuevo) | E9-01, E9-03 a E9-07, E9-16, E9-18 (sin compilar a la vez: §3.2) |
| **E9-16** Revisión de K-E3-7 (H-13; D20) | Leer las lecturas de `kerno-units` frente a `verify_units`, con la tabla de cada lectura y lo que comprueba. Sin cambio de código; un defecto encontrado abre un paquete nuevo | E9-00 | `docs/architecture/etapa9-k-e3-7.md` (nuevo) | Todos los de documentos |
| **E9-17** Comparación con SQLite, nueva pasada (sin umbrales; D6) | Usar el binario existente sin cambios (`spike-bench`), en calma (§3.2, T-SQ1). Sin nombrar terceros que el límite de §5 prohíbe | E9-00 | `spikes/storage/results/2026-11-12-e9/` (nuevo), `docs/testing/sqlite-report-e9.md` (nuevo) | E9-16, E9-18 |
| **E9-18** Nombres de terceros en el informe actual (restricción de §5, T-NM) | Quitar del informe los nombres de terceros distintos de SQLite que el límite prohíbe (el modelo de CPU, las versiones de los crates de enlace) y, por prudencia, los que describen el entorno sin ser necesarios (sistema de archivos, núcleo, reloj de CPU, versión del compilador). Dejar lo que reproduce el resultado: el commit y los comandos | E9-00 | `docs/testing/sqlite-report.md` | E9-15 a E9-17 |
| **E9-19** Conjunto E8-10, congelación (sin medir; D22) | Corpus, consultas y juicios con semilla propia, con vector de consulta en cada una de las cuatro clases (§5, T-E10). Validación de no saturación con el brazo sin señal, antes de congelar; si supera 0,96 se regenera con otra semilla. `MANIFEST` SHA-256 | E9-00 | `crates/kerno-testkit/src/derived_eval2.rs` (nuevo), `crates/kerno-testkit/src/lib.rs` (una línea; único escritor en la etapa), `crates/kerno-testkit/data/derived-eval-2/` (nuevo), `crates/kerno-testkit/tests/derived_eval2.rs` (nuevo), `crates/kerno-testkit/examples/freeze_derived_eval2.rs` (nuevo), `docs/architecture/etapa9-conjunto-evaluacion.md` (nuevo) | E9-01, E9-03 a E9-07, E9-10 a E9-13, E9-15, E9-16, E9-18 |
| **E9-20** Medición E8-10 (una sola vez) | Ejecutar las medidas de §5 sobre el conjunto de E9-19, con la máquina en calma. La salida cruda queda comprometida. Ningún umbral ni ningún fichero se cambia después de medir | E9-19 y E9-08e (para no medir con API en transición) | `crates/kerno-units/tests/derived_measure2.rs` (nuevo), `crates/kerno-units/examples/derived_bench2.rs` (nuevo), `docs/architecture/etapa9-medicion.md` (nuevo), `docs/architecture/etapa9-medicion-salida.txt` (nuevo) | Ninguno: uso exclusivo de la máquina |
| **E9-21** Revisión del caso de uso con la API nueva (FRICTION) | Reescribir el ejemplo con la API de E9-07 a E9-11 y los accesores de E9-08a; actualizar la tabla de §2.4 en la lista de fricción con la disposición de cada punto | E9-07, E9-08e, E9-09, E9-10, E9-11 | `examples/agent-memory/src/**`, `examples/agent-memory/tests/**`, `examples/agent-memory/FRICTION.md`, `examples/agent-memory/README.md` | E9-14, E9-16 a E9-18 |
| **E9-CL** Cierre | Revisión adversarial de E9-01 a E9-21; comprobación de §4 y §5; invariantes (§6); threat model (T-03, T-04, T-09, T-10 y el límite de H-2); DESIGN de `kerno-storage` (H-9); erratas de §2.3 c y f; SAD como versión 0.4.0 con su historial; roadmap | Todos los anteriores que se ejecuten | `docs/architecture/etapa9-cierre.md` (nuevo), `docs/architecture/invariants.md`, `docs/security/threat-model.md`, `docs/architecture/SAD.md` (versión 0.4.0), `docs/architecture/etapa9-adelanto-acta.md` (solo H-12), `crates/kerno-storage/DESIGN.md` (solo H-9), `docs/roadmap/README.md` (sección de la Etapa 9) | — |

### 3.1 Orden de fusión de las líneas `mod` de `lib.rs`

- **`crates/kerno-units/src/lib.rs`**: E9-04, luego E9-05, luego E9-07, luego E9-09. E9-02 (retenido) iría al final. Cada paquete comprueba el conteo de pruebas después de su fusión (§4.2).
- **`crates/kerno-testkit/src/lib.rs`**: solo E9-19.
- **`crates/kerno-core/src/contract.rs`**: solo E9-07.
- **`crates/kerno-units/src/db.rs`**: E9-08a, luego E9-08e, luego E9-08f, luego E9-08g. Además, la integración de E9-07 con E9-08e añadió el accesor interno `inner_engine()`, que usan `reclaim` y `compact`. Ningún otro paquete.
- **`crates/kerno-units/src/vector_store.rs`**, **`types.rs`**, **`error.rs`**, **`derived_stale.rs`**: un solo paquete cada uno (E9-10 y E9-11). Ningún otro paquete de la etapa los escribe.
- **`crates/kerno-storage/src/db.rs`**: solo E9-07, solo el `impl Reclaim`.
- **`crates/kerno-storage/tests/redaction.rs`**, **`group_commit.rs`**: solo E9-03.

### 3.2 Qué corre a la vez, y la regla de la máquina pequeña

- **Lanes independientes:** (1) API de la capa: E9-08a → (E9-08b ∥ E9-08c) → E9-08d → E9-08e → E9-09 → E9-21. (2) Exportación: E9-04 → E9-05 → E9-06 (esperando a E9-07). (3) Reclamación: E9-07. (4) Pruebas nuevas sin conflicto de fichero: E9-01, E9-03, E9-10, E9-11, E9-12, E9-13. (5) Documentos, sin compilar: E9-00, E9-16, E9-18. (6) Conjunto: E9-19 → E9-20.
- **Regla de la máquina de 4 CPU con poco disco:** como mucho **dos trabajos de compilación a la vez**. Los trabajos que miden tiempos o que duran horas corren **solos**: E9-14 (fuzzing), E9-17 (SQLite), E9-20 (medición de E8-10) y E9-15 (mutaciones, que compilan una vez por mutación). Antes de cada medición se anota la carga (`uptime`, `load1`) y el espacio libre.
- Todo se compila y se prueba **solo en release** (`--release`), como exige §4.11.

### 3.3 Camino crítico

**E9-00 → E9-08a → (E9-08b ∥ E9-08c) → E9-08d → E9-08e → E9-09 → E9-21 → E9-CL.** Es la cadena de código más larga. En tiempo de reloj, el cuello es E9-14: 15 objetivos de 30 minutos cada uno (unas 7,5 horas de cómputo exclusivo), que puede empezar en cuanto E9-07 esté fusionado y necesita la máquina libre. E9-20 espera a E9-19 y a E9-08e; E9-CL espera a todos los demás.

Los paquetes retenidos (E9-02) no están en el camino crítico: su retención no bloquea el cierre de la etapa, solo deja I-27 sin verificar (§6) y el cierre lo declara.

## 4. Criterios de aceptación comunes

Cada paquete con código cumple lo siguiente. Un paquete que no lo cumple no se cierra.

1. **Formato y lint.** `cargo fmt --all --check` y `cargo clippy --workspace --all-targets --all-features -- -D warnings` sin errores. Clippy rechaza los tipos de retorno complejos (`type_complexity`): usar alias de tipo.
2. **Conteo de pruebas.** `cargo test -p <crate> --all-features --release` en verde, con el conteo anotado antes y después. El fichero nuevo debe aparecer en el conteo: si el número no se mueve, el runner no lo ve. El conteo no baja respecto de la línea base (T-CNT), salvo prueba retirada con su razón escrita.
3. **Los dos motores.** Toda prueba nueva que use un motor se ejecuta con `emulated_log` y con `kerno_db`, como en las Etapas 4 a 8.
4. **Cada prueba nueva se ve fallar.** Mutación aplicada a una **copia** del árbol, revertida y comprobada con `git diff`. El registro guarda archivo, línea, cambio, comando y la línea `FAILED` de la salida. Una mutación que deja la prueba en verde es un hallazgo, no un paso (lección de F-10 de E8).
5. **Ruta de referencia.** Donde un derivado o una exportación responde a una consulta, la respuesta se compara con la ruta de referencia sobre la misma instantánea (I-32).
6. **Sin dependencias de orden.** Ninguna prueba depende de tiempo, de orden entre pruebas ni de estado compartido. Sin `sleep` para esperar: se sondea una condición.
7. **Sin contenido fuera de `diagnostic`** (I-37) en observaciones y explicaciones; ningún centinela de contenido en errores, informes ni métricas (I-29).
8. **Sin cambios de dependencias.** Ningún `Cargo.toml` ni `Cargo.lock` cambia salvo que una decisión lo pida (I-31). Ningún paquete de §3 lo pide.
9. **Regla de la envoltura** (lección de E8 y de §2.2). Ninguna función pública devuelve el motor, una referencia a él ni el `UnitDb` que guarda un tipo público. Los accesores devuelven datos, no el objeto que guardan (E9-08).
10. **Revisión independiente antes de fusionar.** Cada paquete con código lo revisa alguien distinto de su autor. La revisión deja registro: hallazgos, su estado y la evidencia. No se fusiona con un hallazgo bloqueante abierto.
11. **Máquina pequeña.** Compilar y probar solo en release; un trabajo pesado a la vez (§3.2); espacio libre comprobado antes de cada corrida; carga anotada antes de medir.
12. **Lenguaje.** El código, los comentarios y los nombres de fichero siguen el idioma que ya usa el repositorio (español en `docs/`, identificadores en inglés como en el código). Los informes de trabajo se escriben en inglés.
13. **Lo aceptado no se edita.** Un ADR aceptado no se reescribe para cambiar su decisión; una errata de cita se corrige en el cierre con una nota (§2.3 f). Un fixture referenciado por un ADR aceptado se crea o se declara ausente, no se cambia la cita.

## 5. Umbrales por tema, fijados antes de medir

Ningún umbral de esta sección se cambia después de medir. Un incumplimiento se registra como tal; no se rebaja. Cada umbral se comprueba contra la cota de su métrica. **No hay umbral absoluto de latencia** (D7).

| Código | Tema | Umbral | Cota de la métrica y comprobación |
|---|---|---|---|
| **T-EX1** | Exportación (I-30) | Diferencias del estado lógico entre origen y reimportación = 0 (unidades, revisiones, relaciones, políticas) | Contador entero ≥ 0; el umbral es igualdad a 0 |
| **T-EX2** | Independencia física (I-30) | Bytes del borrador iguales antes y después de flush, `reclaim` y cierre | Igualdad de bytes; no hay tiempo |
| **T-RE1** | Reclamación (ADR-0009, C-1 y C-3) | Archivos vivos que contienen un centinela tras `reclaim` y cierre, sin instantánea abierta = 0 | Contador entero; cota inferior 0 |
| **T-RE2** | Reclamación con instantánea (ADR-0009, C-1) | Archivos vivos con el centinela ⊆ archivos que retiene la instantánea | Igualdad de conjuntos. **No se usa la cláusula de conteo de C-4** (D17) |
| **T-FZ1** | Fuzzing de `kerno-storage` (E9A-04) | 30 minutos por objetivo; fallos = 0; entradas de más de 1 s = 0 | Tiempo fijado en 1 800 s por objetivo; contadores enteros |
| **T-PR1** | Propiedades de `kerno-units` (E9-13) | 2 000 casos por tipo de registro y modo; pánicos = 0; errores solo de clase corrupción o incompatibilidad | Contadores; el conjunto de clases es finito |
| **T-ST1** | Estrés de derivados (E9-12) | Violaciones de invariantes = 0 en 200 ciclos con 4 hilos | Contador; no hay umbral de tiempo, y la duración se informa sin cota |
| **T-FR1** | Fricción (§2.4) | 15 de 15 puntos con disposición (en alcance con prueba, límite declarado o retenido con su decisión) | Cota 15 |
| **T-EN1** | Envoltura (lección de E8) | `grep -rn "pub fn engine" crates/kerno-units/src` sin resultados | Contador = 0 |
| **T-MU1** | Mutación (lección de F-10) | Pruebas nuevas con su mutación registrada = pruebas nuevas | Igualdad de contadores |
| **T-SQ1** | Informe SQLite (D6) | Carga de 1 minuto (`load1`) < 4 al inicio de cada bloque de medición (el número de CPU). Sin umbral de latencia | Precondición de la máquina, no resultado. Si no se cumple, el bloque se repite en calma y se anota |
| **T-NM** | Nombres de terceros en el informe | El informe no nombra terceros distintos de SQLite, ni la CPU, ni versiones de crates de enlace | Búsqueda de los nombres prohibidos: sin resultados. (El límite viene de la nota de restricciones del proyecto; si el propietario lo interpreta de otro modo, se decide en E9-18) |
| **T-CNT** | Conteo de pruebas | `kerno-units` + `kerno-testkit` ≥ 856; `kerno-storage` ≥ 672 (citados, no reejecutados) | Cota inferior; el conteo sube con cada paquete |
| **T-E10-V1** | E8-10: validez de consultas | ≥ 25 consultas con vector de consulta en **cada una** de las 4 clases | Cuenta entera; 25 es alcanzable porque el conjunto se congela con ese mínimo |
| **T-E10-V2** | E8-10: no saturación | nDCG@10 **sin señal**, en conjunto ≤ 0,96. Si lo supera en la validación previa a la congelación, el conjunto se regenera con otra semilla antes de congelarlo | nDCG ∈ [0, 1]. Con 0,96, la mejora de 0,02 de T-E10-S1 es alcanzable (cota 0,98) |
| **T-E10-Q1** | E8-10: recuperación exacta | Recall@10 de la búsqueda exacta persistida frente a fuerza bruta = 1,000 | Recall ∈ [0, 1]; 1,000 es alcanzable porque es la misma búsqueda |
| **T-E10-Q2** | E8-10: calidad con señal | nDCG@10 con señal ≥ 0,80, en conjunto | nDCG ∈ [0, 1] |
| **T-E10-Q3** | E8-10: recuperación con señal | Recall@10 con señal ≥ 0,90, en conjunto | Recall ∈ [0, 1] |
| **T-E10-S1** | E8-10: superioridad (sustituye a Q4 en conjunto) | nDCG@10 con señal − nDCG@10 sin señal ≥ 0,02, en conjunto | La diferencia ∈ [−1, 1]. Con T-E10-V2 la cota es 0,98: 0,02 es alcanzable |
| **T-E10-S2** | E8-10: no inferioridad por clase (sustituye a Q4 por clase) | nDCG@10 con señal − sin señal ≥ −0,01, en **cada una** de las 4 clases | Cota inferior −1; −0,01 es un margen estrecho pero alcanzable. Con T-E10-V1 cada clase se puede calcular |
| **T-E10-Q5** | E8-10: validez de juicios | Unidades sin juzgar en el top 10 ≤ 10 % de las posiciones, por brazo y por clase | Fracción ∈ [0, 1] (como Q5 de E8) |
| **T-E10-R** | E8-10: congelación | `MANIFEST` SHA-256 coincide antes de medir | Igualdad |

**Sobre T-E10:** E8-10 no reemplaza los umbrales Q1 a Q5 de E8: los mantiene, y sustituye a Q4 con S1 y S2, que sí se pueden calcular en las cuatro clases porque E9-19 pone vector de consulta en todas ellas. Así se cierra F-06 de E8. Q2 y Q3 siguen con su valor absoluto, pero T-E10-V2 es lo que evita el efecto techo de F-07.

Sin umbral absoluto de latencia (D7). ADR-0011 dice que un límite incumplido abre una decisión nueva; esta etapa no fija ninguno.

## 6. Invariantes

| ID | Estado al abrir la etapa | Lo que hace la Etapa 9 | Paquete que la verifica |
|---|---|---|---|
| **I-27** | Fase 4, sin verificar | Un token manipulado se rechaza y un token válido no autoriza. **Solo si D1 se decide** | E9-02 (retenido). Sin D1, queda sin verificar y el cierre lo dice |
| **I-28** | Fase 4, sin verificar | Las claves no están en el directorio de datos. **Sin cifrado en 1.0 (D3) no hay claves**: la invariante no aplica. El cierre comprueba que `verify()` no encuentra archivos desconocidos en el directorio, que es lo que se puede decir hoy | E9-CL (D19); la prueba de archivos desconocidos ya existe en `lifecycle_soak.rs` |
| **I-29** | Verificada en `kerno-storage` (promovida el 2026-11-12) | Se extiende a `kerno-units` (errores, informes, observaciones) y a fragmentos en storage | E9-03 |
| **I-30** | Fase 4, sin verificar | La exportación es independiente del formato físico y se puede reimportar en un motor nuevo | E9-04, E9-05, E9-06. La **congelación** del formato es retenida (D12) |
| **I-31** | Verificada por revisión de dependencias | Sin red ni modelo en el núcleo; sin cambios de manifiestos | E9-CL (`git log 52f026f..HEAD -- Cargo.toml Cargo.lock`, que sigue sin commits) |
| **I-42** (propuesta, verificada en E8) | Pendiente de aprobación | Suprimir una fuente deja sin visibilidad a sus derivados en el mismo `ChangeSet`. Se vuelve a verificar con `reclaim` | E9-07 (prueba que la reclamación no altera el `ChangeSet`) y E9-CL. Promoción por D19 |
| **I-43** (propuesta, verificada en E8) | Pendiente de aprobación | Un resumen nunca reemplaza el original mientras exista | E9-CL, con la prueba de E8 y la de E9-11 (mensajes). Promoción por D19 |
| **I-24** | Verificada en índices y derivados | La reconstrucción de un vector sustituido es igual a construirlo de nuevo | E9-10 |
| **I-25** y **I-38** | Verificadas para vectores persistidos; la prueba que varía solo el perfil falta (F-04 de E8) | Cambiar solo `profile` cambia la identidad | E9-10 |
| **I-32** | Verificada | Las consultas por derivados y por búsqueda vectorial coinciden con la ruta de referencia | E9-09 (diferencial) |
| **I-37** | Verificada en observaciones y derivados | Sin contenido en observaciones, explicaciones ni errores nuevos | E9-03, E9-09, E9-11 |
| **I-N1** (**nueva**, propuesta: reclamación) | No existe | La reclamación pública no promete borrado físico y solo retira archivos vivos que ninguna instantánea retiene (ADR-0009, puntos 2 y 3) | E9-07 |
| **I-N2** (**nueva**, propuesta: envoltura) | No existe | Ninguna función pública de la capa devuelve el motor que guarda | E9-08e (T-EN1) |

Las nuevas (I-N1 e I-N2) se numeran al cerrar la etapa, en [invariantes](invariants.md), con los números libres que haya entonces. Esta tabla usa sus nombres provisionales.

## 7. Riesgos

| ID | Riesgo | Tratamiento |
|---|---|---|
| **R-E9-1** | Una decisión retenida (D1 a D4, D12, D15, D16) se toma por inercia en un paquete, y cambia el formato o una promesa sin que el propietario lo haya decidido | §8.1 la marca como retenida; ningún paquete que dependa de ella escribe antes de la decisión; la revisión de cada paquete comprueba que no toca formato |
| **R-E9-2** | Una exportación en borrador se convierte en una promesa de hecho, porque alguien la usa | El borrador va marcado como no congelado en su cabecera y en la documentación (E9-04); la congelación es D12 |
| **R-E9-3** | La reclamación pública se lee como borrado seguro | El texto de la API y de la documentación repite el de ADR-0009 y el threat model §6.5; la prueba de E9-07 mide solo lo que mide ADR-0009 |
| **R-E9-4** | El cambio de `engine()` rompe muchas pruebas y las arregla con parches que ocultan la regla | §3 divide la migración en cinco paquetes; E9-08e solo retira la función cuando el grep da cero; cada migración la revisa alguien distinto de su autor |
| **R-E9-5** | El fuzzing de 30 minutos sobre el código final tarda 7,5 horas de máquina exclusiva | Se planifica con la máquina libre (§3.2); si no cabe, el cierre dice que T-FZ1 no se ha repetido, como hizo el adelanto |
| **R-E9-6** | La medición con la máquina cargada da números que no sirven, como en la Etapa 8 | T-SQ1 (precondición de carga) y ejecución en solitario; los números se informan con su carga |
| **R-E9-7** | El conjunto de E8-10 se ajusta a la implementación, como pasó con el de E7 | Semilla, permutación de identificadores y juicios fijados antes de congelar; `MANIFEST`; juicios escritos desde el relato y no desde el código; T-E10-V2 hace la comprobación antes de la congelación, no después |
| **R-E9-8** | Un brazo de E8-10 sin vector de consulta en una clase vuelve a dejar Q4 sin medir | T-E10-V1 exige 25 por clase, y E9-19 no se congela sin ellas |
| **R-E9-9** | La mutación de las pruebas nuevas no se registra, como en E8 (F-10) | T-MU1 y §4.4; el paquete no se cierra sin el registro |
| **R-E9-10** | K-E3-7 tiene un defecto real, y la revisión lo encuentra tarde | E9-16 se hace al principio (sin dependencias); un defecto abre un paquete propio |
| **R-E9-11** | H-2 (filtro reescrito con CRC válido) se lee como protección del motor | El threat model §6.6 lo dice; E9-CL lo repite en el cierre; D4 lo retiene |
| **R-E9-12** | ADR-0016 se da por aprobado sin la revisión que pidió el propietario | D16 retenida; ningún paquete de la etapa depende de ADR-0016 |
| **R-E9-13** | El fixture de ADR-0020 no se puede reconstruir desde `d01f646` | E9-01 lo declara como ausente y la cita de ADR-0020 queda con su errata en el cierre; no se edita el ADR |
| **R-E9-14** | El informe SQLite conserva nombres de terceros prohibidos por la restricción del proyecto | E9-18 y T-NM; la nueva pasada (E9-17) ya nace sin ellos |
| **R-E9-15** | Una validación de `dim` (E9-10) rompe pruebas que ponen vectores de otra dimensión a propósito | El paquete revisa las pruebas antes de cambiar la validación; si una prueba depende de ello, se corrige la prueba con su razón |
| **R-E9-16** | Ampliar el fuzzing a `kerno-units` con una herramienta nueva añade una cadena de herramientas no aprobada | D21: solo pruebas de propiedades con la biblioteca que ya es dependencia de desarrollo |
| **R-E9-17** | E9-07 cambia `kerno-core` (trait `Reclaim`) y es una superficie pública | Pre-1.0: el cambio se anota en el cierre; el trait tiene un valor por defecto que devuelve «no soportado», como `TestHooks` |

## 8. Decisiones que el propietario debe tomar

Cada una tiene opciones, consecuencias y una recomendación con su razón. La elección de cada una, por delegación o retenida, está en §8.1.

| # | Decisión | Opciones y consecuencias | Recomendación y razón |
|---|---|---|---|
| **D1** | ADR-0005: codec de identificadores públicos | (a) Token aleatorio opaco con mapeo persistido (opción 1 del ADR). Añade un registro de capa `token → UnitId` en la misma transacción: cambio de formato de capa con marca de versión. (b) Token AEAD reversible (opción 2): necesita claves (D2) y tokens más largos. (c) UUIDv7 expuesto: descartado por el SAD | **(a)**. No necesita claves, un token se puede reemitir sin tocar la identidad, y su versión se fija desde el primer día. **Retenida**: cambia formato de capa y la forma del token que ven los usuarios |
| **D2** | ADR-0006: diseño de cifrado y gestión de claves | (a) AEAD por bloque (opción 1 del ADR): usa la ranura reservada de 40 bytes y un `KeyProvider` fuera del directorio. Cambia el formato (bit `AEAD`, hoy rechazado en `header.rs:197`). (b) Cifrado de sistema de archivos delegado al sistema: no cambia formato, pero no protege las copias exportadas (T-03). (c) Cifrado por registro: borrado criptográfico selectivo, más metadatos e índices | **(a)** como diseño cuando D3 se decida a favor. Hoy no se implementa. Razón: es la única opción que usa la ranura ya reservada y da integridad (D4). **Retenida**: diseño criptográfico, irreversible una vez en disco |
| **D3** | ¿Entra el cifrado en 1.0? | (a) Sí, con D2 (a): retrasa 1.0 por el diseño y por las mediciones K-1 a K-4. (b) No: 1.0 no promete confidencialidad en reposo ni gestión de claves, y lo declara. (c) No en 1.0; se añade en 1.x con su propio bit de formato | **(b)**. No hay diseño ni ADR-0006 aceptado, y las mediciones K-1 a K-4 no se pueden hacer sin cifrado. Declarar la ausencia es verdadero; prometer cifrado sin diseño sería una promesa vacía. **Retenida**: es una promesa a usuarios |
| **D4** | Integridad autenticada frente a una reescritura con CRC válido (H-2 del cierre del adelanto) | (a) No añadir integridad autenticada en 1.0. El límite queda en el threat model §6.6, y `verify()` con `deep` es el recurso. (b) Código de autenticación con clave en la ranura de 40 bytes, sin confidencialidad, con bit nuevo. (c) AEAD completo (D2 a). (b) y (c) cambian formato y exigen custodiar una clave | **(a)**. Un código de autenticación sin gestión de claves no protege frente al que la tiene, y cambiar formato sin D2 es tomar dos decisiones irreversibles a la vez. **Retenida**: cambia formato |
| **D5** | ADR-0012: metodología de benchmarks | (a) Aceptar tal como está (aplicada en M0): igualdad de condiciones, repeticiones, durabilidad equivalente y umbrales relativos. (b) Aceptar con cambios antes de medir. (c) Mantener propuesto: las mediciones de la etapa no tienen método aceptado | **(a)**. Ya se aplicó en M0 y en E8. No cambia formato ni promesa. **Tomada por delegación** |
| **D6** | Umbrales frente a SQLite: criterio o informe | (a) Informe sin umbrales, como decidió el propietario el 2026-11-11. (b) Criterio con umbrales fijados antes de medir. (b) exige una medición en calma: la de E8-S0 se hizo con carga de 5 a 14 en 4 CPU, y varios rangos se solapan | **(a)** para 1.0. Sin una línea base limpia, cualquier umbral sería una cifra elegida después de ver datos, como T4 de la Etapa 7. **Tomada por delegación** |
| **D7** | Límite absoluto de latencia | (a) Ninguno: solo cocientes y comparaciones entre rutas. (b) Un límite fijado antes de medir | **(a)**, como D9 de E8. Un límite incumplido abre una decisión nueva (ADR-0011). **Tomada por delegación** |
| **D8** | ¿Formato en disco nuevo en 1.0 para resúmenes o para auditoría? | (a) No: los resúmenes siguen en memoria y la aplicación guarda sus definiciones; no hay registro de auditoría persistido. (b) Sí: descriptores de resúmenes o un registro de auditoría con su bit de formato. (b) obliga a medir centinelas en disco (F-05 de E8) y a un ADR | **(a)**. FRICTION 5 se resuelve documentando, no con formato. **Tomada por delegación**; (b) pasaría a retenida si se pide |
| **D9** | Forma de la API pública antes de 1.0 y puntos de fricción en alcance | (a) Mantener los crates actuales, sin fachada; los cambios antes de 1.0 se permiten y se anotan en el cierre; los 15 puntos tienen disposición (§2.4). (b) Fachada única `kerno` ahora (ADR-0010 opción 1). (c) Congelar la API ya: impide resolver los puntos 1, 2, 8 y 9 | **(a)**. ADR-0010 deja la fachada para cuando haya API estable, y antes de 1.0 no hay promesa que romper. **Tomada por delegación** |
| **D10** | Formato de exportación: borrador o congelado | (a) Borrador versionado y sin congelar (`export-v0-borrador.md`). (b) Congelado ya: una promesa de formato. (c) Sin exportación antes de 1.0: I-30 queda sin verificar | **(a)**. Permite verificar I-30 sin prometer el formato. La congelación es D12. **Tomada por delegación** en lo que es el borrador |
| **D11** | Política de migraciones y fixtures antes de 1.0 | (a) Fixtures de v1 en storage y units; fixture de ADR-0020 reconstruido desde `d01f646` o declarado ausente; `MIGRATION` reservado sin escritor. (b) Definir ya el registro de migración: cambia formato. (c) Solo fixtures de v1 | **(a)**. No cambia formato y deja claro qué versiones se han probado leer. **Tomada por delegación** |
| **D12** | Declarar 1.0 y sus promesas: formato v1 estable, semver de la API, formato de exportación congelado | (a) Declarar al cerrar la Etapa 9 si se cumple D13. (b) Declarar después de D13 y de un uso real fuera del equipo. (c) No declarar 1.0 | **(b)**. [ADR-0019](../adr/0019-licencia-y-contribuciones.md) condiciona el cobro a «un 1.0 con formato estable, migraciones y usuarios reales». El caso de uso de E8-09 es interno. **Retenida**: es la promesa a usuarios más grande de la etapa |
| **D13** | Qué requiere 1.0 y qué no | **Requiere:** (1) Etapa 9 cerrada con §4 y §5 cumplidos o declarados. (2) Los 15 puntos de fricción con disposición (§2.4). (3) E8-10 cerrada con sus umbrales. (4) Fixtures de v1 y el de ADR-0020 probados o declarados. (5) I-30 verificada para el borrador y la reimportación. (6) Reclamación pública con el texto de ADR-0009 y su prueba. (7) Límites declarados en el threat model. **No requiere:** cifrado (D3 b), índice aproximado, búsqueda textual, resumidor real, función de copia (D18), umbral frente a SQLite, identificadores con token (si D1 no se toma, la API usa identificadores de 16 bytes) | **La lista de requisitos**. Es el criterio de la etapa. La declaración de 1.0 es D12. **Tomada por delegación** como criterio |
| **D14** | Reclamación física desde la API pública | (a) `Reclaim` público y `reclaim()` en `UnitDb` y `DerivedDb`, con el texto de ADR-0009 (mecanismo, no borrado físico). (b) No exponerla y declarar `forget` solo lógico: la promesa de la opción 3 queda inalcanzable. (c) Exponerla con texto de borrado físico: contradice el threat model §6.5 | **(a)**. Es lo que ADR-0009 opción 3 promete en su punto 2, y nada más. **Tomada por delegación** |
| **D15** | Texto del CLA (ADR-0019) | (a) Cesión de derechos al propietario. (b) Licencia amplia no exclusiva con derecho a relicenciar. (c) Sin CLA hasta la primera contribución externa; hoy no se acepta ninguna. (a) y (b) son promesas a contribuyentes | **(b)**. ADR-0019 pide conservar la opción de relicenciar y de ofrecer términos comerciales, y (b) lo hace sin ceder la autoría. **Retenida**: es un texto legal que escribe o aprueba el propietario en persona. Esta etapa no lo redacta |
| **D16** | Confirmación de la aceptación de ADR-0016 | (a) Confirmar la aceptación de 2026-11-09 tal como está. (b) Revisar G1 a G7 antes de confirmar. (c) Reemplazarla. El propietario condicionó su aprobación, en la sesión que envió esta tarea, a revisar G1 a G7, y el archivo no referencia ningún acta de aprobación | **(b)**. Hasta revisar el texto, el estado es «pendiente de confirmación». Nada de la Etapa 9 depende de ADR-0016. **Retenida** |
| **D17** | Cláusula de conteo de C-4 (ADR-0009) | (a) Mantener la cláusula como está, con la contradicción registrada. (b) Reescribir la cláusula con nueva fecha para que coincida con la medición | **(a)**. ADR-0009 prohíbe ajustar criterios a la vista de una medición. **Tomada por delegación** |
| **D18** | Asignación de clases (A-1) y declaración de copias (B-1) para 1.0 | (a) Todas las clases en la opción 1 (lo que hace el código); declarar que no hay función de copia y que el plazo de las copias no se promete. (b) Esperar a una función de copia | **(a)**, como D11 de E8. Es la única declaración verdadera hoy. **Tomada por delegación** |
| **D19** | Invariantes: I-28, I-42, I-43 y I-29 para `kerno-units` | (a) I-28 no aplica en 1.0 (sin claves); promover I-42 e I-43 si E9-CL las verifica de nuevo; promover I-29 para `kerno-units` si E9-03 pasa. (b) Dejarlas como propuestas | **(a)**. Son estados, no promesas nuevas. **Tomada por delegación**, revocable |
| **D20** | Herencias de proceso: H-3, F-10 y K-E3-7 | (i) H-3: aceptar la corrección de `src/` que entró en el squash de E9A-03, o pedir revertirla y reaplicarla por separado. (ii) F-10: exigir la evidencia de mutación de E8-06 y E8-07, o aceptar la brecha. (iii) K-E3-7: revisar en esta etapa, o dejarla como estaba | (i) **Aceptar**, con su prueba de regresión y sus dos mutaciones. (ii) **Exigir** la evidencia (E9-15). (iii) **Revisar** en E9-16. **Tomada por delegación**, revocable |
| **D21** | Fuzzing por encima del motor de almacenamiento | (a) Una herramienta de fuzzing nueva en `crates/kerno-units/fuzz` con toolchain nightly: nueva dependencia, nueva aprobación. (b) Pruebas de propiedades con la biblioteca que ya es dependencia de desarrollo de `kerno-units`. (c) No hacerlo | **(b)**. Es el paso más bajo de la escalera (ya existe la dependencia) y no añade cadena de herramientas. La aprobación de E9A-04 era para `kerno-storage`. **Tomada por delegación** |
| **D22** | Conjunto E8-10: composición y umbrales | (a) Aprobar §5 (T-E10-V1, V2, Q1 a Q3, S1, S2, Q5, R). (b) Cambiar umbrales antes de congelar | **(a)**. Los umbrales están fijados antes de cualquier medición del conjunto nuevo y se comprueban contra sus cotas. **Tomada por delegación** |

## 8.1 Elecciones por delegación (2026-11-12)

El propietario delegó el 2026-11-12 las decisiones pendientes del roadmap y de esta acta. Cada decisión se registra aquí con la opción recomendada en §8. Las **tomadas** son revocables por el propietario. Las **retenidas** no se toman por delegación: son irreversibles o cambian una promesa a usuarios, y quedan en espera del propietario en persona.

| # | Estado | Opción registrada | Revocable | Qué queda bloqueado hasta la decisión en persona |
|---|---|---|---|---|
| **D1** | **Retenida** | Ninguna. La recomendación (a) no se aplica | — | E9-02 (identificadores), la parte de tokens de la API, I-27 (sin verificar) |
| **D2** | **Retenida** | Ninguna | — | Todo código de claves o de cifrado; K-1 a K-4 de ADR-0009 sin medir; la opción 2 de ADR-0005 |
| **D3** | **Retenida** | Ninguna. Recomendación (b): sin cifrado en 1.0 | — | La declaración de 1.0 (D12), porque una respuesta distinta cambia la lista de requisitos (D13) |
| **D4** | **Retenida** | Ninguna. Recomendación (a): sin integridad autenticada en 1.0 | — | Cerrar H-2 del adelanto; cualquier cambio de formato de la ranura de 40 bytes |
| **D5** | Tomada | (a): ADR-0012 aceptado tal como está. E9-00 lo escribe en su archivo | Sí | — |
| **D6** | Tomada | (a): informe sin umbrales frente a SQLite | Sí | Un criterio frente a SQLite, que exigiría medir en calma antes de fijarlo |
| **D7** | Tomada | (a): sin límite absoluto de latencia | Sí | — |
| **D8** | Tomada | (a): sin formato en disco para resúmenes ni para auditoría en 1.0 | Sí | Un formato para resúmenes o para auditoría (cambio de formato) |
| **D9** | Tomada | (a): crates actuales, sin fachada; disposiciones de §2.4 | Sí | La fachada única (ADR-0010 opción 1) |
| **D10** | Tomada en lo que es el borrador | (a): borrador de exportación, sin congelar | Sí | La congelación del formato (D12) |
| **D11** | Tomada | (a): fixtures de v1, fixture de ADR-0020 reconstruido o declarado ausente | Sí | Un registro de migración escrito |
| **D12** | **Retenida** | Ninguna. Recomendación (b): declarar 1.0 después de D13 y de un uso real fuera del equipo | — | Declarar 1.0, congelar el formato de exportación y prometer semver de la API |
| **D13** | Tomada como criterio | (a): la lista de requisitos de §8 | Sí | La declaración de 1.0 (D12) |
| **D14** | Tomada | (a): `Reclaim` público y `reclaim()` con el texto de ADR-0009 | Sí | Cualquier texto que diga «borrado seguro» o «borrado físico» |
| **D15** | **Retenida** | Ninguna. Recomendación (b): licencia amplia con derecho a relicenciar | — | Aceptar cualquier contribución externa; cambios en el README y en `CONTRIBUTING.md` que remitan al CLA |
| **D16** | **Retenida** | Ninguna. Recomendación (b): revisar G1 a G7 antes de confirmar | — | Nada de la Etapa 9 depende de ADR-0016; queda visible como «pendiente de confirmación» |
| **D17** | Tomada | (a): cláusula de C-4 sin reescribir; contradicción registrada | Sí | — |
| **D18** | Tomada | (a): todas las clases en la opción 1; sin función de copia declarada | Sí | Una función de copia y su ventana (B-1) |
| **D19** | Tomada | (a): I-28 no aplica en 1.0; I-42 e I-43 promovidas si E9-CL las verifica; I-29 para `kerno-units` si E9-03 pasa | Sí | — |
| **D20** | Tomada | (i) aceptar H-3 con su prueba; (ii) exigir evidencia de F-10 (E9-15); (iii) revisar K-E3-7 (E9-16) | Sí | — |
| **D21** | Tomada | (b): pruebas de propiedades con la biblioteca ya presente; sin herramienta nueva | Sí | Una herramienta de fuzzing nueva en `kerno-units` |
| **D22** | Tomada | (a): umbrales de §5 sin cambios antes de congelar | Sí | — |
| **D23** | Tomada el 2026-11-12 por delegación (E9-08f) | El accesor de escritura crudo de `UnitDb` (`raw_commit`) y sus ganchos de prueba (`hook_flush`, `hook_wait_idle`, `hook_compact_range`) existen solo bajo la característica `test-support` de `kerno-units`, apagada por defecto, y en las compilaciones de las pruebas unitarias del propio crate (`cfg(test)`). Los accesores de lectura (`raw_snapshot`, `raw_get`, `raw_seq`, `storage_stats`) se mantienen sin ella. Detalle en [usos de `engine()`](etapa9-usos-de-engine.md) | Sí | — |

**Lo que sigue bloqueado para el propietario en persona:** D1, D2, D3, D4, D12, D15 y D16. Ninguno de los paquetes E9-00, E9-01, E9-03 a E9-14, E9-15 a E9-21 ni E9-CL depende de ellos, salvo E9-02, que está retenido, y E9-CL, que declara como «sin verificar» lo que dependa de las decisiones retenidas.

## 9. Hallazgos heredados y paquete que los cierra

| Hallazgo | Origen | Estado al abrir la etapa | Paquete o decisión |
|---|---|---|---|
| **H-2** Filtro reescrito con CRC válido: `get` oculta claves interiores | Adelanto, E9A-03 | Abierto, no bloqueante | **D4 retenida**. E9-CL lo deja como límite en el threat model |
| **H-3** Corrección de `src/` dentro del squash de E9A-03 | Adelanto | Abierto, no bloqueante | D20 (i): aceptar a posteriori |
| **H-4** C-4, cláusula de conteo, contradicha | Adelanto | Abierto, no bloqueante | D17: se mantiene contradicha, sin reescribir |
| **H-5** Lo que no se mide: C-5, memoria, cachés, lápidas, hilo de borrado, handle | Adelanto | Abierto, no bloqueante | Sin medir; E9-CL lo declara |
| **H-6** Detector de centinelas solo con texto completo | Adelanto | Abierto | E9-03 (fragmentos) |
| **H-7** 30 minutos de fuzzing sin repetir | Adelanto | No verificado | E9-14 |
| **H-8** Comentario «10^4 commits» en `group_commit.rs` | Adelanto | Abierto | E9-03 |
| **H-9** `DESIGN.md`: «borrado tampoco sobrescribe» con `wal_preallocate` | Adelanto | Abierto | E9-CL |
| **H-12** Cita `sst.rs:332` en el acta del adelanto, en realidad `sst.rs:335` | Adelanto | Abierto | E9-CL |
| **H-13** K-E3-7 sin revisar | Adelanto | Abierto | E9-16; D20 (iii) |
| **H-14** I-27, I-28, I-30 sin verificar | Adelanto | Abierto | E9-02 (retenido), E9-04 a E9-06, E9-CL (D19) |
| **F-02** Error poco claro al reconstruir un resumen | E8 | Abierto | E9-11 |
| **F-03** `read_input` omite una fuente sin error | E8 | Abierto, no alcanzado | E9-11 (error explícito) |
| **F-04** Ninguna prueba cambia solo `profile` | E8 | Abierto | E9-10 |
| **F-05** Resúmenes sin medición de centinelas en disco | E8 | Abierto | D8: no hay formato en disco; la medición pasa a ser obligatoria si D8 cambia |
| **F-06**, **F-07** Q4 no demostrada; efecto techo | E8 | Abiertos | E9-19 y E9-20 (T-E10-V1, S1, S2, V2) |
| **F-08** P2 no mide la lectura de vectores persistidos | E8 | Abierto | Límite declarado: E9-20 no mide lectura |
| **F-09** Orden de bloqueo del commit guardado sin prueba | E8 | Abierto | E9-12 cubre parcialmente; el orden exacto sigue sin prueba determinista (límite declarado) |
| **F-10** Evidencia de mutación de E8-06 y E8-07 | E8 | Abierto | D20 (ii): E9-15 |
| **F-11** Comprobación T5 de ADR-0020 no ejecutada | E8 | Abierto | E9-01 |
| **F-12** Sin registro de las rondas de revisión de E8-03 y E8-04 | E8 | Abierto | Registro en E9-CL: lo que aún se pueda reconstruir de los commits, con la fuente de cada dato |
| **F-13** Quince puntos de fricción | E8 | Abierto | §2.4; E9-07 a E9-11 y E9-21 |

## 10. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-12 | **Acta redactada y aprobada por delegación del propietario, revocable** | §8.1. Las decisiones retenidas (D1 a D4, D12, D15, D16) no se toman por delegación y quedan para el propietario en persona. Ningún código de la Etapa 9 se escribe antes de E9-00. Esta acta no cambia código ni pruebas |
