# Question review (phase 2)

For each question mark: keep / edit / drop, and whether the evidence justifies the answer.
Nothing is frozen until this review is done.

## a-kerno-SAD.txt (30 questions, 0 quotes not found)

### a01 [trajectory] ¿Cómo pasa la elección del motor físico de almacenamiento de ser una decisión abierta en la base 0.1.0 a una decisión resuelta con evidencia en la 0.2.0?

**Reference:** En la 0.1.0 la elección entre B+ tree copy-on-write y LSM figura como decisión que no debe asumirse; la §7.1 indica que depende de ADR-0001 y no de popularidad o resultados de terceros. La 0.2.0 registra como resuelta la elección por LSM nivelado (ADR-0001, Aceptado el 2026-10-28), con el protocolo de commit y la recuperación ya cerrados por ADR-0002.

- `8484:8552` Elección definitiva del motor físico: B+ tree copy-on-write o LSM.
- `17625:17745` La selección queda pendiente de ADR-0001 y no debe basarse solo en popularidad o en resultados publicados por terceros.
- `9493:9937` Resueltos: la elección del motor físico (**LSM nivelado**, ADR-0001), el protocolo de commit, la sincronización y la recuperación (ADR-0002) y las reglas de versionado e integridad (ADR-0003), todos Aceptados el 2026-10-28; la representación binaria concreta es el [formato v1](../file-format/v1.md), que sigue siendo **candidato a congelación** hasta que el propietario acepte [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md).

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a02 [contrast] ¿Coincide el plan de almacenamiento de revisiones de la §8.3 con lo que se construyó en la Etapa 3?

**Reference:** La §8.3 prevé un modelo híbrido de snapshots completos y deltas estructurales. El estado de la Etapa 3 indica que solo se construyeron revisiones de copia completa, porque los deltas no se implementan, aunque sí se construyó restore como nueva revisión.

- `20776:20854` Se evaluará un modelo híbrido de snapshots completos y deltas estructurales.
- `19368:19643` Lo construido: revisiones de copia completa (§8.2 y §8.3; los deltas no se implementan, decisión 4 de la Etapa 3), `restore` que añade una revisión nueva (§8.3, I-22), políticas `none`/`audit`/`versioned` por tipo (§8.4) y retención por lotes dentro de la política.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a03 [trajectory] ¿Cómo evoluciona el plan para la búsqueda vectorial desde la primera fase hasta los índices aproximados tipo HNSW?

**Reference:** Primero se implementa búsqueda vectorial exacta como referencia (baseline). Los índices aproximados solo se introducen después, y el plan exige medirlos frente a esa referencia; HNSW aparece en la secuencia tras la búsqueda exacta y solo si se mide su coste.

- `27246:27404` Primero se implementará búsqueda vectorial exacta como baseline; los índices aproximados deberán medirse contra ella en recall, latencia, memoria y disco.
- `24408:24503` | 6 | HNSW u otro ANN | baja latencia a escala | solo tras medir el coste de búsqueda exacta |
- `10862:10981` **Medir antes de especializar.** HNSW, LSM, B+ tree, grafo u otras estructuras se justifican por workloads y métricas.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a04 [trajectory] ¿Cómo se trata el servidor de red a lo largo del documento, desde el alcance inicial hasta el futuro?

**Reference:** El servidor de red queda fuera de la primera versión, tanto como servidor como protocolo remoto, y el MVP no incluye autenticación de red. Más tarde, la §11.1 lo reconoce como algo que un servidor futuro necesitaría, con autenticación, TLS, aislamiento de tenants y administración propia.

- `6813:6869` Servidor de red, servicio gestionado o protocolo remoto.
- `11478:11535` No habrá un servidor ni autenticación de red en el MVP.
- `27695:27815` Un futuro servidor Kerno necesitará controles de autenticación, TLS, aislamiento de tenants y administración propios.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a05 [trajectory] ¿Cómo se desarrolla el diseño de identificadores públicos desde la decisión sobre UUIDv7 hasta la elección pendiente de estrategia de token?

**Reference:** La decisión D-05 fija UUIDv7 solo como identidad interna y deja la representación pública configurable. La §5.2 la encapsula en una abstracción IdentifierCodec, y la §2.2 deja abierta la estrategia predeterminada entre token con mapeo y token cifrado reversible.

- `7742:7811` UUIDv7 para identidad interna; representación pública configurable.
- `13463:13556` La representación pública será intercambiable mediante una abstracción `IdentifierCodec`.
- `8854:8960` Estrategia predeterminada de identificador público: token aleatorio con mapeo o token cifrado reversible.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a06 [trajectory] ¿Cómo evoluciona el modelo de concurrencia desde el acuerdo inicial de un escritor hasta las restricciones y preguntas abiertas que aparecen después?

**Reference:** La decisión D-03 acuerda un escritor y varios lectores dentro del mismo proceso. La §4.2 descarta explícitamente promesas de escritura multi-proceso, y la §2.2 deja abierto el modelo concreto de MVCC, snapshots y bloqueo entre procesos.

- `7595:7663` Un escritor y varios lectores concurrentes dentro del mismo proceso.
- `12064:12147` Ninguna promesa inicial de escritura multi-proceso, servidor remoto o replicación.
- `8708:8779` Modelo concreto de MVCC, snapshots de lectura y bloqueo entre procesos.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a07 [trajectory] ¿Cómo se relacionan las garantías de confirmación de escritura con los criterios para publicar una beta?

**Reference:** La §6.2 establece que no se confirma una escritura antes de cumplir la durabilidad prometida por la configuración. La §18.3 convierte la pérdida reproducible de una operación confirmada, entre otras cosas, en motivo para no publicar una beta.

- `16809:16915` No se confirma una escritura antes de cumplir la garantía de durabilidad prometida por la configuración.
- `47149:47472` No publicar una versión beta si existe una pérdida reproducible de una operación confirmada bajo la configuración de durabilidad prometida, un camino de recuperación no probado, una migración destructiva sin backup validado o una discrepancia conocida entre índices e información fuente que se oculte al consumidor.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a08 [trajectory] ¿Cómo se relacionan el journal de recuperación y el historial de revisiones a lo largo del documento?

**Reference:** La decisión D-06 establece que el historial de revisiones es independiente del journal de recuperación. La §3 los trata como mecanismos distintos con ciclos de vida diferentes, y la §7.3 admite que compartan secuencias de commit pero no la política de retención.

- `7823:7896` El historial de revisiones es independiente del journal de recuperación.
- `10493:10578` WAL, revisiones y auditoría son mecanismos distintos, con ciclos de vida diferentes.
- `18996:19132` Estos sistemas pueden compartir secuencias de commit o metadatos, pero no compartirán implícitamente la misma política de retención.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a09 [contrast] ¿Los índices secundarios se mantienen dentro de la misma frontera atómica que la transacción, o como estado derivado aparte?

**Reference:** La §9.2 dice que los índices necesarios para la coherencia de una transacción se actualizan en la misma frontera atómica, y que los costosos pueden ser derivados y asíncronos. El estado de la Etapa 4 mantiene los índices de unidades como derivados, escritos en su propio ChangeSet, hasta que un workload con escrituras intercaladas justifique cambiarlo.

- `24953:25076` Los índices necesarios para que una transacción se observe coherentemente se actualizarán en la misma frontera atómica.
- `22670:22843` Se mantienen derivados hasta que un workload con escrituras intercaladas justifique mantenerlos dentro de cada `ChangeSet` (decisión del propietario, 2026-11-05, ADR-0007).

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a10 [trajectory] ¿Cómo evoluciona el tratamiento de los esquemas desde la primera versión hasta la separación de versiones y migraciones?

**Reference:** La primera versión admite documentos sin esquema rígido global y deja para más adelante validación por colección y versiones de esquema. La §13 añade que cada migración necesita precondiciones, validación y una ruta de restauración desde backup, y que el esquema lógico es un eje de versión separado.

- `14083:14176` La primera versión permitirá documentos estructurados sin exigir un esquema rígido global.
- `14177:14299` Más adelante podrán definirse validación por colección, tipos de campos, índices declarativos y versiones de esquema.
- `30370:30509` Toda migración tendrá precondiciones, pasos de ejecución, validación posterior y una ruta documentada de restauración desde un backup.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a11 [trajectory] ¿Cómo se trata la verificación de los backups a lo largo del documento?

**Reference:** La §12 exige que la restauración de un backup sea una operación probada y no una promesa basada en la existencia de un archivo. El criterio del ADR-0009 pide una prueba de extremo a extremo que demuestre qué plazo siguen los backups.

- `29921:30053` Una restauración de backup será una operación probada, no una promesa basada únicamente en la existencia de un archivo de copia.
- `37263:37414` Criterios: una prueba de extremo a extremo demuestra qué se elimina inmediatamente, qué queda sujeto a compactación y qué plazo siguen los backups.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a12 [trajectory] ¿Hasta qué punto la eliminación de datos alcanza a los datos derivados, las revisiones y los backups?

**Reference:** La §8.4 advierte que no toda eliminación se refleja al instante en cada soporte secundario. La §10.3 exige aplicar el borrado también a vectores, chunks, índices y colas, y la §19 identifica como riesgo un borrado que no cubra revisiones, índices y backups.

- `21671:21796` No se promete inmutabilidad ilimitada ni que toda eliminación pueda reflejarse instantáneamente en cada soporte secundario.
- `26780:26871` **Borrado:** aplicar la política también a vectores, chunks, índices y colas pendientes.
- `48115:48220` | Borrado incompleto | Retención no intencional | política que abarque revisiones, índices y backups |

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a13 [trajectory] ¿Cómo se fijan las reglas de medición de benchmarks, desde la metodología general hasta el momento en que se definen los umbrales?

**Reference:** El ADR-0012 busca medir sin declarar éxito a partir de una única prueba sintética. La §17.4 pide no fijar un umbral de velocidad antes de medir el baseline, y la §19 relaciona los benchmarks poco representativos con decisiones equivocadas.

- `38443:38526` Pregunta: cómo medir sin declarar éxito a partir de una única prueba sintética.
- `44405:44523` No establecer un umbral de velocidad antes de medir el baseline; definir objetivos después de conocer los resultados.
- `48329:48441` | Benchmarks poco representativos | Decisiones equivocadas | workloads de LiveSpec, Cauce y memoria de agentes |

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a14 [trajectory] ¿Cómo se separa la generación de embeddings del núcleo a lo largo del documento?

**Reference:** La decisión D-08 deja la generación de embeddings fuera del núcleo y la encomienda a adaptadores externos. La tabla de módulos exige que kerno-embeddings no tenga dependencias obligatorias de red o modelo, y el criterio M0 pide que no haya dependencia funcional obligatoria de IA, red o proveedor externo.

- `7994:8078` Embeddings mediante adaptadores externos; su generación no forma parte del núcleo.
- `15381:15513` | `kerno-embeddings` | contratos de adaptador, versiones y jobs derivados | sin dependencias obligatorias de red/modelo en el core |
- `45200:45276` No hay una dependencia funcional obligatoria de IA, red o proveedor externo.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a15 [causal] ¿Por qué debe cerrarse la decisión de almacenamiento antes de avanzar con el formato físico?

**Reference:** La §15 indica que la decisión sobre ADR-0001 debe cerrarse antes de congelar el formato físico, porque de lo contrario el formato queda atado a un motor no decidido. La §19 considera que elegir mal entre B+ tree y LSM obligaría a rehacer el formato y la recuperación, y la §20 pide aprobar el formato y la semántica de commit antes de crear bases de datos de usuarios.

- `33762:33842` Consecuencia: esta decisión debe cerrarse antes de congelar el formato físico.
- `47647:47745` | Elegir mal B+ tree/LSM | Rehacer formato y recuperación | M0 con ADR y prototipos comparables |
- `49212:49299` Aprobar el formato y la semántica de commit antes de crear bases de datos de usuarios.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a16 [causal] ¿Por qué el token público cifrado reversible no se adopta todavía como formato de identificador público?

**Reference:** Un token cifrado reversible añade gestión criptográfica y rotación de claves, mientras que un token con mapeo añade una correspondencia almacenada. Por eso la §11.2 exige definir antes la gestión de claves y la rotación antes de publicar ese formato.

- `13737:13874` La primera opción añade una correspondencia almacenada; la segunda añade complejidad de gestión criptográfica y rotación de claves.
- `28011:28092` La gestión de claves y la rotación se definirán antes de publicar ese formato.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a17 [evolution] ¿Cómo cambia el estado de ADR-0001 desde el planteamiento inicial del documento hasta su aceptación?

**Reference:** La §15 dice que los ADR propuestos nacen en estado Propuesto / pendiente de aprobación, salvo los confirmados en la sección 2. El ADR-0001 aparece luego como Aceptado el 2026-10-28 con LSM nivelado, y la §17.5 lo exigía como criterio de terminación de M0.

- `32924:33049` su estado inicial es *Propuesto / pendiente de aprobación*, excepto los puntos explícitamente confirmados en la sección 2.
- `33129:33227` **ADR-0001 — Motor de almacenamiento físico.** Estado: **Aceptado** (2026-10-28): LSM nivelado.
- `44606:44706` ADR-0001 selecciona un enfoque o documenta por qué ninguna alternativa debe implementarse todavía.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a18 [evolution] ¿Cómo cambia el plan de índices secundarios entre la secuencia prevista y lo que implementó la Etapa 4?

**Reference:** El plan original proponía índices secundarios ordenados, compuestos, invertidos, vectoriales y de grafo, cada uno tras validar una condición. La Etapa 4 solo implementó el índice primario y un único índice secundario ordenado, IDX_TYPE, justificado por un workload medido, y la decisión de no crear índices sin workload quedó explícita.

- `23974:24100` | 2 | índices secundarios ordenados | igualdad, rangos, ordenación y filtros | tras validar transacciones y almacenamiento |
- `21862:22188` la Etapa 4 implementó, en `kerno-units`, la ruta de referencia sin índices, el índice primario (orden 1 de §9.1) y un único índice secundario ordenado, `IDX_TYPE` por `(unit_type, schema_version)`, justificado por un workload medido (W1, [etapa4-workloads.md](etapa4-workloads.md)); ningún índice se crea sin workload.
- `24707:24758` No se crearán todos los índices automáticamente.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a19 [evolution] ¿Cómo cambia el reparto de crates desde la propuesta de la 0.1.0 hasta el estado de la 0.2.0?

**Reference:** La 0.1.0 proponía nueve crates desde el primer día, aunque el documento aclara que la estructura es lógica. La 0.2.0 registra que solo existen tres crates del producto, y que los demás son la propuesta original y no existen todavía.

- `15628:15718` Esta estructura es lógica, no una obligación de crear nueve crates desde el primer día.
- `32436:32508` Los demás crates de la lista son la propuesta de la 0.1.0 y no existen.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a20 [evolution] ¿Cómo se gestiona el cambio de versión del documento y qué queda reflejado en la 0.2.0?

