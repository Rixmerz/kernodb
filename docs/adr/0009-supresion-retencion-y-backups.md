# ADR-0009 — Supresión, retención y backups

| Campo | Valor |
|---|---|
| Estado | Aceptado (2026-11-12, por el propietario: opción 3, en los términos de la sección [Decisión](#decisión)) |
| Fecha | 2026-10-27 (apertura); 2026-11-11 (criterios fijados antes de medir); 2026-11-12 (aceptado) |
| Decisores | Propietario del proyecto (E9A-00b, [acta de adelanto de la Etapa 9](../architecture/etapa9-adelanto-acta.md) §7, punto 1) |
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

**Aceptada el 2026-11-12 por el propietario: opción 3 (combinación), en estos términos.** Los criterios de la sección anterior se mantienen como se fijaron el 2026-11-11, antes de medir; esta decisión no los cambia.

1. **Garantía inmediata: borrado lógico.** Tras un `delete` o un `delete_range` confirmados, las lecturas posteriores no devuelven el valor borrado ni una clave cubierta por el rango. Una instantánea tomada antes del borrado sigue viendo el estado anterior hasta que se suelta. Criterios I-1 e I-2.
2. **Retirada de los archivos vivos al compactar.** Cuando se compacta el rango afectado con `compact_range` y ninguna instantánea abierta, tomada antes del borrado, retiene el archivo, los archivos vivos del directorio dejan de contener el valor y la clave borrados. Mientras una instantánea así siga abierta, la tabla que retiene permanece en disco y se libera al soltarla. Criterios C-1 a C-4, en lo que toca al plazo de C-4.
3. **Sin borrado físico.** No se promete que los bloques liberados desaparezcan del medio: el motor no tiene mecanismo para sobrescribirlos. Criterio C-5 sin medir; el límite queda declarado en el [threat model §6.5](../security/threat-model.md).
4. **Borrado criptográfico, diferido.** Se difiere hasta que [ADR-0006](0006-cifrado-y-gestion-de-claves.md) (cifrado y gestión de claves) esté aceptado, y solo para los datos que lo requieran. Hoy el formato rechaza el bit `AEAD` con `Incompatible("encryption not supported")`, así que no hay nada que medir. Criterios K-1 a K-4.
5. **Backups, sin plazo declarado.** No se declara ninguna ventana de backup. El plazo se fija cuando exista una función de copia de seguridad en el motor; hoy no la hay en `crates/`. Hasta entonces, esta ADR no afirma ningún plazo para los backups. Criterios B-1 y B-2.

Los términos no incluyen una API de supresión, y esta aceptación no escribe ninguna.

### Relación con los criterios

Lo que se lista como «confirmado» lo muestra la medición de E9A-01 ([evidencia](#evidencia)). Lo que no se confirma se dice tal cual.

| Criterio | Qué muestra la medición | Estado |
|---|---|---|
| I-1 | Tras `delete` y `delete_range`, en cada etapa, `get` y `scan` no devuelven lo borrado (`assert_visible_state`) | **Confirmado** |
| I-2 | La instantánea tomada antes de los borrados sigue leyendo el estado anterior (`snapshot_unchanged`) | **Confirmado** |
| C-1 | Tras `compact_range` y cierre limpio, ningún archivo vivo contiene el valor ni la clave. Antes del cierre, el único archivo con el centinela es una tabla que retiene la instantánea (`000007.sst`), contada en `pinned_files` | **Confirmado** para las etapas observadas. La clase de lápidas de punto en el último nivel no se ejercitó |
| C-2 | Tras el checkpoint de `compact_range`, el MANIFEST no nombra la clave, tampoco con una instantánea abierta | **Confirmado** |
| C-3 | Los fragmentos de la lápida de rango pasan de 1 a 0 tras `compact_range` | **Confirmado** |
| C-4 | El plazo se observa: al soltar la instantánea desaparecen las tres tablas que retenía. El conteo no coincide con los archivos que contienen el centinela: `pinned_files = 3` (`000003.sst`, `000005.sst`, `000007.sst`), y solo `000007.sst` contiene el valor y la clave | **No confirmado tal como está escrito.** La prueba afirma que los archivos con centinela están entre los retenidos, no que el conteo coincida (`suppression_survival.rs`, líneas 536–543) |
| C-5 | No medido. `compact_range` no sobrescribe bloques (documentación de `compact_range` en `crates/kerno-storage/src/db.rs`) | **No medible con E9A-01**; fuera de los términos (punto 3) |
| K-1 a K-4 | El formato no tiene cifrado | **No medibles** (punto 4) |
| A-1 | La opción 3 exige asignar cada colección y clase de dato a una opción. Los términos no hacen esa asignación; el punto 4 la deja para cuando ADR-0006 esté aceptado | **No cumplido** |
| A-2 | Clases de la opción 1 (puntos 1 a 3): I-1, I-2 y C-1 a C-4 confirmados; C-5 sin medir y excluido por el punto 3 | **Parcial** |
| B-1 | Un plazo finito, o la declaración de que no hay ventana y por qué. Los términos no declaran ventana y el punto 5 la difiere | **No cumplido.** Queda abierto hasta que exista la función de backup |
| B-2 | Las copias de backup están fuera del motor (threat model §4) y E9A-01 no las mide | **No medible** (constatación de alcance) |

### Evidencia

Comando, desde la raíz del repositorio, el 2026-11-12:

```
cargo test -p kerno-storage --all-features --release --test suppression_survival -- --nocapture
```

Extractos verbatim de la salida:

```
[survival] P1 (delete: value still in the log, no flush): HOLDS (delete value in ["000004.wal"])
[survival] P1 (delete_range: value still in the log, no flush): HOLDS (range value in ["000004.wal"])
[survival]   compact_range: rewritten=4 pinned_files=3 pinned_bytes=30990 kept_tables=["000003.sst", "000005.sst", "000007.sst"] retained_bytes=31702
[survival] P2 (value in no live file after flush, compact_range, clean close): HOLDS (delete value in [], range value in [])
[survival] P3 (tombstone keeps the key in a live file until compaction discards it): HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
[survival] P4 (flushed log is zeros and holds no value): HOLDS (spare is zeros: Some(true); delete value in ["000007.sst"]; range value in ["000007.sst"])
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

La prueba es una observación del comportamiento actual, no un contrato (su cabecera). Esta ADR no la cambia.

### Consecuencias

- Lo que el motor promete se reduce a los puntos 1 y 2. Ningún texto del producto debe describir `delete` o `compact_range` como borrado seguro ([threat model §6.5](../security/threat-model.md)).
- Si el motor cambia lo que observa E9A-01, la prueba puede cambiar con él; no es una regresión de una garantía, porque el motor no promete ninguna más allá de los puntos 1 y 2.

### Seguimiento

- **C-4:** decidir si el conteo de `pinned_files` debe coincidir con los archivos que contienen el centinela, o si el criterio se reescribe con una nueva fecha. No se ajusta a la vista de la medición.
- **A-1:** asignar cada colección y clase de dato a una opción. Los términos no fijan cuándo se hace.
- **Opción 2 (K-1 a K-4):** depende de [ADR-0006](0006-cifrado-y-gestion-de-claves.md), pendiente.
- **B-1:** fijar el plazo de backup cuando exista una función de copia de seguridad en el motor. Hasta entonces, sin plazo.
- **C-5:** un borrado físico de bloques liberados sería un paquete aparte con su propio criterio ([threat model §4](../security/threat-model.md)).
