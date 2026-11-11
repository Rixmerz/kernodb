# ADR-0009 — Supresión, retención y backups

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §8.4, §11.5, §12, NIST SP 800-88 Rev. 2 |

## Contexto

Borrar un dato actual no borra sus revisiones, índices, embeddings, páginas liberadas, segmentos antiguos ni backups.

**Pregunta:** Cómo coordinar borrado lógico/físico, revisiones, índices, embeddings y backups.

## Opciones consideradas

1. **Borrado lógico + compactación/scrub forzado** del rango afectado.
2. **Borrado criptográfico** (claves por registro o por colección destruidas).
3. **Combinación** según la clase de dato y la política.

## Propuesta inicial (SAD §15)

Políticas explícitas por colección; operaciones de supresión conscientes de datos derivados; documentación de ventanas de expiración de backups.

## Criterios de decisión

Una prueba de extremo a extremo demuestra qué se elimina inmediatamente, qué queda sujeto a compactación y qué plazo siguen los backups.

### Criterios fijados el 2026-11-11, antes de medir

Estos criterios se escriben antes de cualquier ejecución de E9A-01 ([acta de adelanto de la Etapa 9, §3.2](../architecture/etapa9-adelanto-acta.md)). No contienen resultados ni predicciones de resultados, no eligen opción y no cambian el estado de este ADR, que sigue **Pendiente**. Solo los cambia el propietario, con una fecha nueva; no se ajustan a la vista de una medición.

**Unidad de observación.** Un centinela *sobrevive* en una etapa si aparece, en bytes o en su forma ASCII, en algún archivo regular del directorio de la base en esa etapa: `MANIFEST`, `MANIFEST.tmp`, `WAL.spare`, los `NNNNNN.wal` y los `NNNNNN.sst`. La clave y el valor se cuentan por separado, porque el MANIFEST guarda extremos de clave de los archivos que se retiran ([DESIGN.md, `compact_range`](../../crates/kerno-storage/DESIGN.md)). Las etapas son las de E9A-01: antes del flush, tras `compact_range`, tras cierre limpio y tras reapertura. La visibilidad (`get` y `scan`) se observa en las mismas etapas.

**Qué significa «eliminado».** En este ADR, *eliminado de inmediato* significa invisible para las lecturas posteriores al commit, no ausente del medio. El motor no promete lo segundo ([threat model §6.5](../security/threat-model.md)), así que ningún criterio de esta sección lo exige.

#### Criterios de las opciones 1 y 3

- **I-1 (inmediato).** Tras un `delete` o un `delete_range` confirmado, ninguna lectura nueva de la misma base devuelve el valor borrado ni una clave cubierta por el rango ([v1.md §11](../file-format/v1.md), SEM-4). Se contradice si un `get` o un `scan` posterior al commit lo devuelve.
- **I-2 (instantáneas).** Una instantánea tomada antes del commit sigue viendo el estado anterior hasta que se suelta ([v1.md §11](../file-format/v1.md), SEM-4 y SEM-5). Se contradice si la instantánea ve el borrado antes de soltarse.
- **C-1 (archivos vivos).** Tras `compact_range` que cubre la clave y un cierre limpio, todo archivo vivo que aún contenga el centinela debe pertenecer a una de las clases de «Qué puede sobrevivir a `compact_range`» en [DESIGN.md](../../crates/kerno-storage/DESIGN.md) que se ven en el directorio: tablas retenidas por una instantánea (contadas en `pinned_files` y `pinned_bytes`) o lápidas de punto en el último nivel que un movimiento trivial de segundo plano dejó allí. Cualquier otro archivo vivo con el centinela contradice la opción 1. Los bloques liberados, la memoria, la caché y las copias hechas fuera del motor no están en el directorio y no entran en esta comparación.
- **C-2 (MANIFEST).** Tras el checkpoint de `compact_range`, el MANIFEST no contiene la clave suprimida, salvo como extremo de un archivo que sigue en la versión actual. Se contradice si la clave aparece como extremo de un archivo que ya no está en la versión actual.
- **C-3 (lápidas de rango).** Tras `compact_range`, ningún fragmento de lápida de rango que cubra el rango compactado sobrevive en un archivo vivo que no retenga una instantánea. Se contradice si sobrevive.
- **C-4 (plazo).** El plazo de la eliminación de archivos es el retorno de `compact_range` para lo que ninguna instantánea retiene, y la liberación de la última instantánea que retiene un archivo para el resto. La ADR lo escribe en esos términos, que se observan en `pinned_files`, `pinned_bytes` y `snapshot_retained_bytes`, y no como una duración. Se contradice si el conteo de archivos retenidos no coincide con los archivos que siguen conteniendo el centinela.
- **C-5 (scrub).** La parte «scrub forzado» de la opción 1 no tiene mecanismo en el código: `compact_range` no sobrescribe bloques (`crates/kerno-storage/src/db.rs:2009`). E9A-01 no la mide. Si el propietario la quiere, es un paquete aparte con su propio criterio, y su comprobación física queda fuera de lo que el motor puede observar ([threat model §4](../security/threat-model.md)).