**Reference:** La versión 0.1.0 es la baseline aprobada, y los cambios posteriores se publican como una nueva versión del SAD en vez de editarse en silencio. La 0.2.0 solo reescribe las secciones enumeradas en su tabla de cambios, así que el resto del texto conserva el de la 0.1.0.

- `198:331` Baseline 0.1.0 con las actualizaciones de la Fase 1 que lista «Cambios de la versión 0.2.0»; el resto del texto no se ha reescrito
- `1055:1313` La versión 0.1.0 fue la baseline aprobada para revisión. Las decisiones posteriores se registran en [`docs/adr/`](../adr/README.md); cuando un ADR cambia algo de este documento, se publica una nueva versión del SAD en lugar de editar esta silenciosamente.
- `1630:1937` Nota de estado: el motor físico, el protocolo de commit y la representación binaria quedan resueltos por ADR-0001, ADR-0002 y ADR-0003 (Aceptados el 2026-10-28) y por el formato v1 ([ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md), Propuesto). Las demás decisiones de la lista siguen abiertas.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a21 [contrast] ¿Cómo se relaciona la separación entre WAL, revisiones y auditoría con la elección del mecanismo de recuperación?

**Reference:** La §3 presenta WAL como uno de los mecanismos distintos que deben separarse. La §7.2 y el ADR-0002 dejan abierto el mecanismo, porque el concepto de WAL no se impondrá si la arquitectura usa otro, de modo que el principio se formula sin asumir el mecanismo.

- `10493:10578` WAL, revisiones y auditoría son mecanismos distintos, con ciclos de vida diferentes.
- `18020:18116` el concepto de «WAL» no se impondrá si la arquitectura finalmente usa un mecanismo diferente.
- `34021:34145` Opciones: WAL con protocolo de commit; copy-on-write con raíz/manifest de commit; variante apropiada al motor seleccionado.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a22 [contrast] ¿Qué diferencia hay entre el modelo de un escritor por base de datos y lo que el documento permite inferir de él?

**Reference:** La decisión D-03 acuerda un escritor y varios lectores, lo que puede dar la impresión de que no hacen falta atomicidad ni aislamiento. La §4.2 advierte expresamente que no debe inferirse eso, porque el modelo de un escritor no elimina la necesidad de atomicidad, aislamiento ni manejo de cierres inesperados.

- `7595:7663` Un escritor y varios lectores concurrentes dentro del mismo proceso.
- `12304:12422` No debe inferirse que «un escritor» elimina la necesidad de atomicidad, aislamiento o manejo de cierres inesperados.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a23 [trajectory] ¿Cómo se trata el registro de auditoría desde su definición en la arquitectura hasta la política de contenido de cada modo de historial?

**Reference:** La §7.3 define la auditoría como configurable y minimizada. La política audit de la §8.4 conserva metadatos definidos y no necesariamente el contenido anterior, y la §11.4 prohíbe registrar automáticamente el contenido completo, con redacción y retención propias.

- `18904:18994` | Auditoría | registrar operaciones y contexto de seguridad | configurable y minimizada |
- `21353:21451` **audit:** conserva eventos/metadatos definidos, no necesariamente el contenido completo anterior.
- `28713:28825` No se registrará automáticamente el contenido completo de documentos, secretos, tokens ni payloads personales.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a24 [trajectory] ¿Cómo se trata el registro de actor y contexto de los cambios entre el modelo de revisión y la auditoría?

**Reference:** La §8.2 incluye actor, fuente o razón en la revisión solo si están disponibles y la política lo permite, sin inventar contexto cuando faltan. La §11.4 permite que la auditoría registre actor o referencia, junto con otros campos de correlación.

- `20564:20660` actor, fuente, razón o referencia a tarea, si están disponibles y permitidos por la política.
- `20662:20736` La ausencia de actor o razón debe ser válida; no se inventará contexto.
- `28596:28712` Los eventos podrán registrar actor/referencia, operación, recurso, resultado, timestamp, secuencia y correlación.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a25 [trajectory] ¿Cómo se trata la afirmación de cumplimiento legal a lo largo del documento?

**Reference:** El documento renuncia desde el principio a prometer cumplimiento legal automático: la §1.2 lo presenta como capacidad explícita sin prometer cumplimiento de una ley, y la §1.4 excluye garantizar conformidad legal de cualquier aplicación. La §19 cierra el riesgo de promesas legales excesivas con mapeo de controles y revisión jurídica.

- `6018:6129` Diseñar seguridad y privacidad como capacidades explícitas, sin prometer cumplimiento automático de una ley.
- `7271:7358` Garantía de que Kerno, por sí solo, hace legalmente conforme a cualquier aplicación.
- `48530:48665` | Promesas legales excesivas | Riesgo contractual y reputacional | mapear controles a requisitos; revisión jurídica del caso de uso |

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a26 [trajectory] ¿Cómo se pasa de la redacción de ADR iniciales al inicio de la implementación del núcleo persistente?

**Reference:** La §20 pide primero escribir ADR-0001 y ADR-0002 como borradores con hipótesis y criterios. La §17 limita el M0 a elegir el enfoque de almacenamiento sin escribir aún el motor completo, y la misma §20 deja la implementación del núcleo para después de cerrar M0.

- `48985:49070` Escribir ADR-0001 y ADR-0002 como borradores con hipótesis y criterios de decisión.
- `42458:42599` No escribir aún el motor completo. Elegir el enfoque de almacenamiento y registrar las garantías que la implementación deberá satisfacer.
- `49303:49388` Empezar la implementación del núcleo persistente únicamente después de cerrar M0.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a27 [trajectory] ¿Cómo se relacionan el orden transaccional y el UUIDv7 del identificador interno?

**Reference:** La §21 presenta RFC 9562 como fuente de UUIDv7 con su campo temporal. La §5.2 aclara que, aunque UUIDv7 da un orden temporal aproximado, el orden de commits lo asigna una secuencia monotónica de la base, no el identificador.

- `49800:49887` RFC 9562 — Universally Unique IDentifiers (UUIDs). Define UUIDv7 y su campo temporal.
- `13280:13461` UUIDv7 ofrece orden temporal aproximado, pero no será el orden transaccional de Kerno: el orden de commits se representará mediante una secuencia monotónica asignada por la base.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a28 [trajectory] ¿Cómo se relacionan la fase de decisiones de almacenamiento y la beta en el plan de implementación?

**Reference:** La Fase 0 existe para resolver la incertidumbre de almacenamiento antes de congelar el formato. La beta de la Fase 6 exige documentación, matriz de compatibilidad y criterios de uso, y el documento advierte que una beta segura puede requerir de seis a doce meses o más.

- `39319:39402` Objetivo: resolver la incertidumbre de almacenamiento antes de congelar el formato.
- `41954:42065` Salida: beta documentada con limitaciones, matriz de compatibilidad y criterios explícitos de uso recomendado.
- `42179:42361` Un prototipo básico puede aparecer antes; una beta segura y fiable puede requerir seis a doce meses o más, y la preparación para sistemas críticos requiere validación adicional.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a29 [local] ¿Cuánto tiempo se estimó inicialmente para la Fase 1, Núcleo persistente?

**Reference:** La Fase 1 se estimó en 3 a 5 semanas, y el documento aclara que es una estimación inicial.

- `40033:40100` Fase 1 — Núcleo persistente (3–5 semanas, estimación inicial)

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### a30 [causal] ¿Por qué la consulta responde por la ruta de referencia cuando un índice derivado no está listo?

**Reference:** La §9.2 obliga a aplicar una estrategia documentada cuando el índice está atrasado y prohíbe fingir frescura. La Etapa 4 aplica esa regla: los índices de unidades son derivados, y cuando no están listos, query contesta por la ruta de referencia sin fingir frescura.

- `25326:25540` Si el índice está atrasado, Kerno debe aplicar una estrategia documentada, como esperar, consultar una ruta exacta alternativa o devolver un estado de no disponibilidad para esa consulta; no debe fingir frescura.
- `22339:22669` Los índices de unidades son derivados (ADR-0007): los escribe `rebuild_index` en su propio `ChangeSet`, exponen `ready`/`building`/`stale`/`failed` y su watermark (I-23), un índice reconstruido es igual a uno nuevo (I-24) y `query` contesta por la ruta de referencia cuando el índice no está listo, sin fingir frescura (I-32).

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

## b-arxiv-2609.00063v1.txt (30 questions, 0 quotes not found)

### b01 [trajectory] How does the paper's account of LLM performance shift from its abstract to its conclusion regarding recall and the rejection of unsupported hypotheses?

**Reference:** The abstract frames LLMs as showing strong recall but poor rejection of unsupported hypotheses, presented as an analysis finding. The conclusion repeats this split but sharpens it, saying LLMs consistently struggle to reject unsupported claims and often produce hallucinated citations.

- `1899:2087` Our analysis shows that while LLMs exhibit strong recall, they often perform poorly at providing valid scientific articles and evidence for support and at rejecting unsupported hypotheses.
- `25073:25325` LLMs sometimes perform well at identifying true causal relationships, but they consistently struggle to reject unsupported claims and to reliably reference real, traceable evidence from the scientific literature, often producing hallucinated citations.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b02 [trajectory] How does the recall finding in the abstract relate to the specificity results reported later in the Results section?

**Reference:** The abstract credits LLMs with strong recall, which the Results confirm for most models. The Results then show specificity between 0.17 and 0.50, so the same models that rarely miss true hypotheses also fail to reject false ones.

- `1899:2087` Our analysis shows that while LLMs exhibit strong recall, they often perform poorly at providing valid scientific articles and evidence for support and at rejecting unsupported hypotheses.
- `14754:14932` The low specificity observed for all models ( 0.17 – 0.50 ) indicates a limited capacity to correctly identify false causal hypotheses, resulting in a high false positive rate.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b03 [trajectory] Across the paper, how is Mistral's performance and behavior described, from model size to its search strategy to its retrieval setup?

**Reference:** Mistral is introduced as one of the smallest models at about 12 billion parameters, which the paper links to its low specificity. Later it is described as relying on keyword searches that give illustrative examples without strong causal analysis, and as not retrieving external documents at all.

- `22374:22574` Mistral and Qwen are the smallest models with around 12 billion and 30 billion parameters respectively which likely explains their low specificity ( 0.17 ) and low percentage of real retrieved papers.
- `22012:22197` Mistral also does a keyword search through databases like PubMed, Web of Science, and Google Scholar, but mainly provides illustrative examples of papers without strong causal analysis.
- `23511:23689` Mistral, DeepSeek-R1, DeepSeek-V3, and Qwen stated that they do not retrieve external documents or search the web and their responses are generated from their internal knowledge.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b04 [trajectory] How is Llama-4-Maverick's standing traced across the paper's accuracy and evidence results?

**Reference:** Llama-4-Maverick is reported as the top model on F1, specificity and conclusion accuracy. It is the only model to reach a nonzero real-abstract rate, at 17%, even though most models fail to extract abstracts exactly.

- `22955:23067` Llama-4-Maverick achieved the highest F1 score ( 0.88 ), specificity ( 0.50 ), and correct conclusions ( 0.82 ).
- `16855:16993` All LLMs fail to accurately extract exact abstracts from the cited papers, except for Llama-4-Maverick, which achieves an accuracy of 17%.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b05 [trajectory] How does the paper's justification for using these 17 hypotheses change from the introduction to the methods, and what does it say about their ground truth?

**Reference:** The introduction says the hypotheses were not on the web during the study, so the LLMs could not have trained on them, and that clinicians validated them. The methods explain that the 17 hypotheses come from prior ICD-10 work and that four expert clinicians validated them, which gives a ground truth.

- `4645:5098` While the number of hypothesis we evaluate is fairly small (17), it is important to note that 1) when this study was conducted, these hypotheses were not available on the web and therefore could not have been used by the LLMs in their training, 2) the hypotheses have been validated by clinicians, and 3) we annotate all the LLM answers across multiple dimensions for a total of 1,067 annotation points which constitutes a significant annotation effort.
- `7806:8005` We use a set of 17 causal hypotheses discovered in prior work (Adhikari et al., 2025), which analyzes ICD-10 diagnosis codes associated with repeat emergency room (ER) visits among diabetic patients.
- `8006:8118` The hypotheses have been validated by four expert clinicians, therefore we have ground truth for their validity.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b06 [trajectory] How does the paper's research question on hypothesis validity lead into its research question on evidence, and how does Results section 3 connect the two?

**Reference:** RQ1 asks whether LLMs can accurately determine the validity of causal hypotheses, and RQ2 asks whether they produce accurate, relevant evidence for their claims. Results section 3 links them by checking whether the hypotheses that LLMs classify as true are actually backed by scientific evidence.

- `7626:7703` RQ1: Can LLMs accurately determine the validity of medical causal hypotheses?
- `7710:7804` RQ2: Do LLMs produce accurate and relevant scientific evidence to support their causal claims?
- `16078:16181` In RQ2, we evaluate whether hypotheses classified as true by LLMs are supported by scientific evidence.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b07 [trajectory] How does the annotation procedure move from the claim that multi-stage evaluation is essential to the agreement process used to build the final dataset?

**Reference:** The paper argues that a model can correctly label a hypothesis as true while hallucinating article metadata, so checking must happen at several levels. It then describes two annotators labeling every response independently, comparing their labels, and re-examining disagreements to reach a final consensus.

- `12239:12443` This multi-stage evaluation is essential because an LLM may correctly identify a hypothesis as true while simultaneously hallucinating or misrepresenting article metadata, references, or textual evidence.
- `12586:12676` To ensure reliability, two annotators independently label the entire set of LLM responses.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b08 [trajectory] How is the figure of 1,067 annotation points described in the abstract compared with the methods, and how does the count of criteria differ?

**Reference:** Both the abstract and the methods report 1,067 annotation points. The abstract says each piece of evidence was annotated on six criteria, while the methods describe five criteria applied to each real paper, so the criteria count is inconsistent.

- `1726:1898` We systematically annotate the scientific evidence they provide according to six criteria (a total of 1,067 annotation points) and assess them with nine evaluation metrics.
- `12926:13078` The final set contains 1,067 annotation points covering whether each provided paper is real, and if so, answering the five criteria for each real paper.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b09 [trajectory] How does the prompt's requirement for three supporting articles carry through to the quote-level results?

**Reference:** The prompt asks for the three strongest pieces of evidence from peer-reviewed papers, including direct quotes. The results then show that real quotes are rare, with 0 to 7 per model, and only GPT-4o is a partial exception.

- `8881:9017` If support is indicated, the model is instructed to provide the three strongest pieces of evidence from peer-reviewed scientific papers.
- `18467:18611` Real quotes supporting the hypotheses are generally rare (ranging from 0 to 7), with GPT-4o being a slight exception, producing seven instances.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b10 [trajectory] How does the paper's treatment of association versus causation evolve from the description of search strategies to the reported results?

**Reference:** In the search-process section, DeepSeek R1 is described as rejecting articles that show mere associations. The results then report association counts that are low for every model, from 0 to 3, which suggests most evidence was framed as causal.

