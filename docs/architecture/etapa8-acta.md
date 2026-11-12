# Acta de entrada de la Etapa 8: consolidación y embeddings

| Campo | Valor |
|---|---|
| Fecha | 2026-11-12 |
| Estado | **Aprobada por el propietario el 2026-11-12**, con las decisiones D1 a D12 de §8.1 (D10 en opción b). Ver §0 |
| Acto | Alcance, condiciones de inicio, decisiones y paquetes E8-S0 y E8-00 a E8-09 |
| Documentos | [Roadmap, Etapa 8](../roadmap/README.md) · [acta de la Etapa 7](etapa7-acta.md) · [cierre de la Etapa 7](etapa7-cierre.md) · [medición de la Etapa 7](etapa7-medicion.md) · [ranking (E7-04)](etapa7-ranking.md) · [conjunto de evaluación de la Etapa 7](etapa7-conjunto-evaluacion.md) · [acta de adelanto de la Etapa 9](etapa9-adelanto-acta.md) · [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) · [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) · [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) · [ADR-0003](../adr/0003-formato-fisico-y-versionado.md) · [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) · [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md) · [invariantes](invariants.md) · [SAD §9, §10, §16](SAD.md) · [especificación v0.2.0 §10, §11, §14, §16](../spec/especificacion-arquitectonica-v0.2.0.md) |

## 0. Estado

Esta acta está **aprobada por el propietario el 2026-11-12**. Describe el estado del repositorio tal como se leyó el 2026-11-12 y qué debe hacerse en la Etapa 8. El propietario aprobó las decisiones D1 a D12 con la recomendación de cada una, salvo D10, donde eligió la opción (b); la elección de cada decisión está en §8.1.

E8-00 queda hecho con esta aprobación. E8-S0 (informe SQLite, sin umbrales) está **hecho** (`078839a`). E8-08 (cierre) está **hecho** el 2026-11-12 con [cierre de la Etapa 8](etapa8-cierre.md): cerrada como trabajo de ingeniería, Q4 no demostrada, decisiones de §8 tomadas por delegación del propietario y revocables. Ningún código de la Etapa 8 se escribe sin pasar por E8-00.

Lo que el propietario ya ha decidido y esta acta recoge (§1.1 y §8.1) no se vuelve a proponer.

## 1. Condiciones de inicio

La Etapa 8 depende de la 7 y de la 6 en el roadmap. Su condición de inicio es «ADR-0018 aceptado; supresión de derivados definida (ADR-0009)».

| Condición | Estado leído el 2026-11-12 | Qué hace falta |
|---|---|---|
| Etapa 7 cerrada | **Cumplida.** El propietario aprobó el cierre el 2026-11-12, y [cierre de la Etapa 7](etapa7-cierre.md) lo dice en su encabezado y en su §9 | Ninguna |
| Etapa 6 cerrada | [Cierre](etapa6-cierre.md) aprobado por delegación el 2026-11-11; caché desactivada por defecto | Ninguna |
| Ranking apagado por defecto y marcado como experimental | Cumplido en el código: `RankMode` tiene `Off` como variante por defecto (`crates/kerno-units/src/assoc_rank.rs`, `#[default]`). La decisión del propietario de 2026-11-12 la confirma | Que la documentación de la Etapa 8 lo diga con esas palabras (E8-08) |
| ADR-0018 (versionado y reconstrucción de derivados) | **Aceptado** el 2026-11-12 con la opción 2 (registro común de derivados), en su archivo y en `docs/adr/README.md` | **Cumplida** (gate E8-00) |
| ADR-0009 (supresión, retención y backups) | **Aceptado** el 2026-11-12 con la opción 3 (ver §1.1). Su archivo `docs/adr/0009-supresion-retencion-y-backups.md` y `docs/adr/README.md` dicen «Aceptado», con fecha y con la opción 3. Ese archivo registra la cláusula de conteo de C-4 como contradicha y B-1 sin plazo declarado | **Cumplida** (gate E8-00). El seguimiento de C-4 y B-1 está en el propio ADR |
| Criterios de ADR-0009 fijados antes de medir | Cumplido: el texto de criterios está en ADR-0009, con fecha 2026-11-11, antes de cualquier ejecución de E9A-01 | Ninguna |
| Supresión medida (E9A-01) | Existe `crates/kerno-storage/tests/suppression_survival.rs` y el commit `2bab299`. Su cabecera dice que las predicciones P1 a P4 se cumplen. Es una observación del comportamiento actual, no un contrato | Ninguna para E8; E8-05 la extiende a derivados |
| Informe comparativo con SQLite, sin umbrales (decisión del propietario de 2026-11-11, punto 2) | **No existe.** `docs/testing/f1-report.md` menciona SQLite solo en su método y no contiene resultados. `docs/testing/` no tiene ningún informe con ese nombre | **Hecho** (E8-S0, `078839a`): [informe](../testing/sqlite-report.md) sin umbrales, en `docs/testing/sqlite-report.md` |
| Caso de uso real antes de 1.0 (decisión del propietario de 2026-11-11, punto 3) | No aplica a la entrada de la Etapa 8: es un requisito para 1.0 | Paquete E8-09 (§3), fuera del camino crítico de la Etapa 8 |
| Decisión del propietario sobre reescribir T4 (cierre de la Etapa 7, §8.2) | **Decidida el 2026-11-12: opción (b).** T4 se deja como está en el acta de la Etapa 7 y no se reescribe | Ninguna (§8.1, D10) |

### 1.1 Decisiones del propietario que esta acta recoge

Comunicadas el 2026-11-12:

1. Se aprueba el cierre de la Etapa 7.
2. La Etapa 8 arranca con el ranking de resultados asociativos **apagado por defecto y como experimental**. El orden por defecto sigue siendo «saltos, luego identificador».
3. Se acepta ADR-0009 con la **opción 3** (combinación según la clase de dato): borrado lógico inmediato más retirada de los archivos vivos en la compactación cuando ninguna instantánea anterior sigue abierta. No se promete borrado físico. El borrado criptográfico queda para ADR-0006. No se declara ningún plazo de backup hasta que exista una función de backup.

Del 2026-11-11 siguen vigentes: el informe SQLite se adelanta antes de la Etapa 8 y no lleva umbrales; el caso de uso real se construye antes de un 1.0; las decisiones de la Etapa 9 adelantada (fuzzing con cobertura y estado de ADR-0013) se aprobaron por delegación en su propia acta.

## 2. Alcance

### 2.1 Objetivos

1. **Registro común de derivados** (ADR-0018, opción 2): cada derivado tiene un descriptor con sus fuentes exactas (unidad y revisión), su método o modelo, su versión, sus parámetros, su estado (`ready`, `building`, `stale`, `failed`, `superseded`) y si es reconstruible.
2. **Obsolescencia y reconstrucción por dependencia**: cambiar una fuente marca sus derivados como `stale` en el mismo `ChangeSet`. Reconstruir produce lo mismo que construir de nuevo (I-24).
3. **Vectores como derivados**: persistir embeddings ligados a su revisión fuente y a su perfil de modelo (I-25). Decidido por el propietario (§8.1, D2): sí, condicionado a que E8-03 pase.
4. **Supresión de fuentes y derivados** según ADR-0009 opción 3 (§5.3).
5. **Consolidación con procedencia**: un resumen es un derivado con sus fuentes. Nunca reemplaza el original auditable (spec §10).
6. **Adaptadores fuera del núcleo** (I-31): el núcleo guarda el resultado y su descriptor; quien genera el embedding o el resumen es un adaptador o un job externo, con un contrato sin red en el núcleo.

### 2.2 Punto de partida (lo leído en el código)

- **No se persiste ningún vector ni ningún embedding.** Ningún fichero de `crates/kerno-units/src` ni de `crates/kerno-storage/src` menciona `embedding`, y los ficheros de persistencia (`db.rs`, `codec.rs`, `index.rs`) no usan `f32`. Los vectores existen solo en memoria, los entrega quien llama (`VectorSet` y `VectorQuery` en `crates/kerno-units/src/assoc_rank.rs`, ADR-0011). La búsqueda vectorial es exacta, por fuerza bruta (`VectorSet::search`). No hay índice aproximado.
- **No hay registro de derivados.** Los derivados que existen hoy son tres (§5.1): los índices de unidades (`IDX_TYPE` y el índice primario, con estado y watermark, I-23), la caché de resultados (solo en memoria, invalidada por dependencias, desactivada por defecto) y el ranking (en memoria, apagado).
- **La caché de resultados** (`crates/kerno-units/src/cache.rs`) guarda la huella de la consulta con sus dependencias: `schema_version`, `planner_version`, estado e watermark del índice, y `commit_seq`. `Dependency::ALL` fija el orden de comparación y `first_change` dice cuál cambió. No se persiste.
- **No hay código de exportación** en `crates/`, ni función de backup. Las ventanas de copias no pueden medirse (ADR-0009, B-2).
- **Adyacencias de relaciones**: son índices primarios, escritos en el mismo `ChangeSet` que la relación (I-21), no derivados.
- **Dependencias**: `Cargo.toml` y `Cargo.lock` no cambian desde antes de E7-00 (cierre de la Etapa 7, §4, I-31).

### 2.3 Fuera de alcance

- **Índice vectorial aproximado (ANN).** Sin benchmark que lo justifique (ADR-0011, roadmap). La Etapa 8 no lo escribe.
- **Búsqueda textual (BM25) y fusión híbrida.** Son de la capa de consulta y de otra fase (SAD §10.4).
- **Modelos de embeddings o de resúmenes reales.** Están detrás de adaptadores (I-31). Esta etapa no llama a ningún modelo ni a ningún proveedor.
- **Ranking por defecto.** Queda apagado (§1.1). Activarlo exige decisión posterior con un conjunto nuevo (§6.2, Q6).
- **Relaciones inferidas.** La [acta de la Etapa 7](etapa7-acta.md) §2.3 las asignaba a la Etapa 8, pero el roadmap no las incluye entre los entregables de la Etapa 8. Esta acta las deja fuera. El propietario decidió dejarlas fuera (D8, §8.1).
- **Cifrado y borrado criptográfico.** ADR-0006 sigue pendiente. K-1 a K-4 de ADR-0009 no se aplican aquí.
- **Exportación e importación** (I-30) y migraciones con fixtures: Etapa 9.
- **Usar la caché de resultados en consultas asociativas**: fuera, como en la Etapa 7.
- **Cambios de valores por defecto** de `kerno-storage` (`Config::default`) y de `Config::limits`.
- **Modificar archivos del formato v1 congelado** (ADR-0013). Cualquier cambio persistente sigue §5.4.

## 3. Paquetes de trabajo

Todos los paquetes con código cumplen las reglas comunes de §4. Cada uno escribe solo en los ficheros que indica. Un paquete que añade un módulo nuevo debe declararlo en `crates/kerno-units/src/lib.rs` (o en `crates/kerno-testkit/src/lib.rs`): ese fichero es compartido, así que **los paquetes que añaden un `mod` no corren a la vez**. Los ficheros de prueba nuevos bajo `tests/` no necesitan tocar `lib.rs`.

