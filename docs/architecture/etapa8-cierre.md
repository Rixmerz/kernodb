# Cierre de la Etapa 8

| Campo | Valor |
|---|---|
| Estado | **Cerrada como trabajo de ingeniería.** Decisiones del cierre (§8) tomadas por delegación del propietario el 2026-11-12, revocables por él. **Q4 no demostrada** (no cumplida): se registra así, no como cumplida. Ningún hallazgo bloqueante abierto (§4) |
| Fecha | 2026-11-12 (E8-08) |
| Acto | Revisión adversarial de E8-01 a E8-07 y E8-09; comprobación de los criterios de §4 de la acta, de los umbrales de §6 y de los invariantes de §7; actualización de invariantes, roadmap, SAD y acta (acta §3, fila E8-08) |
| Base revisada | Árbol en `53b7fa1` (último commit de E8-07 antes de este cierre). Este cambio solo toca `docs/` y no añade pruebas ni código |
| Documentos | [Acta de la Etapa 8](etapa8-acta.md) · [conjunto de evaluación](etapa8-conjunto-evaluacion.md) · [medición](etapa8-medicion.md) · [salida cruda](etapa8-medicion-salida.txt) · [informe SQLite (E8-S0)](../testing/sqlite-report.md) · [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) · [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md) · [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) · [uso real (E8-09)](../../examples/agent-memory/FRICTION.md) · [invariantes](invariants.md) · [roadmap, Etapa 8](../roadmap/README.md#etapa-8--consolidación-y-embeddings) · [SAD 0.3.0](SAD.md) · [cierre de la Etapa 7](etapa7-cierre.md) |

Este documento describe la etapa tal como queda en el repositorio. Cada cifra sale de un comando ejecutado para este cierre (§9), de la salida cruda comprometida o de un fichero citado. Lo que no se ha comprobado se marca **no verificado**.

**Ningún umbral, ninguna respuesta esperada, ningún peso y ningún fichero congelado se ha cambiado en E8-08.** Tampoco se ha cambiado código ni pruebas. Los defectos encontrados se registran en §4 con su reproducción; ninguno se ha corregido.

## 0. Resultado en breve

### 0.1 Calidad frente a §6.2 (conjunto nuevo, 100 consultas; 25 con vector de consulta)

| Umbral (acta §6.2) | Medido | Estado |
|---|---|---|
| **Q1** recall@10 de la búsqueda exacta persistida frente a fuerza bruta = 1,000 | 1,000; 25 de 25 listas idénticas | **cumple** |
| **Q2** nDCG@10 con señal ≥ 0,80, en conjunto | 1,000 sobre las 25 consultas con vector | **cumple**, con efecto techo (§3) |
| **Q3** recall@10 con señal ≥ 0,90, en conjunto | 1,000 sobre las 25 consultas con vector | **cumple**, con efecto techo (§3) |
| **Q4** nDCG@10 con señal ≥ sin señal − 0,01, en conjunto y en cada clase | Cumple donde se puede calcular (`vector_neighbourhood`, 1,000 frente a 1,000). **Tres de las cuatro clases no son medibles**: no tienen vector de consulta | **no demostrada. No cerrada.** No se registra como cumplida |
| **Q5** sin juzgar ≤ 10 % del top 10, en cada brazo | Máximo 1,1 % (`ranked`, 8 de 708) | **cumple** |
| **Q6** (ranking frente a orden simple) | No medido: solo si el propietario propone activar el ranking. No es criterio de cierre | no medido |

### 0.2 Rendimiento frente a §6.3

| Umbral | Medido, doce corridas | Estado |
|---|---|---|
| **P1** escritura con descriptor / sin descriptor ≤ 1,50 | 1,165 a 1,238 | **cumple** |
| **P2** búsqueda con vectores persistidos / en memoria ≤ 1,10 | 0,921 a 1,007 | **cumple**, con el límite de P2 descrito en §3 |
| **P3** conjunto de `stale` = derivados de las fuentes cambiadas | Igualdad de conjuntos, ambos motores, antes y tras reabrir | **cumple** |
| **P4** reconstrucción byte a byte igual a construir de nuevo | 286 de 286, ambos motores | **cumple** |

### 0.3 Lo que este cierre afirma y lo que no

1. La mecánica de derivados (registro, obsolescencia, reconstrucción, persistencia de vectores, supresión de fuentes y derivados, resúmenes con procedencia) está construida y tiene pruebas que pasan en los dos motores (§1, §9).
2. La calidad de recuperación **no** queda demostrada: Q4 no está cerrada, Q2 y Q3 dan el valor máximo en los dos brazos (efecto techo), y los vectores son deterministas con semilla (§3, §5).
3. Un derivado borrado o obsoleto no se sirve; la supresión no promete borrado físico (§5).
4. Ningún defecto bloqueante queda abierto. Hay hallazgos no bloqueantes con su razón (§4).

## 1. Paquetes de trabajo

Para cada paquete: lo que pedía la acta (§3), lo entregado, los commits y el estado. Los comandos y su salida real están en §9.

| WP | Pedía la acta | Entregado | Commits (de `git log`) | Estado |
|---|---|---|---|---|
| **E8-S0** informe SQLite sin umbrales | Comparación `kerno` frente a SQLite, durabilidades pareadas, sin umbrales, binario existente | Informe [sqlite-report.md](../testing/sqlite-report.md) y salida cruda en `spikes/storage/results/2026-11-12-e8-s0/`. Sin umbrales. Par `Buffered` con garantía no idéntica, dicho en el informe. Primera pasada `Sync` interrumpida por falta de espacio; recortes declarados en su §6 | `078839a`, `349d735` | hecho |
| **E8-00** puerta de entrada | Aprobación de la acta, ADR-0018 aceptado, estado de ADR-0009 en su archivo, D1 a D12 registradas | Hecho según [acta §0](etapa8-acta.md) y [§10](etapa8-acta.md) | `7d656ac` | hecho |
| **E8-01** conjunto nuevo congelado | Corpus, consultas y juicios con semilla propia; `MANIFEST` SHA-256; generador en `kerno-testkit` | `crates/kerno-testkit/src/derived_eval.rs`, `crates/kerno-testkit/data/derived-eval/`, [conjunto](etapa8-conjunto-evaluacion.md). `MANIFEST` comprobado antes de medir. Pruebas en `crates/kerno-testkit/tests/derived_eval.rs` (15 pruebas) | `6b01dde`, `f3ddb2c` (fusión `bec6f36`) | hecho. Ver F-10 para la mutación |
| **E8-02** registro en memoria | `DerivedDescriptor` con fuentes exactas, método, versión, parámetros; estados; índice inverso | `crates/kerno-units/src/derived.rs`, `tests/derived_registry.rs` (13 pruebas) | `3dc63f0` | hecho |
| **E8-03** obsolescencia y reconstrucción | Cambio de fuente marca `stale` en el mismo `ChangeSet`; reconstruir igual que construir; método o versión nuevos crean derivado y marcan `superseded` | `crates/kerno-units/src/derived_stale.rs`, `tests/derived_stale.rs` (46 pruebas). Historial de rondas en §7.1 | `d6b8ee9` (squash), `6bbadbd`, `23af819` y otros, según §7.1 | hecho |
| **E8-04** persistencia de vectores (condicional, D2 = persistir) | Vectores como derivados con bit de versión de capa, ADR nuevo, formato de bytes de la capa, reglas DV | `crates/kerno-units/src/vector_store.rs`, `tests/derived_vectors.rs` (77 pruebas), [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md). Desviación respecto de D2 en §7.2 | `ea2f232`, `d0ab6cc` (ADR); `ea533ca`, `ca30c28`, `e5a6d0b` (código) | hecho |
| **E8-05** supresión de fuentes y derivados | Borrar unidad o retirar revisión retira en el mismo `ChangeSet` los derivados; prueba de supervivencia con centinelas | `crates/kerno-units/src/suppression.rs`, `tests/derived_suppression.rs` (19 pruebas). Sin centinela de resúmenes: F-05 | `91b8373`, `0bb9ecb` (revisión), `9461d7a` | hecho, con F-05 |
| **E8-06** consolidación con procedencia | Resumen como derivado con fuentes obligatorias; adaptador como trait sin red; implementación de prueba determinista; el original siempre legible | `crates/kerno-units/src/consolidation.rs`, `tests/consolidation.rs` (30 pruebas, 15 por motor). Sin resumidor real (D12) | `5be0a06`, `5350599`, `9461d7a` | hecho |
| **E8-07** medición | Calidad sobre el conjunto nuevo y rendimiento; salida cruda comprometida | `tests/derived_measure.rs` (4 pruebas), `examples/derived_bench.rs`, [medición](etapa8-medicion.md), [salida cruda](etapa8-medicion-salida.txt) | `ca60831`, `53b7fa1` (fusión `1124a34`) | hecho. Ver F-10 y §3 |
| **E8-09** caso de uso real (pre-1.0) | Memoria para un agente con solo la API pública; registro de derivados si E8-05 está cerrado | `examples/agent-memory/` (fuera del workspace), [FRICTION.md](../../examples/agent-memory/FRICTION.md) con 15 puntos | `5518b08`, `56971dd` | hecho. Lista de fricción en §6 |
| **E8-08** cierre | Revisión adversarial, promoción de I-25 e I-38 y nuevas (§7), roadmap, SAD como versión nueva | Este documento, [invariantes](invariants.md) §2.6, [roadmap](../roadmap/README.md), [SAD 0.3.0](SAD.md) | este cambio | hecho |

Ningún paquete añadió dependencias: `Cargo.toml` y `Cargo.lock` no han cambiado desde la base de la Etapa 7 (`52f026f`), comprobado con `git log` (§9, comando 6).

## 2. Revisión adversarial (parte 1 de la tarea)

### 2.1 Lo que se ejecutó

- **Suite de `kerno-units` y `kerno-testkit` en release**, con todas las funciones: 856 aprobadas, 0 fallidas, 8 ignoradas (§9, comando 1). Ninguna de las ocho ignoradas es de Etapa 8: una es el regenerador de vectores de la capa 1 (`struct_vectors.rs:769`, «writes the committed vectors») y siete son fragmentos de documentación de `kerno-testkit` y de `kerno-units` marcados sin ejecutar.
- **Cada fichero de prueba nuevo aparece en la salida de `cargo test`** (§9, comando 2). Ninguno está fuera del alcance del runner: no hay `autotests = false` ni `[[test]]` en los `Cargo.toml` de `kerno-units` ni de `kerno-testkit`.
- **Sondas en una copia del árbol** (`git archive` de `53b7fa1`, fuera del árbol de trabajo, borrada al terminar), en release, en los dos motores (`emulated_log` y `kerno_db`), con un fichero de prueba propio que usa el mismo arnés que el conjunto. Resultados en §2.3.
- **Lectura del código** de `consolidation.rs`, `suppression.rs` y del camino de apertura y de confirmación de `derived_stale.rs`, que son las piezas sin revisión independiente anterior.

### 2.2 Qué se revisó y con qué resultado

| Pieza | Revisado por | Resultado |
|---|---|---|
| `consolidation.rs` (E8-06) | Lectura del código y sondas P3, P5 | Sin defecto. `summary()` sirve solo un resumen `ready` con el marcador de tipo; `get_unit` no tiene camino hacia el texto de un resumen. Dos observaciones no bloqueantes: F-02 y F-03 |
| `suppression.rs` (E8-05, arreglos de `0bb9ecb`) | Lectura y sondas P2, P4 | Sin defecto. `source_gone` retira un derivado si la unidad falta, está `Deleted` o su revisión fuente es menor que `oldest`. La ruta de retención reconcilia el registro aunque la retención falle. Relaciones: sin derivados que borrar, como dice el propio fichero |
| Camino de apertura de `derived_stale.rs` tras la revisión de E8-05 | Lectura y sondas P3, P4 | Sin defecto. Un derivado cuya fuente cambió o se borró mientras la base estaba cerrada se registra `stale` o se descarta, nunca `ready` sin recalcular. Un valor se conserva solo para `ready` (`values` se borra al pasar a `stale`, `failed` o `superseded`) |
| Confirmación de `DerivedChangeSet::commit` | Lectura | El registro se bloquea antes del commit y se libera tras la marca. La propia nota del código dice que ese orden no tiene prueba: F-09 |
| Persistencia de vectores (E8-04) | Lectura, pruebas | Sin ejecución de la comprobación de T5 (binario de referencia): F-11 |
| Medición (E8-07) | Lectura de la salida y del documento | Sin defecto. La mutación de cada prueba no se hizo: F-10 |

### 2.3 Sondas en copia del árbol, en los dos motores

El fichero de sondas (cinco pruebas, dos motores) y su salida real están en §9, comando 7. Resumen:

| Sonda | Qué hace | Resultado |
|---|---|---|
| P1 borrado y restauración de la misma unidad en un solo `ChangeSet` | Construye un resumen, borra y restaura la fuente en el mismo `ChangeSet` | `restore` tras `delete` en el mismo `ChangeSet` **se rechaza** en ambos motores (`is_ok=false`). El caso que podría borrar un resumen que debía quedar `stale` no es alcanzable por la API. No es hallazgo |
| P2 retención que purga la revisión de la que se construyó un resumen, y reapertura | Resumen sobre rev 1, edición, resumen sobre rev 2, `KeepLast(1)`, cierre, reapertura con ambos descriptores | Ambos motores: `removed=1`; el resumen de la revisión purgada desaparece; el de la revisión actual sigue sirviendo `aaaa`; la reapertura informa exactamente el id purgado como descartado. **Pasa** |
| P3 resumen `stale` al cerrar; reapertura y reconstrucción | Cambio de fuente, cierre, reapertura con el descriptor viejo, reconstrucción | Ambos motores: reabre `stale`, no sirve texto, la reconstrucción da un id nuevo y el viejo deja de existir. **Pasa.** Observación: construir de nuevo sobre las mismas fuentes da `Duplicate` («a derived datum with this identity is already registered»): F-02 |
| P4 fuente borrada con la base cerrada; reapertura | Resumen sobre dos unidades, borrado de una de ellas con la base cerrada, reapertura | Ambos motores: se devuelve el id como descartado, no hay texto, y la otra fuente sigue leyéndose. **Pasa** |
| P5 regla diferencial: ninguna lectura devuelve un resumen en lugar de una fuente | Dos resúmenes y un resumen superado, edición de la fuente, cierre, lectura | Ambos motores: `summary()` es `None` para el `stale` y para el superado; `get_unit` de la fuente es igual a la lectura de una base plana. **Pasa** |

Las cinco sondas se ejecutaron con `--test-threads=1`. El resultado es `10 passed; 0 failed`, y se leen con `--nocapture` las tres observaciones de P1 a P3 que aparecen en la salida (§9, comando 7).

### 2.4 Lo que la revisión no hizo

- **Mutación de las pruebas nuevas** (acta §4, punto 4): no se ejecutó en esta revisión. Ver F-10.
- **La comprobación T5 de ADR-0020** (construir el binario de `d01f646` y abrir el fixture): no se ejecutó (F-11).
- **Reordenación del bloqueo del commit**: no hay prueba que lo cubra, y la propia nota del código lo dice (F-09).

## 3. Medición: resultado tal como se obtuvo

Esta sección es el resultado de la medición (`etapa8-medicion.md`), sin cambios. Las cifras salen de la salida cruda comprometida.

- **Q1, Q2, Q3 y Q5 y P1 a P4 cumplen**, en el sentido de su umbral. Los cuatro de rendimiento están en las doce corridas de P1 y P2 y en la igualdad de P3 y P4 (§0.2).
- **Q4 no está cerrada.** Solo una de las cuatro clases de consulta del conjunto congelado lleva vector de consulta: `vector_neighbourhood`, 25 de 25. Las clases `temporal`, `provenance` y `combined` tienen 0 de 25 vectores. Como Q4 pide «en conjunto y en cada clase», no se puede calcular en tres de cuatro clases. Se registra como **no demostrada**. No se inventaron vectores de consulta para no cambiar el conjunto congelado.
- **Q2, Q3 y Q4 dan 1,000 en el brazo con señal y también en el brazo sin ella.** Es un efecto techo: estas 25 consultas no distinguen un brazo de otro. Los umbrales se cumplen, pero la medición aporta poca evidencia de calidad. No se cambió el conjunto ni el umbral después de medir.
- **P2 no mide el coste de leer vectores persistidos.** Compara dos búsquedas sobre el mismo `VectorSet` ya cargado. La lectura única de 20 000 vectores (unos 10 a 11 ms) se informa aparte y no entra en el cociente. El coste en frío de abrir la base (156 a 162 ms) tampoco entra.
- **Los vectores son deterministas con semilla** (D5). Miden la mecánica, no la calidad semántica de un modelo.
- **Corpus, consultas y juicios los escribió el mismo equipo.** No hay juicio independiente (acta §6.1, límite declarado).

## 4. Hallazgos

Estados: **resuelto**; **abierto y bloqueante**; **abierto y no bloqueante**, con su razón. Clase de evidencia: **demostrado por un comando**, **leído del código** o **no verificado**.

**Bloqueantes abiertos: ninguno.** Ningún hallazgo de esta revisión contradice un invariante de §7 ni un criterio de §4 de forma que impida el cierre de ingeniería. Por eso la decisión (a) de §8 se mantiene.

| ID | Dónde | Hallazgo | Estado | Clase de evidencia | Razón |
|---|---|---|---|---|---|
| F-01 | `derived_stale.rs` y sondas P1 | Ninguno: `restore` tras `delete` en un mismo `ChangeSet` se rechaza. Se anota porque la rama `deleted` del `commit` no se limpia, y sería un borrado de más si la restauración se permitiera | resuelto por la API (no alcanzable) | demostrado (sonda P1, ambos motores) | No hay camino que lo produzca |
| F-02 | `consolidation.rs` `build_summary`, `derived_stale.rs` `register` | Tras reconstruir un resumen, construir de nuevo sobre las mismas fuentes devuelve `Duplicate` con el texto «a derived datum with this identity is already registered». Quien quiera el resumen tiene que conservar el id nuevo. El error dice poco | **abierto y no bloqueante** | demostrado (sonda P3) | Es coherente con la identidad (I-24): una misma identidad no se registra dos veces. Es un problema de ergonomía, recogido en el grupo D de §6 |
| F-03 | `derived_stale.rs` `read_input` y `valid_inputs`, líneas 354 a 372 | `read_input` devuelve `None` si el head no tiene carga útil, y `valid_inputs` (usado por `rebuild` y `supersede`) entonces **omite** esa fuente sin error, mientras que `build` la rechaza con `MissingSource`. Solo una unidad borrada no tiene carga útil; las unidades borradas ya se retiran del registro en el commit, en la apertura y en la retención, así que el camino no se alcanza | **abierto y no bloqueante** | leído del código; no alcanzado por ninguna sonda | No se alcanza. Si una vez se alcanzara, `rebuild` podría producir un derivado `ready` con menos fuentes que el que construiría `build`. Conviene una prueba o un error explícito antes de la API pública |
| F-04 | `derived.rs:190` (identidad) y `tests/derived_registry.rs` | La identidad incluye el perfil (`field(self.profile.as_bytes())`), como exige I-25. Ninguna prueba cambia **solo** el perfil y comprueba que la identidad cambia: `each_identity_field_changes_the_identity` no lo varía, y `derived_vectors.rs` usa el perfil como dato, no como comprobación | **abierto y no bloqueante** | leído del código; grep de `profile` en `tests/` | La propiedad se cumple en el código. Falta la prueba que la muestre. Pendiente para el seguimiento de la Etapa 8 |
| F-05 | `tests/derived_suppression.rs`, cabecera «Sentinels in summaries: declared gap» | Los resúmenes no tienen medición de supervivencia de centinelas en disco: viven solo en memoria, así que no hay bytes que medir. El acta §5.3 punto 5 pedía el centinela. Las pruebas de centinelas de I-37 sí cubren los resúmenes (`sentinels.rs`, `no_sentinel_in_summary_types_or_errors`, en los dos motores) | **abierto y no bloqueante** | demostrado (cabecera del fichero y `sentinels.rs`) | El resumen no persiste ningún byte (`a_summary_leaves_nothing_on_disk`). Si los resúmenes llegan a persistir, la medición pasa a ser obligatoria |
| F-06 | Medición, §2 de [medición](etapa8-medicion.md) | Q4 no demostrada: tres de cuatro clases sin vector de consulta | **abierto y no bloqueante** (decisión (a) del propietario, §8) | demostrado (salida cruda) | El conjunto congelado no tiene vectores de consulta en esas clases. Cerrarlo exige un conjunto nuevo, que es el seguimiento de §8 (c) |
| F-07 | Medición, Q2, Q3 y Q4 | Efecto techo: 1,000 en ambos brazos en las 25 consultas con vector | **abierto y no bloqueante** | demostrado (salida cruda) | Límite del conjunto; no se cambia después de medir. Lo cubre el seguimiento de §8 (c) |
| F-08 | Medición, P2 | P2 no mide el coste de leer vectores persistidos | **abierto y no bloqueante** | demostrado (diseño del banco, §3.1 de la medición) | El banco compara dos búsquedas sobre el mismo conjunto cargado. Una medición de lectura exige otro diseño |
| F-09 | `derived_stale.rs`, `DerivedChangeSet::commit`, líneas 495 a 502 | El orden «bloqueo del registro, commit, marca» del commit guardado no tiene prueba. La propia nota del código dice que mostrarlo exige un lector que compita con el commit, que no se puede hacer determinista | **abierto y no bloqueante** | leído del código | La corrección se sostiene en el razonamiento de la nota. No hay caso de prueba que lo contradiga |
| F-10 | Acta §4, punto 4 (mutación de cada prueba nueva) | La mutación registrada de cada prueba nueva no está en la evidencia de E8-07 (`etapa8-medicion.md` §6, punto 8: «no hecho en este paquete»). Para E8-06 no hay registro de mutación en los commits. Para E8-03 y E8-05 sí hay (ecafa8f, 0bb9ecb). Esta revisión no ejecutó mutaciones | **abierto y no bloqueante**, con la salvedad siguiente | no verificado | Es una brecha de proceso, no un defecto de código: las pruebas pasan y su cobertura de rama se ha leído. **Salvedad:** la acta dice que un paquete que no cumple §4 no se cierra. El propietario debe aceptar o exigir esa evidencia antes de que el cierre sea definitivo |
| F-11 | ADR-0020, T5 y criterio de rechazo de un binario sin la característica | La comprobación de T5 (construir el binario de `d01f646` y abrir `derived-v2/db-with-vector`) no se ejecutó en esta revisión | **abierto y no bloqueante** | no verificado | Requiere un binario de referencia que no se construyó aquí. Las pruebas T6 y T7 (`layer1_reader_refuses_derived_fixture`, `open_derived_fixture_writes_nothing`) pasan en la suite (§9) |
| F-12 | Historial de E8-03 y E8-04 | No hay en el repositorio un registro de las rondas de revisión ni de sus hallazgos. Se reconstruyen en §7.1 a partir de los mensajes de commit | **abierto y no bloqueante** | leído del historial | Para la memoria del proyecto, las rondas se registran en §7.1 con la fuente de cada una |
| F-13 | `examples/agent-memory/FRICTION.md` | Quince puntos de fricción de la API pública, de los que uno toca una promesa (§6) | **abierto y no bloqueante** para la Etapa 8; **bloquea 1.0**, como trabajo previo a 1.0 (acta §1, E8-09 y decisión (d) de §8) | demostrado (el ejemplo se compila y se ejecuta fuera del workspace según su README) | Fuera del camino crítico de la Etapa 8 (acta §3, E8-09) |
| F-14 | `docs/roadmap/README.md`, Etapa 8 | El roadmap decía que E8-S0 seguía pendiente, aunque `docs/testing/sqlite-report.md` está comprometido desde `078839a` | **resuelto** en este cierre | demostrado (`git log`, fichero presente) | Corregido en la actualización del roadmap |
| F-15 | Suite, `struct_vectors.rs:769` | Una prueba ignorada, que regenera vectores de la capa 1. No es de la Etapa 8 | **resuelto por diseño** (no se ejecuta en `cargo test`) | demostrado (salida de la suite) | Queda como está |

**Estado del hallazgo bloqueante: no hay ninguno abierto.** Ningún hallazgo de la tabla impide cerrar la Etapa 8 como trabajo de ingeniería. F-10 es la única salvedad de proceso, y la deja al propietario.

## 5. Lo que queda sin demostrar y sin construir

1. **Los resúmenes son solo en memoria.** No existe medición de supervivencia de centinelas para ellos, porque no hay ningún byte en disco que medir (F-05). Un resumen no sobrevive a la reapertura salvo que quien llama guarde su descriptor y lo vuelva a pasar; la reconstrucción pasa por `open_summaries`, que recalcula solo si las fuentes siguen iguales.
2. **La eliminación de una relación no borra ningún dato derivado**, porque ningún dato derivado tiene una relación como fuente. Esto es una decisión de diseño y lo prueba `deleting_a_relation_removes_no_derived_datum`. No es una garantía de que una relación futura no pueda ser fuente: si lo es, la regla es nueva.
3. **El orden de bloqueo del commit guardado no tiene prueba** (F-09).
4. **Vectores deterministas con semilla no dicen nada de un modelo real.** La calidad de embeddings reales no se ha medido, y no se ha decidido un modelo (D5 (b) sigue sin decidir).
5. **Corpus, consultas y juicios los escribió el mismo equipo.** No hay juicio independiente (acta §6.1). Con 25 consultas por clase, las diferencias pequeñas no son significativas.
6. **Sin índice aproximado, sin búsqueda textual, sin fusión híbrida, sin resumidor real** (acta §2.3 y D12). Ninguno está construido.
7. **Sin función de copia de seguridad**, así que no hay ventana de copias que declarar (ADR-0009, B-1; D11). No se promete plazo.
8. **Sin borrado físico y sin borrado criptográfico** (ADR-0006 pendiente). El borrado es lógico de inmediato y los bytes pueden seguir en archivos vivos hasta la compactación que los cubra.
9. **La compactación no es alcanzable por la envoltura pública de E8-09**. Véase el grupo F de §6.

## 6. Lista de fricción del caso de uso (E8-09)

Los quince puntos de [FRICTION.md](../../examples/agent-memory/FRICTION.md), agrupados como trabajo previo a 1.0. El punto marcado con **(promesa)** toca una promesa.

### Grupo A: lectura y consulta de derivados (puntos 1, 2, 9 y 15)

- **1.** `DerivedDb` no lista, no consulta ni lee historial. Sin `snapshot`, `query` ni `history`; el catálogo de unidades que usa el ejemplo es una segunda fuente de verdad.
- **2.** No hay forma de hallar los derivados de una unidad: `dependents_of_unit` existe en el registro, pero `DerivedDb` solo ofrece lecturas por `DerivedId`.
- **9.** Dos mundos de vectores que no se encuentran: los persistidos se leen uno a uno; la búsqueda vive en memoria y copia todos los vectores `ready` en cada llamada.
- **15.** El tipo de unidad del catálogo comparte espacio de nombres con los datos de la aplicación, y el historial por defecto crece con cada escritura.

### Grupo B: identidad y reconstrucción (puntos 3 y 8)

- **3.** Un id derivado cambia en cada reconstrucción, y el registro que lleva la aplicación no se puede actualizar en la misma transacción. La única ruta atómica (`supersede_vector`) deja basura `superseded` en el almacén. `replace_vector` es `pub(crate)`.
- **8.** Sustituir un vector obsoleto por uno calculado fuera pasa por la API de «recalcular» con una clausura que ignora sus argumentos.

### Grupo C: resúmenes y apertura (punto 5)

- **5.** No hay una forma de abrir vectores persistidos y resúmenes a la vez: `open_persisted` carga vectores, `open_summaries` no. Los resúmenes son solo en memoria, así que la aplicación tiene que guardar su propia definición.

### Grupo D: ergonomía de creación e identificadores (puntos 6, 7 y 10)

- **6.** Los identificadores de unidad son 16 bytes opacos, y la fuente de ids se fija antes de abrir la base.
- **7.** `create` devuelve `UnitId`, pero `put_vector` quiere `RevisionId`. El número de la primera revisión se escribe a mano.
- **10.** Una receta de vector exige muchos campos, y nada liga `dim` y `seed` al vector que se produce.

### Grupo E: errores, cierre y apertura (puntos 11, 12, 13 y 14)

- **11.** `DerivedDb::close` devuelve `()` y descarta el informe de cierre.
- **12.** Tres tipos de error sin un tipo común; los mensajes de `DerivedStaleError` no nombran ninguna unidad.
- **13.** Abrir la base necesita tres crates y un objeto de rasgo; no hay `open_dir` para el motor por defecto.
- **14.** Una segunda apertura del mismo directorio falla con un error de almacenamiento que no distingue «bloqueada» de «corrupta».

### Grupo F: borrado y espacio (punto 4) **(promesa)**

- **4.** **(promesa)** `forget` es solo lógico. El texto sigue en los archivos después de `forget`, de la retención y del cierre: lo prueba `forgotten_text_stays_in_the_files_after_retention`, que lo encuentra en `000003.sst`. `UnitDb::compact` necesita `TestHooks` y `KernoDb::compact_range` es un método de prueba, así que ambos quedan detrás de `DerivedDb`, que oculta el motor. La documentación de `suppression.rs` lo dice («No promise of physical erasure»), pero **no hay ninguna vía pública para obtener la compactación que haría cumplir una petición de borrado**. Una aplicación que tenga que atender un borrado no tiene cómo. Esto toca la promesa de ADR-0009 opción 3 (borrado lógico inmediato y retirada en la compactación), que la API pública no puede cumplir por sí misma.

**Orden de trabajo antes de 1.0.** La decisión (d) de §8 convierte esta lista en la entrada del trabajo de API pública. Su orden propuesto: primero el grupo F (la promesa), después B y A, que cambian la forma de los tipos; C y D después; E al final, porque no cambia ningún tipo público.

## 7. Hechos de proceso

### 7.1 E8-03: rondas de revisión

El acta y el repositorio cuentan **cinco rondas** para E8-03 (dato de la tarea). El historial de commits que tocan `derived_stale.rs` o su prueba, en orden, es el siguiente. **La asignación de cada commit a una ronda no está escrita en ningún documento del repositorio** (F-12); es una lectura de los mensajes:

| Ronda | Commit | Qué dice el mensaje | Qué encontró |
|---|---|---|---|
| 1 | `b938ece` | Primera implementación: `DerivedDb`, marcado por dependencia bajo el bloqueo del registro, reconstrucción y superseded; 16 pruebas nuevas | Implementación inicial |
| 2 | `23af819` | Paquete de E8-03; la sesión se cortó por el límite de uso; `Locks-Verified: no (blocked)` | Sin revisión completa: el paquete no quedó verificado |
| 3 | `3528ef9` | Nueva pasada tras fallos de compilación por una caché de compilación obsoleta; ningún fichero cambió | Ninguno; se repitió la verificación |
| 4 | `ecafa8f` | Cierra **dos bloqueantes** de la revisión: la guarda de duplicados en `rebuild` y el término de revisión en `input_hash`; y añade pruebas por mutación de cada rama de `derived_stale.rs`. Suite de 675 a 688 pruebas | Dos bloqueantes resueltos; huecos de mutación cubiertos |
| 5 | `3238aa4` | La prueba de fallo de commit pasa a inyectar un fallo de sincronización en `SimVfs` en los dos motores | La prueba del fallo de commit no era una prueba real |
| 6 | `6bbadbd` | Paquete final de E8-03; el trabajador agotó sus turnos sin veredicto; el comando de verificación pasó en su lugar | Sin hallazgo nuevo; verificación completa en la sesión |

Según la tarea, el número correcto es cinco rondas. El historial tiene seis commits con revisión visible, porque el primero (`23af819`) fue un intento interrumpido. Esta revisión no puede confirmar la cuenta de cinco sin el registro que la tarea menciona: queda como **no verificada** en la cuenta exacta (F-12).

### 7.2 E8-04: dos rondas

| Ronda | Commit | Qué encontró |
|---|---|---|
| 1 | `ea533ca` | Paquete de E8-04; el trabajador agotó sus turnos sin veredicto; el comando de verificación pasó en su lugar |
| 2 | `ca30c28` | Un hallazgo: `derived_follow` no seguía las fuentes cuando el `ChangeSet` ya había escrito un registro derivado en una base de capa 1 (`db.rs:1005`). Se añadieron cinco pruebas por motor (persistir y actualizar, persistir y borrar, actualizar y persistir, borrar y persistir, dos unidades) |

### 7.3 Desviación de ADR-0020 respecto de la decisión D2

La decisión D2 (acta §8 y §8.1) dice que los vectores se persisten con «un bit de característica nuevo» y que el bit es `incompat`, en el formato de cabecera de §5.4 del acta. **ADR-0020 no hace eso.** El bit se declara como **versión 2 de la capa de unidades** en la marca `LAYER_META "layer"`, con la característica `DERIVED_VECTORS` (ADR-0020, Decisión 2 y Opciones 2). La razón, según el propio ADR, son cuatro:

1. Los archivos ya escritos no llevan el bit en la cabecera, y ponérselo a todos exigiría reescribir el motor; el MANIFEST se reescribe en cada *checkpoint*.
2. El motor no sabe qué es un derivado y no tiene concepto de capa; la cabecera tendría que cambiar sus reglas.
3. Cambiar `KNOWN_INCOMPAT` es un cambio de reglas del formato congelado (ADR-0013), con regeneración de vectores.
4. Una build antigua solo fallaría si el bit estuviera en el MANIFEST, que se reemplaza en cada *checkpoint*; eso acoplaría el motor a la capa.

La marca de versión de capa cumple la misma función que `incompat`: una build de capa 1 rechaza al abrir la base (UNIT-16, `IncompatibleLayer`). La acta lo registra ya en §8.1 (D2: «El bit se declara como versión 2 de la capa de unidades, no en el motor»). **La desviación es de forma, no de propósito:** el resultado que pedía D2, que una build antigua no escriba sobre derivados sin marcar, se conserva.

## 8. Decisiones del propietario en este cierre

Tomadas **por delegación del propietario el 2026-11-12**, revocables por él. Se registran como se han enunciado, sin añadir nada.

- **(a)** La Etapa 8 queda **cerrada como trabajo de ingeniería**, con **Q4 registrada como no demostrada**, no como cumplida.
- **(b)** **Ningún umbral y ningún fichero congelado se reescribe después de medir.**
- **(c)** Se abre un **paquete de seguimiento antes de 1.0**: un segundo conjunto de evaluación, congelado por separado, en el que **cada clase de consulta lleve vectores de consulta** y cuyas consultas sean lo bastante difíciles para no saturar; sus umbrales se fijan antes de medirlo, y se mide una sola vez.
- **(d)** La lista de fricción (§6) se convierte en la entrada del trabajo de la API pública antes de 1.0.
- **(e)** El ranking queda **desactivado por defecto y experimental**, como se decidió al cerrar la Etapa 7.

**Condición de (a).** Si la revisión encontrara un defecto bloqueante, (a) no se sostendría y no se cerraría nada. No hay ningún defecto bloqueante en §4. La salvedad F-10 (evidencia de mutación) queda abierta para que el propietario la acepte o la exija.

El paquete de seguimiento del punto (c) se llama **E8-10** en el roadmap. Su contenido completo está en [roadmap, Etapa 8](../roadmap/README.md#etapa-8--consolidación-y-embeddings).

## 9. Verificación: comandos y salida real

Todos se ejecutaron en el árbol de trabajo o en la copia de sondas indicada.

### 9.1 Suite de `kerno-units` y `kerno-testkit` en release (comando 1)

```text
cargo test -p kerno-units -p kerno-testkit --all-features --release
```

Resultado: código de salida 0. Totales de todas las cajas (pruebas unitarias, de integración y de documentación):

```text
passed=856 failed=0 ignored=8
```

Las ocho ignoradas: `struct_vectors.rs:769` («writes the committed vectors») y siete fragmentos de documentación sin ejecutar en `kerno-testkit` y `kerno-units`.

### 9.2 Los ficheros de prueba nuevos aparecen en el runner (comando 2)

Salida de `cargo test`, filtrada a los ficheros de Etapa 8:

```text
tests/derived_eval.rs          15 passed
tests/derived_registry.rs      13 passed
tests/derived_stale.rs         46 passed
tests/derived_suppression.rs   19 passed
tests/derived_vectors.rs       77 passed
tests/consolidation.rs         30 passed   (15 pruebas × 2 motores)
tests/derived_measure.rs        4 passed
```

Y, en el mismo comando, `tests/cache.rs` (4), `tests/codecs.rs` (21) y `tests/assoc_rank.rs` (19) con sus cifras propias.

### 9.3 Formato (comando 3)

```text
cargo fmt --all --check
fmt exit=0
```

### 9.4 Clippy (no ejecutado en este cierre)

El cierre solo cambia documentación. La última salida de `cargo clippy --workspace --all-targets --all-features -- -D warnings` de la Etapa 8 está citada en [medición §8](etapa8-medicion.md) (`Lints: 0`). **No se ha vuelto a ejecutar.**

### 9.5 Pruebas citadas (comando 4)

Cada nombre citado en §5 y en [invariantes](invariants.md) §2.6 se buscó con `grep -rl` en `crates/*/tests` y `crates/*/src`. Todos existen. Ninguno falta.

### 9.6 Dependencias (comando 6)

```text
git log --oneline 52f026f..HEAD -- Cargo.toml Cargo.lock crates/kerno-units/Cargo.toml crates/kerno-testkit/Cargo.toml | wc -l
0
```

### 9.7 Sondas en copia del árbol (comando 7)

Copia de `53b7fa1` (`git archive`) en un directorio de trabajo, con el fichero de sondas, ejecutada en release con `--test-threads=1` y `--nocapture`:

```text
running 10 tests
test emulated_log::t1_delete_then_restore ... P1 restore after delete in one change set: is_ok=false
test emulated_log::t2_retention_then_reopen ... P2 removed=1 unreadable=0
P2 reopen dropped=1 (expect old id only)
test emulated_log::t3_stale_at_close ... P3 same sources again: Err: a derived datum with this identity is already registered
test emulated_log::t4_delete_while_closed ... ok
test emulated_log::t5_no_read_returns_a_summary ... ok
test kerno_db::t1_delete_then_restore ... P1 restore after delete in one change set: is_ok=false
test kerno_db::t2_retention_then_reopen ... P2 removed=1 unreadable=0
P2 reopen dropped=1 (expect old id only)
test kerno_db::t3_stale_at_close ... P3 same sources again: Err: a derived datum with this identity is already registered
test kerno_db::t4_delete_while_closed ... ok
test kerno_db::t5_no_read_returns_a_summary ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

La copia se borra al terminar la revisión.

## 10. Documentos que este cierre actualiza

| Documento | Cambio |
|---|---|
| [`etapa8-acta.md`](etapa8-acta.md) | E8-08 marcado hecho (§3); filas de §1, §7 y §10 cerradas con su resultado |
| [`invariants.md`](invariants.md) | I-25 e I-38 promovidas a Etapa 8 con sus pruebas (§2.6); I-42 e I-43 añadidas como propuestas verificadas, pendientes de aprobación |
| [`../roadmap/README.md`](../roadmap/README.md) | Etapa 8 cerrada con los puntos abiertos y el paquete E8-10 |
| [`SAD.md`](SAD.md) | Versión 0.3.0: derivados, vectores persistidos, supresión y resúmenes tal como están construidos |