- `20389:20453` It rejects as evidence any articles that show mere associations.
- `18727:19015` The association count, which indicates whether an article reports only an association rather than a direct causal relationship, is low across all models (0–3), showing that only a small portion of the evidence provided by LLMs reports associations instead of potential causal relations.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b11 [trajectory] How does the strictness of the prompt's exact-match requirement connect to the abstract-extraction failures reported later?

**Reference:** The prompt template insists on EXACT abstracts, titles and quotes and rejects summaries. The results then show that nearly every model fails to extract exact abstracts from the cited papers, with Llama-4-Maverick the only exception.

- `6354:6461` Emphasis on EXACT. I don’t want summaries of abstracts, summaries of quotes or summaries of paper titles.
- `16855:16993` All LLMs fail to accurately extract exact abstracts from the cited papers, except for Llama-4-Maverick, which achieves an accuracy of 17%.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b12 [trajectory] How is the design of the prompt linked to the parsing procedure, and what happens when the parsing fails?

**Reference:** The Methods say the standardized prompt yields highly structured outputs so that they can be parsed automatically. When automatic parsing failed in rare cases, the authors extracted the components by hand.

- `9181:9285` This design ensures highly structured outputs, facilitating automated parsing and systematic evaluation.
- `10819:10917` In rare cases where automatic parsing fails, we manually extract the components from the response.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b13 [trajectory] How do Gemini Pro's and DeepSeek-R1's evidence results connect across the Table 1 discussion and the model-size discussion?

**Reference:** Both models produce many real papers and correct URLs in the evidence table. The model-size discussion then identifies Gemini Pro as having the highest real-paper and correct-URL rates, with DeepSeek-R1 also performing well on real papers despite its small active parameter count.

- `17732:17955` DeepSeek-R1 and Gemini Pro yield a comparatively high number of real papers (26 and 28, respectively) as well as correct URLs (11 and 21), whereas models such as Qwen and Mistral produce few or no real papers or valid URLs.
- `22630:22891` Gemini 2.5 Pro, with an estimated 128-140 billion parameters, achieves the highest values for real papers ( 0.58 ) and correct URLs ( 0.44 ), while DeepSeek-R1, despite using only 37 billion active parameters per token, also performs well with 0.54 real papers.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b14 [trajectory] How does the paper's view of the number of references shift from the abstract's claim about valid articles to the Table 1 discussion?

**Reference:** The abstract says LLMs often perform poorly at providing valid scientific articles. The Table 1 discussion then shows that models generate similar totals of papers and URLs, so the number of references does not indicate that the evidence is reliable.

- `1899:2087` Our analysis shows that while LLMs exhibit strong recall, they often perform poorly at providing valid scientific articles and evidence for support and at rejecting unsupported hypotheses.
- `17956:18237` Although most models generate a comparable number of total papers and URLs (roughly 42–48), the proportion of valid evidence differs substantially across models, indicating that generating many references does not necessarily correspond to providing reliable supporting evidence.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b15 [trajectory] How does the paper's description of the 136 evaluated responses follow from the abstract's scope?

**Reference:** The abstract says eight LLMs were assessed on 17 hypotheses. The methods then say this yields 136 responses, each with three articles supporting the hypothesis, which gives the total number of evidence items the study had to check.

- `1545:1725` We assess the performance of eight LLMs on 17 medical causal hypotheses to evaluate whether they can reliably verify these hypotheses using scientific evidence from the literature.
- `10651:10818` As a result, we obtain responses from all eight LLMs across 17 hypotheses, for a total of 136 responses with 3 articles for each response that supports the hypothesis.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b16 [trajectory] How does GPT-4o's data collection and performance vary across the paper, from the methods to the recall and quote results?

**Reference:** GPT-4o is the only model whose responses were collected manually, because it has no free tier. Its recall is lower than most models at 0.54, while it produces the most real quotes, seven, showing a mixed profile across sections.

- `10275:10386` The only exception is GPT-4o, which does not offer a free tier, and from which we collected responses manually.
- `14544:14753` A recall of 1.0 for most models indicates that these models correctly identified all true causal hypotheses, whereas GPT-4o (recall = 0.54 ) failed to detect a substantial portion of true causal relationships.
- `18467:18611` Real quotes supporting the hypotheses are generally rare (ranging from 0 to 7), with GPT-4o being a slight exception, producing seven instances.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b17 [trajectory] How do the reasoning styles that the paper attributes to particular models relate to the performance numbers reported for those models?

**Reference:** The search-process section says DeepSeek-R1, ChatGPT and Gemini Pro reason more carefully than models that rely on surface keyword searches. The evidence table then shows DeepSeek-R1 and Gemini Pro with comparatively many real papers and correct URLs.

- `19876:20106` Deepseek R1, ChatGPT, and Gemini Pro demonstrate more sophisticated reasoning, while other models, including Mistral, Qwen, and Deepseek V3 rely more on surface level keyword searches for papers directly mentioning the hypothesis.
- `17732:17955` DeepSeek-R1 and Gemini Pro yield a comparatively high number of real papers (26 and 28, respectively) as well as correct URLs (11 and 21), whereas models such as Qwen and Mistral produce few or no real papers or valid URLs.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b18 [causal] Why are the specificity values of the smallest models low, according to the paper?

**Reference:** The paper attributes the low specificity of Mistral and Qwen to their small size, about 12 and 30 billion parameters. Low specificity is reported for all models, so the small-model explanation is a partial account of a general weakness.

- `22374:22574` Mistral and Qwen are the smallest models with around 12 billion and 30 billion parameters respectively which likely explains their low specificity ( 0.17 ) and low percentage of real retrieved papers.
- `14754:14932` The low specificity observed for all models ( 0.17 – 0.50 ) indicates a limited capacity to correctly identify false causal hypotheses, resulting in a high false positive rate.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b19 [causal] Why do the authors credit external document retrieval with better evidence quality, and how do they qualify that claim?

**Reference:** Models that retrieve external information produced more real papers, valid URLs and accurate quotes than models that relied only on internal knowledge. The authors then conclude that retrieval and internal reasoning both shaped evidence quality, so retrieval is not the only cause.

- `24153:24374` Models that are retrieving external information generally produced better results, including higher numbers of real papers, valid URLs, and accurate quotations, versus models that relied exclusively on internal knowledge.
- `24618:24796` This suggests that both external document retrieval and the model’s internal reasoning abilities contributed to the quality and verifiability of scientific evidence generation.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b20 [causal] What explains the over-prediction of true causal hypotheses, according to the paper?

**Reference:** Precision values between 0.69 and 0.79 show that models tend to over-predict true hypotheses, which produces many false positives. The search-process section suggests a cause: several models rely on surface keyword searches for papers that mention the hypothesis, not on checking causal evidence.

- `14392:14543` Precision values in the range of 0.69-0.79 indicate a tendency to over-predict true causal hypotheses, resulting in a higher number of false positives.
- `19876:20106` Deepseek R1, ChatGPT, and Gemini Pro demonstrate more sophisticated reasoning, while other models, including Mistral, Qwen, and Deepseek V3 rely more on surface level keyword searches for papers directly mentioning the hypothesis.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b21 [causal] Why do the authors say the evidence provided by LLMs is unreliable even when papers are supplied?

**Reference:** The authors state that even when papers are given, models often fail to supply correct sources. They attribute this to hallucinated citations or an inability to pull source information accurately from real papers, which the conclusion says leads to hallucinated citations.

- `16649:16854` This indicates that, even when papers are provided, LLMs often fail to supply correct sources, either due to hallucinated citations or an inability to accurately extract source information for real papers.
- `25073:25325` LLMs sometimes perform well at identifying true causal relationships, but they consistently struggle to reject unsupported claims and to reliably reference real, traceable evidence from the scientific literature, often producing hallucinated citations.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b22 [evolution] How does the paper's framing of its study size change from the abstract to the conclusion?

**Reference:** The abstract presents the work as a preliminary, small-scale study. The conclusion keeps that label but says the findings should be investigated further on a larger scale, so the framing moves from a finished result to a call for more work.

- `1215:1388` Here, we present a preliminary, small-scale study that investigates the accuracy of LLMs in evaluating causal medical claims and supporting them with peer-reviewed research.
- `25326:25481` These findings highlight a gap between causal reasoning and trustworthy scientific evidence grounding and should be further investigated on a larger scale.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b23 [evolution] How does the paper's recommendation on trusting LLMs for verification get stronger from the abstract to the conclusion?

**Reference:** The abstract says LLMs cannot yet be trusted fully to verify causal relationships. The conclusion goes further, saying they should be used with caution and should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- `2088:2250` These findings highlight a critical limitation of current LLMs, as they cannot yet be trusted fully to verify causal relationships from the biomedical literature.
- `25482:25691` These limitations indicate that, in their current form, LLMs should be used with caution for causal verification tasks and should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b24 [evolution] How does the instruction to answer 'No' when evidence is absent develop from the prompt design to the analysis of the models' search behavior?

**Reference:** The methods require that, when the literature does not support a hypothesis, the model answers No and explains its conclusion. The later analysis uses those explanations to compare how each model searched when no support existed.

- `9286:9450` If the scientific literature does not support the causal hypothesis, the model is instructed to respond with “No” and to explain how it reached this conclusion.
- `19782:19875` Our prompt asks each LLM to explain how it decided when there is no support for a hypothesis.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b25 [local] How many of the 17 hypotheses are supported by the clinician ground truth?

**Reference:** Eleven of the 17 hypotheses are supported, so the set mixes supported and unsupported claims.

- `8323:8561` Considering a mixture of supported and unsupported claims (11 of the 17 hypotheses are supported) ensures that the evaluation remains meaningful, while also enabling an effective assessment of an LLM’s ability to evaluate causal claims.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b26 [contrast] How does the paper's recommended use of LLMs in the introduction compare with its final guidance in the conclusion?

**Reference:** The introduction describes LLMs as having the potential to act as assistants in finding evidence for causal hypotheses. The conclusion instead says they should be used with caution for causal verification and not as autonomous tools for biomedical reasoning.

- `2824:3037` Large language models (LLMs) are particularly good at processing and organizing such unstructured text at scale and have the potential to act as assistants in finding evidence for causal hypotheses and mechanisms.
- `25482:25691` These limitations indicate that, in their current form, LLMs should be used with caution for causal verification tasks and should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b27 [contrast] Does retrieval guarantee the best overall performance, and how do the results for retrieval-using models compare with the overall best model?

**Reference:** Models that retrieve external documents generally give better evidence. However, the paper notes that retrieval does not guarantee the best overall performance, since Llama-4-Maverick has the top F1 and conclusion accuracy despite emphasizing internal reasoning.

- `24153:24374` Models that are retrieving external information generally produced better results, including higher numbers of real papers, valid URLs, and accurate quotations, versus models that relied exclusively on internal knowledge.
- `24375:24617` However, document retrieval does not guarantee the best overall performance, as Llama-4 Maverick achieves the highest F1 score (0.88) and conclusion accuracy (0.82) despite emphasizing internal reasoning rather than active document retrieval.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b28 [contrast] How does the paper describe Llama-4-Maverick's retrieval behavior in different sections, and do the descriptions agree?

**Reference:** The search-process section describes Llama's reasoning as a two-phase search that attends to underlying biology. The retrieval section says Llama retrieves several external documents from various databases, while another sentence calls its approach internal reasoning rather than active retrieval, so the descriptions conflict.

- `21496:21694` Llama’s reasoning follows a clean two-phase structure: an initial broad search, followed by a refined second search with tighter filters, paying attention to the biology underlying the hypothesis.
- `23805:23898` while Llama-4-Maverick reports retrieving several external documents using various databases.
- `24375:24617` However, document retrieval does not guarantee the best overall performance, as Llama-4 Maverick achieves the highest F1 score (0.88) and conclusion accuracy (0.82) despite emphasizing internal reasoning rather than active document retrieval.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b29 [contrast] Does model size fully explain the performance differences among the evaluated LLMs?

**Reference:** Smaller models like Mistral and Qwen show low specificity and few real papers, and larger models generally do better. However, the paper says model size alone does not fully explain performance and that architecture, training data and retrieval also matter.

- `22374:22574` Mistral and Qwen are the smallest models with around 12 billion and 30 billion parameters respectively which likely explains their low specificity ( 0.17 ) and low percentage of real retrieved papers.
- `23068:23129` However, model size alone does not fully explain performance.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### b30 [contrast] In the abstract, the study's criteria count is given as six, and in the methods it is given as five. How do these two passages describe the annotation criteria?

**Reference:** The abstract says the evidence was annotated on six criteria, which a reader would take as the full set. The methods say each real paper was judged on five criteria, so the two sections disagree on how many criteria were applied.

- `1726:1898` We systematically annotate the scientific evidence they provide according to six criteria (a total of 1,067 annotation points) and assess them with nine evaluation metrics.
- `12926:13078` The final set contains 1,067 annotation points covering whether each provided paper is real, and if so, answering the five criteria for each real paper.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

## c-gutenberg-244.txt (28 questions, 2 quotes not found)

### c01 [trajectory] How does Watson's account of his own nerves change between the opening chapter and the end of Part I?

**Reference:** In Chapter I Watson tells Stamford his nerves are shaken and he cannot stand noise, which is why he wants a quiet lodger. By the close, after the Brixton case, his nerves are still disturbed: even a veteran of the battlefield, he tingles at the thought of the murderer's crimes.

- `14978:15104` I keep a bull pup,” I said, “and I object to rows because my nerves are shaken, and I get up at all sorts of ungodly hours
- `109654:109740` My nerves, which were steady enough on the field of battle tingled as I thought of it.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c02 [trajectory] How is the bloodhound image used for different characters from Part I to the end of Part II?

**Reference:** Holmes first compares Lestrade's rival, the 'old hound', with his own 'hounds and not the wolf' metaphor, placing himself among hunters. Watson later calls himself and Holmes an 'amateur bloodhound' in the cab, and in Part II the image becomes Jefferson Hope himself, a 'human bloodhound' pursuing his enemies for years.

- `54565:54716` It’s all very well for you to laugh, Mr. Sherlock Holmes. You may be very smart and clever, but the old hound is the best, when all is said and done.
- `206668:206849` Year passed into year, his black hair turned grizzled, but still he wandered on, a human bloodhound, with his mind wholly set upon the one object upon which he had devoted his life.
- `72650:72785` Leaning back in the cab, this amateur bloodhound carolled away like a lark while I meditated upon the many-sidedness of the human mind.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c03 [trajectory] What role does Holmes's violin play across the novel, and how does Watson read it at the end of the Brixton investigation?

**Reference:** Early on Holmes's violin is a source of irritation that Watson tolerates, and Watson remarks that a well-played violin is a treat for the gods. Later, during quiet nights of thought, the violin's wailing becomes the sound of Holmes wrestling with the problem, which Watson hears from his bed.

- `15356:15430` A well-played violin is a treat for the gods—a badly-played one——”
- `86603:86751` long into the watches of the night I heard the low, melancholy wailings of his violin, and knew that he was still pondering over the strange problem

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c04 [trajectory] How does the word RACHE and the theories around it develop from Lestrade's discovery to Hope's own explanation?