| WP | Contenido | Depende de | Escribe | Puede correr junto a |
|---|---|---|---|---|
| **E8-S0** Informe SQLite, sin umbrales | Comparación `kerno` frente a SQLite en las cargas que ambos pueden ejecutar, con la durabilidad equivalente que ya declara [f1-objetivos](../testing/f1-objetivos-rendimiento.md): `Sync` frente a `journal_mode=WAL` con `synchronous=FULL`; `Buffered` frente a `synchronous=NORMAL`, anotando que la garantía ante corte de energía no es idéntica. Sin umbrales (decisión de 2026-11-11). Se ejecuta con el binario de benchmark existente | — | `docs/testing/sqlite-report.md` (nuevo), `spikes/storage/results/` (salidas nuevas) | E8-00, E8-01, E8-02 |
| **E8-00** Puerta de entrada | Aprobación de esta acta. Aceptación de ADR-0018 (opción 2, o la que decida el propietario). Estado de ADR-0009 escrito en su archivo. Decisiones D1 a D12 de §8 registradas. Cierre de la Etapa 7 marcado como aprobado | Aprobación de esta acta | `docs/adr/0018-*.md` (solo la sección de estado y la decisión), `docs/adr/README.md`, `docs/roadmap/README.md` | E8-S0 |
| **E8-01** Conjunto de evaluación nuevo, congelado | Corpus, consultas y respuestas esperadas con grado de relevancia (§6.1), generados con semilla propia, con `MANIFEST` SHA-256. Generador en `kerno-testkit`, que no depende de `kerno-units` | E8-00 | `crates/kerno-testkit/src/derived_eval.rs` (nuevo), `crates/kerno-testkit/src/lib.rs` (una línea `mod`), `crates/kerno-testkit/data/derived-eval/` (nuevo), `docs/architecture/etapa8-conjunto-evaluacion.md` (nuevo) | E8-S0, E8-02 (siempre que E8-02 no toque `lib.rs` en el mismo momento) |
| **E8-02** Registro de derivados en memoria | Tipo `DerivedDescriptor` con fuentes exactas (unidad, revisión), método, versión, parámetros y hash de entrada; estados `ready`, `building`, `stale`, `failed`, `superseded`; índice inverso fuente → derivados mantenido por el propio registro. Sin persistencia | E8-00 | `crates/kerno-units/src/derived.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea `mod`), `crates/kerno-units/tests/derived_registry.rs` (nuevo) | E8-S0, E8-01 |
| **E8-03** Obsolescencia y reconstrucción | Un `ChangeSet` que cambia una fuente marca como `stale` sus derivados en el mismo `ChangeSet`. Reconstruir igual que construir de nuevo. Cambiar método o versión crea un derivado nuevo y marca el anterior `superseded` | E8-02 | `crates/kerno-units/src/derived_stale.rs` (nuevo), `crates/kerno-units/src/lib.rs` (una línea `mod`), `crates/kerno-units/tests/derived_stale.rs` (nuevo) | Ninguno de los que añaden `mod` |
| **E8-04** Persistencia de vectores (**condicional**) | Solo si D2 = persistir y ADR nuevo aceptado (§5.4). Vectores como derivados ligados a su revisión y a su perfil de modelo, en el formato con un bit de característica nuevo | E8-03; decisión D2; ADR nuevo | `crates/kerno-units/src/vector_store.rs` (nuevo), `crates/kerno-units/tests/derived_vectors.rs` (nuevo), `docs/file-format/` (documento nuevo para el bit), `crates/kerno-units/src/lib.rs` | — |
| **E8-05** Supresión de fuentes y derivados | Borrar una unidad, una revisión (retención) o una relación elimina en el mismo `ChangeSet` los derivados de esa fuente (§5.3). Prueba de qué sobrevive, en la forma de E9A-01, con centinelas también en vectores y resúmenes | E8-03; E8-04 si D2 = persistir | `crates/kerno-units/src/suppression.rs` (nuevo), `crates/kerno-units/src/lib.rs`, `crates/kerno-units/tests/derived_suppression.rs` (nuevo) | — |
| **E8-06** Consolidación con procedencia | Contrato de resumen como derivado: fuentes obligatorias, descriptor, y el original siempre legible. Adaptador como trait sin red; una implementación de prueba determinista. Sin modelo real | E8-03, E8-05 | `crates/kerno-units/src/consolidation.rs` (nuevo), `crates/kerno-units/src/lib.rs`, `crates/kerno-units/tests/consolidation.rs` (nuevo) | — |
| **E8-07** Medición | Calidad sobre el conjunto nuevo (§6.2) y rendimiento (§6.3). Salida cruda comprometida | E8-01 a E8-06 | `crates/kerno-units/tests/derived_measure.rs` (nuevo), `crates/kerno-units/examples/derived_bench.rs` (nuevo), `docs/architecture/etapa8-medicion.md` (nuevo), `docs/architecture/etapa8-medicion-salida.txt` | — |
| **E8-08** Cierre (**hecho** el 2026-11-12, [cierre](etapa8-cierre.md)) | Revisión adversarial de E8-01 a E8-07. Promoción de I-25, I-38 y las nuevas (§7). Actualización de [invariantes](invariants.md) (I-25 pasa de «Fase 5» a Etapa 8), roadmap y [SAD](SAD.md) como versión nueva numerada | E8-00 a E8-07 | `docs/architecture/etapa8-cierre.md` (nuevo), `docs/architecture/invariants.md`, `docs/roadmap/README.md`, `docs/architecture/SAD.md` (nueva versión) | — |
| **E8-09** Caso de uso real (pre-1.0) | Memoria para un agente, con solo la API pública y como lo usaría un tercero. Usa el registro de derivados si E8-05 está cerrado | E8-05 (no bloquea el cierre de la Etapa 8) | `examples/agent-memory/**` (fuera del workspace, con dependencia por ruta), sin cambios en `Cargo.toml` del workspace | Puede correr después de E8-05, junto a E8-07 |

### 3.1 Qué corre a la vez

- **Ahora, sin esperar al propietario:** nada de E8 escribe código antes de la aprobación de esta acta. Lo único que puede correr a la vez tras la aprobación es **E8-S0**, que no toca código.
- **Tras E8-00:** E8-01 y E8-02 en paralelo, porque escriben ficheros distintos. Sus líneas `mod` en `lib.rs` se fusionan en el orden en que lleguen; cada uno comprueba el conteo de pruebas después de la fusión.
- **Secuencial:** E8-03 después de E8-02; E8-05 y E8-06 después de E8-03; E8-04 solo si D2 lo pide.
- **Camino crítico:** E8-00 → E8-02 → E8-03 → E8-05 → E8-07 → E8-08.

## 4. Criterios de aceptación comunes

Cada paquete con código cumple lo siguiente. Un paquete que no lo cumple no se cierra.

1. `cargo fmt --all --check` y `cargo clippy --workspace --all-targets --all-features -- -D warnings` sin errores. Clippy rechaza tipos de retorno complejos (`type_complexity`): usar alias de tipo.
2. `cargo test -p kerno-units -p kerno-testkit --all-features --release` en verde, con el conteo de pruebas anotado antes y después. El fichero nuevo debe aparecer en el conteo: si el número no se mueve, el runner no lo ve.
3. Toda prueba nueva se prueba en **los dos motores**, `emulated_log` y `kerno_db`, como en las Etapas 4 a 7.
4. **Cada prueba nueva se ve fallar** con una mutación aplicada a una **copia** del árbol, revertida y comprobada con `git diff`. La mutación queda registrada con su línea. Una mutación que deja la prueba en verde es un hallazgo, no un paso.
5. Donde un derivado responde a una consulta, la respuesta se compara con la **ruta de referencia**: la misma consulta sobre la misma instantánea, con el derivado sin usar (diferencial, I-32).
6. Ninguna prueba depende de tiempo, de orden entre pruebas ni de estado compartido.
7. Ningún literal de consulta, contenido ni vector aparece fuera del nivel `diagnostic` (I-37). Las pruebas de centinelas de la Etapa 5 se extienden a los nuevos tipos.
8. Ningún cambio en `Cargo.toml` ni `Cargo.lock` salvo que la decisión D2 o D8 lo pida. Si lo pide, se revisa aparte (I-31).

## 5. Derivados: contrato, invalidación y supresión

### 5.1 Qué es derivado hoy

| Dato | Derivado de | Dónde vive | Cómo se invalida hoy |
|---|---|---|---|
| Índice primario de unidades y `IDX_TYPE` | Las cabezas de las unidades | Persistido, en `kerno-units` (ADR-0007) | `status` y `watermark` (I-23). Una consulta que no está `Ready` en su instantánea va a la ruta de referencia |
| Caché de resultados | Estado de la base en una instantánea | Solo en memoria, desactivada por defecto | Por dependencias: `schema_version`, `planner_version`, estado e watermark del índice, `commit_seq` (`cache.rs`) |
| Vectores de búsqueda | Revisión fuente y perfil de modelo | **Entregados por quien llama, en memoria** | No aplica: no se guardan. Cada uso parte de lo que entrega quien llama |
| Ranking | Señales de la consulta y el resultado | Solo en memoria, apagado | No se persiste |

Las **adyacencias de relaciones** no son derivados: son primarias y van en el mismo `ChangeSet` que la relación (I-21).

Con esta tabla, el registro de ADR-0018 no nace vacío: su primer uso lo tienen el índice y la caché, que ya tienen estado y dependencias. El registro los puede describir sin cambiar su formato.

### 5.2 Invalidación por dependencia

- Cada derivado guarda, en su descriptor, **las fuentes exactas**: la unidad y la revisión (o el rango de `commit_seq`), el método o modelo con su versión, y los parámetros que cambian el resultado.
- **Cambiar una fuente** (un `ChangeSet` que toca la unidad o la revisión) marca como `stale` a sus derivados **en ese mismo `ChangeSet`**, a través del índice inverso. Así el estado nunca queda atrás de la base (I-21, I-23).
- **Cambiar el método, el modelo o la versión** no reescribe el derivado: crea uno nuevo y marca el anterior `superseded` (ADR-0018, opción 2).
- **Reconstruir** rehace el derivado desde las fuentes que sigan válidas. Su resultado debe ser igual al de construir de nuevo sobre los mismos datos (I-24). Un derivado nunca es la fuente de verdad: si se pierde, se rehace.
- **Una caché no se persiste** (la de resultados no lo hace), así que su invalidación ocurre en memoria y no toca el formato.

### 5.3 Cómo llega una supresión a los derivados persistidos

Esta sección es el tratamiento que pide ADR-0009 opción 3. Lo que sigue es lo que la Etapa 8 implementará, no lo que el motor hace hoy.

1. **Mismo `ChangeSet`.** Borrar una unidad, una revisión (retención) o una relación escribe, en el mismo `ChangeSet`, el borrado de sus derivados. Se obtienen con el índice inverso fuente → derivados. Es la regla de atomicidad de I-21 aplicada a derivados.
2. **Lo que se elimina de inmediato** (criterio I-1 de ADR-0009, extendido a derivados): tras el commit, ninguna lectura nueva devuelve el vector, el resumen o el descriptor de la fuente borrada. Cubre `get`, `scan` y las consultas de E8.
3. **Instantáneas abiertas** (criterio I-2): una instantánea tomada antes del commit sigue viendo sus derivados hasta que se suelta.
4. **Lo que sobrevive en el disco** (criterio C-1 aplicado a derivados): los bytes del vector o del resumen pueden seguir en un WAL o en una tabla hasta que una compactación cubra su rango. Tras `compact_range` y cierre limpio, el centinela de ese vector no aparece en ningún archivo vivo que ninguna instantánea retenga (C-1 y C-3). El plazo es el de ADR-0009 (C-4), no una duración.
5. **Resúmenes**: un resumen tiene como fuentes un conjunto de unidades. Borrar **cualquiera** de ellas borra el resumen en el mismo `ChangeSet`. El resumen nunca mantiene el texto de una fuente borrada, y nunca sustituye al original mientras el original exista (spec §10).
6. **Copias de seguridad**: no se declara ningún plazo. ADR-0009, B-1, pide una ventana declarada por escrito; esa declaración es «no hay función de backup en el motor; el plazo de las copias que haga el usuario no se promete». Cuando exista una función de backup, su ventana se declara en ese momento.
7. **Borrado criptográfico**: fuera de la Etapa 8. Depende de ADR-0006, pendiente. No se promete.
8. **Sin promesa de borrado físico**: el motor no garantiza que los bytes desaparezcan del medio (threat model §6.5).

Lo que E8-05 mide con centinelas: las etapas de E9A-01 (antes del flush, tras `compact_range`, tras cierre limpio, tras reapertura), para `delete` de una unidad con vector y con resumen. Las predicciones se escriben antes de ejecutar, como en E9A-01.

### 5.4 Formato persistente

Pregunta: **¿cambia el formato persistente en la Etapa 8?** La respuesta depende de D2 y de qué se persista.

- **Sin persistencia** (E8-02, E8-03, E8-05 y E8-06 en memoria): el formato no cambia. El registro se reconstruye desde las fuentes al abrir la base. Es válido porque un derivado es reconstruible (ADR-0018, criterio 2).
- **Con persistencia** de descriptores o de vectores (E8-04): **sí cambia**. La decisión de cómo se persiste, dónde viven las claves, qué bit y qué atomicidad está en [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md), aceptado el 2026-11-12. Debe ir por la regla de versionado del formato y no por una edición del formato congelado:
  - ADR-0003 §1 y §3: un cambio se añade como **bit de característica**. Un bit `compat` se ignora; un bit `ro_compat` abre solo en lectura; un bit `incompat` se rechaza.
  - Los derivados requieren **`incompat`**. Un lector antiguo que ignorase esos registros podría escribir sobre la base sin marcar sus derivados como obsoletos: un estado incorrecto invisible. ADR-0003 §3 dice que una versión antigua nunca escribe sobre un formato que no entiende.
  - **ADR-0013 no se edita**. Está aceptado y el formato v1 congelado el 2026-10-31. Los archivos v1 y sus vectores no cambian. El bit nuevo se describe en un documento de formato nuevo, y el cambio necesita un ADR nuevo.
  - **ADR-0011 no se edita**. Dice que persistir vectores exige un ADR que lo reemplace (ADR-0011, Consecuencias). El ADR nuevo tomará el siguiente número libre (0020; el último existente es 0019); el propietario lo asigna al abrirlo.
- **Versión de los datos derivados**: independiente de la versión física (ADR-0003 §4). Cambiar el modelo o el algoritmo marca los derivados como `superseded` y no migra el formato.

## 6. Conjunto de evaluación nuevo y umbrales

### 6.1 Conjunto nuevo

No se usa el conjunto de la Etapa 7 para medir nada de esta etapa. Ese conjunto ya ha visto los resultados (cierre de la Etapa 7, §8.2), y medirlo otra vez no aporta evidencia.

- **Semilla distinta** de la de la Etapa 7, fijada en el generador antes de escribir código de recuperación de E8.
- **Identificadores asignados por permutación aleatoria con semilla**, no en orden de creación. Así el desempate por identificador no favorece a ninguna estructura. Esto corrige la señal estructural que la Etapa 7 no pudo descartar (medición §5, punto 2).
- **Corpus**: unas 2 000 unidades para calidad, con relaciones de las cuatro clases de procedencia y distractores. Escala de rendimiento: 20 000 unidades (como en la Etapa 7).
- **Consultas**: al menos 100, en cuatro clases de al menos 20 cada una: recuperación por vecindad y vector; temporal; procedencia; y combinada. Un mismo conjunto contiene las consultas de vector, con vectores **generados por una función determinista con semilla** (ver D5).
- **Juicios**: grado de relevancia 0 a 3, escrito a partir del relato con el que se genera el corpus, no a partir del código de `kerno-units`. Un juicio nuevo exige volver a congelar el `MANIFEST` antes de medir.
- **Convención de juicio corregida**: las unidades **sin juzgar** en el top 10 de un brazo no cuentan como grado 0. Se excluyen del cálculo y su número se informa por brazo. Si más del 10 % de las posiciones del top 10 de un brazo están sin juzgar, el conjunto se considera incompleto y se juzgan más unidades antes de medir. Así se corrige el sesgo de R-5 de la Etapa 7 (§9).
- **Congelación**: `MANIFEST` con SHA-256 de cada fichero, comprobado por una prueba antes de cada medición, como en la Etapa 7.
- **Límite declarado**: el relato, las consultas y los juicios los escribe el mismo equipo. Ningún juicio independiente existe. Así se dice en el cierre.

### 6.2 Calidad (cortes en 10)

Los umbrales se fijan en esta acta, antes de cualquier medición del conjunto nuevo. Cada uno se comprueba contra la cota de su métrica antes de fijarlo (ver la columna de comprobación).

| Código | Umbral | Comprobación contra la métrica |
|---|---|---|
| **Q1** | La búsqueda vectorial exacta de E8 da el mismo resultado que la de fuerza bruta de referencia: recall@10 = 1,000 | El recall está en [0, 1]; 1,000 es alcanzable porque el brazo es la misma búsqueda |
| **Q2** | nDCG@10 con señal vectorial ≥ 0,80, en conjunto | nDCG está en [0, 1]; la Etapa 7 midió 0,912 con el ranking |
| **Q3** | Recall@10 con señal vectorial ≥ 0,90, en conjunto | Recall en [0, 1]; la Etapa 7 midió 0,939 |
| **Q4** | Con señal vectorial, nDCG@10 ≥ nDCG@10 sin ella − 0,01, **en conjunto y en cada clase** | Es una no inferioridad: la cota queda por debajo del valor medido sin ella, así que no exige más que 1 |
| **Q5** | Validez del conjunto: unidades sin juzgar en el top 10 ≤ 10 % de las posiciones, en cada brazo | Es una condición de validez, no de calidad. Si falla, no se mide: se juzga más |
| **Q6** | **Solo si el propietario propone activar el ranking por defecto en una etapa posterior**: nDCG@10 ≥ orden simple + 0,02 y no inferior en más de 0,01 en ninguna clase | La cota exige ≤ 1: si el orden simple supera 0,98, el umbral es inalcanzable y se reescribe antes de medir. **No es un criterio de cierre de la Etapa 8** |

Q6 se añade porque la decisión de §1.1 deja el ranking apagado y experimental. Esta etapa no la mide para activarlo.

Lo que **no** se fija aquí, y por qué: un umbral de calidad absoluto para el ranking con vectores. Sin una medición de la señal vectorial en un conjunto con vectores, no hay línea base de la que partir. El resultado de Q1 a Q5 es el que existe en el momento de cerrar E8.

### 6.3 Rendimiento

Los umbrales de rendimiento son de comparación entre rutas o entre escrituras, para que no dependan de la máquina. Se fijan aquí y se miden en la misma máquina, con las mismas repeticiones que en la Etapa 7.

| Código | Umbral | Comprobación |
|---|---|---|
| **P1** | Escritura de una fuente con su descriptor de derivado: p50 ≤ 1,50 × la escritura de la misma fuente sin descriptor, con 20 000 unidades | Un cociente de dos p50 medidos en la misma sesión; 1,50 es un techo, no una meta |
| **P2** | Búsqueda con vectores persistidos (solo si D2 = persistir): p50 ≤ 1,10 × la misma búsqueda con los vectores entregados en memoria | Mismos datos y misma consulta; un techo de 10 % de sobrecoste de lectura |
| **P3** | Invalidación: las entradas marcadas `stale` son exactamente los derivados de las fuentes que cambiaron | Es un contador, no un tiempo: se comprueba igualdad |
| **P4** | Reconstrucción: el resultado es igual, byte a byte, al de construir de nuevo (I-24) | Igualdad, no tiempo. El tiempo de reconstruir se informa sin umbral |

Sin umbral absoluto de latencia. La [acta de la Etapa 7](etapa7-acta.md) §6.3 tampoco lo tenía, y la [medición](etapa7-medicion.md) §4 mostró que el tiempo absoluto depende de la carga de la máquina. Si el propietario quiere un límite absoluto, es la decisión D9, y se fija antes de medir.

## 7. Invariantes

| ID | Estado actual | Lo que hace la Etapa 8 | Paquete |
|---|---|---|---|
| **I-25** | «Fase 5» en [invariantes](invariants.md) | Todo derivado de embedding referencia la revisión fuente exacta y el perfil de modelo; cambiar la fuente lo marca obsoleto. Se verifica y pasa a «Etapa 8» | E8-02, E8-03, E8-04 si D2. **Estado tras E8-08:** promovida a Etapa 8 para vectores persistidos ([invariantes §2.6](invariants.md)); la prueba que varíe solo el perfil falta (F-04). Pendiente de aprobación del propietario |
| **I-38** | «Etapa 8» | Todo derivado referencia sus fuentes exactas, su método y su versión; cambiar una fuente lo marca obsoleto o crea una versión nueva. Se verifica | E8-02, E8-03. **Estado tras E8-08:** verificada y promovida ([invariantes §2.6](invariants.md)). Pendiente de aprobación del propietario |
| **I-24** | Verificada en índices | Se extiende a derivados: reconstruido igual que nuevo | E8-03, E8-07 (P4) |
| **I-21** | Verificada en unidades y relaciones | Se extiende a derivados: el descriptor, su estado y su borrado van en el mismo `ChangeSet` que la fuente | E8-05 |
| **I-1, I-2** (ADR-0009) | Verificadas para el valor | Se extienden a derivados: invisibles tras el commit; visibles en la instantánea abierta | E8-05 |
| **I-31** | Verificada por revisión de dependencias | Ningún adaptador ni contrato de consolidación introduce red ni modelo en el núcleo | E8-06, E8-08 |
| **I-37** | Verificada en observaciones | Los centinelas de vectores y resúmenes no salen fuera de `diagnostic` | E8-05, E8-07 |
| **I-41** | Propuesta de promoción (Etapa 7) | Sin cambio; el ranking apagado sigue cumpliéndola | — |
| **I-42** (**propuesta**) | No existe | Suprimir una fuente deja sin visibilidad a sus derivados en el mismo `ChangeSet` (§5.3) | E8-05. **Estado tras E8-08:** verificada en pruebas ([invariantes §2.6](invariants.md)); pendiente de aprobación del propietario |
| **I-43** (**propuesta**) | No existe | Un resumen nunca reemplaza el original mientras el original exista y sea auditable (spec §10) | E8-06. **Estado tras E8-08:** verificada en pruebas ([invariantes §2.6](invariants.md)); pendiente de aprobación del propietario |

I-42 e I-43 son propuestas. Ninguna se promueve hasta que E8-08 la verifique y el propietario la apruebe. La [acta de adelanto de la Etapa 9](etapa9-adelanto-acta.md) §6 dice que, si ADR-0009 se acepta, la Etapa 8 propondrá su invariante de supresión. Esta acta la propone (I-42) sin ir más allá de ADR-0009.

## 8. Decisiones que el propietario debe tomar

Cada una tiene su recomendación. La elección del propietario de cada una está en §8.1, después de esta tabla.

| # | Decisión | Opciones y consecuencias | Recomendación |
|---|---|---|---|
| **D1** | ADR-0018 | (a) Aceptar la opción 2 (registro común). (b) Aceptar con cambios. (c) Mantener propuesto y dejar E8 sin entrar | **(a)**. Sin registro común, cada derivado trae su propia lógica de supresión y de frescura (ADR-0018, opción 1) |
| **D2** | ¿Se persisten los vectores en la Etapa 8? | (a) **Sí**, como derivados con bit `incompat` y ADR nuevo (§5.4; [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md)). Cumple «invalida o reconstruye» con datos reales. Exige cambio de formato. (b) **No**: el registro solo describe los vectores que entrega quien llama. La aceptación de «derivado vinculado a sus fuentes» se cumple solo en memoria. (c) Aplazar a una etapa posterior con ANN | **(a)**, si E8-03 pasa. Sin persistencia no hay derivado persistido que invalidar, y la aceptación de la Etapa 8 queda vacía en su parte de embeddings. Estado: ADR-0020 aceptado el 2026-11-12; E8-03 pasó |
| **D3** | Ranking por defecto en la Etapa 8 | Ya decidido el 2026-11-12: apagado y experimental | Ninguna acción |
| **D4** | Umbrales Q1 a Q5 y P1 a P4 de §6 | (a) Aprobarlos tal como están. (b) Modificar antes de medir. Una vez medido el conjunto nuevo, cambiarlos es un incumplimiento, no una corrección | **(a)** |
| **D5** | Origen de los vectores del conjunto | (a) **Vectores deterministas con semilla**, generados por función de prueba. Miden la mecánica de la búsqueda y del registro, no la calidad semántica de un modelo. (b) Un modelo externo, ejecutado fuera del núcleo y de la CI, con su coste y su versión registrados. No se nombra aquí ningún modelo. Exige decisión aparte | **(a)** en esta etapa. Las consecuencias de (b) son distintas: no hay medición de calidad de embeddings reales hasta que se decida |
| **D6** | Caso de uso real (E8-09): cuándo | (a) Después de E8-05, antes de 1.0. (b) Tras el cierre de la Etapa 8. (c) Después de 1.0 | **(a)**. Es el único punto donde se prueba la API desde fuera; un caso así en la Etapa 9 llegaría tarde |
| **D7** | SQLite: umbrales frente a SQLite (pendiente 3 del roadmap) | (a) Sin umbrales, solo informe (decisión de 2026-11-11). (b) Fijar umbrales antes del informe | **(a)**. Un umbral frente a SQLite sin datos previos repite el error de T4 |
| **D8** | Relaciones inferidas | (a) Fuera de la Etapa 8 (§2.3). (b) Dentro, con su propio descriptor de procedencia (spec §12, I-39) | **(a)**. El roadmap no las pone entre los entregables de la Etapa 8. Si entran, sus invariantes se cumplen primero en la Etapa 4 o 5 |
| **D9** | Límite absoluto de latencia de búsqueda | (a) Ninguno: solo comparaciones entre rutas (§6.3). (b) Un límite fijado antes de medir | **(a)**. ADR-0011 dice que un límite incumplido abre una decisión nueva, y no hay línea base de latencia con vectores persistidos |
| **D10** | Reescribir T4 (cierre de la Etapa 7, §8.2) | (a) Reescribirlo antes de cualquier medición nueva con un umbral alcanzable, congelado con fecha. (b) Dejarlo como está en el acta de la Etapa 7 | **(a)**, con el texto de la Etapa 7 sin cambiar. No afecta al conjunto nuevo de §6, que tiene sus propios umbrales |
| **D11** | Backups: ventana declarada (ADR-0009, B-1) | (a) Declarar que el motor no tiene función de backup y que el plazo de las copias no se promete. (b) Esperar a una función de backup antes de escribir la declaración | **(a)**. Es la única declaración verdadera hoy |
| **D12** | Consolidación: alcance de E8 | (a) Solo el contrato de resumen como derivado, con un adaptador de prueba (E8-06). (b) Además, un resumidor real | **(a)**. Un resumidor real necesita un modelo, que la Etapa 8 no fija (D5) |

### 8.1 Elecciones del propietario (2026-11-12)

El propietario aprobó esta acta el 2026-11-12 con la recomendación de cada decisión, salvo D10. D3 y D7 ya estaban decididas antes de esta aprobación, y se registran así.

| # | Elección del propietario | Origen |
|---|---|---|
| **D1** | **(a)**: ADR-0018 aceptado con la opción 2 (registro común de derivados) | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D2** | **(a)**: los vectores se persisten en la Etapa 8 como derivados, con bit `incompat` y ADR nuevo (previsto como ADR-0020, §5.4). Condicionado a que E8-03 pase. El formato v1 congelado no se toca. **Condición cumplida el 2026-11-12** (E8-03: 46 y 13 pruebas en verde). **ADR-0020 aceptado el 2026-11-12 por delegación del propietario**, revocable por él. El bit se declara como versión 2 de la capa de unidades, no en el motor (ADR-0020, §Opciones 2) | Aprobación de la acta, 2026-11-12, con la recomendación; ADR-0020 por delegación |
| **D3** | Ya decidida: ranking desactivado por defecto y marcado como experimental. Sin acción adicional | Decisión del propietario de 2026-11-12 (§1.1, punto 2) |
| **D4** | **(a)**: umbrales Q1 a Q5 y P1 a P4 de §6 aprobados tal como están | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D5** | **(a)**: vectores deterministas con semilla, generados por una función de prueba. No se usa ningún modelo externo | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D6** | **(a)**: caso de uso real (E8-09) después de E8-05 y antes de 1.0 | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D7** | Ya decidida: informe SQLite sin umbrales (opción (a)) | Decisión del propietario de 2026-11-11 (roadmap, decisiones de 2026-11-11, punto 2). No es una decisión nueva |
| **D8** | **(a)**: relaciones inferidas fuera de la Etapa 8 | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D9** | **(a)**: sin límite absoluto de latencia de búsqueda; solo comparaciones entre rutas (§6.3) | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D10** | **(b)**: T4 del acta de la Etapa 7 se deja como está y no se reescribe. **No es la recomendación, que era (a)** | Elección del propietario, 2026-11-12 |
| **D11** | **(a)**: se declara que el motor no tiene función de backup y que el plazo de las copias no se promete | Aprobación de la acta, 2026-11-12, con la recomendación |
| **D12** | **(a)**: solo el contrato de resumen como derivado, con un adaptador de prueba (E8-06). Sin resumidor real | Aprobación de la acta, 2026-11-12, con la recomendación |

Las dos condiciones que este párrafo pedía están cumplidas el 2026-11-12: la aprobación de esta acta (§10) y el estado de ADR-0018 y ADR-0009 escrito en sus archivos.

## 9. Riesgos

| ID | Riesgo | Tratamiento propuesto |
|---|---|---|
| **R-E8-1** | ADR-0009 aceptado en el propietario pero no en el archivo: E8 se apoya en una decisión que el repositorio no muestra | E8-00 exige el estado escrito en `0009` y en `docs/adr/README.md` |
| **R-E8-2** | El conjunto nuevo se ajusta a la implementación, como pasó con el de la Etapa 7 | Semilla y permutación de identificadores fijadas antes de escribir código de recuperación; `MANIFEST` con SHA-256; juicios escritos desde el relato, no desde el código |
| **R-E8-3** | La convención de juicio favorece a un brazo, como R-5 de la Etapa 7 | Unidades sin juzgar excluidas y contadas por brazo; Q5 como condición de validez |
| **R-E8-4** | Un conjunto sintético no representa la memoria real del agente | Se declara en el cierre. El caso de uso real (E8-09) mide la API desde fuera, pero no la calidad semántica |
| **R-E8-5** | Los vectores deterministas con semilla no dicen nada sobre calidad semántica de un modelo | Declarado en D5. Q1 a Q4 miden la mecánica, no la calidad de embeddings reales |
| **R-E8-6** | Cambiar el formato sin el bit adecuado deja a un lector antiguo escribir sobre derivados no invalidados | Bit `incompat` (§5.4); prueba de rechazo de un lector que no conoce el bit, con el directorio igual byte a byte |
| **R-E8-7** | Una supresión deja bytes del vector o del resumen en archivos vivos hasta la compactación | Es el comportamiento que ADR-0009 opción 3 acepta. Se mide con centinelas en E8-05 y se declara como límite. No se promete borrado físico |
| **R-E8-8** | Una instantánea abierta retiene derivados borrados | Criterio I-2 de ADR-0009. El plazo es la liberación de la instantánea (C-4), no una duración |
| **R-E8-9** | Un derivado queda `stale` pero se sirve como vigente tras reabrir la base, si el registro no se persiste | Sin persistencia, el registro se reconstruye al abrir; un derivado no reconstruido se marca `failed` o no se sirve, nunca `ready` (I-23). Con persistencia, lo cubre R-E8-6 |
| **R-E8-10** | El registro crece en el índice inverso con muchos derivados por fuente | ADR-0018, condiciones para reabrir: si el coste del índice inverso es desproporcionado, se reabre el ADR. Medir en E8-07 |
| **R-E8-11** | Un resumen real podría reemplazar silenciosamente al original | I-43 (propuesta). El contrato de E8-06 obliga a que el original siga legible. Revisión adversarial en E8-08 |
| **R-E8-12** | H-1 del índice (cierre de la Etapa 7, R-2): con 64 relaciones o más, una entrada ausente en ambos extremos no se detecta en consulta | Sigue abierto. E8 no lo cierra. Los derivados que dependen de adyacencia no se declaran completos por el índice: se verifican con `verify_units` o con la ruta de referencia |
| **R-E8-13** | Los derivados de la Etapa 8 heredan el sesgo de la convención de juicio de la Etapa 7 si se reutiliza su código de medición | E8-07 no reutiliza `assoc_measure.rs`: escribe su propia medición, con la convención de §6.1 |
| **R-E8-14** | El informe SQLite se lee como umbral aunque no lo tenga | Lo dice el título y el texto de E8-S0: sin umbrales (D7) |
| **R-E8-15** | Retrasar E8-09 hasta 1.0 deja la API sin probar desde fuera | D6. Si el propietario elige (c), se registra como riesgo |

## 10. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-12 | **Propuesta. Pendiente de aprobación del propietario** | Ninguna parte de la Etapa 8 se aprueba por delegación. Gates de E8-00: esta acta, ADR-0018, ADR-0009 escrito en su archivo, cierre de la Etapa 7 marcado como aprobado, y decisiones D1 a D12. Ningún código de la Etapa 8 antes de esa aprobación |
| 2026-11-12 | **Cierre de la Etapa 8 (E8-08), hecho** | [Cierre](etapa8-cierre.md). Cerrada como trabajo de ingeniería; Q4 registrada como no demostrada; sin hallazgo bloqueante abierto; decisiones (a) a (e) tomadas por delegación del propietario el 2026-11-12, revocables por él. Ningún umbral ni fichero congelado modificado |
| 2026-11-12 | **Aprobada por el propietario** | D1 a D12 con la recomendación de §8, salvo D10: opción (b), T4 no se reescribe (§8.1). ADR-0018 aceptado con la opción 2 y ADR-0009 aceptado con la opción 3, ambos en su archivo. Cierre de la Etapa 7 aprobado. E8-00 queda hecho; E8-S0 sigue pendiente. No se modifica ningún umbral de §6 |
</content>
