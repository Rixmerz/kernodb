# Etapa 5 · E5-04 — Centinelas

Contrato de E5-04 ([acta de la Etapa 5](etapa5-acta.md) §3; [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md), I-37, I-29, IU-10). La prueba está en `crates/kerno-units/tests/sentinels.rs`.

## 1. Qué se planta y dónde se busca

Un centinela es una cadena que ninguna capa de Kerno escribiría por sí sola. Se planta en cada punto por el que entra contenido de usuario:

| Punto | Centinela |
|---|---|
| Tipo de unidad | `SNT-TYPE-…` |
| Carga útil y metadatos | `SNT-PAYLOAD-…`, `SNT-METADATA-…` |
| Tipo de relación | `SNT-RELTYPE-…` |
| Procedencia | fuente (`Imported`) y origen (`Algorithmic`, `Inferred`) |
| Valores de filtro | el tipo, y una versión de esquema de 10 cifras |
| Tipo desmesurado (error de escritura) | `SNT-OVERSIZE-…` |

Cada uno se busca en tres formas, las que tendría en un `Display` o un `Debug`: el texto, la lista decimal de bytes de un `Vec<u8>` y el hex.

Se busca en todo lo que la capa dice de una consulta: los registros (`{:#?}`) y los contadores del observador, `explain_filter` y `explain_relations` (el plan, sus candidatos, la huella en `Debug` y `Display`), los caminos y fallbacks de la respuesta, y los errores de consultas, de la reconstrucción del índice y de una escritura rechazada (`Display` y `Debug`). Se repite en `off`, `aggregate`, `fingerprint` y `diagnostic`, sin y con un registro de unidad dañado (la lectura falla con corrupción).

El acta pide ningún centinela fuera de `diagnostic`. El texto de la forma que añade `diagnostic` lleva marcadores tipados (`units[unit_type=?text]`), así que la prueba exige más: ninguno en ningún nivel. Quien haga que `diagnostic` lleve un literal a propósito relaja esa única aserción y no toca los niveles inferiores.

## 2. Pruebas

| Prueba | Qué demuestra |
|---|---|
| `no_sentinel_in_any_observability_explanation_or_error_output` | La búsqueda anterior. Además, cada huella informada (planes y registros, en cada nivel) es igual a la de la misma forma con otros literales: un literal que entra en la entrada del hash nunca aparece como texto y la búsqueda sola no lo vería. Con un control: la respuesta al llamador sí contiene los centinelas, y la salida buscada no está vacía (hay registros a partir de `fingerprint`, forma en `diagnostic`, la escritura desmesurada rechazada y al menos un error de consulta) |
| `levels_below_diagnostic_carry_no_shape_text` | Mitad estructural de I-37: `fingerprint_text` solo existe en `diagnostic` y es la forma con marcadores |

Cada una corre sobre `Emulated<LogEngine>` y sobre `KernoDb`.

## 3. Conteo de pruebas

Sumas de las líneas `test result` de cada ejecución, antes (commit de E5-03) y después de este paquete; ninguna prueba falla en ninguna de las dos:

| Suite | Antes | Después |
|---|---|---|
| `cargo test --workspace --exclude kerno-storage --all-features --release` | **611** pasan, 8 ignoradas | **615** pasan, 8 ignoradas |
| `cargo test -p kerno-storage --all-features --release` | 649 pasan, 4 ignoradas | 649 pasan, 4 ignoradas (sin cambios: este paquete no toca `kerno-storage`) |
| Suite completa (suma de las dos) | **1260** | **1264** |

+4: dos pruebas por dos motores, `Emulated<LogEngine>` y `KernoDb`. El binario `tests/sentinels.rs` informa `4 passed`. El 611 de partida coincide con el cierre de [E5-03](etapa5-observacion.md) §6.

## 4. La prueba puede fallar

Cada prueba nueva se vio fallar. Invertir la aserción de búsqueda (`!text.contains` → `text.contains`, solo en `tests/sentinels.rs`): fallan las 2 pruebas de búsqueda. Mutaciones «copiar el literal en la huella o en la explicación» del acta, aplicadas al código de producción y restauradas con `git checkout` (`cargo test -p kerno-units --all-features --release --test sentinels`):

| Mutación | Resultado |
|---|---|
| Copiar el tipo del filtro en la forma de unidades (`NormalizedQuery::of_filter`) | fallan las 4: la huella difiere de la forma sin literales ya en `Off` (`fingerprint units/… at level Off depends on a literal`), y la forma de `diagnostic` no es la esperada. Con la comprobación de huellas desactivada, la búsqueda de texto también falla: `unit type sentinel "SNT-TYPE-7f3a91c2" leaked at level Diagnostic` |
| Copiar el tipo de relación en la forma de relaciones (`NormalizedQuery::of_relations`) | fallan las 4: `fingerprint relations/… at level Off depends on a literal` y la forma de `diagnostic` |
| Copiar el tipo de unidad o de relación solo en la entrada del hash (campo nuevo en `NormalizedQuery`, `hasher.update`), sin tocar el texto | fallan las 2 de búsqueda por la comprobación de huellas, en `Off`. **Hallazgo:** con la búsqueda de texto sola, las 4 siguen en verde; por eso se añadió la comprobación |
| Copiar la versión de esquema del filtro en el coste del candidato de referencia (`unit_candidates`, la explicación) | fallan las 2 de búsqueda: `schema version sentinel "4071337791" leaked at level Off` (`Cost(4071337791)` en el plan) |

## 5. Revisión del threat model

[T-11](../security/threat-model.md) cubre observaciones y explicaciones de plan: adversarios, respuesta y residuo. Lo que la prueba **no** garantiza: que un campo nuevo no filtre por un canal que la prueba no lee (por eso ADR-0015 exige ampliarla con cada campo), la memoria del proceso y la clave (fuera de alcance), y los contadores y duraciones como canal lateral.