**Reference:** Lestrade first reads RACHE as the beginning of the name Rachel, a woman's name, and Holmes corrects him that it is German for 'revenge'. Much later Hope explains that he copied the word from a New York case, hoping to mislead the police toward secret societies, so the inscription was a deliberate blind.

- `54439:54564` You mark my words, when this case comes to be cleared up you will find that a woman named Rachel has something to do with it.
- `58588:58690` “‘Rache,’ is the German for ‘revenge’; so don’t lose your time looking for Miss Rachel.”
- `228057:228228` I remembered a German being found in New York with RACHE written up above him, and it was argued at the time in the newspapers that the secret societies must have done it.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c05 [trajectory] How does the wedding ring pass from one person to another across the story, and what does it come to signify?

**Reference:** Lestrade finds the ring in Chapter III, where it first suggests a woman's involvement. Holmes uses a copy of it as bait in the advertisement, and Hope admits he took the real ring from Lucy's body at her burial. In his confession Hope holds it up before Drebber as he dies, so it becomes the emblem of Lucy's wrong and his vengeance.

- `50249:50329` There’s been a woman here,” he cried. “It’s a woman’s wedding-ring.”
- `202860:202901` he took the wedding-ring from her finger.
- `214250:214360` I took the marriage ring from her dead finger, and I vowed that his dying eyes should rest upon that very ring

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c06 [trajectory] How does Holmes's attitude to fame and recognition shift from the first chapters to the end of the novel?

**Reference:** At the start Holmes boasts that he means to make his name famous and that no one has brought his talent to crime. By the end he gives a bitter answer that what matters is what people can be made to believe, and he jokes that the whole study was only to win the police a testimonial.

- `243782:243863` “That’s the result of all our Study in Scarlet: to get them a testimonial!”

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c07 [trajectory] What happens to Stangerson across Part II, from his first appearance as an Elder to his death?

**Reference:** Stangerson first appears as the Elder who takes the castaway John Ferrier and Lucy into his waggon and teaches them the faith. Later he and Drebber are rivals for Lucy, and the Elder is said to have shot her father. He ends as Drebber's secretary who flees with him and is killed at Halliday's Hotel by Hope.

- `143938:144034` “Take him, Brother Stangerson,” he said, “give him food and drink, and the child likewise.
- `200998:201125` They’d both been in the party that followed them, and Stangerson had shot her father, which seemed to give him the best claim
- `185222:185257` “Shall I tell Brother Drebber?”

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c08 [trajectory] How does the countdown that the Mormon leadership imposes on John Ferrier develop from its announcement to the last days?

**Reference:** Brigham Young first gives Ferrier a month to choose a husband for Lucy, and he threatens him with death. Ferrier then receives a note with twenty-nine days, and the anonymous numbers count down daily on his walls until the last day, which the story explicitly identifies as the balance of Young's month.

- `166505:166578` “She shall have a month to choose,” said Young, rising from his seat.
- `175259:175319` Twenty-nine days are given you for amendment, and then——
- `175649:175735` The twenty-nine days were evidently the balance of the month which Young had promised.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c09 [trajectory] How does the poison pill Hope prepared from the York College laboratory connect to the test Holmes carries out in Chapter VII?

**Reference:** Hope explains in his confession that he stole a South American alkaloid at York College and made it into soluble pills, each packed in a box beside a harmless one. Holmes later tests the two pills from Lestrade's box on a dog and discovers that one is deadly and the other inert, which confirms Hope's method.

- `222822:222945` I worked this alkaloid into small, soluble pills, and each pill I put in a box with a similar pill made without the poison.
- `116325:116425` Of the two pills in that box one was of the most deadly poison, and the other was entirely harmless.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c10 [trajectory] How is Hope's aortic aneurism introduced and what does it finally do to his story?

**Reference:** Watson discovers the aneurism when he feels Hope's chest in the station, and Hope says a doctor has told him it will burst soon. This illness explains why Hope gives his confession quickly. Hope dies of it in his cell the night after his capture.

- `211801:211855` “Why,” I cried, “you have an aortic aneurism!”
- `232687:232811` On the very night after his capture the aneurism burst, and he was found in the morning stretched upon the floor of the cell

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c11 [trajectory] How does Jefferson Hope's life change from his courtship of Lucy to his single-minded pursuit of revenge?

**Reference:** In Part II Hope is a suitor who swears in his heart to win Lucy and to succeed in everything he undertakes. After Lucy's forced marriage and death, his strong will and endurance become directed entirely to vengeance, which he follows for decades across continents.

- `156713:156833` He swore in his heart that he would not fail in this if human effort and human perseverance could render him successful.
- `198142:198228` His strong will and untiring energy should, he determined, be devoted to that one end.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c12 [trajectory] How does Watson's role as recorder of the case change across the novel?

**Reference:** At the start Watson is only a lodger who writes up Holmes's life as a reminiscence. In Part II Chapter VI he has direct access to Lestrade's note-book to record Hope's confession. In the final chapter he insists the public shall know the facts because he has them in his journal.

- `213357:213459` I can vouch for the accuracy of the subjoined account, for I have had access to Lestrade’s note-book
- `243865:243965` “Never mind,” I answered, “I have all the facts in my journal, and the public shall know them.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c13 [trajectory] Why does the Mormon Council's authority matter to the Ferriers' flight, and how does the phrase 'the Holy Four' recur across the story?

**Reference:** Brigham Young invokes the Holy Four when he threatens Ferrier in Part II Chapter III, so their orders are the reason Ferrier fears for his life. Ferrier later answers a sentinel's challenge with their name as his authority to pass, and Hope learns from Cowper that the Holy Four have issued a warrant against him.

- `166723:166939` “It were better for you, John Ferrier,” he thundered, “that you and she were now lying blanched skeletons upon the Sierra Blanco, than that you should put your weak wills against the orders of the Holy Four!”
- `188209:188247` “The Holy Four,” answered Ferrier.
- `200055:200137` There is a warrant against you from the Holy Four for assisting the Ferriers away.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c14 [trajectory] How does Holmes's use of the telegraph develop through the investigation?

**Reference:** In Chapter IV Holmes sends a long telegram before going to interview Rance. In Chapter V he reports that his American telegram has been answered and his view is confirmed. In Chapter VII he explains that he telegraphed the Cleveland police about Drebber's marriage, which revealed Jefferson Hope as the rival.

- `58884:58977` Sherlock Holmes led me to the nearest telegraph office, whence he dispatched a long telegram.
- `78689:78742` “I have just had an answer to my American telegram.
- `239710:239850` I telegraphed to the head of the police at Cleveland, limiting my enquiry to the circumstances connected with the marriage of Enoch Drebber.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c15 [trajectory] How does John Ferrier's willingness to fight the Mormons change from his first confrontation to his preparations for flight?

**Reference:** Ferrier first threatens his visitors violently, saying he will 'start the smiting' and nearly fetches his gun. Later he quietly cleans and loads the old shotgun while he prepares to flee. His anger has turned into a quiet defensive readiness.

- `173352:173416` “Then I’ll start the smiting,” exclaimed Ferrier furiously
- `169790:169884` he carefully cleaned and loaded the rusty old shotgun which hung upon the wall of his bedroom.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c16 [causal] Why did Lucy Ferrier die, according to the narrative?

**Reference:** Lucy's death is traced to two causes: the terrible death of her father and the forced marriage to Drebber. Her father had earlier told her he would rather see her in the grave than married to either of the Mormon suitors. The narrator attributes her decline to the grief and the marriage together.

- `173713:173798` “I would sooner see you in your grave, my girl, than the wife of either of them.”
- `201922:202120` Whether it was the terrible death of her father or the effects of the hateful marriage into which she had been forced, poor Lucy never held up her head again, but pined away and died within a month.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c17 [causal] Why was RACHE written in blood on the wall, according to the murderer and to Holmes?

**Reference:** Holmes says the writing was a blind meant to send the police toward socialism and secret societies, not toward a German. Hope confesses that the impulse was a mischievous wish to mislead the police, copied from a New York case he had read about.

- `62970:63118` As to poor Lestrade’s discovery it was simply a blind intended to put the police upon a wrong track, by suggesting Socialism and secret societies.
- `227939:228056` Perhaps it was some mischievous idea of setting the police upon a wrong track, for I felt light-hearted and cheerful.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c18 [causal] Why was there blood on the floor of the Brixton house, and what does Holmes deduce from it about the murderer?

**Reference:** Holmes concludes that the blood came from the murderer's nose while he was excited, not from any wound on Drebber. This matches Hope's confession that his nose bled freely during the killing, which Holmes had inferred as a sign of a robust, ruddy man.

- `239155:239328` I had already come to the conclusion, since there were no signs of a struggle, that the blood which covered the floor had burst from the murderer’s nose in his excitement.
- `227779:227854` The blood had been streaming from my nose, but I had taken no notice of it.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c19 [causal] Why did Jefferson Hope come to 221B Baker Street, and what made him believe the trap was safe?

**Reference:** Holmes places an advertisement offering a found ring at Watson's address, reasoning that the murderer would miss the ring and come to reclaim it without suspecting a trap. Hope confirms that he saw the advertisement and thought it might be a plant or might be the ring he wanted.

- `78081:78146` He would come. He will come. You shall see him within an hour?”
- `231708:231808` I saw your advertisement, and I thought it might be a plant, or it might be the ring which I wanted.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c20 [causal] Why was Joseph Stangerson killed, according to Jefferson Hope's own account?

**Reference:** Hope tracked Stangerson to Halliday's Private Hotel, and after explaining Drebber's death he gave Stangerson the same choice of pills. Stangerson refused and attacked him, and Hope says he stabbed him in self-defence.

- `229238:229344` I knew that he was staying at Halliday’s Private Hotel, and I hung about all day, but he never came out.
- `229985:230142` Instead of grasping at the chance of safety which that offered him, he sprang from his bed and flew at my throat. In self-defence I stabbed him to the heart.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c21 [contrast] How does Gregson's confidence about Charpentier compare with what is later established about him?

**Reference:** Gregson announces triumphantly that the man is under lock and key and that the case is solved. After Stangerson is found murdered, Holmes and the others conclude that Arthur Charpentier could not have been involved in the second killing, so Gregson's confident solution is shown to be wrong.

- `92702:92770` “The right track! Why, sir, we have the man under lock and key.”
- `118270:118338` Young Charpentier could not have been engaged in this second affair.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c22 [contrast] How does Holmes's early claim that there are no crimes compare with his later admission?

**Reference:** In Chapter II Holmes complains there are no crimes and no criminals worth his skill. After the Brixton case he admits he was wrong, so his earlier view is contradicted by the murder mystery that engages him.

- `34897:34976` “There are no crimes and no criminals in these days,” he said, querulously.
- `38665:38759` “I said just now that there were no criminals. It appears that I am wrong—look at this!”

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c23 [evolution] How does Watson's opinion of Holmes's theory of observation change between the two chapters where it is discussed?

**Reference:** Watson first dismisses the magazine article on observation and deduction as ineffable twaddle and something only an armchair theorist would write. After the Brixton case he tells Holmes that he has brought detection as near to an exact science as it will ever be brought, so his scepticism turns into full admiration.

- `29622:29745` “What ineffable twaddle!” I cried, slapping the magazine down on the table, “I never read such rubbish in my life.”
- `63715:63855` “I shall never do that,” I answered; “you have brought detection as near an exact science as it ever will be brought in this world.”

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c24 [evolution] How does John Ferrier's relationship with the Mormons change from his arrival to his decision to leave Utah?

**Reference:** When the Mormons rescue him in the desert, Ferrier accepts any terms they set. Years later he refuses to submit to the Prophet and says he does not care about 'knuckling under' to any man, so he plans to leave Utah for good.

- `143747:143806` “Guess I’ll come with you on any terms,” said Ferrier
- `168960:169050` I don’t care about knuckling under to any man, as these folk do to their darned prophet.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c25 [evolution] How does Lestrade's theory of the case change from the Rachel idea to the end of the investigation?

**Reference:** Lestrade first insists that a woman named Rachel is behind the crime. Later Gregson reports that Lestrade has abandoned that line and gone after the secretary Stangerson, which turns out to be a wrong track too, since Stangerson is himself murdered.

- `54307:54438` Mean? Why, it means that the writer was going to put the female name Rachel, but was disturbed before he or she had time to finish.
- `93730:93960` The fun of it is,” he cried, “that that fool Lestrade, who thinks himself so smart, has gone off upon the wrong track altogether. He is after the secretary Stangerson, who had no more to do with the crime than the babe unborn.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c26 [evolution] How does Holmes's willingness to explain his method change between Chapter IV and the final chapter?

**Reference:** In Chapter IV Holmes refuses to reveal much more of the case, saying he does not want to seem an ordinary man. By the end, after the arrest, he invites the detectives and Watson to ask any question and promises to answer, and he walks through his full chain of reasoning.

- `63447:63505` I’m not going to tell you much more of the case, Doctor.
- `123851:123975` You are very welcome to put any questions that you like to me now, and there is no danger that I will refuse to answer them.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c27 [local] In what month and year did John Ferrier die, according to the inscription on his grave?

**Reference:** The grave-marker inscription reads that John Ferrier, formerly of Salt Lake City, died on August 4th, 1860.

- `197054:197076` Died August 4th, 1860.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### c28 [local] At what number on which street was the house in which Drebber's body was found?

**Reference:** Drebber's body was found at Number 3, Lauriston Gardens, off the Brixton Road, an empty house that the narrator describes as having an ill-omened look.

- `42772:42837` Number 3, Lauriston Gardens wore an ill-omened and minatory look.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

## d-gutenberg-2701.txt (30 questions, 3 quotes not found)

### d01 [trajectory] How does Ishmael's attitude toward Queequeg change from the night they first share a bed to the point where he commits himself to sailing with him?

**Reference:** At first Ishmael is so suspicious of the harpooneer that he resolves to sleep on the bench rather than share a bed. After the strange night with the tomahawk and idol, he decides a sober cannibal is better company than a drunken Christian. By the time of the whaling voyage he has bonded with Queequeg so completely that he clings to him for the rest of the voyage.

- `80267:80327` Better sleep with a sober cannibal than a drunken Christian.
- `156922:157022` From that hour I clove to Queequeg like a barnacle; yea, till poor Queequeg took his last long dive.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d02 [trajectory] How does the crew's knowledge of Captain Ahab move from secondhand rumor to direct experience of him as the Pequod sails?

**Reference:** Before sailing, Ishmael only hears about Ahab from Peleg, who says Ahab has lost a leg to a whale. Once at sea, Ishmael finally sees Ahab standing on the quarter-deck, and the reality exceeds what he had imagined. By the time of the dream chapter, Stubb is convinced that Ahab has some hidden purpose involving a white whale.

- `177794:177875` Clap eye on Captain Ahab, young man, and thou wilt find that he has only one leg.
- `281316:281386` Reality outran apprehension; Captain Ahab stood upon his quarter-deck.
- `299519:299561` Ahab has that that’s bloody on his mind.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d03 [contrast] How does Bildad's religious profession compare with his conduct as a ship's owner and officer?