#### Criterios de la opción 2

La opción 2 no se puede medir con E9A-01 mientras no haya cifrado: el formato rechaza el bit `AEAD` con `Incompatible("encryption not supported")` ([threat model §6.3](../security/threat-model.md)). Estos criterios no fijan cómo se generan ni se guardan las claves, que decide ADR-0006 ([acta §2.3](../architecture/etapa9-adelanto-acta.md)).

- **K-1 (inmediato).** Tras destruir la clave de un registro o de una colección, sus datos no se recuperan del directorio ni de una copia, ni con una base reabierta sin esa clave. Se contradice si una reapertura sin la clave recupera el dato.
- **K-2 (compactación).** Lo que queda sujeto a compactación son bytes cifrados. Su presencia no contradice la opción 2 mientras se cumpla K-1. Un centinela en claro en un archivo que debía estar cifrado sí la contradice.
- **K-3 (copias).** Ninguna clave, viva o destruida, está en el directorio de datos ni en las copias que produce el motor (T-03). Se contradice si una copia contiene una clave.
- **K-4 (plazo).** El plazo de la supresión es el momento de la destrucción de la clave. A partir de ahí, la permanencia de los bytes cifrados deja de importar para la supresión.

#### Criterios de la opción 3

- **A-1 (asignación completa).** La opción 3 declara, por colección y clase de dato (SAD §15), qué opción se aplica. Una colección sin clase o una clase sin opción la contradicen.
- **A-2 (remisión).** Cada clase asignada a la opción 1 cumple I-1, I-2 y C-1 a C-5. Cada clase asignada a la opción 2 cumple K-1 a K-4.

#### Backups (comunes a las tres opciones)

- **B-1 (plazo declarado).** Para cada clase de dato, la ADR declara la ventana durante la que una copia de backup puede seguir conteniendo el dato suprimido, como duración finita, o declara que no hay ventana y por qué.
- **B-2 (no medible con E9A-01).** E9A-01 no confirma ni contradice B-1. Las copias de backup están fuera del motor («se documentan las ventanas, no se prometen», [threat model §4](../security/threat-model.md)), y `crates/` no tiene código de exportación ([acta §2.3](../architecture/etapa9-adelanto-acta.md)). B-1 se cumple con una declaración escrita que el propietario acepta, no con una medición. Por eso, la frase de 2026-10-27 sobre una prueba de extremo a extremo que demuestre el plazo de los backups no se puede cumplir con E9A-01, y este criterio la precisa.

#### Qué confirma o contradice E9A-01

| Criterio | ¿Lo puede confirmar o contradecir E9A-01? | Confirma | Contradice |
|---|---|---|---|
| I-1 | Sí | Lecturas posteriores al commit sin el valor | Un `get` o `scan` que lo devuelve |
| I-2 | Sí | La instantánea previa lee el valor anterior | La instantánea ve el borrado antes de soltarse |
| C-1 | Sí | Los archivos vivos con el centinela son solo de las clases retenidas o de lápidas de último nivel | Un archivo vivo con el centinela fuera de esas clases |
| C-2 | Sí | El MANIFEST tras el checkpoint no nombra la clave | La clave aparece como extremo de un archivo fuera de la versión actual |
| C-3 | Sí | Ninguna lápida de rango del alcance en archivos vivos sin instantánea | Una lápida de rango del alcance en un archivo vivo sin instantánea |
| C-4 | Parcialmente | Los conteos de `pinned_*` y `snapshot_retained_bytes` coinciden con los archivos que retienen el centinela | Un conteo que no coincide con esos archivos |
| C-5 | No | — | — |
| K-1 a K-4 | No (no hay cifrado) | — | — |
| A-1, A-2 | Solo la parte de la opción 1 | Las clases de la opción 1 cumplen I-1 a C-5 | Una clase de la opción 1 que incumple alguno |
| B-1 | No | — | — |
| B-2 | — | Es una constatación sobre el alcance de E9A-01 | — |

Esta sección no elige opción, no fija ningún umbral de tiempo ni de bytes y no decide si ADR-0009 se acepta: eso es E9A-00b y lo decide el propietario. El riesgo T-09 del threat model sigue abierto.

## Lo que aportó M0

M0 hizo visible que el borrado físico depende del motor: en CoW B+ tree las páginas liberadas conservan bytes antiguos hasta reutilizarse; en LSM las versiones antiguas persisten en SST y WAL hasta compactación y reciclaje. Ver §5 del [diseño de revisiones](../architecture/revision-model-design.md).

## Dependencias

Depende de ADR-0001, ADR-0004 y ADR-0006.

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