**Reference:** Bildad presents himself as a pious Quaker who quotes Scripture and warns others about their souls. Yet the narrator says he was hard-hearted as a chief mate and that he forbids profane songs while the crew sings a bawdy chorus. The gap between his professed piety and his practical conduct is a recurring irony.

- `185609:185706` For a pious man, especially for a Quaker, he was certainly rather hard-hearted, to say the least.
- `242124:242371` Nevertheless, not three days previous, Bildad had told them that no profane songs would be allowed on board the Pequod, particularly in getting under weigh; and Charity, his sister, had placed a small choice copy of Watts in each seaman’s berth.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d04 [trajectory] How does Ishmael's response to Queequeg's idol-worship develop from his first participation in the rite to his later view of Queequeg's religious practices?

**Reference:** On the first night Ishmael reasons his way into joining the idol ritual, kindling the shavings and kissing the idol's nose for the sake of friendship. Later, during Queequeg's Ramadan, he treats the faith with tolerance and lets Queequeg rest in it. The early participation and later live-and-let-be stance show a gradual shift in his stance towards other religions.

- `138117:138376` So I kindled the shavings; helped prop up the innocent little idol; offered him burnt biscuit with Queequeg; salamed before him twice or thrice; kissed his nose; and that done, we undressed and went to bed, at peace with our own consciences and all the world.
- `200670:200773` Queequeg thought he knew what he was about, I suppose; he seemed to be content; and there let him rest.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d05 [trajectory] How does Ishmael's judgment of the ragged prophet Elijah shift from dismissing him to being haunted by him as the voyage begins?

**Reference:** At first Ishmael and Queequeg dismiss Elijah as a crazy humbug trying to be a bugbear. Elijah's cryptic questions keep returning to Ishmael, though, and by the time the Pequod is at sea, his incoherences recur to Ishmael with a subtle energy he cannot shake. Dismissal turns into lingering unease over the course of the story.

- `225997:226065` and agreed that he was nothing but a humbug, trying to be a bugbear.
- `279302:279477` This was strangely heightened at times by the ragged Elijah’s diabolical incoherences uninvitedly recurring to me, with a subtle energy I could not have before conceived of.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d06 [causal] Why does Ahab lash out at Stubb over a trivial matter of muffling the noise of his ivory leg?

**Reference:** Ahab's savagery stems from the loss of his leg to the white whale, which left him moody and violent. Peleg attributes his bad temper to the bleeding stump and the accursed whale. When Stubb suggests muffling the ivory heel, Ahab's rage erupts at him as an insult, showing the wound behind his mood.

- `198281:198458` I know, too, that ever since he lost his leg last voyage by that accursed whale, he’s been a kind of moody—desperate moody, and savage sometimes; but that will all pass off.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d07 [trajectory] How does Ahab's obsession with the white whale develop from the quarter-deck oath to the night-time charts?

**Reference:** Ahab's obsession starts as a visible, public purpose in the quarter-deck scene, where his pacing leaves marks that reflect one unceasing thought. Ishmael later argues that the monomania did not begin at the moment Ahab lost his leg but grew during the homeward voyage. By the night-time chart scenes, the same purpose has become a settled, methodical maze of calculations that fills every night.

- `363741:363916` Did you fixedly gaze, too, upon that ribbed and dented brow; there also, you would see still stranger foot-prints—the foot-prints of his one unsleeping, ever-pacing thought.
- `409861:409977` It is not probable that this monomania in him took its instant rise at the precise time of his bodily dismemberment.
- `441469:441656` For with the charts of all four oceans before him, Ahab was threading a maze of currents and eddies, with a view to the more certain accomplishment of that monomaniac thought of his soul.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d08 [trajectory] How does Starbuck's position toward Ahab's quest change from his first objection to the later account of his inner state?

**Reference:** In the quarter-deck scene, Starbuck openly objects that he came to hunt whales, not to serve his captain's vengeance. Much later, the narrator says Starbuck's obedience is only forced, since Ahab controls his will through a magnetic hold. Starbuck's private abhorrence of the quest persists beneath that outward compliance.

- `370973:371172` “I am game for his crooked jaw, and for the jaws of Death too, Captain Ahab, if it fairly comes in the way of the business we follow; but I came here to hunt whales, not my commander’s vengeance.
- `473885:474176` Starbuck’s body and Starbuck’s coerced will were Ahab’s, so long as Ahab kept his magnet at Starbuck’s brain; still he knew that for all this the chief mate, in his soul, abhorred his captain’s quest, and could he, would joyfully disintegrate himself from it, or even frustrate it.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d09 [causal] Why does Ahab keep the ship's ordinary whaling routine going instead of pursuing Moby Dick alone?

**Reference:** Ahab announced his private aim to the crew at the quarter-deck, and that announcement exposed him to a charge of usurpation. To protect his command, he decided to keep the voyage's nominal business going and to show his usual interest in whaling. The sudden public revelation of his purpose is the cause, and the later need to keep up ordinary duties is the effect.

- `476866:477127` Having impulsively, it is probable, and perhaps somewhat prematurely revealed the prime but private purpose of the Pequod’s voyage, Ahab was now entirely conscious that, in so doing, he had indirectly laid himself open to the unanswerable charge of usurpation
- `477837:478119` Ahab plainly saw that he must still in a good degree continue true to the natural, nominal purpose of the Pequod’s voyage; observe all customary usages; and not only that, but force himself to evince all his well known passionate interest in the general pursuit of his profession.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d10 [evolution] How does the stated goal of the chase change from Ahab's first oath to the ship's turn at the end of the Albatross chapter?

**Reference:** At the quarter-deck, Ahab vows to chase the whale around the Cape of Good Hope, the Horn, the Norway Maelstrom and beyond. When the Pequod meets the Albatross, Ahab orders the ship to keep her heading round the world. The goal has widened from pursuing one whale across specific dangerous waters to a global circuit, and Ishmael questions whether that circuit leads anywhere.

- `370170:370322` Aye, aye! and I’ll chase him round Good Hope, and round the Horn, and round the Norway Maelstrom, and round perdition’s flames before I give him up.
- `528960:529148` But turning to the steersman, who thus far had been holding the ship in the wind to diminish her headway, he cried out in his old lion voice,—“Up helm! Keep her off round the world!”

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d11 [contrast] How does Ahab's madness appear to Ishmael in the Sunset chapter compared with his calculated shrewdness later in the voyage?

**Reference:** In the Sunset soliloquy, Ahab calls himself demoniac and says that Starbuck thinks him mad. Later, the narrator describes the monomania as subtle and says Ahab shows superlative shrewdness in managing the crew. The contrast places raving obsession and careful calculation side by side in the same man.

- `381121:381198` They think me mad—Starbuck does; but I’m demoniac, I am madness maddened!
- `474492:474772` the subtle insanity of Ahab respecting Moby Dick was noways more significantly manifested than in his superlative sense and shrewdness in foreseeing that, for the present, the hunt should in some way be stripped of that strange imaginative impiousness which naturally invested it;

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d12 [trajectory] How does the cash motive for the hunt change from the gold ounce nailed to the mast to Ahab's later reasoning about the crew's wages?

**Reference:** At the quarter-deck, Ahab nails a gold ounce to the mast as a reward for whoever first spots the white whale. Later, Ahab reasons that the crew must have regular cash in view to keep them loyal to the hunt. The reward used to rouse them becomes a deliberate way of managing their steadiness over the long voyage.

- `367635:367732` look ye, whosoever of ye raises me that same white whale, he shall have this gold ounce, my boys!
- `476500:476575` I will not strip these men, thought Ahab, of all hopes of cash—aye, cash.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d13 [trajectory] How does Ahab's pursuit of Moby Dick develop across the chapters, from his reaction to the dead whale to his plan for the Pacific?

**Reference:** After Stubb's whale is killed, Ahab's mood turns to dissatisfaction, because the dead whale reminds him that Moby Dick is still unslain and no ordinary catch advances his monomaniac aim. Later, Ahab's route is planned around that aim: he sweeps the cruising grounds so that he can meet Moby Dick on the Line in the Pacific, where he is most likely to be found. The obsession starts as frustration at a single kill and becomes a fixed plan that organises the whole voyage.

- `647463:647671` as if the sight of that dead body reminded him that Moby Dick was yet to be slain; and though a thousand other whales were brought to his ship, all that would not one jot advance his grand, monomaniac object.
- `836247:836394` where Ahab, though everywhere else foiled in his pursuit, firmly counted upon giving battle to Moby Dick, in the sea he was most known to frequent;

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d14 [trajectory] How does Ishmael's bond with Queequeg change from the monkey-rope episode to the rescue of Tashtego?

**Reference:** In the monkey-rope chapter, Ishmael describes Queequeg as his inseparable twin brother, joined to him by a rope that makes Ishmael's own fate depend on Queequeg's. Later, when Tashtego falls into the whale's head, Queequeg dives in to rescue him, and Ishmael reports the dive as an act of his brave friend. The bond moves from a fearful, bound dependence to open admiration of Queequeg's courage.

- `706751:706885` Queequeg was my own inseparable twin brother; nor could I any way get rid of the dangerous liabilities which the hempen bond entailed.
- `756024:756105` The next, a loud splash announced that my brave Queequeg had dived to the rescue.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d15 [contrast] Why do the English and American draughtsmen get criticised for their whale pictures, while the French are praised, and how do the whalemen's own carvings compare?

**Reference:** The narrator praises the French for capturing the spirit of the hunt, while English and American draughtsmen only present the mechanical outline of the whale, which he compares to sketching a pyramid's profile. Much later he praises the whalemen's own carvings on sperm-whale teeth and bone as lively sketches. So the contrast is between lifeless outline drawing by professionals and vivid, action-filled work by the men who actually hunt whales.

- `601330:601504` For the most part, the English and American whale draughtsmen seem entirely content with presenting the mechanical outline of things, such as the vacant profile of the whale;
- `604796:605240` Throughout the Pacific, and also in Nantucket, and New Bedford, and Sag Harbor, you will come across lively sketches of whales and whaling-scenes, graven by the fishermen themselves on Sperm Whale-teeth, or ladies’ busks wrought out of the Right Whale-bone, and other like skrimshander articles, as the whalemen call the numerous little ingenious contrivances they elaborately carve out of the rough material, in their hours of ocean leisure.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d16 [causal] Why did the Pequod come close to capsizing while the dead whale was being hauled in, given the load it was already carrying?

**Reference:** Earlier, the ship was already loaded with a sperm whale's head and a right whale's carcass hanging from its side, so it lay heavily strained and resembled an overloaded mule. When the right whale sank with great force, the ship was dragged sideways, and Starbuck's refusal to let go of the body pushed it nearly over. The earlier overload is the underlying cause, and the sinking body is the immediate trigger.

- `725044:725188` The carcases of both whales had dropped astern; and the head-laden ship not a little resembled a mule carrying a pair of overburdening panniers.
- `790936:791118` But a sudden stop was put to further discoveries, by the ship’s being unprecedentedly dragged over sideways to the sea, owing to the body’s immensely increasing tendency to sink.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d17 [evolution] How did the material of the whale-line change over time, and why?

**Reference:** The whale-line was first made of the best hemp, lightly treated with tar, because tar would make it too stiff for close coiling and would not add strength. In later years, American whalers almost entirely replaced hemp with Manilla rope, which is less durable but stronger, softer and more elastic. The proposal therefore moved from the traditional hemp line to Manilla, chosen for its strength and flexibility rather than its durability.

- `620768:620915` The line originally used in the fishery was of the best hemp, slightly vapored with tar, not impregnated with it, as in the case of ordinary ropes;
- `621376:621580` Of late years the Manilla rope has in the American fishery almost entirely superseded hemp as a material for whale-lines; for, though not so durable as hemp, it is stronger, and far more soft and elastic;

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d18 [trajectory] How is the sea's threat to men described at the start of the Brit chapter, and how is it then linked to the sea's threat to its own creatures?

**Reference:** The Brit chapter first says the sea is an everlasting unknown that has always destroyed men, and that men have lost their sense of its full awfulness through repeated exposure. The narrator then says the sea is also a fiend to its own offspring, dashing even the mightiest whales against rocks alongside wrecked ships. The view extends from danger to humans to a general cruelty of the ocean that spares no creature, including its own.

- `612305:612487` though, by vast odds, the most terrific of all mortal disasters have immemorially and indiscriminately befallen tens and hundreds of thousands of those who have gone upon the waters;
- `613657:613877` But not only is the sea such a foe to man who is an alien to it, but it is also a fiend to its own off-spring; worse than the Persian host who murdered his own guests; sparing not the creatures which itself hath spawned.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d19 [trajectory] How does Pip's condition change from the bright, jolly boy of his early chapters to the damaged castaway the crew sees later?

**Reference:** Pip begins as a genial, very bright boy whose brightness is only temporarily dimmed by the panic of the fishery. After Stubb leaves him in the sea and the ship rescues him, the crew regards him as an idiot. Much later he is a fragile, babbling figure who calls out for himself as a lost coward, and Ahab takes him in.

- `903608:903712` Pip, though over tender-hearted, was at bottom very bright, with that pleasant, genial, jolly brightness
- `910580:910651` but from that hour the little negro went about the deck an idiot; such,
- `1120747:1120818` Pip? whom call ye Pip? Pip jumped from the whale-boat. Pip’s missing.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d20 [trajectory] How does Ahab's relationship to the doubloon change from the first time he reads its emblems to the moment he claims it as his own?

**Reference:** At first Ahab reads the doubloon as a mirror of his own self, seeing the three peaks and the volcano as images of himself. Later, in the chase, he treats the coin as a personal prize that fate has reserved for him alone. The coin changes from a symbol he interprets to a reward he feels entitled to.

- `943009:943065` The firm tower, that is Ahab; the volcano, that is Ahab;
- `1170438:1170467` reserved the doubloon for me.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d21 [trajectory] How does Starbuck's stance toward Ahab shift from the moment he resolves to quit the doubloon's warning to the moment he is alone with Ahab's loaded musket?

**Reference:** Starbuck first reads the doubloon as a sad but true warning and resolves to quit it, lest Truth shake him falsely. Much later, with Ahab's musket in his hands, he is tempted toward murder. The honest mate moves from moral resistance to a dark consideration of killing his captain.

- `944675:944712` quit it, lest Truth shake me falsely.
- `1105779:1105867` Starbuck was an honest, upright man; but out of Starbuck’s heart, at that instant when

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d22 [trajectory] How does Ahab's trust in his navigational instruments change across the chapters, from the quadrant he smashes to the log-line he relies on?

**Reference:** Ahab first rejects the quadrant as a mocking toy and smashes it, declaring he will no longer guide himself by it. After the compasses are reversed, he mocks the quadrant again and makes a needle of his own. He then relies on the level log and line as his only means of determining the ship's place.

- `1081945:1081991` no longer will I guide my earthly way by thee;
- `1114420:1114498` yesterday I wrecked thee, and to-day the compasses would fain have wrecked me.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d23 [contrast] How does Pip's pledge to Ahab compare with Stubb's earlier treatment of Pip in the boat?

**Reference:** Stubb warned Pip that he would not pick him up if he jumped, and then left him behind in the sea. Much later, Pip tells Ahab that he will never desert him as Stubb deserted Pip. The passage sets Stubb's abandonment against Pip's loyalty to Ahab.

- `907866:907945` Stick to the boat, Pip, or by the Lord, I won’t pick you up if you jump; mind
- `1144200:1144250` But I will never desert ye, sir, as Stubb did him.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d24 [evolution] How does the coffin built for Queequeg end up as the ship's life-buoy?

**Reference:** The carpenter first builds the coffin at Queequeg's request, and Queequeg lies in it and blesses it as his final bed. After he recovers, the coffin is left unused. Later, when no cask can be found, Queequeg's hint leads Starbuck to order it rigged as the life-buoy for the stern.

- `1041346:1041427` the carpenter was at once commanded to do Queequeg’s bidding, whatever it might
- `1127662:1127718` “A life-buoy of a coffin!” cried Starbuck, starting.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d25 [trajectory] How does Starbuck's appeal to Ahab to abandon the chase change from the second day to the third day?

**Reference:** On the second day Starbuck argues that the chase is impious and asks what more Ahab wants, warning that the whale will swamp the crew. On the third day, before the final encounter, he pleads more personally, begging Ahab to desist while it is still not too late, and says that Moby Dick does not seek Ahab and that Ahab is the one seeking the whale. His tone moves from warning about the crew to a direct, emotional plea to Ahab himself.

- `1204025:1204098` “Great God! but for one single instant show thyself,” cried Starbuck;
- `1220900:1220974` “Oh! Ahab,” cried Starbuck, “not too late is it, even now, the third

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d26 [contrast] How does Ahab's stance on omens in his speech to his men contrast with his private reflection afterwards?

**Reference:** Publicly, Ahab urges his men to believe in omens and to laugh and cry encore at them, presenting the prophecy of the drowning things rising twice as a reason for courage. Alone afterwards, he admits that he had said the same thing to Starbuck about his broken boat and that he cannot drive out of others' hearts what is fixed in his own. The contrast is between the confident encouragement he gives others and his private awareness that he does not believe it himself.

- `1206060:1206116` The things called omens! And yesterday I talked the same

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d27 [causal] Why does the Pequod end up destroyed, given Ahab's persistence in the chase?

**Reference:** Ahab's refusal to abandon the hunt is rooted in his pride and fixed will, as the narrator frames him as someone who never changes his course once decided. When Ahab, having been flung into the sea and then hauled back, leads the boat in a final attack, the whale turns and rams the ship's bow. The ship then sinks, and Ahab's cry about the second hearse shows he sees the ship's loss as the consequence of his own pursuit.

- `1204898:1204925` Ahab is for ever Ahab, man.
- `1228948:1229022` “The ship! The hearse!—the second hearse!” cried Ahab from the boat;

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d28 [evolution] How does Ahab's understanding of the Parsee's prophecy about the hearse develop from the moment the Parsee's body is seen to his final acceptance of it?

**Reference:** When the Parsee's body appears lashed to the whale, Ahab is shaken and says he has been befooled, recognising the prophecy of a hearse. Later, as the whale rams the ship, he asks for the second hearse and declares that all coffins and hearses should sink together into one common pool. His understanding moves from being fooled by the prophecy to accepting that he cannot have a hearse of his own and must die with the ship and whale.

- `1219794:1219819` “Befooled, befooled!”
- `1230072:1230124` Sink all coffins and all hearses to one common pool!

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d29 [trajectory] What happens to the Parsee across the narrative, from his disappearance during the chase to the epilogue's account of the survivor?

**Reference:** The Parsee vanishes during the chase after being caught in the tangles of Ahab's line, and the crew cannot find him. He later reappears lashed to the whale's back, which fulfils Ahab's sense that the Parsee would go before him. In the epilogue, the narrator explains that after the Parsee's disappearance he himself took the place of Ahab's bowsman, linking the Parsee's fate to the narrator's survival.

- `1202955:1203026` “The Parsee!” cried Stubb—“he must have been caught in——”
- `1232970:1233043` It so chanced, that after the Parsee’s disappearance, I was he whom the

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### d30 [local] In which chapter and on which day does the final chase take place, according to the chapter heading?

**Reference:** The final chase is in Chapter 135, titled "The Chase.—Third Day."

- `1207240:1207276` CHAPTER 135. The Chase.—Third Day.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

## e-arxiv-2606.05658v1.txt (30 questions, 2 quotes not found)

### e01 [trajectory] How does the paper's verdict on agentic enhancements, as stated in the abstract, carry through to its final conclusion?

**Reference:** The abstract says agentic enhancements are not universally beneficial and must be applied selectively. The conclusion repeats this lesson, adding that enhancements need explicit cost awareness rather than being applied uniformly.

- `1478:1644` These contrasting results show that agentic enhancements are not universally beneficial and must be applied selectively according to query and domain characteristics.
- `27520:27649` The clear lesson is that agentic enhancements must be applied selectively and with explicit cost awareness rather than uniformly.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e02 [trajectory] How does the latency cost of query decomposition in the results lead to the future-work recommendation about cost-aware orchestration?

**Reference:** The results section reports that decomposition on DevOps more than doubles latency, from 21 s to 48 s. The future-work section responds to the large latency penalties of agentic components by proposing an explicit cost model for the orchestrator.

- `28313:28656` Incorporating a cost model that accounts for retrieval time, the number of reasoning steps, and per-call inference cost would let the orchestrator weigh expected quality gains against their computational price, which is essential for latency-sensitive DevOps deployments where the sixfold reflection slowdown we observed would be unacceptable.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e03 [contrast] How does the effect of query decomposition differ between the DevOps corpus and the MuSiQue benchmark?

**Reference:** On DevOps, decomposition improves nearly every retrieval metric, including MRR and Success@5. The conclusion says decomposition improved quality on the structured domain but reduced ranking precision on the multi-hop benchmark.

- `20456:20670` On the DevOps corpus (Table III), it improves nearly every retrieval metric: overall score rises from 0.814 to 0.855, MRR from 0.556 to 0.722, Success@5 reaches a perfect 1.0, and citation accuracy climbs to 0.917.
- `27244:27519` Across a structured DevOps corpus and the multi-hop MuSiQue benchmark, query decomposition improved retrieval quality in the structured domain but reduced ranking precision in the multi-hop setting, and reflection delivered marginal quality gains at substantial latency cost.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e04 [contrast] How do the paper's findings on decomposition and on reflection differ in their overall verdict?

**Reference:** Decomposition is described as domain-dependent: helpful in structured domains but harmful to ranking on multi-hop data. Reflection is described as giving only limited, inconsistent gains relative to its cost in both domains.

- `4180:4389` We characterize the tradeoffs introduced by decomposition and reflection, showing empirically that decomposition is domain-dependent and that reflection adds significant latency for inconsistent quality gains.
- `25225:25292` Reflection offers limited, inconsistent gains relative to its cost.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e05 [causal] Why does decomposition broaden topic coverage while lowering ranking precision on MuSiQue, and what remedy does the paper propose for this?

**Reference:** Decomposition splits a query into sub-queries, which fragments the contextual signal that ranking depends on, so precision drops even as coverage grows. The future-work section proposes preserving contextual dependencies between sub-tasks so that the ranking signal is not fragmented.

- `20970:21123` Decomposition therefore broadens coverage but fragments the contextual signal that ranking depends on, harming precision in genuinely multi-hop settings.
- `28797:29141` Promising directions include generating more semantically coherent sub-queries, explicitly preserving contextual dependencies between sub-tasks so that ranking signal is not fragmented, and integrating structured reasoning representations such as knowledge or reasoning graphs to capture inter-step relationships that flat sub-query lists miss.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e06 [causal] Why does reflection multiply latency so sharply on MuSiQue, according to the paper's account of how the reflection loop works?

**Reference:** Reflection is a bounded retry loop in which an evaluator flags issues and the query is reprocessed. On MuSiQue this raises latency roughly sixfold, from 17 s to 104 s, while overall score falls, and the paper notes that more selective re-execution of only the implicated components could cut this cost.

- `29229:29532` More sophisticated designs could introduce fine-grained error detection and classification, selective re-execution of only the pipeline components implicated in a failure rather than the whole query, and confidence-based stopping criteria that halt refinement once marginal gains fall below a threshold.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e07 [causal] Why are decomposed queries adopted for only a small share of queries, even on MuSiQue?

**Reference:** The Orchestrator's routing is rule-based, and the paper identifies its decomposition and reflection triggers as static, heuristic rules. This conservative trigger means that many complex queries that could benefit are not routed to decomposition, which the paper flags as a target for future optimization.

- `12468:12501` Its decision logic is rule-based:
- `27685:27796` A central limitation we observed is that decomposition and reflection are triggered by static, heuristic rules.
- `23718:23890` Second, decomposition introduces a new processing pathway but never dominates the pipeline, indicating that the orchestrator invokes it selectively rather than reflexively.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e08 [evolution] How does the paper's assessment of reflection change between its introduction and its conclusion?

**Reference:** The introduction describes reflection as adding significant latency for inconsistent quality gains. The conclusion restates this as reflection delivering only marginal quality gains at a substantial latency cost, so the verdict stays negative but is narrowed to marginal gains.

- `4180:4389` We characterize the tradeoffs introduced by decomposition and reflection, showing empirically that decomposition is domain-dependent and that reflection adds significant latency for inconsistent quality gains.
- `27244:27519` Across a structured DevOps corpus and the multi-hop MuSiQue benchmark, query decomposition improved retrieval quality in the structured domain but reduced ranking precision in the multi-hop setting, and reflection delivered marginal quality gains at substantial latency cost.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e09 [evolution] How does the paper's proposed orchestration strategy change from the current design to future work?

**Reference:** The current Orchestrator uses rule-based routing with heuristic triggers. Future work proposes replacing these static rules with learning-based strategy selection that predicts the optimal reasoning strategy from query features.

- `12468:12501` Its decision logic is rule-based:
- `27797:27947` Future work can replace these with learning-based strategy selection, in which the system predicts the optimal reasoning strategy from query features.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e10 [evolution] How does the paper's treatment of query decomposition develop from the introduction to the future-work section?

**Reference:** The introduction presents decomposition as a generic agentic capability for splitting a complex query into sub-questions. The future-work section treats the current decomposition strategy as something to refine, specifically to fix the precision loss seen on multi-hop data.

- `3110:3308` Instead of a single retrieve-then-generate step, an agentic system can decompose a complex query into sub-questions, gather evidence for each, and refine its answer through self-reflection [11, 12].
- `28693:28796` The current decomposition strategy can be refined to address the precision loss seen on multi-hop data.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e11 [trajectory] How does the reflection design move from the four-axis critique described in the methods to the targeted reflection proposed as future work?

**Reference:** The methods describe an evaluator that critiques answers along four axes and triggers reprocessing. The future-work section proposes more targeted reflection that aims to recover its quality benefits while shedding much of its current cost.

- `13988:14218` The Answer Evaluator Agent adds a self-reflective layer that critiques generated responses along four axes: relevance to the query, citation accuracy, support (grounding in retrieved context), and hallucination detection [11, 49].
- `29533:29640` Such targeted reflection would aim to recover its quality benefits while shedding much of its current cost.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e12 [trajectory] How does the role of document metadata develop from the classification step to the preprocessing section?

**Reference:** The query classifier section explains that metadata filtering by document type narrows the candidate space and improves semantic precision. The preprocessing section then says this metadata is what lets the agentic pipeline reason over structure rather than surface similarity.

- `13052:13304` Because the vector store keeps document metadata alongside dense embeddings, filtering by attributes such as document type (for example, restricting retrieval to runbooks or incident reports) narrows the candidate space and improves semantic precision.
- `15495:15599` This metadata is what allows the agentic pipeline to reason over structure, not just surface similarity.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e13 [trajectory] How does the paper's privacy motivation for running the system locally develop from the introduction to the implementation section?

**Reference:** The introduction describes the system as built on a fully local, privacy-first inference stack. The implementation section explains that the entire stack runs locally because operational documentation in DevOps is often sensitive.

- `3671:3898` We implement a complete Agent-Orchestrated Adaptive RAG system composed of a Query Classifier, a Query Decomposer, an Answer Evaluator, and a central Orchestrator, built on a fully local, privacy-first inference stack [13, 14].
- `17586:17726` The entire stack runs locally, a deliberate privacy-first choice for the DevOps setting, where operational documentation is often sensitive.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e14 [trajectory] How do the limitations on corpus scale in the threats-to-validity section connect to the proposed scalability work at the end?

**Reference:** The limitations section notes the DevOps corpus is modest, so results on a larger production corpus may differ and absolute values are only indicative. The future-work section calls for evaluation at larger scale and under realistic conditions, including much larger corpora.

- `25596:25950` First, the DevOps knowledge base is modest in scale (80 documents, roughly 10,000 words) and was generated with LLM assistance before manual validation; while it is designed to emulate realistic structural heterogeneity, results on a larger production corpus may differ, and the absolute metric values should be read as indicative rather than definitive.
- `30138:30297` Finally, the system should be evaluated at larger scale and under realistic conditions, including noisy queries, much larger corpora, and concurrent user load.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e15 [trajectory] How does the paper move from identifying multi-hop structure in queries to the actual routing outcome on MuSiQue?

**Reference:** The decomposer section says it first detects multi-hop structure, flagging queries with multiple entities or chained dependencies. The strategy-distribution results show that on MuSiQue complex queries dominate and decomposition is adopted for roughly 16% of them.

- `13591:13695` (i) detect multi-hop structure, flagging queries that involve multiple entities or chained dependencies;
- `23099:23250` On MuSiQue, complex queries dominate, and decomposition is adopted for roughly 16% of queries, reducing reliance on undifferentiated complex retrieval.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e16 [trajectory] How does the adaptive-routing idea progress from prior work on Corrective and Adaptive RAG to the paper's concluding call for orchestration?

**Reference:** The related-work section notes that Adaptive-RAG routes queries to retrieval strategies based on predicted question difficulty. The abstract concludes that the paper argues for adaptive, cost-aware orchestration rather than uniformly aggressive reasoning pipelines.

- `9115:9313` Corrective RAG [43] added an explicit retrieval-correction step, and Adaptive-RAG [44] routed queries to retrieval strategies of differing complexity based on predicted question difficulty [45, 46].
- `1645:1756` Our findings argue for adaptive, cost-aware orchestration rather than uniformly aggressive reasoning pipelines.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e17 [trajectory] How does the decomposition idea trace from Least-to-Most prompting in the background section to the Query Decomposer in the architecture?

**Reference:** The background section describes Least-to-Most prompting as solving sub-problems in order, and says this aligns with the decomposition strategy adopted here. The architecture section defines the Query Decomposer as splitting a complex query into ordered sub-queries.

- `9788:9991` Least-to-Most prompting [48] decomposes a hard problem into a sequence of simpler sub-problems solved in order, an approach that aligns closely with the query decomposition strategy adopted in this work.
- `13455:13573` The Query Decomposer Agent enables multi-hop reasoning by splitting a complex query into ordered sub-queries [48, 51].

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e18 [contrast] How does the paper characterize the difference between naive pipelines and agentic systems, and where does it apply this difference to its own design?

**Reference:** The related-work section contrasts a Naive pipeline's predetermined sequence with an agentic system that determines its control flow at runtime. The architecture section applies this to metadata: agentic RAG can reason over structured attributes, which purely similarity-driven Naive pipelines cannot.

- `8094:8315` Whereas a Naive pipeline executes a predetermined sequence, an agentic system determines its control flow at runtime, using intermediate outputs to guide subsequent actions and invoking external tools only as needed [39].
- `13305:13426` This separates Agentic RAG from purely similarity-driven Naive pipelines, which cannot reason over structured attributes.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e19 [trajectory] How does the paper's description of its own agentic approach evolve from the abstract to the related-work discussion?

**Reference:** The abstract introduces dynamic query decomposition, iterative retrieval, and a bounded self-reflective loop. The related-work section says the system shares the Agentic RAG lineage but is distinguished by an explicit multi-agent decomposition architecture evaluated across two contrasting domains.

- `778:955` This paper presents an Agent-Orchestrated Adaptive RAG framework that introduces dynamic query decomposition, iterative retrieval, and a bounded self-reflective evaluation loop.
- `9314:9499` Our system shares this lineage but is distinguished by an explicit multi-agent decomposition architecture that we evaluate across two contrasting domains rather than a single benchmark.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e20 [trajectory] How does the trade-off between coverage and precision on MuSiQue appear in the results and in the trade-off analysis?

**Reference:** The MuSiQue results show topic coverage rising sharply while MRR and Success@5 collapse. The tradeoff analysis generalizes this as decomposition broadening coverage but diluting the contextual signal, which lowers MRR and Success@5 in multi-hop settings.

- `20785:20969` Although citation accuracy reaches 1.0 and topic coverage rises sharply (0.609 to 0.859), ranking-based metrics collapse: MRR falls from 0.469 to 0.102 and Success@5 from 1.0 to 0.063.
- `24625:24809` Precision vs. coverage: decomposition broadens retrieval coverage but can dilute the contextual signal that ranking metrics depend on, lowering MRR and Success@5 in multi-hop settings.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e21 [trajectory] How does the paper's account of why retrieval must be treated as a sequential process relate to its earlier discussion of iterative multi-hop retrieval?

**Reference:** The background section says multi-hop RAG variants retrieve iteratively rather than once, refining context across reasoning steps. The preprocessing section extends this by saying agents can issue multiple retrieval calls and re-rank results, turning retrieval into a sequential decision process.

- `7784:7919` Multi-hop RAG variants extend this further by retrieving iteratively rather than once, refining the context across reasoning steps [7].
- `16509:16772` In the agentic setting the index is not a passive store but an interactive component: agents may issue multiple retrieval calls, adjust filters, and re-rank results across reasoning steps, turning retrieval from a single lookup into a sequential decision process.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e22 [trajectory] How does the paper's view of the control-versus-efficiency tradeoff relate to the modular agent design introduced at the outset?

**Reference:** The introduction presents the system as composed of separate Query Classifier, Decomposer, Evaluator and Orchestrator agents. The tradeoff analysis says these modular agents improve interpretability and adaptivity but add LLM calls and retrieval operations that limit scalability.

- `3671:3898` We implement a complete Agent-Orchestrated Adaptive RAG system composed of a Query Classifier, a Query Decomposer, an Answer Evaluator, and a central Orchestrator, built on a fully local, privacy-first inference stack [13, 14].
- `24810:24992` Control vs. efficiency: modular agents improve interpretability and enable adaptive behavior, but at the cost of additional LLM calls and retrieval operations that limit scalability.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e23 [trajectory] How does the paper move from treating preprocessing as a source of structure to proposing live-system grounding as future work?

**Reference:** The preprocessing section explains that Docling converts documents into structured Markdown that preserves hierarchy and semantic blocks. The integration future-work section proposes querying live monitoring or ticketing APIs so responses reflect the current system state rather than static documentation.

- `15025:15289` We therefore convert source files (PDF, DOCX, YAML) to structured Markdown using IBM Docling [52, 53], which uses layout-aware models to preserve visual hierarchy and decompose documents into semantic blocks rather than emitting an undifferentiated stream of text.
- `29904:30092` In a DevOps context this could mean querying live monitoring systems or ticketing APIs, enabling responses grounded in the current state of a system rather than static documentation alone.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e24 [trajectory] How does the paper's framing of MuSiQue, as introduced in the datasets section, connect to the routing finding that advanced strategies are used more often on it?

**Reference:** The datasets section describes MuSiQue as open-domain compositional reasoning where evidence spans multiple documents. The strategy analysis finds that the multi-hop MuSiQue benchmark draws on advanced strategies far more often than the structured DevOps corpus.

- `18920:19053` MuSiQue [15] represents open-domain compositional reasoning, where evidence must be integrated across multiple independent documents.
- `23506:23717` First, strategy selection is strongly dependent on dataset complexity: the structured DevOps corpus favors standard retrieval, whereas the multi-hop MuSiQue benchmark draws on advanced strategies far more often.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e25 [trajectory] How does the paper's bounded design of the reflection loop appear in the abstract and in the reflection-design discussion?

**Reference:** The abstract describes a bounded self-reflective evaluation loop. The future-work discussion explains that the current reflection is a bounded retry loop and that more sophisticated designs could replace it.

- `778:955` This paper presents an Agent-Orchestrated Adaptive RAG framework that introduces dynamic query decomposition, iterative retrieval, and a bounded self-reflective evaluation loop.
- `29180:29228` Reflection in this work is a bounded retry loop.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e26 [trajectory] How does the paper's stated naive-RAG limitation lead to the motivation for query decomposition in the architecture?

**Reference:** The introduction says conventional single-pass RAG struggles with queries that require reasoning across several documents or chaining intermediate results. The architecture section responds with a Query Decomposer that splits complex queries into ordered sub-queries.

- `2789:2953` This works well for isolated factual lookups, but it struggles with queries that require reasoning across several documents or chaining intermediate results [7, 8].
- `13455:13573` The Query Decomposer Agent enables multi-hop reasoning by splitting a complex query into ordered sub-queries [48, 51].

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e27 [local] What chunking configuration is used to segment documents before embedding?

**Reference:** Documents are segmented with a token-based semantic chunking strategy using 600-token chunks with a 100-token overlap.

- `15600:15713` Documents are segmented with a token-based semantic chunking strategy: 600-token chunks with a 100-token overlap.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e28 [local] Which generator and how many embedding models does the study use, and what limitation does the paper attach to this choice?

**Reference:** The study uses a single generator, Llama-3.1-8B-Instruct, and a single embedding model. The paper notes that the interaction between agentic strategies and model capacity is unexplored, and that a stronger generator might absorb some errors decomposition compensates for.

- `26186:26460` Third, the entire study uses a single generator (Llama-3.1-8B-Instruct) and a single embedding model; the interaction between agentic strategies and model capacity is unexplored, and a stronger generator might absorb some errors that decomposition currently compensates for.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e29 [local] What is the iteration cap on the reflection loop, and why is it set?

**Reference:** The reflection loop is capped at two iterations. The cap is set to balance potential quality gains against responsiveness, keeping cost and latency bounded.

- `14403:14541` To bound cost and latency, the reflection loop is capped at two iterations, balancing potential quality gains against responsiveness [46].

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### e30 [local] What share of MuSiQue queries is routed to decomposition, and how is that described?

**Reference:** Decomposition is adopted for roughly 16% of MuSiQue queries. The paper describes this as reducing reliance on undifferentiated complex retrieval.

- `23099:23250` On MuSiQue, complex queries dominate, and decomposition is adopted for roughly 16% of queries, reducing reliance on undifferentiated complex retrieval.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

## f-kerno-adrs.txt (30 questions, 3 quotes not found)

### f01 [trajectory] ¿Cómo influye el requisito de un modo de durabilidad relajada, planteado en el contexto de ADR-0001, en la decisión final del motor?

**Reference:** El contexto de ADR-0001 dice que Kerno quiere un modo de durabilidad relajada sin perder consistencia, y que esto pesa en la elección. En la decisión, el B+ tree copy-on-write se descarta porque ofrecer esa garantía exige puntos de sincronización y reutilización diferida de páginas, mientras que en el LSM sale del propio diseño del WAL. El requisito inclina así la balanza hacia el LSM.

- `4310:4426` Kerno quiere ofrecer un modo de durabilidad relajada sin comprometer la consistencia (ADR-0002), así que esto pesa.
- `4120:4266` Ofrecer en un CoW la misma garantía requiere puntos de sincronización y reutilización diferida de páginas (diseño esbozado, no implementado).
- `14958:15170` escribe ~100× más bytes por commit pequeño, tiene peores colas en commits durables pequeños, lee peor con caché fría y no ofrece un modo de durabilidad relajada consistente sin trabajo adicional sustancial.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f02 [trajectory] ¿Cómo evoluciona el requisito de validar el motor en el hardware objetivo desde el seguimiento inicial de ADR-0001 hasta el estado registrado en la Fase 1?

**Reference:** El seguimiento inicial exigía repetir los benchmarks de M0 en el hardware objetivo antes de cerrar la Fase 1. Más tarde, en el seguimiento de WP14, la medición de salida se hizo en la clase de VM de M0, no cumplió T1, y la repetición en el hardware objetivo seguía abierta. El requisito no se cerró, sino que quedó pendiente.

- `16965:17058` Repetir los benchmarks de M0 en el hardware objetivo (riesgo R-S3) antes de cerrar la Fase 1.
- `17725:17785` La repetición en el hardware objetivo (R-S3) sigue abierta.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f03 [contrast] ¿Por qué la correctitud no decide por sí sola entre el B+ tree y el LSM, si ambos pasan la misma batería de pruebas?

**Reference:** Ambos prototipos pasan la misma batería compartida y ninguno devolvió datos erróneos ni retrocedió en silencio. Sin embargo, ambos tuvieron defectos reales corregidos, con sutilezas de protocolo en lugares distintos, así que la correctitud no desempata y la decisión se apoya en rendimiento, durabilidad y operación.

- `3043:3110` Ninguno devolvió nunca datos erróneos ni retrocedió en silencio.
- `3538:3776` **La correctitud no desempata**: ambas arquitecturas tienen protocolos con sutilezas, aunque en sitios distintos (invariantes de meta y páginas libres en el B+ tree; ciclo de vida de archivos y concurrencia con hilos de fondo en el LSM).

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f04 [trajectory] ¿Cómo cambia el diseño del historial de revisiones desde la propuesta del SAD hasta la decisión aceptada?

**Reference:** El SAD proponía un híbrido de snapshot más delta con límite de profundidad. La decisión aceptada guarda una copia completa de la carga útil por revisión, sin deltas hasta que los benchmarks los justifiquen. El híbrido queda como evolución futura, sujeta a medición y a un ADR nuevo.

- `36782:36930` Revision Engine separado del journal; `ChangeSet` opcional; snapshot + delta con límite de profundidad; restauración que crea una nueva revisión.
- `37765:37865` cada revisión guarda su carga útil entera; no hay deltas hasta que los benchmarks los justifiquen.
- `38413:38538` La opción 3 (híbrido con límite de profundidad) del SAD queda como evolución futura, sujeta a medición y a un ADR nuevo.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f05 [causal] ¿Por qué se postergaron los deltas en el historial de revisiones, si el diseño de M0 pedía al motor muy poco?

**Reference:** El diseño de revisiones no exige nada del motor más allá de batches atómicos y scans por rango, así que el almacenamiento de versiones no obliga a elegir deltas. Como ningún benchmark ha justificado todavía el coste de reconstrucción de deltas, la decisión aceptada fija copia completa y deja el campo kind reservado para más adelante.

- `37316:37404` El diseño no requiere nada del motor más allá de batches atómicos y scans por rango.
- `37765:37865` cada revisión guarda su carga útil entera; no hay deltas hasta que los benchmarks los justifiquen.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f06 [evolution] ¿Cómo evoluciona la decisión sobre la consistencia del índice primario, desde su aceptación inicial hasta la decisión del propietario sobre el punto abierto?

**Reference:** Inicialmente se aceptó que el índice primario fuera consistente con el commit y que los secundarios fueran derivados con estado y watermark. La implementación lo hizo derivado con frescura explícita, y el propietario aprobó esa implementación. Mantenerlo dentro de cada ChangeSet o reconstruirlo en segundo plano queda condicionado a un workload medido con escrituras intercaladas.

- `46624:46832` Aceptada la opción recomendada: el índice primario es consistente con el commit; los índices secundarios son derivados asíncronos con estado (`ready`/`building`/`stale`/`failed`) y watermark de secuencia.
- `48392:48469` Esta decisión dice que el índice primario es «consistente con el commit».
- `48751:48984` Si un workload medido lo justifica, el siguiente paso es mantenerlo dentro de cada `ChangeSet` (siempre `Ready`, con el coste de escritura que K-E3-9 ya señala) o reconstruirlo en segundo plano; el formato y el estado ya lo admiten.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f07 [contrast] ¿En qué se diferencian los índices de adyacencia de relaciones de los índices de unidades de ADR-0007?

**Reference:** Los índices de unidades de ADR-0007 son derivados asíncronos con estado de frescura y watermark. Los índices de adyacencia de relaciones, en cambio, son transaccionales: se actualizan en el mismo ChangeSet que el registro, son primarios y no necesitan watermark.

- `50414:50504` Los índices de adyacencia de relaciones no cambian: son transaccionales (ADR-0014, I-21).
- `100320:100494` Los índices de adyacencia son **primarios** (se actualizan en el mismo `ChangeSet`, ADR-0007), no secundarios con estado de frescura: E4-03 no necesita watermark para ellos.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f08 [evolution] ¿Cómo evoluciona la decisión sobre relaciones desde la nota de E4-00 hasta la aceptación de la opción 2?

**Reference:** La nota de E4-00 mantenía el ADR como propuesto y fijaba la semántica de rechazo para archivar o borrar unidades con relaciones vivas, mientras la decisión final dependería del prototipo. La opción 2 de registro más índices transaccionales se aceptó el 2026-11-05 con la evidencia de E4-02.

- `101241:101307` Sigue *Propuesto*. La decisión final sale del prototipo de E4-02.
- `101308:101403` Semántica aprobada por el propietario para unidades archivadas o borradas con relaciones vivas
- `97445:97494` aceptada el 2026-11-05 con la evidencia de E4-02.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f09 [causal] ¿Por qué el rechazo de archivar o borrar unidades con relaciones vivas hace que la supresión física sea más sencilla y deja pendiente ADR-0009?

**Reference:** Con el rechazo, nunca puede existir una relación apuntando a una unidad archivada o borrada, así que la supresión física no tiene relaciones colgantes que tratar. Por eso la política de supresión de ADR-0009 puede seguir pendiente mientras se mantenga ese rechazo; relajarlo a cascada o a relación colgante obligaría a resolverla.

- `99886:100086` Con el rechazo de E4-02 no puede existir una relación hacia una unidad archivada o borrada, por lo que la supresión física no tiene relaciones colgantes que tratar mientras se mantenga ese rechazo.
- `44883:44993` Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f10 [causal] ¿Por qué la opción de unidad por relación escribe más que la opción de registro con índices de adyacencia?

**Reference:** Cada relación como unidad hereda toda la maquinaria de una unidad: cabeza, REV_META, REV_PAYLOAD y pertenencia al ChangeSet. La opción de registro con índices escribe solo tres claves pequeñas por relación, por lo que el coste de escritura es menor.

- `92392:92582` La opción 1 escribe *más* porque cada relación hereda la maquinaria de la unidad (cabeza, `REV_META`, `REV_PAYLOAD` y miembro del `ChangeSet`); la opción 2 escribe tres claves pequeñas.
- `86275:86331` Vecinos por rango; coste: tres escrituras por relación.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f11 [trajectory] ¿Cómo se traslada el requisito de no guardar literales de las consultas desde el contexto de ADR-0015 hasta la prueba con centinelas?

**Reference:** El contexto indica que la observabilidad puede filtrar literales y que la especificación exige no guardar valores sensibles por defecto. La decisión sustituye los literales por marcadores tipados antes de calcular la huella, y la prueba de centinelas comprueba que ningún valor aparece en las salidas de ningún nivel, incluido diagnostic.

- `102415:102553` La especificación exige no guardar valores sensibles por defecto; el SAD §11.4 prohíbe registrar contenido completo, secretos y tokens.
- `104164:104252` Ninguno aparece, tampoco en `diagnostic`: el texto de la forma lleva marcadores tipados.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f12 [evolution] ¿Cómo pasa ADR-0015 de propuesta a aceptada, y qué evidencia lo respalda?

**Reference:** La propuesta de niveles de detalle con redacción por defecto tenía fecha de 2026-10-28. Fue aceptada el 2026-11-06 por decisión del propietario del 2026-11-05 y con la evidencia de E5-04, en particular la prueba de centinelas y la medición de coste.

- `66322:66344` | Fecha | 2026-10-28 |
- `105927:106034` Opción 2, aceptada el 2026-11-06 con la evidencia de arriba, por decisión del propietario del 2026-11-05.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f13 [contrast] ¿Qué ocurre con el umbral de coste de la observabilidad al medirlo, y cómo se trata el incumplimiento?

**Reference:** El umbral fijado antes de medir era de un 5 % sobre la p99 de la consulta observada. El nivel diagnostic en 20 000 unidades, ejecución 1, llegó a +5,9 %, por lo que se registró como riesgo sin bloquear la aceptación, dado que diagnostic solo se activa de forma explícita.

- `105319:105392` Umbral fijado antes de medir: ≤ 5 % de la p99 de la consulta observada.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f14 [contrast] ¿En qué difiere la garantía G4 de ADR-0016 sobre cachés de resultados de la propuesta de decisión del mismo ADR?

**Reference:** La propuesta de decisión permitía que una caché de resultados sirviera lecturas de otra secuencia si se probaba que los datos leídos no cambiaron. La garantía G4, fijada después por el propietario, elimina esa excepción y liga la caché a la secuencia de commit de su instantánea, hasta que un ADR posterior defina esa prueba.

- `112963:113136` una caché de resultados además registra la secuencia de commit de la instantánea y no sirve lecturas de otra secuencia salvo prueba de que los datos leídos no cambiaron.
- `111089:111213` se elimina la excepción «salvo prueba de que los datos leídos no cambiaron» hasta que un ADR posterior defina esa prueba

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f15 [local] ¿Qué opción se propone para la adaptación operacional de ADR-0016?

**Reference:** Se propone la opción 2: una única optimización reversible, como un plan reutilizable o una caché de resultados. La propuesta queda sujeta a las garantías G1 a G7 y pendiente de aprobación del propietario.

- `112519:112615` Pendiente de aprobación del propietario. Propuesta: opción 2, sujeta a las garantías G1–G7.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f16 [contrast] ¿Qué papel tienen las observaciones de ejecución frente a la comprobación de validez de las rutas aprendidas en ADR-0016?

**Reference:** Las observaciones solo proponen candidatos y estiman costes, nunca demuestran que una ruta sea válida. La validez se comprueba en cada uso: una ruta aprendida pasa por las mismas comprobaciones que la planificada, como lectura verificada, filtro, estado activo y orden.

- `110345:110481` `ExecutionObservation` solo propone candidatos y estima costes; nunca demuestra que una ruta sea válida ni sustituye una comprobación.
- `111307:111446` Una ruta aprendida pasa por las mismas comprobaciones que la planificada: lectura verificada, filtro, estado activo y orden (`check_units`)

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f17 [trajectory] ¿Cómo se traducen los términos inspirados en el cerebro a conceptos técnicos del modelo, y qué queda excluido?

**Reference:** La unidad neuronal se entiende como una Unit con identidad y ciclo de vida, no con estado dinámico propio. Las conexiones son Edge, la plasticidad son ajustes de estadísticas o prioridades y la consolidación son artefactos derivados. Quedan excluidos el olvido que borre contenido por baja frecuencia de acceso y las inferencias presentadas como declaradas.

- `115370:115507` Rechazada en la propuesta: haría que las lecturas muten datos (spec §2) y mezclaría aprendizaje operacional con contenido (spec §10).
- `116451:116572` El «olvido» nunca borra contenido por baja frecuencia de acceso; solo políticas explícitas de retención (spec §10).
- `116575:116668` Toda «conexión» inferida guarda origen y confianza y no se presenta como declarada (I-39).

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f18 [trajectory] ¿Cómo se amplía el alcance de ADR-0007 y ADR-0011 hasta el contrato común de derivados de ADR-0018?

**Reference:** ADR-0007 cubre la frescura de los índices y ADR-0011 los metadatos de embeddings, pero faltaba un contrato común para todos los derivados. ADR-0018 propone un registro común de descriptores con fuentes, versión, estado y reconstruibilidad, y lo aplicará primero a los índices de la Etapa 4 y a las optimizaciones de la Etapa 6.

- `117856:118063` ADR-0007 cubre la frescura de índices y ADR-0011 los metadatos de embeddings; falta un contrato común para todos los derivados que diga cómo se versionan, cuándo quedan obsoletos y cómo se reconstruyen.
- `118276:118551` todo derivado lleva un descriptor con fuentes (unidad y revisión exactas o rango de secuencia), método o modelo y versión, parámetros que afectan al resultado, fecha, hash de la entrada, estado (`ready`, `building`, `stale`, `failed`, `superseded`) y si es reconstruible.
- `120269:120393` Aplicar el contrato a los índices de la Etapa 4 y a las optimizaciones de la Etapa 6 antes de los embeddings de la Etapa 8.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f19 [trajectory] ¿Cómo tratan ADR-0003 y ADR-0018 el cambio de versión de un derivado?

**Reference:** ADR-0003 dice que cambiar la versión de un índice derivado no requiere migrar el formato físico: el índice se marca stale y se reconstruye desde la fuente. ADR-0018 propone, para un cambio de modelo o versión, crear derivados nuevos y marcar los anteriores como superseded, en lugar de reescribirlos.

- `31540:31601` el índice se marca `stale` y se reconstruye desde la fuente.
- `119487:119601` Un cambio de modelo o versión crea derivados nuevos (`superseded` para los anteriores) en lugar de reescribirlos.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f20 [trajectory] ¿Cómo evoluciona el estado de la congelación del formato entre ADR-0003 y ADR-0013?

**Reference:** ADR-0003 fijaba que el formato concreto se congelaría al cerrar la Fase 1, y su seguimiento aún lo marcaba como pendiente. ADR-0013 registra la congelación del 2026-10-31: el bit DRAFT se retiró y los archivos escritos con él se rechazan como Incompatible.

- `27434:27481` el formato concreto se congela al cerrar Fase 1
- `35462:35499` Estado de la congelación: pendiente.
- `83465:83625` El bit `DRAFT` (punto 10) se retiró en la congelación del 2026-10-31; los archivos escritos con él se rechazan con `Incompatible("pre-freeze draft format")`.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f21 [causal] ¿Por qué existe el bit DRAFT en el formato antes de la congelación?

**Reference:** Hasta la congelación, todo archivo lleva el bit incompat 63. Sin esa marca, una base creada con el borrador podría abrirse después de la congelación y leerse mal con las reglas definitivas. Por eso, al congelar, las bases del borrador se rechazan explícitamente.

- `74202:74285` Sin marca: una base del borrador podría abrirse tras la congelación y leerse mal.
- `73788:73982` Hasta la congelación todo archivo lleva `incompat` bit 63; al congelar (WP14) el bit sale del conjunto conocido y una base del borrador se rechaza con `Incompatible("pre-freeze draft format")`.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f22 [trajectory] ¿Cómo se resuelve la cuestión abierta sobre la cabecera del WAL entre la consecuencia descrita en ADR-0013 y su cierre en la revisión?

**Reference:** La primera trama de cada WAL lleva synced_len 0 porque W-1 no sincronizaba la cabecera, lo que dejaba abierta la posibilidad de perder un commit confirmado si la cabecera se dañaba. La cuestión quedó como decisión del propietario, y en la revisión M1 se cerró: el escritor sincroniza la cabecera en W-1 y WAL-R6 rechaza cualquier trama válida bajo una cabecera rota.

- `80716:80806` la primera trama de cada WAL lleva `synced_len = 0`, porque W-1 no sincroniza la cabecera.
- `81466:81521` Queda como decisión del propietario (ver Seguimiento).
- `81580:81686` el escritor sincroniza la cabecera en W-1 y WAL-R6 rechaza cualquier trama válida bajo una cabecera rota.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f23 [contrast] ¿Por qué ante un bit ro_compat desconocido se abre la base en solo lectura, y no se rechaza ni se abre en lectura-escritura por archivo?

**Reference:** Rechazar con Incompatible impediría leer datos que un lector v1 sí entiende. Abrir en lectura-escritura por archivo es peligroso porque una compactación reescribiría el archivo sin conservar lo que no se entiende. La solución adoptada es abrir la base entera en solo lectura.

- `73490:73563` Rechazar con `Incompatible`: impide leer datos que un lector v1 entiende.
- `73569:73701` Abrir en lectura-escritura por archivo: un escritor reescribiría ese archivo en una compactación sin conservar lo que no entiende.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f24 [trajectory] ¿Cómo pasa la API pública de la Etapa 3 desde el trait Engine del spike hasta la separación por crates aceptada?

**Reference:** El trait Engine del spike era un contrato interno de comparación: no tenía transacciones read-modify-write, cursores, cancelación ni errores por categoría. Para la Etapa 3 se aceptó el crate kerno-units separado de kerno-storage, mientras que la fachada pública y la política de semver quedan para cuando exista una API estable.

- `56679:56770` carece de transacciones read-modify-write, cursores, cancelación y errores por categoría.
- `57655:57808` La fachada pública única (`kerno`, opción 1) y la política de semver se deciden cuando exista una API pública estable; esta aceptación no las fija.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f25 [trajectory] ¿Cómo cambia el criterio para fijar umbrales de rendimiento entre ADR-0012, ADR-0014 y ADR-0016?

**Reference:** ADR-0012 indica que no se fijó ningún umbral de velocidad antes de medir y propone, con el baseline de M0, objetivos para la Fase 1. ADR-0014 fija sus criterios con antelación y no usa el modo --quick para elegir umbrales, y ADR-0016 exige que los umbrales se fijen antes de medir siguiendo ADR-0012.

- `64718:64768` Con el baseline de M0 se proponen, para la Fase 1:
- `86876:86995` El banco se ejecutó antes en modo `--quick` solo para depurarlo; sus resultados no se usaron para elegir los umbrales.
- `108733:108841` con umbrales fijados antes de medir (ADR-0012), incluyendo coste de mantenimiento, memoria y almacenamiento.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f26 [causal] ¿Por qué el codec por defecto de identificadores públicos es un token aleatorio con mapeo y no un token cifrado?

**Reference:** El codec por defecto es el token aleatorio con mapeo porque no necesita claves que gestionar, rotar ni perder. El token cifrado queda como alternativa implementable cuando ADR-0006 defina el proveedor de claves y la rotación, que está pendiente, de modo que el camino por defecto no depende de material criptográfico.

- `39602:39699` ✔ No hay claves que gestionar, rotar o perder; un token filtrado no revela nada del ID interno.
- `42342:42470` No hay material criptográfico en el camino crítico por defecto; ADR-0006 puede cerrarse después sin bloquear las fases 1–3.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f27 [evolution] ¿Cómo evoluciona la adopción de BLAKE3 para el hash de contenido, desde la propuesta de ADR-0008 hasta su aceptación?

**Reference:** La propuesta inicial usaba CRC-32C provisional, y añadir una dependencia criptográfica debía justificarse y entrar con la aceptación del ADR. Al aceptarse el 2026-10-31, BLAKE3-256 pasó a ser el hash de integridad de REV_PAYLOAD, con la dependencia blake3 incorporada en kerno-units en E3-02, mientras CRC-32C quedó provisional.

- `52671:52826` Añadir la dependencia exige justificar que ni la biblioteca estándar ni el manifiesto actual la cubren, y entra con la aceptación de este ADR, no antes.
- `53281:53465` Ni la biblioteca estándar ni el manifiesto tenían un hash criptográfico, y por eso la dependencia `blake3` (sin funciones por defecto, solo `std`) entró en `kerno-units` con E3-02.
- `53760:53864` CRC-32C (`alg = 0`) sigue siendo provisional, sin garantía criptográfica, y no sustituye a BLAKE3-256.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f28 [local] ¿Qué le ocurre al handle de Kerno tras un fsync fallido?

**Reference:** El handle queda envenenado y no se reintenta asumiendo que los datos llegaron al disco. La recuperación decide con lo que realmente está en disco.

- `21496:21585` el handle queda envenenado y la recuperación decide con lo que realmente está en disco.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f29 [local] ¿Cuál es el nivel de observabilidad por defecto, y cómo se activa el nivel diagnostic?

**Reference:** El nivel por defecto es off. El nivel diagnostic solo se activa de forma explícita.

- `106204:106279` El nivel por defecto es `off`; `diagnostic` solo se activa explícitamente.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer

### f30 [local] ¿Qué formato tienen los tokens por defecto de los identificadores públicos?

**Reference:** El formato es k{versión}_ seguido de base32 de 128 bits aleatorios, con un prefijo de tipo opcional definido por la aplicación.

- `41581:41704` con formato `k{versión}_{base32 de 128 bits aleatorios}` y, opcionalmente, un prefijo de tipo definido por la aplicación.

- [ ] keep  - [ ] edit  - [ ] drop  - [ ] evidence justifies the answer
