# Question review (phase 2)

For each question mark: keep / edit / drop, and whether the evidence justifies the answer.
Nothing is frozen until this review is done.

## a-kerno-SAD.txt (30 questions, 1 quotes not found)

### a01 [evolution] ¿Cómo evoluciona la decisión sobre el motor de almacenamiento físico desde que se plantea como pregunta abierta hasta su resolución en la 0.2.0?

**Reference:** Al principio la selección del motor físico quedaba pendiente de ADR-0001 y no debía basarse solo en la popularidad ni en resultados publicados por terceros. La 0.2.0 la da por resuelta: LSM nivelado (ADR-0001), Aceptado el 2026-10-28 junto con el protocolo de commit y recuperación (ADR-0002) y las reglas de versionado e integridad (ADR-0003).

- `17625:17745` La selección queda pendiente de ADR-0001 y no debe basarse solo en popularidad o en resultados publicados por terceros.
- `9493:9720` Resueltos: la elección del motor físico (**LSM nivelado**, ADR-0001), el protocolo de commit, la sincronización y la recuperación (ADR-0002) y las reglas de versionado e integridad (ADR-0003), todos Aceptados el 2026-10-28;

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence shows the pending-then-resolved arc. The 'basarse en evidencia' wording overreaches, so the reference now follows the quoted text.

### a02 [trajectory] ¿Qué trayectoria sigue la congelación del formato binario v1 y qué condición la gobierna?

**Reference:** El formato v1 está documentado, pero sigue como candidato a congelación hasta que el propietario acepte ADR-0013, que nace en la Fase 1 como requisito de la congelación. El plan además exige aprobar el formato y la semántica de commit antes de crear bases de datos de usuarios.

- `9721:9937` la representación binaria concreta es el [formato v1](../file-format/v1.md), que sigue siendo **candidato a congelación** hasta que el propietario acepte [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md).
- `38854:38949` ADR-0013 (formato v1 y protocolo del motor) nace en la Fase 1 y es requisito de la congelación
- `49212:49299` Aprobar el formato y la semántica de commit antes de crear bases de datos de usuarios.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages support the candidate status, the dependency on ADR-0013 and the requirement to approve before user databases are created.

### a03 [contrast] ¿Cómo se contrasta el tratamiento de un servidor de red en la primera versión frente a la visión de un futuro servidor Kerno?

**Reference:** La primera versión excluye explícitamente un servidor de red, servicio gestionado o protocolo remoto. Sin embargo, el documento anticipa que un futuro servidor necesitará autenticación, TLS, aislamiento de tenants y administración propios, así que lo deja para más adelante.

- `6813:6869` Servidor de red, servicio gestionado o protocolo remoto.
- `27695:27815` Un futuro servidor Kerno necesitará controles de autenticación, TLS, aislamiento de tenants y administración propios.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both sides of the contrast are quoted: the exclusion in the first version and the controls a future server will need.

### a04 [causal] ¿Por qué la búsqueda vectorial exacta se implementa antes que los índices aproximados como HNSW?

**Reference:** El documento exige medir antes de especializar: HNSW u otras estructuras solo se justifican por workloads y métricas. Por eso la búsqueda exacta sirve como baseline, y los índices aproximados deben compararse contra ella en recall, latencia, memoria y disco.

- `10895:10981` HNSW, LSM, B+ tree, grafo u otras estructuras se justifican por workloads y métricas.
- `27246:27404` Primero se implementará búsqueda vectorial exacta como baseline; los índices aproximados deberán medirse contra ella en recall, latencia, memoria y disco.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The measure-first principle and the exact-search baseline are in separate passages and together justify the answer.

### a05 [contrast] ¿En qué difiere la estrategia de almacenamiento de revisiones que se propone de la que realmente se ha construido?

**Reference:** El diseño evalúa un modelo híbrido de snapshots completos y deltas estructurales, y el ADR-0004 propone snapshot más delta con límite de profundidad. La implementación de la Etapa 3, sin embargo, usa solo revisiones de copia completa, porque los deltas no se implementaron.

- `20776:20854` Se evaluará un modelo híbrido de snapshots completos y deltas estructurales.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): There is only one passage, on the hybrid model. Nothing supports the ADR-0004 proposal or the full-copy-only implementation in Etapa 3.

### a06 [trajectory] ¿Cómo cambia el estatus de la gestión de índices derivados desde el ADR-0007 pendiente hasta la decisión del propietario de 2026-11-05?

**Reference:** ADR-0007 (consistencia y reconstrucción de índices) aparece primero como Pendiente. Después, el propietario decide el 2026-11-05 mantener los índices como derivados hasta que un workload con escrituras intercaladas justifique mantenerlos dentro de cada ChangeSet.

- `36106:36185` **ADR-0007 — Consistencia y reconstrucción de índices.** Estado: Pendiente.
- `22670:22843` Se mantienen derivados hasta que un workload con escrituras intercaladas justifique mantenerlos dentro de cada `ChangeSet` (decisión del propietario, 2026-11-05, ADR-0007).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence does not mention the question about transactions or the 'own ChangeSet' detail, so those are removed.

### a07 [causal] ¿Por qué un índice atrasado no puede presentarse como si estuviera al día, según las reglas de consistencia y los criterios de publicación?

**Reference:** La consulta debe aplicar una estrategia documentada cuando el índice está atrasado, y no debe fingir frescura. Además, una discrepancia conocida entre índices e información fuente que se oculte al consumidor bloquea la publicación de una beta.

- `25326:25540` Si el índice está atrasado, Kerno debe aplicar una estrategia documentada, como esperar, consultar una ruta exacta alternativa o devolver un estado de no disponibilidad para esa consulta; no debe fingir frescura.
- `47381:47472` una discrepancia conocida entre índices e información fuente que se oculte al consumidor.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The consistency rule and the beta blocking criterion are both quoted.

### a08 [trajectory] ¿Cómo evoluciona la gestión de identificadores públicos desde las opciones iniciales hasta la fase de implementación?

**Reference:** Inicialmente la estrategia predeterminada de identificador público es una cuestión abierta entre token aleatorio con mapeo o token cifrado reversible. El documento introduce después una abstracción IdentifierCodec, y la Fase 4 la convierte en una política inicial de IDs públicos.

- `8854:8960` Estrategia predeterminada de identificador público: token aleatorio con mapeo o token cifrado reversible.
- `13463:13556` La representación pública será intercambiable mediante una abstracción `IdentifierCodec`.
- `41080:41135` `IdentifierCodec` y política inicial de IDs públicos;

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages trace the open question, the IdentifierCodec abstraction and the initial policy.

### a09 [contrast] ¿Qué diferencia hay entre el orden temporal de UUIDv7 y el orden de los commits en Kerno?

**Reference:** RFC 9562 define UUIDv7 y su campo temporal, que aporta un orden temporal aproximado. En Kerno ese orden no es el transaccional: el orden de commits se representa mediante una secuencia monotónica asignada por la base.

- `13280:13461` UUIDv7 ofrece orden temporal aproximado, pero no será el orden transaccional de Kerno: el orden de commits se representará mediante una secuencia monotónica asignada por la base.
- `49853:49887` Define UUIDv7 y su campo temporal.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The reference wrongly suggests that the RFC limits Kerno's ordering. It is reworded to reflect both passages correctly.

### a10 [causal] ¿Por qué no se cifran bits del UUIDv7 para producir un identificador público?

**Reference:** El documento prohíbe cifrar selectivamente bits del UUIDv7 ni inventar transformaciones propias, porque la seguridad sin criptografía casera exige usar algoritmos y bibliotecas consolidados. Kerno solo define la integración y la gestión de claves.

- `13918:14009` No se cifrarán selectivamente bits del UUIDv7 ni se inventará una transformación propia.
- `10756:10858` Se utilizan algoritmos y bibliotecas consolidados; Kerno define la integración y el manejo de claves.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The prohibition and the principle of using established libraries are both quoted.

### a11 [trajectory] ¿Cómo se trata la durabilidad de una escritura confirmada a lo largo del documento, desde la regla de commit hasta los criterios de publicación?

**Reference:** La regla de escritura dice que no se confirma antes de cumplir la garantía de durabilidad prometida por la configuración. Esa misma garantía vuelve a aparecer en M0 como comparación que no debe mezclar durabilidad fuerte con relajada, y en los criterios de la beta, donde la pérdida reproducible de una operación confirmada bloquea la publicación.

- `16809:16915` No se confirma una escritura antes de cumplir la garantía de durabilidad prometida por la configuración.
- `44295:44402` No comparar durabilidad fuerte de una opción con durabilidad relajada de otra como si fueran equivalentes.
- `47189:47291` una pérdida reproducible de una operación confirmada bajo la configuración de durabilidad prometida

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages trace the durability guarantee through the commit rule, M0 and the beta criteria.

### a12 [local] ¿Qué modelo de concurrencia se acuerda para escritores y lectores dentro de una base de datos?

**Reference:** Un único escritor activo por base de datos y varios lectores concurrentes dentro del mismo proceso.

- `11798:11837` - Un escritor activo por base de datos.
- `11838:11894` - Varios lectores concurrentes dentro del mismo proceso.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The multi-process claim is not in the evidence and has been removed.

### a13 [local] ¿Cómo se entrega Kerno en su despliegue inicial?

**Reference:** Kerno se ejecuta embebido en el proceso consumidor y expone una biblioteca Rust. No hay servidor ni autenticación de red en el MVP.

- `11305:11391` Kerno se ejecutará embebido en el proceso consumidor y expondrá una biblioteca Rust.
- `11478:11535` No habrá un servidor ni autenticación de red en el MVP.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claim about the CLI is not in the evidence and has been removed.

### a14 [evolution] ¿Cómo evoluciona la restauración de una revisión desde la propuesta de ADR-0004 hasta lo implementado en la Etapa 3?

**Reference:** El ADR-0004 propone que la restauración cree una nueva revisión, y la sección 8.3 fija que no se borran revisiones posteriores. La Etapa 3 implementa restore como una operación que añade una revisión nueva, lo que cumple lo propuesto.

- `35104:35147` restauración que crea una nueva revisión.
- `21043:21199` Restaurar una revisión producirá una nueva revisión equivalente al contenido histórico seleccionado; no borrará silenciosamente revisiones posteriores.
- `19486:19540` `restore` que añade una revisión nueva (§8.3, I-22)

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The proposal, the rule in §8.3 and the Etapa 3 implementation are all quoted.

### a15 [causal] ¿Por qué el journal de recuperación se mantiene separado del historial de revisiones y la auditoría?

**Reference:** El documento se propone separar el journal de recuperación del historial permanente de revisiones y de la auditoría porque, aunque estos sistemas pueden compartir secuencias de commit o metadatos, no deben compartir implícitamente la misma política de retención.

- `5632:5726` Separar el journal de recuperación del historial permanente de revisiones y de la auditoría.
- `18996:19132` Estos sistemas pueden compartir secuencias de commit o metadatos, pero no compartirán implícitamente la misma política de retención.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The original reasoning was circular. The new reference rests on the retention-policy difference stated in the evidence.

### a16 [trajectory] ¿Cómo se extiende la supresión de datos a capas derivadas, y qué se deja fuera de la versión 0.2.0?

**Reference:** La política de borrado debe aplicarse también a vectores, chunks, índices y colas pendientes. Sin embargo, la supresión física (ADR-0009) y la diferencia estructural (ADR-0008) quedan fuera, y ADR-0009 (supresión, retención y backups) sigue Pendiente.

- `26780:26871` **Borrado:** aplicar la política también a vectores, chunks, índices y colas pendientes.
- `19644:19737` Quedan fuera la supresión física (ADR-0009, K-E3-6) y la diferencia estructural (ADR-0008).
- `36936:37005` **ADR-0009 — Supresión, retención y backups.** Estado: Pendiente.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence does not show 'Etapa 3' or that ADR-0008 is pending, so the reference keeps only what is quoted.

### a17 [contrast] ¿Cómo se contrastan la propuesta de crates del repositorio y el estado real del código en la 0.2.0?

**Reference:** La estructura propuesta es lógica y no obliga a crear nueve crates desde el primer día. En la 0.2.0, los crates de la lista que no se han creado siguen siendo la propuesta de la 0.1.0 y no existen.

- `15628:15718` Esta estructura es lógica, no una obligación de crear nueve crates desde el primer día.
- `32436:32508` Los demás crates de la lista son la propuesta de la 0.1.0 y no existen.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence does not name the crates that exist, so the names are removed.

### a18 [evolution] ¿Qué se espera de la Fase 1 y cómo se refleja su estado en la 0.2.0?

**Reference:** El plan original pedía un motor experimental que demostrara invariantes de persistencia y superara la matriz inicial de recuperación. La 0.2.0 registra que la Fase 1 tiene implementación completa, aunque algunos exit items siguen abiertos con su motivo documentado.

- `40285:40396` Salida: motor experimental que demuestra invariantes de persistencia y pasa la matriz inicial de recuperación.
- `2203:2308` Estado de la Fase 0 (cerrada) y de la Fase 1 (implementación completa; exit items abiertos y su motivo).

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the planned exit for Fase 1 and its status in 0.2.0 are quoted.

### a19 [causal] ¿Por qué el núcleo persistente no debe empezar hasta cerrar M0?

**Reference:** Cada tema debe resolverse en un ADR antes de que sus supuestos queden codificados de forma difícil de revertir. ADR-0001 debe cerrarse antes de congelar el formato físico, y por eso el plan retrasa la implementación del núcleo hasta cerrar M0.

- `9311:9457` Cada tema deberá resolverse en un Architecture Decision Record (ADR) antes de que sus supuestos queden codificados de forma difícil de revertir.
- `33762:33842` Consecuencia: esta decisión debe cerrarse antes de congelar el formato físico.
- `49303:49388` Empezar la implementación del núcleo persistente únicamente después de cerrar M0.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages support the causal chain: ADRs before codifying, ADR-0001 before freezing the format, the core after M0.

### a20 [contrast] ¿Qué se promete y qué no se promete sobre la inmutabilidad del historial de revisiones?

**Reference:** La restauración no borra silenciosamente revisiones posteriores, así que el historial no se pierde por operaciones de restauración. Pero el documento no promete inmutabilidad ilimitada ni que cualquier eliminación se refleje al instante en todos los soportes secundarios.

- `21043:21199` Restaurar una revisión producirá una nueva revisión equivalente al contenido histórico seleccionado; no borrará silenciosamente revisiones posteriores.
- `21671:21796` No se promete inmutabilidad ilimitada ni que toda eliminación pueda reflejarse instantáneamente en cada soporte secundario.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): What is promised and what is not are each quoted.

### a21 [trajectory] ¿Cómo evoluciona la idea de un EXPLAIN de consultas desde su mención como futuro hasta la versión de la Etapa 5?

**Reference:** El documento describe un futuro EXPLAIN que mostrará índices usados, filtros, costes observados y si intervino una estructura derivada. La Etapa 5 implementa explain_filter y explain_relations, que muestran el plan sin literales.

- `25783:25896` Un futuro `EXPLAIN` mostrará índices usados, filtros, costes observados y si intervino una estructura derivada.
- `23401:23478` `explain_filter` y `explain_relations` muestran el plan sin literales (I-37).

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The future EXPLAIN and the Etapa 5 explain functions are both quoted.

### a22 [trajectory] ¿Cómo se pasa de la visión de búsqueda híbrida en la sección de IA al plan de fases?

**Reference:** La evolución prevista es búsqueda textual (p. ej. BM25), búsqueda vectorial y filtros estructurados. El plan de fases lo concreta con vectores exactos, un adaptador de embeddings y un ranking híbrido inicial.

- `26998:27102` La evolución prevista es búsqueda textual (p. ej. BM25) + búsqueda vectorial + filtros estructurados.
- `41452:41497` - vectores exactos y adaptador de embeddings;
- `41539:41566` - ranking híbrido inicial.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'sección 10' reference and the baseline claim are not in the evidence and have been removed.

### a23 [causal] ¿Por qué no se crean índices automáticamente en Kerno?

**Reference:** Porque cada índice consume disco, memoria y trabajo de mantenimiento. En consecuencia, ningún índice se crea sin un workload que lo justifique.

- `24707:24822` No se crearán todos los índices automáticamente. Cada índice consume disco, memoria y trabajo de mantenimiento;
- `22151:22188` ningún índice se crea sin workload.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence does not show 'definición explícita' or the Etapa 4 attribution, so both are removed.

### a24 [evolution] ¿Cómo evolucionan los prototipos de la Fase 0 hasta su uso como referencia en la estructura del repositorio?

**Reference:** La Fase 0 pedía prototipos pequeños y comparables de B+ tree copy-on-write y LSM, sin pretender que fueran motores de producción. En la 0.2.0 los spikes spike-btree y spike-lsm quedan congelados como línea base de rendimiento.

- `39508:39623` prototipos pequeños y comparables para B+ tree copy-on-write y LSM, sin pretender que sean motores de producción;
- `32559:32629` `spike-btree` y `spike-lsm` congelados como línea base de rendimiento

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the Fase 0 prototypes and the frozen spikes are quoted.

### a25 [contrast] ¿Cómo se contrasta la regla de no fijar umbrales antes de medir con el tratamiento de los incumplimientos de la Etapa 3?

**Reference:** La metodología dice que no se establece un umbral de velocidad antes de medir el baseline. Aun así, la Etapa 3 documenta incumplimientos de U2 y U6 que el propietario aceptó sin rebajar umbrales.

- `44405:44523` No establecer un umbral de velocidad antes de medir el baseline; definir objetivos después de conocer los resultados.
- `19909:20011` Ambos son incumplimientos documentados que el propietario aceptó el 2026-11-04, sin rebajar umbrales.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The methodology rule and the accepted non-compliances without lowered thresholds are both quoted.

### a26 [causal] ¿Por qué la generación de embeddings queda fuera del núcleo de Kerno?

**Reference:** El núcleo almacena vectores y ofrece contratos de búsqueda, pero no descarga modelos ni exige internet. El criterio de aceptación de M0 exige que no haya dependencia funcional obligatoria de IA, red o proveedor externo.

- `25993:26150` Kerno almacenará vectores y ofrecerá contratos de búsqueda; no descargará modelos, no exigirá acceso a internet y no generará embeddings en el núcleo.
- `45200:45276` No hay una dependencia funcional obligatoria de IA, red o proveedor externo.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The core's scope and the M0 criterion of no mandatory dependency are both quoted.

### a27 [trajectory] ¿Cómo se trata SQL a lo largo del documento, desde los no objetivos hasta la primera tanda de commits?

**Reference:** SQL aparece primero como no objetivo, ya que la primera API será tipada y programática. Se confirma en el diseño de la API de consultas y se excluye también de la primera tanda de commits.

- `6933:7003` SQL completo y su parser; la primera API será tipada y programática.
- `25635:25670` SQL no entra en el alcance inicial.
- `49432:49537` No empezar por HNSW, embeddings, SQL, un lenguaje de grafos, un servidor de red ni un sistema de agentes.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages show SQL excluded consistently through the document.

### a28 [local] ¿Qué papel tienen los casos de uso de referencia como LiveSpec, Cauce y memoria de agentes en el diseño?

**Reference:** Los casos de uso son workloads de evaluación y no dependencias del motor. Tampoco son modelos que deban imponerse a todos los usuarios.

- `6641:6764` Estos casos serán workloads de evaluación, no dependencias del motor ni modelos que deban imponerse a todos los usuarios.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): A single passage states the answer directly.

### a29 [local] ¿Cuánto dura la Fase 0 según el plan de implementación?

**Reference:** La Fase 0, de spike arquitectónico y decisiones, tiene una duración estimada de una a dos semanas.

- `39252:39317` ### Fase 0 — Spike arquitectónico y decisiones (1–2 semanas)

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The heading states the 1–2 week duration.

### a30 [evolution] ¿Cómo trata el documento la versión 0.1.0 frente a las actualizaciones de la 0.2.0?

**Reference:** Cuando un ADR cambia algo del documento, se publica una nueva versión del SAD en lugar de editar la anterior en silencio. Todo lo que no figura entre los cambios de la 0.2.0 conserva el texto de la 0.1.0; por ejemplo, los crates no creados siguen siendo la propuesta de la 0.1.0.

- `1190:1313` cuando un ADR cambia algo de este documento, se publica una nueva versión del SAD en lugar de editar esta silenciosamente.
- `1372:1428` todo lo que no figura ahí conserva el texto de la 0.1.0
- `32436:32508` Los demás crates de la lista son la propuesta de la 0.1.0 y no existen.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

## b-arxiv-2609.00063v1.txt (30 questions, 0 quotes not found)

> reviewer (claude-opus-5-5): The claim that Fase 1 surpassed the 0.1.0 text in detail is not in the evidence and has been removed.

### b01 [local] How many of the 17 causal hypotheses are supported, and why does this mix of supported and unsupported claims matter for the evaluation?

**Reference:** 11 of the 17 hypotheses are supported, which leaves 6 unsupported. The authors say this mix keeps the evaluation meaningful and allows an effective assessment of an LLM's ability to evaluate causal claims.

- `8323:8561` Considering a mixture of supported and unsupported claims (11 of the 17 hypotheses are supported) ensures that the evaluation remains meaningful, while also enabling an effective assessment of an LLM’s ability to evaluate causal claims.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Evidence gives 11/17 and says the mix keeps evaluation meaningful and assesses causal-claim evaluation; it doesn't mention rejecting versus accepting claims.

### b02 [local] How many annotation points make up the final annotated dataset, and what does each point cover?

**Reference:** The final set contains 1,067 annotation points. Each point covers whether a provided paper is real and, if it is, answers the five criteria for that paper.

- `12926:13078` The final set contains 1,067 annotation points covering whether each provided paper is real, and if so, answering the five criteria for each real paper.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The passage states 1,067 points and what each covers.

### b03 [local] Which model's responses were collected by hand rather than through the automated pipeline, and what is the reason given?

**Reference:** GPT-4o. Its responses were collected manually because it does not offer a free tier.

- `10275:10386` The only exception is GPT-4o, which does not offer a free tier, and from which we collected responses manually.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claim that the other models were chosen for API and free-tier access is not in the evidence.

### b04 [local] What abstract-extraction accuracy did Llama-4-Maverick achieve, and how does this compare with the other models?

**Reference:** Llama-4-Maverick reached 17% accuracy in extracting exact abstracts. Every other LLM failed to extract exact abstracts accurately, so it was the only model with any meaningful success on this criterion.

- `16855:16993` All LLMs fail to accurately extract exact abstracts from the cited papers, except for Llama-4-Maverick, which achieves an accuracy of 17%.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The passage gives 17% and says all other LLMs fail.

### b05 [causal] Why do Mistral and Qwen show low specificity and few real retrieved papers, according to the paper's explanations across its model-size and retrieval discussions?

**Reference:** The authors say Mistral and Qwen are the smallest models (about 12B and 30B parameters), which likely explains their low specificity (0.17) and low share of real papers. Separately, both models state that they do not retrieve external documents or search the web, and that they answer from internal knowledge.

- `22374:22574` Mistral and Qwen are the smallest models with around 12 billion and 30 billion parameters respectively which likely explains their low specificity ( 0.17 ) and low percentage of real retrieved papers.
- `23511:23689` Mistral, DeepSeek-R1, DeepSeek-V3, and Qwen stated that they do not retrieve external documents or search the web and their responses are generated from their internal knowledge.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The paper attributes the result to model size. The no-retrieval statement also covers the DeepSeek models, so the 'weak grounding' synthesis goes too far.

### b06 [causal] Why does a correct verdict on a hypothesis not establish that an LLM's evidence is reliable, and where does the paper show this failure happening in practice?

**Reference:** A model can label a hypothesis true while hallucinating or misrepresenting article metadata, so the verdict and the evidence must be checked separately. In practice, even when papers are supplied, models often give wrong sources because of hallucinated citations or failed extraction of real-paper information.

- `12239:12443` This multi-stage evaluation is essential because an LLM may correctly identify a hypothesis as true while simultaneously hallucinating or misrepresenting article metadata, references, or textual evidence.
- `16649:16854` This indicates that, even when papers are provided, LLMs often fail to supply correct sources, either due to hallucinated citations or an inability to accurately extract source information for real papers.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the rationale and the observed failure are stated.

### b07 [causal] Why are the 17 hypotheses considered a sound test set for the LLMs despite there being only 17 of them?

**Reference:** The hypotheses were not on the web when the study was run, so they could not have been in the LLMs' training data. They were also validated by clinicians, and the authors annotated many outputs per model, which together give the small set a useful test.

- `4645:5098` While the number of hypothesis we evaluate is fairly small (17), it is important to note that 1) when this study was conducted, these hypotheses were not available on the web and therefore could not have been used by the LLMs in their training, 2) the hypotheses have been validated by clinicians, and 3) we annotate all the LLM answers across multiple dimensions for a total of 1,067 annotation points which constitutes a significant annotation effort.
- `8323:8561` Considering a mixture of supported and unsupported claims (11 of the 17 hypotheses are supported) ensures that the evaluation remains meaningful, while also enabling an effective assessment of an LLM’s ability to evaluate causal claims.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The first passage lists all three justifications directly.

### b08 [contrast] Do retrieval-using models consistently outperform models that rely on internal knowledge, and how does Llama-4-Maverick fit this pattern?

**Reference:** Retrieval-using models generally produced more real papers, valid URLs and accurate quotations than internal-knowledge models. Llama-4-Maverick, however, reports no active document retrieval yet achieves the highest F1 and conclusion accuracy, so retrieval does not guarantee the best overall performance.

- `24153:24374` Models that are retrieving external information generally produced better results, including higher numbers of real papers, valid URLs, and accurate quotations, versus models that relied exclusively on internal knowledge.
- `24375:24617` However, document retrieval does not guarantee the best overall performance, as Llama-4 Maverick achieves the highest F1 score (0.88) and conclusion accuracy (0.82) despite emphasizing internal reasoning rather than active document retrieval.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages directly support the contrast.

### b09 [contrast] How does the volume of references that models produce compare with the proportion of those references that are valid?

**Reference:** Most models produce a similar total number of papers and URLs, roughly 42 to 48. The share of valid evidence varies widely across models, so generating many references does not mean the references are reliable.

- `17956:18237` Although most models generate a comparable number of total papers and URLs (roughly 42–48), the proportion of valid evidence differs substantially across models, indicating that generating many references does not necessarily correspond to providing reliable supporting evidence.
- `16649:16854` This indicates that, even when papers are provided, LLMs often fail to supply correct sources, either due to hallucinated citations or an inability to accurately extract source information for real papers.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The first passage states the volume-versus-validity contrast.

### b10 [contrast] How do DeepSeek-R1 and Gemini Pro compare with Qwen and Mistral in both search reasoning and the number of real papers they produce?

**Reference:** DeepSeek-R1 and Gemini Pro are described as showing more sophisticated reasoning, and they also yield the most real papers (26 and 28). Qwen and Mistral rely on surface-level keyword searches and produce few or no real papers or valid URLs.

- `19876:20106` Deepseek R1, ChatGPT, and Gemini Pro demonstrate more sophisticated reasoning, while other models, including Mistral, Qwen, and Deepseek V3 rely more on surface level keyword searches for papers directly mentioning the hypothesis.
- `17732:17955` DeepSeek-R1 and Gemini Pro yield a comparatively high number of real papers (26 and 28, respectively) as well as correct URLs (11 and 21), whereas models such as Qwen and Mistral produce few or no real papers or valid URLs.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the reasoning and the counts are supported.

### b11 [contrast] Does larger model size reliably predict better performance, and what does the paper say about the strongest model's results?

**Reference:** Not reliably. Gemini 2.5 Pro (an estimated 128–140B parameters) has the highest real-paper (0.58) and correct-URL (0.44) rates, but DeepSeek-R1, with only 37B active parameters, also performs well (0.54 real papers). The authors state that model size alone does not fully explain performance.

- `22630:22891` Gemini 2.5 Pro, with an estimated 128-140 billion parameters, achieves the highest values for real papers ( 0.58 ) and correct URLs ( 0.44 ), while DeepSeek-R1, despite using only 37 billion active parameters per token, also performs well with 0.54 real papers.
- `23068:23129` However, model size alone does not fully explain performance.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claims 'larger models generally performed better' and 'models differ in which metrics they excel at' are not in the evidence.

### b12 [evolution] How does the annotation procedure move from checking the hypothesis verdict to checking individual articles?

**Reference:** Annotators first check whether the hypothesis verdict matches the clinician ground truth, regardless of evidence. Only when the LLM claims support do they check the evidence, and each real paper then gets further criteria such as abstract, quote and support checks.

- `11099:11332` First, we assess whether the model’s decision regarding the hypothesis is correct, independent of any evidence it provides, by determining whether the response matches the ground truth obtained from clinicians (Correct Conclusion).
- `11333:11442` If the LLM claims that the hypothesis is supported, we then annotate the validity of its supporting evidence.
- `11658:11991` If an article is a Real Paper, then we also annotate whether the abstract is provided accurately from the original paper (Real Abstract), whether the quoted text appears exactly in the paper and supports the model’s stated causal claim (Real Quote), whether the article itself supports the hypothesis (Paper Supporting Hypothesis).

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Three passages trace the staged procedure.

### b13 [evolution] How does the response-processing pipeline change from automatic parsing to manual handling of the outputs?

**Reference:** Responses are first parsed automatically according to the prompt instructions. Only in rare cases where parsing fails do the authors manually extract the components, so the pipeline moves from automated to manual handling.

- `10582:10650` Then we automatically parse it according to the prompt instructions.
- `10819:10917` In rare cases where automatic parsing fails, we manually extract the components from the response.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both steps are stated.

### b14 [evolution] How does the reliability process for annotation develop from independent labeling to the final dataset?

**Reference:** Two annotators independently label the entire set of LLM responses. Where they disagree, the responses are re-examined to verify their validity and reach a final consensus.

- `12586:12676` To ensure reliability, two annotators independently label the entire set of LLM responses.
- `12744:12853` In cases of disagreement, the responses are re-examined to verify their validity and reach a final consensus.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claim that the agreed dataset is used for all evaluations is not in the evidence.

### b15 [trajectory] Where do the 17 hypotheses come from, and how is their validity established?

**Reference:** They come from prior work (Adhikari et al., 2025) that analyzed ICD-10 diagnosis codes associated with repeat ER visits among diabetic patients. Four expert clinicians validated them, which provides ground truth.

- `7806:8005` We use a set of 17 causal hypotheses discovered in prior work (Adhikari et al., 2025), which analyzes ICD-10 diagnosis codes associated with repeat emergency room (ER) visits among diabetic patients.
- `8006:8118` The hypotheses have been validated by four expert clinicians, therefore we have ground truth for their validity.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages come from the methods. Nothing from the introduction is given, so I reframed this as a factual question.

### b16 [trajectory] How does the paper's framing of LLM strengths and failures in the abstract compare with its final conclusion?

**Reference:** The abstract says LLMs show strong recall but perform poorly at providing valid articles and evidence and at rejecting unsupported hypotheses. The conclusion repeats this: LLMs sometimes do well at identifying true causal relationships but consistently struggle to reject unsupported claims and to reference real, traceable evidence, and they often hallucinate citations.

- `1899:2087` Our analysis shows that while LLMs exhibit strong recall, they often perform poorly at providing valid scientific articles and evidence for support and at rejecting unsupported hypotheses.
- `25073:25325` LLMs sometimes perform well at identifying true causal relationships, but they consistently struggle to reject unsupported claims and to reliably reference real, traceable evidence from the scientific literature, often producing hallucinated citations.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'recommends caution' statement is not in the cited passages.

### b17 [trajectory] How does the paper's view of checking the verdict and evidence separately develop between the methods and the RQ2 results?

**Reference:** The methods first assess whether the model's verdict matches the clinician ground truth, independent of any evidence it gives. The RQ2 section explains why the evidence is evaluated separately: a model may correctly label a hypothesis as true while hallucinating or misrepresenting article metadata.

- `11099:11332` First, we assess whether the model’s decision regarding the hypothesis is correct, independent of any evidence it provides, by determining whether the response matches the ground truth obtained from clinicians (Correct Conclusion).
- `16182:16387` This evaluation measures the reliability of LLM decisions with respect to scientific sources, since a model may correctly label a hypothesis as true while hallucinating or misrepresenting article metadata.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The cited methods passage only states that the verdict is assessed independently of evidence. The hallucination rationale appears in the RQ2 passage.

### b18 [trajectory] How do the paper's recall and specificity findings relate to its conclusion about rejecting unsupported claims?

**Reference:** Most models reach a recall of 1.0, so they correctly identify all true hypotheses, while GPT-4o's recall is only 0.54. The conclusion agrees that LLMs sometimes do well at identifying true causal relationships, but it says they consistently struggle to reject unsupported claims.

- `14544:14753` A recall of 1.0 for most models indicates that these models correctly identified all true causal hypotheses, whereas GPT-4o (recall = 0.54 ) failed to detect a substantial portion of true causal relationships.
- `25073:25325` LLMs sometimes perform well at identifying true causal relationships, but they consistently struggle to reject unsupported claims and to reliably reference real, traceable evidence from the scientific literature, often producing hallucinated citations.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The cited passages do not mention low specificity.

### b19 [trajectory] How does the paper's stance on using LLMs for verification change from the introduction to the conclusion?

**Reference:** The introduction stresses caution in high-stakes settings such as medicine and law, and the need to assess the limitations of relying on LLMs. The conclusion makes this concrete: LLMs should be used with caution for causal verification and should not be relied on as autonomous tools for evidence-based biomedical reasoning.

- `3038:3241` However, in high-stakes environments, such as in medical and legal settings, it is important to exercise caution and to carefully assess the limitations and potential pitfalls of relying on these models.
- `25482:25691` These limitations indicate that, in their current form, LLMs should be used with caution for causal verification tasks and should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'potential as assistants' framing is not in this question's evidence.

### b20 [trajectory] How does the introduction's view of LLMs as potential assistants relate to the conclusion's warning against autonomous use?

**Reference:** The introduction presents LLMs as having the potential to act as assistants in finding evidence for causal claims. The conclusion limits that potential, saying they should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- `2824:3037` Large language models (LLMs) are particularly good at processing and organizing such unstructured text at scale and have the potential to act as assistants in finding evidence for causal hypotheses and mechanisms.
- `25482:25691` These limitations indicate that, in their current form, LLMs should be used with caution for causal verification tasks and should not be relied upon as autonomous tools for evidence-based biomedical reasoning.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages directly support the contrast.

### b21 [trajectory] How does the paper's concern with hallucination move from its evaluation design to its conclusion?

**Reference:** The evaluation design checks for real articles, valid DOIs, accurate abstracts and relevant quotes to tell whether the LLM grounds its reasoning in evidence rather than generating hallucinated claims. The conclusion reports that LLMs struggle to reference real, traceable evidence and often produce hallucinated citations.

- `9618:9851` The ability to return real articles, valid DOI links, accurate abstracts, and relevant quotes is important for assessing whether the LLM is grounding its reasoning in evidence-based sources rather than generating hallucinated claims.
- `25073:25325` LLMs sometimes perform well at identifying true causal relationships, but they consistently struggle to reject unsupported claims and to reliably reference real, traceable evidence from the scientific literature, often producing hallucinated citations.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The first passage is from the evaluation design, not the introduction.

### b22 [trajectory] How does the paper's research gap about retrieval in the introduction relate to what the results show about GPT-4o and Llama-4-Maverick?

**Reference:** The introduction notes that prior causal-LLM studies did not consider LLMs' retrieval capabilities for providing evidence. The results address this gap: GPT-4o states that it retrieves and analyzes external scientific documents, and Llama-4-Maverick reports retrieving several external documents from various databases.

- `3797:3938` However, these studies have not considered the retrieval capabilities of LLMs in providing scientific evidence to substantiate causal claims.
- `23690:23898` In contrast, GPT-4o states that it retrieves and analyzes external scientific documents and bibliographic records, while Llama-4-Maverick reports retrieving several external documents using various databases.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'central finding about grounding' claim is not supported.

### b23 [trajectory] What do the results show about LLMs' ability to provide correct sources and real abstracts?

**Reference:** Even when papers are provided, LLMs often fail to supply correct sources, either because they hallucinate citations or cannot extract information from real papers. Real abstracts are extremely limited and appear only once, for Llama-4-Maverick.

- `18612:18726` In contrast, real abstracts generated by the LLMs are extremely limited, appearing only once for Llama-4-Maverick.
- `16649:16854` This indicates that, even when papers are provided, LLMs often fail to supply correct sources, either due to hallucinated citations or an inability to accurately extract source information for real papers.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): There is no quote evidence, and the claimed progression is invented.

### b24 [trajectory] What do the Gemini Pro retrieval claims suggest about its performance, given its reported top results?

**Reference:** Gemini Pro states that it can provide titles, DOI links and summaries of scientific literature from the web. Gemini 2.5 Pro also has the highest real-paper (0.58) and correct-URL (0.44) rates, which is consistent with its claimed retrieval ability.

- `23899:24152` Gemini Flash and Gemini Pro state that they could retrieve information from the web, but Gemini Flash state that it does not do so for the prompt, whereas Gemini Pro stated that it could provide titles, DOI links, and summaries of scientific literature.
- `22630:22891` Gemini 2.5 Pro, with an estimated 128-140 billion parameters, achieves the highest values for real papers ( 0.58 ) and correct URLs ( 0.44 ), while DeepSeek-R1, despite using only 37 billion active parameters per token, also performs well with 0.54 real papers.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The causal link is speculative, so the answer should only note consistency.

### b25 [trajectory] How does Mistral's described search approach relate to its statement about retrieval?

**Reference:** Mistral does keyword searches through databases such as PubMed, Web of Science and Google Scholar, but it mainly gives illustrative example papers without strong causal analysis. It also states that it does not retrieve external documents or search the web, and that it answers from internal knowledge.

- `22012:22197` Mistral also does a keyword search through databases like PubMed, Web of Science, and Google Scholar, but mainly provides illustrative examples of papers without strong causal analysis.
- `23511:23689` Mistral, DeepSeek-R1, DeepSeek-V3, and Qwen stated that they do not retrieve external documents or search the web and their responses are generated from their internal knowledge.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Mistral's poor evidence output is not in the cited passages.

### b26 [trajectory] How does the paper's association criterion from the methods connect to DeepSeek-R1's described search behavior?

**Reference:** The methods say papers reporting only associations are annotated as not reporting direct causal relationships. DeepSeek-R1's described behavior matches this criterion because it rejects articles showing mere associations as evidence.

- `11992:12158` For the papers that do not support the hypothesis, we annotate whether the article reports only an association rather than a direct causal relationship (Association).
- `20389:20453` It rejects as evidence any articles that show mere associations.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages support the link.

### b27 [trajectory] How does the small scale of the study appear in the abstract and then in the conclusion?

**Reference:** The abstract presents the work as a preliminary, small-scale study of LLM accuracy on causal medical claims. The conclusion repeats the limitation and says the findings should be further investigated on a larger scale.

- `1215:1388` Here, we present a preliminary, small-scale study that investigates the accuracy of LLMs in evaluating causal medical claims and supporting them with peer-reviewed research.
- `25326:25481` These findings highlight a gap between causal reasoning and trustworthy scientific evidence grounding and should be further investigated on a larger scale.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages are explicit.

### b28 [trajectory] What does the Llama-4-Maverick reasoning pattern look like in the search-process section, and how does its reported performance relate to it?

**Reference:** Llama follows a clean two-phase search: a broad initial search, then a refined search with tighter filters that attends to the biology behind the hypothesis. Llama-4-Maverick also has the highest F1 (0.88), specificity (0.50) and correct-conclusion rate (0.82).

- `21496:21694` Llama’s reasoning follows a clean two-phase structure: an initial broad search, followed by a refined second search with tighter filters, paying attention to the biology underlying the hypothesis.
- `22892:23067` GPT-4o has the highest number of real quotations ( 0.41 ), and Llama-4-Maverick achieved the highest F1 score ( 0.88 ), specificity ( 0.50 ), and correct conclusions ( 0.82 ).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The paper does not explicitly tie Llama's performance to its reasoning in this evidence.

### b29 [trajectory] How does the paper use the sentence 'Their performance reflects the level of reasoning' to explain performance, and how does this relate to its later claim about model size?

**Reference:** In the search-process section, the authors say LLM performance reflects the level of reasoning shown in their searches. In the later model-size discussion, they say model size alone does not fully explain performance, which leaves room for factors such as reasoning.

- `20175:20225` Their performance reflects the level of reasoning.
- `23068:23129` However, model size alone does not fully explain performance.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Architecture or training' is not in the evidence.

### b30 [trajectory] How does Qwen's search behavior relate to the real-paper and URL output the paper reports for it?

**Reference:** Qwen names databases and keyword strings but jumps straight to a conclusion without explaining its reasoning. The results show that Qwen and Mistral produce few or no real papers or valid URLs, so the weak search matches the weak output.

- `22198:22319` Qwen names databases, lists explicit keyword strings, and jumps straight to a conclusion without unpacking its reasoning.
- `17732:17955` DeepSeek-R1 and Gemini Pro yield a comparatively high number of real papers (26 and 28, respectively) as well as correct URLs (11 and 21), whereas models such as Qwen and Mistral produce few or no real papers or valid URLs.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

## c-gutenberg-244.txt (30 questions, 6 quotes not found)

> reviewer (claude-opus-5-5): Both the search behavior and the poor output are supported.

### c01 [trajectory] How does Watson's opinion of Sherlock Holmes change between his early private judgement and his later remark about Holmes's detective work?

**Reference:** At first Watson privately thinks Holmes may be clever but is certainly very conceited. Later he tells Holmes that he has brought detection as near an exact science as it will ever be. His view moves from irritation to admiration.

- `34798:34895` “This fellow may be very clever,” I said to myself, “but he is certainly very conceited.”
- `63715:63855` “I shall never do that,” I answered; “you have brought detection as near an exact science as it ever will be brought in this world.”

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The two quotes support conceit turning to admiration. The 'exact science' line comes early, not at the end, and 'absurd theories' is not in the evidence.

### c02 [trajectory] What does Holmes say about theorizing before the evidence is complete, and how does he later revise his own confidence after the pill experiment fails?

**Reference:** Holmes warns that it is a capital mistake to theorize before you have all the evidence, because it biases the judgment. After the pill result he says he ought to know that when a fact seems opposed to a long train of deductions, it always turns out to bear another interpretation. Of the two pills, one was deadly poison and the other harmless.

- `42321:42452` “No data yet,” he answered. “It is a capital mistake to theorize before you have all the evidence. It biases the judgment.”
- `116153:116324` I ought to know by this time that when a fact appears to be opposed to a long train of deductions, it invariably proves to be capable of bearing some other interpretation.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support the answer. The dog detail and the 'more faith' wording are not in the evidence, so I removed them.

### c03 [trajectory] How is Watson's composure under pressure presented, from his war experience to the Brixton Road investigation?

**Reference:** Watson says he saw his comrades hacked to pieces at Maiwand without losing his nerve. During the case, though, his nerves, which were steady enough on the battlefield, tingle as he thinks about the affair. His battlefield composure does not hold up against the mystery.

- `75492:75566` I saw my own comrades hacked to pieces at Maiwand without losing my nerve.
- `109654:109740` My nerves, which were steady enough on the field of battle tingled as I thought of it.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The two quotes support the contrast. The detail about the dead man's face is not in the evidence.

### c04 [trajectory] How does the RACHE message on the wall develop as a clue, from Holmes's first dismissal to the explanation given later by the murderer?

**Reference:** Lestrade reads RACHE as the start of the name Rachel, and Holmes mocks him for chasing a woman, noting that Rache is German for revenge. Later the murderer says he wrote it to imitate a German, after a New York case, so the police would suspect secret societies. The clue was a deliberate decoy from the start.

- `58591:58687` ‘Rache,’ is the German for ‘revenge’; so don’t lose your time looking for Miss Rachel.
- `228057:228228` I remembered a German being found in New York with RACHE written up above him, and it was argued at the time in the newspapers that the secret societies must have done it.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Holmes's dismissal of 'Miss Rachel' and the murderer's New York explanation together support the answer.

### c05 [trajectory] How does the wedding ring move through the story, from the Lauriston Gardens scene to its final fate?

**Reference:** At Lauriston Gardens a woman's wedding ring is found, which makes Lestrade think a woman was involved. The ring had been taken from Lucy's finger after her death, and at the end the murderer holds Lucy's marriage ring in front of Drebber's eyes.

- `50246:50329` “There’s been a woman here,” he cried. “It’s a woman’s wedding-ring.”
- `202827:202901` and then, snatching up her hand, he took the wedding-ring from her finger.
- `227380:227456` I laughed as I saw it, and held Lucy’s marriage ring in front of his eyes.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The bait advertisement and the murderer going back for the ring are not in the evidence. The three quotes support the reduced answer.

### c06 [trajectory] How does John Ferrier's position on his daughter marrying a Mormon change, and what does it show about his refusal?

**Reference:** Ferrier had always privately resolved never to let his daughter marry a Mormon. After the two young suitors come to press their claim, he declares that he would sooner see her in her grave than married to either of them. His private resolve becomes open defiance.

- `159787:159919` He had always determined, deep down in his resolute heart, that nothing would ever induce him to allow his daughter to wed a Mormon.
- `173623:173798` “The young canting rascals!” he exclaimed, wiping the perspiration from his forehead; “I would sooner see you in your grave, my girl, than the wife of either of them.”

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support this. The detail that he said it to the suitors themselves and the claim that he stayed silent in public are not in the evidence.

### c07 [trajectory] How is the countersign exchanged by the Mormon sentinels used, from the garden scene to the mountain pass?

**Reference:** In the garden the Mormon watchers pass on the sign 'Nine to seven!'. At the mountain pass the sentinel challenges the fugitives with 'Nine from seven', the same kind of countersign Hope had overheard, which he uses to get them past the guard.

- `185259:185326` “Pass it on to him, and from him to the others. Nine to seven!”
- `188348:188390` “Nine from seven,” cried the sentinel.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes show the countersign in the two places. The exact reply phrases are not in the evidence, so I removed them.

### c08 [trajectory] How does Holmes's use of street children develop from his early remark about them to the organised search for the cab driver?

**Reference:** Holmes first notes that the street Arabs go everywhere and hear everything and only need organisation. He later says he set up a detective corps of them and sent them to every cab proprietor in London, which is how the murderer, a cabman, was found.

- `91764:91891` These youngsters, however, go everywhere and hear everything. They are as sharp as needles, too; all they want is organisation.
- `241357:241520` I therefore organized my Street Arab detective corps, and sent them systematically to every cab proprietor in London until they ferreted out the man that I wanted.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The early remark about organisation and the later organised corps sent to cab proprietors support the answer.

### c09 [trajectory] What role does Holmes's violin play in the story, and how is it linked to his moods across the early and later chapters?

**Reference:** In the early chapters the violin reflects Holmes's thoughts and moods, with melancholy or cheerful chords depending on what is on his mind. In the Brixton Road chapter Watson hears his violin late into the night while Holmes ponders the problem. The instrument stands for his unresolved thinking.

- `25020:25075` Clearly they reflected the thoughts which possessed him
- `86603:86689` long into the watches of the night I heard the low, melancholy wailings of his violin,

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The music reflecting his thoughts and the later melancholy late-night playing support the link between the violin and his moods.

### c10 [trajectory] How were the pills Hope used prepared, and what did Holmes's test of them show?

**Reference:** Hope worked the alkaloid into small soluble pills and put each poisoned pill in a box with a similar pill made without the poison. Holmes's test showed that of the two pills in the box, one was deadly poison and the other entirely harmless.

- `222822:222945` I worked this alkaloid into small, soluble pills, and each pill I put in a box with a similar pill made without the poison.
- `116325:116425` Of the two pills in that box one was of the most deadly poison, and the other was entirely harmless.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The York College, the choice given to the victims and Lestrade's role are not in the evidence. The two quotes support the reduced answer.

### c11 [trajectory] How does Holmes use the phrase 'study in scarlet' early in the case and again at the end, and how does its tone change?

**Reference:** Early on Holmes calls the case the finest study he ever came across, 'a study in scarlet'. At the end he says bitterly that the result of all their Study in Scarlet is to get the detectives a testimonial. The phrase turns from enthusiasm into an ironic joke about who gets the credit.

- `72145:72259` I might not have gone but for you, and so have missed the finest study I ever came across: a study in scarlet, eh?
- `243782:243863` “That’s the result of all our Study in Scarlet: to get them a testimonial!”

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The original question was confused, and the 'scarlet thread' image is not in the evidence. The two quotes support the reworded version.

### c12 [trajectory] How does Watson's role in the case develop, from being Holmes's companion to being the one who records the story?

**Reference:** Part of the account is taken from what is recorded in Dr. Watson's Journal. At the end Watson says he has all the facts in his journal and that the public shall know them.

- `208522:208613` as duly recorded in Dr. Watson’s Journal, to which we are already under such obligations.
- `243898:243965` I have all the facts in my journal, and the public shall know them.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support Watson's role as recorder. The parts about him as a curious lodger and the newspaper praise are not in the evidence.

### c13 [trajectory] How do Holmes's remarks about Lestrade and Gregson change, from his early comments to the later case?

**Reference:** At first Holmes calls them quick and energetic but conventional, and says Gregson is the smartest of the Scotland Yarders. When the newspaper credits them with the capture, Holmes mocks it as the credit of the detectives, and says the glory belongs to the Yard officers. Holmes's praise becomes sarcasm.

- `243171:243319` It is an open secret that the credit of this smart capture belongs entirely to the well-known Scotland Yard officials, Messrs. Lestrade and Gregson.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): There is only one quote. Holmes's early remarks about Lestrade and Gregson are missing, so the trajectory is unsupported.

### c14 [trajectory] How does Jefferson Hope's idea of vengeance develop from the grave of John Ferrier to his confession in prison?

**Reference:** After Ferrier's death Hope feels that only thorough and complete retribution, brought by his own hand, can ease his grief. In his confession he refines this: there is no satisfaction in vengeance unless the offender has time to realize who is striking him and why.

- `197994:198141` he felt that the only one thing which could assuage his grief would be thorough and complete retribution, brought by his own hand upon his enemies.
- `218913:219057` There is no satisfaction in vengeance unless the offender has time to realize who it is that strikes him, and why retribution has come upon him.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support this. The 'officer of justice' claim is not in the evidence.

### c15 [trajectory] How does the number countdown from Brigham Young's ultimatum shape the Ferriers' fear?

**Reference:** The threatening note says twenty-nine days are given for amendment. Later the figure 2 appears on the wall of Ferrier's house, meaning the next day is the last of the allotted time. The shrinking count builds the pressure toward their flight.

- `175256:175322` “Twenty-nine days are given you for amendment, and then——”
- `178179:178299` That morning had shown the figure 2 upon the wall of his house, and the next day would be the last of the allotted time.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The month given by Brigham Young and the specific numbers 28 and 27 are not in the evidence. The note and the figure 2 are.

### c16 [causal] Why does Holmes conclude that the victim was poisoned, and what supports this conclusion?

**Reference:** Holmes sniffed the dead man's lips at the scene and detected a slightly sour smell. From this he concluded that the man had had poison forced on him.

- `49671:49772` Finally, he sniffed the dead man’s lips, and then glanced at the soles of his patent leather boots.
- `237259:237398` Having sniffed the dead man’s lips I detected a slightly sour smell, and I came to the conclusion that he had had poison forced upon him.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The look of horror and the absence of wounds are not in the evidence. The sniffing and sour smell are.

### c17 [causal] Why was Jefferson Hope identified as the suspect from the Cleveland telegram?

**Reference:** The Cleveland reply told Holmes that Drebber had applied for the protection of the law against an old rival in love named Jefferson Hope, who was then in Europe. Drebber had earlier recognized Hope in the street and read murder in his eyes. This made Hope the obvious suspect.

- `207108:207242` It chanced, however, that Drebber, looking from his window, had recognized the vagrant in the street, and had read murder in his eyes.
- `239878:240053` It told me that Drebber had already applied for the protection of the law against an old rival in love, named Jefferson Hope, and that this same Hope was at present in Europe.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support this. Hope being detained is not in the evidence.

### c18 [causal] Why does Lucy Ferrier die, and what earlier event sets it in motion?

**Reference:** Lucy is offered a choice between the sons of Stangerson and Drebber. She is later forced into a hateful marriage, and after that and her father's terrible death she pines away and dies within a month.

- `165989:166103` Stangerson has a son, and Drebber has a son, and either of them would gladly welcome your daughter to their house.
- `201922:202120` Whether it was the terrible death of her father or the effects of the hateful marriage into which she had been forced, poor Lucy never held up her head again, but pined away and died within a month.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The two quotes support the causal chain. Brigham Young as the speaker and the marriage being to Drebber specifically are not explicit in the evidence.

### c19 [causal] Why did the murderer write RACHE on the wall, according to Holmes and to the murderer himself?

**Reference:** Holmes says the writing was simply a ruse to divert the inquiry into a wrong channel. The murderer says that what had puzzled the New Yorkers would puzzle the Londoners, so he printed the word on the wall in his own blood. Both treat it as a deliberate false trail.

- `63386:63446` It was simply a ruse to divert inquiry into a wrong channel.
- `228229:228388` I guessed that what puzzled the New Yorkers would puzzle the Londoners, so I dipped my finger in my own blood and printed it on a convenient place on the wall.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The point about the German lettering and the mention of socialists are not in the evidence. The two quotes support the reduced answer.

### c20 [causal] Why did Jefferson Hope's wedding ring come back into play as bait, and why did the murderer respond to the advertisement?

**Reference:** Holmes reasons that the murderer had dropped the ring while stooping over Drebber's body and would come back for it. Holmes uses the advertisement as bait. The murderer later admits he sent an accomplice because he thought the ad might be a trap or the ring he had lost.


- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): There is no evidence.

### c21 [contrast] How do the newspapers and Holmes differ on the likely motive for the Brixton Road murder?

**Reference:** The papers blame political refugees and revolutionists because of the German name, the lack of any other motive and the inscription. Holmes notes that robbery was not the object, since nothing was taken, and asks whether the motive was politics or a woman.

- `87315:87494` The German name of the victim, the absence of all other motive, and the sinister inscription on the wall, all pointed to its perpetration by political refugees and revolutionists.
- `237845:238010` And now came the great question as to the reason why. Robbery had not been the object of the murder, for nothing was taken. Was it politics, then, or was it a woman?

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both quotes support the contrast. The 'deliberate, methodical' killing is not in the evidence.

### c22 [contrast] How do Holmes and Gregson differ in their method of asking for information about the murder?

**Reference:** Holmes presses Gregson to telegraph again about whatever circumstance the whole case hinges on. Holmes himself telegraphs Cleveland and limits his enquiry to the circumstances of Enoch Drebber's marriage, a narrow and decisive question.

- `52300:52417` “Nothing else? Is there no circumstance on which this whole case appears to hinge? Will you not telegraph again?”
- `239710:239850` I telegraphed to the head of the police at Cleveland, limiting my enquiry to the circumstances connected with the marriage of Enoch Drebber.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Gregson's specific Stangerson enquiry is not in the evidence. Holmes's pressing of Gregson and his own focused telegram are.

### c23 [contrast] How does Holmes respond to the public credit given to the Scotland Yard detectives for the capture?

**Reference:** Early in the story Holmes says the detectives will get the credit whatever happens, and he calls being an unofficial personage the reason. When the Echo praises them, he says that what you do in the world matters less than what people believe you did. He treats the public credit as cynical and unimportant.

- `40982:41140` Supposing I unravel the whole matter, you may be sure that Gregson, Lestrade, and Co. will pocket all the credit. That comes of being an unofficial personage.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): A contrast question needs more than one source, and the Echo response is missing.

### c24 [evolution] How does the detectives' theory of the suspect develop from the first description to Holmes's identification of Jefferson Hope?

**Reference:** Holmes first describes the murderer from the evidence as a tall, square-toed man smoking a Trichinopoly cigar. Later he reasons that the murderer must have been the cabman, so he identifies Jefferson Hope as a jarvey. The suspect moves from a physical sketch to a named individual through deduction.

- `57799:58007` There has been murder done, and the murderer was a man. He was more than six feet high, was in the prime of life, had small feet for his height, wore coarse, square-toed boots and smoked a Trichinopoly cigar.
- `240812:240947` All these considerations led me to the irresistible conclusion that Jefferson Hope was to be found among the jarveys of the Metropolis.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The early physical description and the later conclusion about the jarveys support the answer.

### c25 [evolution] How do the detectives' suspects change between their first and second reports on the case?

**Reference:** Gregson first announces that they have the man under lock and key. After the second crime it is accepted that young Charpentier could not have been involved in that second affair, which undermines Gregson's suspect.

- `92702:92770` “The right track! Why, sir, we have the man under lock and key.”
- `118270:118338` Young Charpentier could not have been engaged in this second affair.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Lestrade's report and the claim that 'suspects shift to Stangerson' are not supported. Stangerson was a victim, not a suspect.

### c26 [evolution] How does Jefferson Hope's plan for dealing with Drebber change, from the first problem he describes to the chance that resolves it?

**Reference:** Hope's difficult problem was how to get Drebber to the Brixton Road house. He had almost decided to take him out into the country and confront him in some deserted lane when Drebber himself solved the problem for him.

- `219625:219709` How to get Drebber to that house was the difficult problem which I had now to solve.
- `221580:221756` I might take him right out into the country, and there in some deserted lane have my last interview with him. I had almost decided upon this, when he solved the problem for me.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The drunkenness and gin palace details are not in the evidence. The two quotes support the rest.

### c27 [evolution] How do the Ferriers' escape plans change from the month they first expect to the night they actually leave?

**Reference:** At first Ferrier plans to leave Utah at the end of the month and to wait for Jefferson Hope's return. When the threat grows, Hope arrives unannounced, and they flee at night through the window with the horses. The plan changes from a leisurely departure to a night escape.


- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): There is no evidence.

### c28 [local] Where was Watson wounded, and by what?

**Reference:** Watson was struck on the shoulder by a Jezail bullet, which shattered the bone and grazed the subclavian artery.

- `1819:1932` There I was struck on the shoulder by a Jezail bullet, which shattered the bone and grazed the subclavian artery.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The quote supports the wound and the weapon. Maiwand is not in this quote.

### c29 [local] What age is the little girl whom John Ferrier rescues in the desert?

**Reference:** The little girl is about five years old.

- `129166:129268` As he spoke he unwrapped the grey shawl and extricated a pretty little girl of about five years of age

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The quote states her age. The Alkali Plain and adoption details are not in the evidence.

### c30 [local] What is the address of the rooms Holmes and Watson take in London?

**Reference:** Holmes and Watson take rooms at 221B, Baker Street. They inspect the suite and move in together.

- `16692:16817` We met next day as he had arranged, and inspected the rooms at No. 221B, Baker Street, of which he had spoken at our meeting.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

## d-gutenberg-2701.txt (30 questions, 2 quotes not found)

> reviewer (claude-opus-5-5): The quote states 221B, Baker Street.

### d01 [trajectory] How does Ishmael's attitude toward Queequeg change from his first night at the Spouter-Inn to the point where they become bosom friends?

**Reference:** At first Ishmael is suspicious of Queequeg and resolves that the harpooneer must get into bed before he does. After the two share a pipe and a long conversation, Queequeg pledges friendship, and Ishmael admits that the savage has softened his bitterness toward the world.

- `55798:56027` I could not help it, but I began to feel suspicious of this “dark complexioned” harpooneer. At any rate, I made up my mind that if it so turned out that we should sleep together, he must undress and get into bed before I did.
- `134433:134579` I felt a melting in me. No more my splintered heart and maddened hand were turned against the wolfish world. This soothing savage had redeemed it.
- `136114:136361` and when our smoke was over, he pressed his forehead against mine, clasped me round the waist, and said that henceforth we were married; meaning, in his country’s phrase, that we were bosom friends; he would gladly die for me, if need should be.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): All three passages back the shift from suspicion to bosom friendship.

### d02 [contrast] How do Starbuck and Stubb differ in their attitudes toward fear, courage and death in the whale fishery?

**Reference:** Starbuck values courage only as a useful, practical thing and will have no man in his boat who is not afraid of a whale. Stubb, by contrast, is happy-go-lucky and takes perils as they come with an indifferent air, calm even in the most dangerous moments of the chase; what he thought of death itself, there is no telling.

- `264281:264368` “I will have no man in my boat,” said Starbuck, “who is not afraid of a whale.”
- `264870:265036` Starbuck was no crusader after perils; in him courage was not a sentiment; but a thing simply useful to him, and always at hand upon all mortally practical occasions.
- `269092:269327` A happy-go-lucky; neither craven nor valiant; taking perils as they came with an indifferent air; and while engaged in the most imminent crisis of the chase, toiling away, calm and collected as a journeyman joiner engaged for the year.
- `269944:269997` What he thought of death itself, there is no telling.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence doesn't say Starbuck is a superstitious Quaker, so that claim is removed; the rest is supported.

### d03 [causal] Why does Ahab become so moody and grim by the time the Pequod is at sea, and what is the cause that the text connects to his condition?

**Reference:** Peleg attributes Ahab's moodiness to the loss of his leg to a whale, along with the sharp shooting pains of his bleeding stump. When the Pequod sails, Ahab appears on deck with a face of 'crucifixion' and a 'mighty woe', showing that the old injury has left a lasting, brooding grief.

- `198281:198458` I know, too, that ever since he lost his leg last voyage by that accursed whale, he’s been a kind of moody—desperate moody, and savage sometimes; but that will all pass off.
- `198180:198280` it was the sharp shooting pains in his bleeding stump that brought that about, as any one might see.
- `284753:284911` And not only that, but moody stricken Ahab stood before them with a crucifixion in his face; in all the nameless regal overbearing dignity of some mighty woe.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Peleg links the moodiness to the lost leg and stump pains, and the deck scene shows the lasting woe.

### d04 [evolution] How does Ishmael's view of Yojo's plan for choosing the ship change from the moment Queequeg proposes it to the moment the Pequod is signed?

**Reference:** Ishmael at first does not like Queequeg's (Yojo's) plan for choosing the ship at all. Yet after boarding the Pequod he decides it is the very ship for them, and after signing the papers he has no doubt that the Pequod is the ship Yojo provided.

- `169783:169904` Now, this plan of Queequeg’s, or rather Yojo’s, touching the selection of our craft; I did not like that plan at all.
- `171234:171358` and finally, going on board the Pequod, looked around her for a moment, and then decided that this was the very ship for us.
- `195075:195286` And, after signing the papers, off I went; nothing doubting but that I had done a good morning’s work, and that the Pequod was the identical ship that Yojo had provided to carry Queequeg and me round the Cape.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence doesn't say Ishmael wanted Queequeg's judgment instead, so that detail is removed.

### d05 [trajectory] How does the relationship between the two owners, Peleg and Bildad, move from open hostility at the start of the voyage's preparations to an affectionate parting at Nantucket?

**Reference:** During the outfitting Peleg roars at Bildad and curses his advice. At the parting, Bildad grasps Peleg's hand and gazes at him, and a tear twinkles in Peleg's eye, so the hostility gives way to fondness.

- `192297:192589` “Thou Bildad!” roared Peleg, starting up and clattering about the cabin. “Blast ye, Captain Bildad, if I had followed thy advice in these matters, I would afore now had a conscience to lug about that would be heavy enough to founder the largest ship that ever sailed round Cape Horn.”
- `245956:246076` convulsively grasped stout Peleg by the hand, and holding up a lantern, for a moment stood gazing heroically in his face
- `246160:246316` As for Peleg himself, he took it more like a philosopher; but for all his philosophy, there was a tear twinkling in his eye, when the lantern came too near.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The fiery-pit warning and an explicit threat are not in the evidence, so they are removed.

### d06 [trajectory] How does Ishmael's judgment of the ragged prophet Elijah shift from the wharf conversation to the moment Elijah questions them about the men on the ship?

**Reference:** After the wharf meeting Ishmael and Queequeg dismiss Elijah as a humbug trying to be a bugbear. When Elijah later asks plainly whether they saw men going toward the ship, Ishmael is struck by the matter-of-fact question and answers seriously that he thought he saw four or five men.

- `225882:226065` Elijah! thought I, and we walked away, both commenting, after each other’s fashion, upon this ragged old sailor; and agreed that he was nothing but a humbug, trying to be a bugbear.
- `234081:234160` “Did ye see anything looking like men going towards that ship a while ago?”
- `234162:234307` Struck by this plain matter-of-fact question, I answered, saying, “Yes, I thought I did see four or five men; but it was too dim to be sure.”

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence doesn't show Ishmael becoming unsettled, so the answer now sticks to what the text shows.

### d07 [trajectory] How does Ahab's relationship to the white whale change from the quarter-deck speech, where the crew learns of Moby Dick's role in his injury, to the later account of his monomania?

**Reference:** On the quarter-deck Ahab blames the white whale for taking his leg and vows to chase him around the world, a personal grievance. Later the narrator says Ahab piled onto the whale's hump all the rage and hate of his whole race from Adam down. The grievance grows into a totalizing obsession.

- `369979:370236` “Aye, aye! it was that accursed white whale that razeed me; made a poor pegging lubber of me for ever and a day!” Then tossing both arms, with measureless imprecations he shouted out: “Aye, aye! and I’ll chase him round Good Hope, and round the Horn
- `409654:409859` He piled upon the whale’s white hump the sum of all the general rage and hate felt by his whole race from Adam down; and then, as if his chest had been a mortar, he burst his hot heart’s shell upon it.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claim about bodily woes and the world's malice is not in the evidence and is dropped.

### d08 [contrast] Ahab tells the crew that his vengeance is worth a premium, but later the narrator shows him privately worrying about the crew's money. How do these two positions differ?

**Reference:** On the quarter-deck Ahab waves off Starbuck's point about the Nantucket market, saying his vengeance will fetch a great premium. Privately, though, he decides he will not take away the men's hopes of cash, because cash they scorn now could later mutiny in them and cashier Ahab. His public scorn for profit is contradicted by his private calculation.

- `371173:371314` How many barrels will thy vengeance yield thee even if thou gettest it, Captain Ahab? it will not fetch thee much in our Nantucket market.”
- `371319:371651` Nantucket market! Hoot! But come closer, Starbuck; thou requirest a little lower layer. If money’s to be the measurer, man, and the accountants have computed their great counting-house the globe, by girdling it with guineas, one to every three parts of an inch; then, let me tell thee, that my vengeance will fetch a great premium
- `476500:476774` I will not strip these men, thought Ahab, of all hopes of cash—aye, cash. They may scorn cash now; but let some months go by, and no perspective promise of it to them, and then this same quiescent cash all at once mutinying in them, this same cash would soon cashier Ahab.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The answer is reworded to match the cash passage.

### d09 [causal] Why does Ahab keep the Pequod performing ordinary whaling duties and maintaining mast-head watches, even though his real purpose is the white whale?

**Reference:** By announcing the voyage's private purpose (to chase the white whale), Ahab laid himself open to the charge of usurpation. So he saw that he must still largely stay true to the Pequod's natural, nominal whaling purpose.

- `476866:477127` Having impulsively, it is probable, and perhaps somewhat prematurely revealed the prime but private purpose of the Pequod’s voyage, Ahab was now entirely conscious that, in so doing, he had indirectly laid himself open to the unanswerable charge of usurpation
- `370323:370489` And this is what ye have shipped for, men! to chase that white whale on both sides of land, and over all sides of earth, till he spouts black blood and rolls fin out.
- `477837:477960` Ahab plainly saw that he must still in a good degree continue true to the natural, nominal purpose of the Pequod’s voyage

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The porpoise and crew-refusal details are not in the evidence and are removed.

### d10 [evolution] How does the crew's commitment to the hunt change from Ahab's first offer of the gold ounce to the moment the harpooneers swear the oath over the harpoon-sockets?

**Reference:** It starts as a reward: Ahab holds up a sixteen-dollar Spanish gold ounce for the crew. It becomes a ritual pact when Ahab has the harpooneers drink and swear 'Death to Moby Dick!'

- `366703:366837` Look ye! d’ye see this Spanish ounce of gold?”—holding up a broad bright coin to the sun—“it is a sixteen dollar piece, men.
- `378855:378963` Drink, ye harpooneers! drink and swear, ye men that man the deathful whaleboat’s bow—Death to Moby Dick!

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The nailing, the cheering and the socket details are not in the evidence, so the answer is trimmed to what is quoted.

### d11 [local] According to the text, which ship was cruising in the Pacific Ocean in 1820, who was her captain, and where was he from?

**Reference:** The ship Essex, Captain Pollard, of Nantucket.

- `460325:460423` In the year 1820 the ship Essex, Captain Pollard, of Nantucket, was cruising in the Pacific Ocean.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The sinking by a whale is not in the evidence, so the question and answer now ask only for what the passage states.

### d12 [trajectory] How does Starbuck's stance toward Ahab change from the Captain's dinner, where he is deferential, to the chapter called Dusk, where he admits he is trapped by Ahab?

**Reference:** At dinner Starbuck takes his meat from Ahab 'as though receiving alms', showing servile deference to the captain. By the Dusk chapter, after the oath and the crew's frenzy, he is bound to Ahab against his conscience, describing his duty as 'to obey, rebelling'. His deference has become a reluctant, resentful submission.

- `339724:339868` the mate received his meat as though receiving alms; and cut it tenderly; and a little started if, perchance, the knife grazed against the plate
- `382312:382446` My soul is more than matched; she’s overmanned; and by a madman! Insufferable sting, that sanity should ground arms on such a field!
- `382807:382910` Oh! I plainly see my miserable office,—to obey, rebelling; and worse yet, to hate with touch of pity!

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The dinner passage and the Dusk passages together show deference turning into rebellious obedience.

### d13 [trajectory] How does the image of the sperm whale's head as a riddle or Sphinx develop from Ahab's first address to it to the later chapters on its brow and brain?

**Reference:** The hanging head first seems to Ahab's view like the Sphynx's in the desert. Later the narrator puts the whale's brow before the reader with 'Read it if you can,' and calls the sperm whale physiognomically a Sphinx whose brain, to the phrenologist, is an unsquarable circle.

- `688699:688823` It was a black and hooded head; and hanging there in the midst of so intense a calm, it seemed the Sphynx’s in the desert.
- `765335:765386` I but put that brow before you. Read it if you can.
- `765410:765552` If the Sperm Whale be physiognomically a Sphinx, to the phrenologist his brain seems that geometrical circle which it is impossible to square.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The demand that the head speak and the 'outworks' detail are not in the evidence and are removed.

### d14 [trajectory] How does the image of the whale-line and bondage change from the warning that all men are born with halters round their necks, to the Siamese tie between Ishmael and Queequeg, to the closing claim that every reader is a Loose-Fish and a Fast-Fish?

**Reference:** First, whale-lines stand for the silent perils all men live with, like halters around the neck. In the monkey-rope scene the line becomes an 'elongated Siamese ligature' uniting Ishmael and Queequeg. Finally the idea of being fastened or free is applied to the globe and to the reader, who is both a Loose-Fish and a Fast-Fish.

- `628339:628573` But why say more? All men live enveloped in whale-lines. All are born with halters round their necks; but it is only when caught in the swift, sudden turn of death, that mortals realize the silent, subtle, ever-present perils of life.
- `706700:706750` So, then, an elongated Siamese ligature united us.
- `876302:876415` What is the great globe itself but a Loose-Fish? And what are you, reader, but a Loose-Fish and a Fast-Fish, too?

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The answer is trimmed to what the three passages actually say.

### d15 [trajectory] How does the scene of the sharks and birds feeding on the dead whale move from the night of the Sabbath feast into the mournful image of a funeral?

**Reference:** On the night of the feast, thousands of sharks swarm and feast on the dead whale's fatness. Later the narrator recasts the scene as a doleful, mocking funeral, with sea-vultures in pious mourning and air-sharks in black.

- `650581:650736` Mingling their mumblings with his own mastications, thousands on thousands of sharks, swarming round the dead leviathan, smackingly feasted on its fatness.
- `684168:684313` There’s a most doleful and most mocking funeral! The sea-vultures all in pious mourning, the air-sharks all punctiliously in black or speckled.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'vultureism' reflection is not in the evidence and is removed.

### d16 [contrast] How does the narrator's judgment of the dignity of the sperm whale's head differ from his judgment of the right whale's head?

**Reference:** The narrator first ranks the sperm whale's head above the right whale's for pervading dignity and character. Later he jokingly calls the right whale a sulky king who is ill-suited to a diadem. The right whale is thus treated as the less noble of the two.

- `726649:726753` As you behold it, you involuntarily yield the immense superiority to him, in point of pervading dignity.
- `736518:736599` But if this whale be a king, he is a very sulky looking fellow to grace a diadem.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages are present and support the contrast.

### d17 [causal] Why does the Pequod come close to capsizing when the sinking whale is still tied to her, and what role does Starbuck play in it?

**Reference:** The ship was already strained from carrying the two whale heads. Then the sinking body dragged her over sideways. Starbuck, who was in charge, held on to the body so resolutely that the ship would have capsized had it kept holding on.

- `724058:724291` In good time, Flask’s saying proved true. As before, the Pequod steeply leaned over towards the sperm whale’s head, now, by the counterpoise of both heads, she regained her even keel; though sorely strained, you may well believe.
- `791119:791339` However, Starbuck, who had the ordering of affairs, hung on to it to the last; hung on to it so resolutely, indeed, that when at length the ship would have been capsized, if still persisting in locking arms with the body
- `790936:791118` But a sudden stop was put to further discoveries, by the ship’s being unprecedentedly dragged over sideways to the sea, owing to the body’s immensely increasing tendency to sink.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The fasteners breaking is not in the quoted evidence, so it is removed.

### d18 [evolution] How does Stubb's idea about the ambergris change from a passing remark to a scheme and then to the actual salvage?

**Reference:** Stubb first wonders whether the old man has thought of it and decides it's worth trying. He then plans with the Guernsey-man, who as interpreter would tell the French captain whatever he pleased as if from Stubb. Finally Stubb pretends to cast off, the Frenchman sails away, and the Pequod slips in between him and Stubb's whale.

- `886044:886113` I wonder now if our old man has thought of that. It’s worth trying.
- `891499:891769` According to this little plan of theirs, the Guernsey-man, under cover of an interpreter’s office, was to tell the Captain what he pleased, but as coming from Stubb; and as for Stubb, he was to utter any nonsense that should come uppermost in him during the interview.
- `894829:895021` Presently a breeze sprang up; Stubb feigned to cast off from the whale; hoisting his boats, the Frenchman soon increased his distance, while the Pequod slid in between him and Stubb’s whale.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Finding the ambergris and any later order to desist are not in the evidence, so those are removed.

### d19 [trajectory] How does Pip's fate develop from the moment Stubb abandons him at sea to his later bond with Ahab?

**Reference:** Stubb's ruthless order to stay in the boat leaves Pip adrift, and the ship's rescue leaves him permanently changed, so that the crew think him an idiot. Much later, Ahab takes the mad boy into his own cabin, making the same castaway who was abandoned the one person Ahab feels bound to.

- `907863:907951` “Stick to the boat, Pip, or by the Lord, I won’t pick you up if you jump; mind that.
- `910522:910679` By the merest chance the ship itself at last rescued him; but from that hour the little negro went about the deck an idiot; such, at least, they said he was.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): No passage shows Pip's later bond with Ahab, so the trajectory is only half-supported.

### d20 [trajectory] How does Queequeg's coffin change from a sea-chest and an offering for his death into a life-buoy?

**Reference:** At Queequeg's bidding, the carpenter is ordered to make what he asks for (his coffin). Later Queequeg hints at using his coffin, and it is proposed as the life-buoy, to which Starbuck cries in shock, 'A life-buoy of a coffin!'

- `1041290:1041436` Now, when this strange circumstance was made known aft, the carpenter was at once commanded to do Queequeg’s bidding, whatever it might include.
- `1127576:1127660` by certain strange signs and inuendoes Queequeg hinted a hint concerning his coffin.
- `1127662:1127718` “A life-buoy of a coffin!” cried Starbuck, starting.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The sea-chest, the carving and the lack of a light cask are not in the evidence, so the answer is trimmed.

### d21 [causal] Why does Starbuck take up the loaded musket in the cabin during the typhoon, and why does he finally put it back?

**Reference:** Starbuck remembers that Ahab once pointed that very loaded musket at him and would have shot him, and this pushes him toward using it on Ahab. After wrestling inwardly 'with an angel', he puts the musket back in its rack and leaves.

- `1034293:1034437` Ahab seized a loaded musket from the rack (forming part of most South-Sea-men’s cabin furniture), and pointing it towards Starbuck, exclaimed:
- `1106034:1106137` “He would have shot me once,” he murmured, “yes, there’s the very musket that he pointed at me;
- `1109645:1109770` Starbuck seemed wrestling with an angel; but turning from the door, he placed the death-tube in its rack, and left the place.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The fear of doom and the 'no lawful way' reasoning are not in the evidence, so the answer sticks to what is shown.

### d22 [contrast] Contrast Ahab's cold rejection of the Rachel's captain with his later tearful, tender moment on the calm sea.

**Reference:** Ahab turns away from the Rachel's captain with averted face and goes below, an unconditional rejection of his earnest plea. Later, on the calm sea, Ahab drops a tear into the sea, and the narrator says the whole Pacific does not hold such wealth as that one drop.

- `1142127:1142298` Hurriedly turning, with averted face, he descended into his cabin, leaving the strange captain transfixed at this unconditional and utter rejection of his so earnest suit.
- `1161048:1161177` From beneath his slouched hat Ahab dropped a tear into the sea; nor did all the Pacific contain such wealth as that one wee drop.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The talk of his wife and child is not in the evidence and is removed.

### d23 [evolution] How does the harpoon Ahab orders from Perth change from a request for an unbreakable weapon into an instrument he believes holds the whale's death?

**Reference:** Ahab first asks Perth for a harpoon that a thousand yoke of fiends could not part. It is baptized 'in nomine diaboli'. Later Ahab holds it up and declares that he holds the whale's death in his hand, its barbs tempered in blood and by lightning.

- `1059396:1059485` “I, too, want a harpoon made; one that a thousand yoke of fiends could not part, Perth;
- `1062492:1062557` “Ego non baptizo te in nomine patris, sed in nomine diaboli!”
- `1157170:1157292` “Look ye, Nantucketer; here in this hand I hold his death! Tempered in blood, and tempered by lightning are these barbs;

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The horseshoe stubs and the Delight are not in the evidence, so the answer is aligned to the passages.

### d24 [trajectory] How does the doubloon change from a talisman the crew revere to a prize Ahab claims as his own?

**Reference:** The mariners, however wanton, revere the doubloon as the white whale's talisman. Later Ahab claims it, declaring 'the doubloon is mine, Fate reserved the doubloon for me.'

- `941438:941544` however wanton in their sailor ways, one and all, the mariners revered it as the white whale’s talisman.
- `1170367:1170467` “Not the same instant; not the same—no, the doubloon is mine, Fate reserved the doubloon for me.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The guarding from theft and the offer to whoever kills the whale are not in the evidence, so they are removed.

### d25 [trajectory] How does Ahab's response to the Parsee's prophecy change from the moment the Parsee vanishes to the moment his body is seen tied to the whale?

**Reference:** When the Parsee disappears, Ahab treats the prophecy that the Parsee would go before him, yet be seen again, as a riddle that 'pecks my brain' and vows to solve it. When the Parsee's torn body appears lashed to the whale, Ahab accepts that the prophecy has come true, calls it the 'hearse' the Parsee promised, and treats it as a fulfilment he must still follow to the end.

- `1219794:1219978` “Befooled, befooled!”—drawing in a long lean breath—“Aye, Parsee! I see thee again.—Aye, and thou goest before; and this, _this_ then is the hearse that thou didst promise.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): Only one passage is given, and it doesn't show Ahab's earlier reaction, so the trajectory isn't supported.

### d26 [contrast] How do Ahab and Starbuck each justify their obedience, and how do their accounts of who they are obeying differ?

**Reference:** Ahab presents himself as the Fates' lieutenant who acts under orders and expects his crew to obey him. Starbuck, by contrast, frames the same obedience as a possible betrayal of God, saying he fears he disobeys God by obeying Ahab. The two men thus locate final authority in different places: Ahab in fate, Starbuck in God.

- `1205039:1205140` Fool! I am the Fates’ lieutenant; I act under orders. Look thou, underling! that thou obeyest mine.
- `1211046:1211192` “God keep us, but already my bones feel damp within me, and from the inside wet my flesh. I misdoubt me that I disobey my God in obeying him!”

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Ahab's 'Fates' lieutenant' line and Starbuck's 'disobey my God' line support the contrast.

### d27 [causal] Why does the Pequod ultimately go down, given the warnings that Starbuck gives during the chase?

**Reference:** On the third day Starbuck begs Ahab to give up, saying Moby Dick does not seek him and it is Ahab who madly seeks the whale. Ahab keeps going, and as disaster strikes the ship Starbuck cries, 'Oh, Ahab, Ahab, lo, thy work,' blaming the destruction on Ahab's persistence.

- `1220900:1221066` “Oh! Ahab,” cried Starbuck, “not too late is it, even now, the third day, to desist. See! Moby Dick seeks thee not. It is thou, thou, that madly seekest him!”
- `1226879:1226934` Oh, Ahab, Ahab, lo, thy work. Steady! helmsman, steady.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The details of the ramming are not in the evidence, so the answer rests on Starbuck's warning and his final cry.

### d28 [evolution] How does Ahab's plan for his boats change from the morning after the first broken boat to the moment he sends the others back to the ship on the third day?

**Reference:** After the first wreck Ahab orders the rest of the spare boats lowered and rigged and the boats' crews mustered to keep chasing. On the third day he sends his mates back to the ship, telling them to repair the useless boats if they can, and saying that if not, Ahab alone is enough to die.

- `1202289:1202439` Up helm, then; pile on the sail again, ship keepers! down the rest of the spare boats and rig them—Mr. Starbuck away, and muster the boat’s crews.
- `1220055:1220201` Away, mates, to the ship! those boats are useless now; repair them if ye can in time, and return to me; if not, Ahab is enough to die—Down, men!

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The Parsee detail is not in this evidence and is removed.

### d29 [trajectory] How does Starbuck's appeal to Ahab change from the moment he first begs him to stop hunting the whale to his final plea as Ahab's boat leaves the ship?

**Reference:** Starbuck first protests in God's name that Ahab will never capture the whale, calling the chase worse than devil's madness. At the last he pleads emotionally — 'Oh, my captain, my captain! … go not' — pointing to his own tears as a sign of how deeply he means it.

- `1204025:1204220` “Great God! but for one single instant show thyself,” cried Starbuck; “never, never wilt thou capture him, old man—In Jesus’ name no more of this, that’s worse than devil’s madness.
- `1214469:1214617` “Oh, my captain, my captain!—noble heart—go not—go not!—see, it’s a brave man that weeps; how great the agony of the persuasion then!”

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The swamping warning and the claim that Ahab refused both times are not in the evidence, so they are removed.

### d30 [trajectory] How does the coffin, which Ahab rejects as his own fate, reappear in the epilogue, and what does that reversal reveal?

**Reference:** Ahab, before the final chase, declares that no coffin or hearse can be his and that only hemp can kill him, and the line then catches him round the neck and pulls him from the boat. In the epilogue, the coffin life-buoy is what keeps Ishmael afloat for a day and a night, so the coffin Ahab refused becomes the means of survival for the one man who escapes. The reversal shows the fate Ahab tried to avoid falling on him and sparing another.

- `1217718:1217919` “Drive, drive in your nails, oh ye waves! to their uttermost heads drive them in! ye but strike a thing without a lid; and no coffin and no hearse can be mine:—and hemp only can kill me! Ha! ha!”
- `1230451:1230497` but the flying turn caught him round the neck,
- `1233914:1234015` Buoyed up by that coffin, for almost one whole day and night, I floated on a soft and dirgelike main.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

## e-arxiv-2606.05658v1.txt (30 questions, 2 quotes not found)

> reviewer (claude-opus-5-5): Ahab's 'no coffin… hemp only' line, the line catching his neck, and Ishmael buoyed by the coffin all support the reversal.

### e01 [trajectory] How does the paper's view of query decomposition change from its introduction to its closing summary of findings?

**Reference:** Decomposition is introduced as a way to break a complex query into sub-questions, gather evidence for each, and refine the answer. The findings conclude that it helps in structured domains with clean chained relationships but is not universally beneficial and degrades ranking precision on harder multi-hop data.

- `3110:3308` Instead of a single retrieve-then-generate step, an agentic system can decompose a complex query into sub-questions, gather evidence for each, and refine its answer through self-reflection [11, 12].
- `25019:25224` Decomposition helps in structured domains where chained reasoning maps cleanly onto well-defined relationships, but it is not universally beneficial and degrades ranking precision on harder multi-hop data.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The introduction and findings passages support both parts of the answer.

### e02 [trajectory] How does the paper move from describing the reflection loop's design to judging its results?

**Reference:** Reflection is designed as a bounded loop capped at two iterations to control cost and latency. The results section then judges that, as designed, reflection imposes a large computational cost without reliable quality gains.

- `14403:14541` To bound cost and latency, the reflection loop is capped at two iterations, balancing potential quality gains against responsiveness [46].
- `22104:22233` These results indicate that, as currently designed, reflection imposes a large computational cost without reliable quality gains.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The cap of two iterations and the judgment that reflection is costly without reliable gains are both quoted.

### e03 [trajectory] How is the two-dataset framing set up in the abstract and carried through to the conclusion?

**Reference:** The abstract introduces the DevOps knowledge base and MuSiQue as complementary datasets. The conclusion restates the contrasting outcomes for each: decomposition improves the structured domain but reduces ranking precision on multi-hop data, while reflection gives marginal gains at substantial latency cost.

- `956:1100` We evaluate the system across two complementary datasets: a domain-specific DevOps knowledge base and the multi-hop reasoning benchmark MuSiQue.
- `27244:27519` Across a structured DevOps corpus and the multi-hop MuSiQue benchmark, query decomposition improved retrieval quality in the structured domain but reduced ranking precision in the multi-hop setting, and reflection delivered marginal quality gains at substantial latency cost.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The abstract's framing of the two datasets and the contrasting outcomes in the conclusion are both supported.

### e04 [trajectory] How does latency develop from a measured cost of decomposition into a design requirement for future work?

**Reference:** The tradeoff analysis notes that every agentic addition increases response time, with reflection on MuSiQue approaching a sixfold slowdown that is likely unacceptable in real-time DevOps use. Future work then calls for orchestration to become explicitly cost-aware because of these large latency penalties.

- `24450:24624` Accuracy vs. latency: every agentic addition increases response time, with reflection on MuSiQue approaching a sixfold slowdown—likely unacceptable in real-time DevOps use.
- `28195:28312` Given the large latency penalties measured for agentic components, orchestration should become explicitly cost-aware.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 21 s to 48 s figures are not in the evidence, so the answer now uses only the quoted claims.

### e05 [trajectory] How does the paper's treatment of the decomposition trigger move from an observed result to a proposed target for improvement?

**Reference:** In the strategy distribution results, the authors argue the decomposition trigger is conservative, since many complex MuSiQue queries (49% remain complex-RAG) are not routed to decomposition. In the limitations, they identify the cause as decomposition and reflection being triggered by static, heuristic rules.

- `23891:24173` Third, the comparatively small share of decomposed queries—even on MuSiQue, where 49% of queries remain in the complex-RAG category—suggests that the decomposition trigger is conservative and that not all complex queries that could benefit are actually routed for decomposition.
- `27685:27796` A central limitation we observed is that decomposition and reflection are triggered by static, heuristic rules.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The learning-based selection proposal is not in the cited evidence, so the answer stops at the limitation.

### e06 [trajectory] How does the precision-versus-coverage problem with decomposition move from the results to the proposed fixes?

**Reference:** The results show decomposition broadens coverage but fragments the contextual signal that ranking depends on, harming precision on multi-hop queries. Future work states that the decomposition strategy can be refined to address this precision loss on multi-hop data.

- `20970:21123` Decomposition therefore broadens coverage but fragments the contextual signal that ranking depends on, harming precision in genuinely multi-hop settings.
- `28693:28796` The current decomposition strategy can be refined to address the precision loss seen on multi-hop data.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The specific fixes (more coherent sub-queries, preserved dependencies) are not in the cited passages, so the answer is reduced to what is quoted.

### e07 [trajectory] How does the paper's concern about the small size of its DevOps corpus evolve from the limitations section to future work?

**Reference:** The limitations section says the 80-document corpus is modest in scale, so absolute metric values should be read as indicative rather than definitive. Future work calls for evaluation at larger scale, with much larger corpora, noisy queries, and concurrent load.

- `25596:25950` First, the DevOps knowledge base is modest in scale (80 documents, roughly 10,000 words) and was generated with LLM assistance before manual validation; while it is designed to emulate realistic structural heterogeneity, results on a larger production corpus may differ, and the absolute metric values should be read as indicative rather than definitive.
- `30138:30297` Finally, the system should be evaluated at larger scale and under realistic conditions, including noisy queries, much larger corpora, and concurrent user load.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both passages are quoted and support the answer.

### e08 [trajectory] How does the decomposition strategy's link to Least-to-Most prompting connect to the refinements proposed in the conclusion?

**Reference:** The related work says decomposition aligns closely with Least-to-Most prompting, which solves simpler sub-problems in order. The later refinements aim to keep dependencies between sub-tasks and to move beyond flat sub-query lists toward structured reasoning representations.

- `9788:9991` Least-to-Most prompting [48] decomposes a hard problem into a sequence of simpler sub-problems solved in order, an approach that aligns closely with the query decomposition strategy adopted in this work.
- `28797:29141` Promising directions include generating more semantically coherent sub-queries, explicitly preserving contextual dependencies between sub-tasks so that ranking signal is not fragmented, and integrating structured reasoning representations such as knowledge or reasoning graphs to capture inter-step relationships that flat sub-query lists miss.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The Least-to-Most link and the proposed refinements are both quoted.

### e09 [trajectory] How does the role of chunking shift from the related-work survey to the paper's own pipeline argument?

**Reference:** The related work lists chunking strategies as one of the advanced RAG improvements for context selection. The method section argues that segmentation involves a recall–precision tradeoff and that poor segmentation can split the evidence needed for a multi-hop reasoning chain.

- `7197:7350` Advanced RAG counters this with query rewriting and expansion [33], re-ranking of candidates, and chunking strategies that improve context selection [8].
- `15983:16229` Segmentation involves a recall–precision tradeoff: larger chunks improve semantic completeness while smaller chunks sharpen retrieval specificity, and improper segmentation can isolate evidence needed to reconstruct a multi-hop reasoning chain.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Chunking appears in related work and the recall–precision tradeoff appears in the method, both quoted.

### e10 [trajectory] How does the idea of tool use move from the related-work discussion to the future-work proposals?

**Reference:** Related work notes that Toolformer showed models can learn to call external tools and that ReAct interleaves reasoning and actions. Future work proposes, for DevOps, querying live monitoring systems or ticketing APIs so responses reflect the current system state rather than static documentation alone.

- `8866:9114` The ReAct framework [12] formalized the interleaving of reasoning traces and actions, Toolformer [42] showed that models can learn to call external tools, and Self-RAG [11] introduced learned reflection signals for selective retrieval and critique.
- `29904:30092` In a DevOps context this could mean querying live monitoring systems or ticketing APIs, enabling responses grounded in the current state of a system rather than static documentation alone.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Knowledge graphs and symbolic engines are not in the cited evidence, so they are removed from the answer.

### e11 [trajectory] How does the fully local, privacy-first design choice relate to the later proposal to query live systems?

**Reference:** The contributions state the system is built on a fully local, privacy-first inference stack. The future-work section proposes querying live monitoring systems and ticketing APIs, which would take the DevOps system beyond static, locally stored documentation.

- `3671:3898` We implement a complete Agent-Orchestrated Adaptive RAG system composed of a Query Classifier, a Query Decomposer, an Answer Evaluator, and a central Orchestrator, built on a fully local, privacy-first inference stack [13, 14].
- `29904:30092` In a DevOps context this could mean querying live monitoring systems or ticketing APIs, enabling responses grounded in the current state of a system rather than static documentation alone.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The local, privacy-first stack and the live-system proposal are both quoted.

### e12 [trajectory] How is MuSiQue characterized in the related work, and how does that framing return in the future-work section?

**Reference:** Related work says MuSiQue was built so that answering requires evidence integration across independent documents and resists single-hop shortcuts. The future-work section uses this contrast to propose learning when decomposition helps (DevOps) and when it harms ranking (MuSiQue).

- `10341:10535` MuSiQue [15] was deliberately constructed by composing single-hop questions so that answering requires genuine evidence integration across independent documents and resists single-hop shortcuts.
- `27948:28163` Reinforcement learning or meta-learning could be used to balance quality against efficiency directly, learning when decomposition genuinely helps—as in the DevOps domain—and when it harms ranking, as on MuSiQue.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The MuSiQue description and its use as a contrast in future work are both quoted.

### e13 [trajectory] How does hallucination move from being the motivation for RAG to a criterion inside the answer evaluator?

**Reference:** The introduction names factual hallucination as one of two persistent LLM limitations that RAG addresses. The method section later makes hallucination detection one of the four axes the Answer Evaluator critiques.

- `1897:2159` Large Language Models (LLMs) have shown strong capabilities in natural language understanding and generation, yet they remain constrained by two persistent limitations: factual hallucination and an inability to access knowledge beyond their training data [1, 2].
- `13988:14218` The Answer Evaluator Agent adds a self-reflective layer that critiques generated responses along four axes: relevance to the query, citation accuracy, support (grounding in retrieved context), and hallucination detection [11, 49].

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Hallucination as motivation and as an evaluator axis are both quoted.

### e14 [trajectory] How does citation accuracy change across the abstract, the decomposition results, and the reflection results?

**Reference:** The abstract says reflection improves citation accuracy at a substantial latency cost. Decomposition raises DevOps citation accuracy to 0.917, while on MuSiQue reflection gives only a slight citation gain alongside a falling overall score.

- `1101:1477` Using metrics that include overall score, citation accuracy, mean reciprocal rank, and topic coverage, we find that query decomposition yields consistent gains in the structured domain (overall score +0.04 , MRR +0.17 on DevOps) but degrades ranking precision on the multi-hop benchmark, while the reflection mechanism improves citation accuracy at a substantial latency cost.

- [ ] keep  - [ ] edit  - [x] drop  - [ ] evidence justifies the answer

> reviewer (claude-opus-5-5): There is only one evidence passage, and the 0.917 figure and MuSiQue reflection claims are unsupported.

### e15 [trajectory] How does metadata-aware filtering appear first in the query classifier description and then in the indexing details?

**Reference:** The classifier section says it categorizes requests into document types to enable metadata-aware filtering before similarity ranking. The indexing section explains that chunks are stored with metadata and attribute constraints are applied before ranking to narrow the candidate space.

- `12904:13051` The Query Classifier Agent categorizes each incoming request into document types, enabling metadata-aware filtering before similarity ranking [45].
- `16231:16508` Each chunk is embedded with the BGE encoder and indexed in FAISS together with its metadata, enabling hybrid filtering in which attribute constraints (for example, restricting to runbooks or incident reports) are applied before similarity ranking to narrow the candidate space.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The classifier and indexing passages both describe filtering before ranking.

### e16 [trajectory] How does the idea of routing by question difficulty from related work reappear in the Orchestrator and later in future work?

**Reference:** Related work says Adaptive-RAG routes queries to strategies by predicted question difficulty. The Orchestrator routes queries based on the classifier output and triggers decomposition when required, and future work proposes learning-based strategy selection that predicts the optimal reasoning strategy from query features.

- `9115:9313` Corrective RAG [43] added an explicit retrieval-correction step, and Adaptive-RAG [44] routed queries to retrieval strategies of differing complexity based on predicted question difficulty [45, 46].
- `12289:12467` The Orchestrator is the central controller. It routes queries based on the classifier output, triggers decomposition when required, and manages the evaluation–reflection cycle.
- `27797:27947` Future work can replace these with learning-based strategy selection, in which the system predicts the optimal reasoning strategy from query features.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The Orchestrator passage does not say 'rule-based', so the answer is adjusted.

### e17 [contrast] Does the decomposition effect hold the same direction in the structured and multi-hop datasets?

**Reference:** No. The abstract describes contrasting results showing agentic enhancements are not universally beneficial, and on MuSiQue ranking metrics collapse under decomposition (MRR falls from 0.469 to 0.102, Success@5 from 1.0 to 0.063) even though citation accuracy reaches 1.0 and topic coverage rises.

- `1478:1644` These contrasting results show that agentic enhancements are not universally beneficial and must be applied selectively according to query and domain characteristics.
- `20785:20969` Although citation accuracy reaches 1.0 and topic coverage rises sharply (0.609 to 0.859), ranking-based metrics collapse: MRR falls from 0.469 to 0.102 and Success@5 from 1.0 to 0.063.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The cited abstract passage speaks of contrasting results, not explicit gains, so the wording is aligned with it.

### e18 [contrast] How do decomposition and reflection differ in their effect on DevOps overall score?

**Reference:** Decomposition raises the DevOps overall score (the abstract reports gains in the structured domain). Reflection lowers it, with overall score falling from 0.870 to 0.781, no gain in recall or ranking, and latency roughly doubling.

- `1204:1477` we find that query decomposition yields consistent gains in the structured domain (overall score +0.04 , MRR +0.17 on DevOps) but degrades ranking precision on the multi-hop benchmark, while the reflection mechanism improves citation accuracy at a substantial latency cost.
- `21787:21916` On DevOps, overall score actually decreases from 0.870 to 0.781 with no gain in recall or ranking, while latency roughly doubles.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The +0.04 gain from decomposition and the 0.870 to 0.781 drop from reflection are both quoted.

### e19 [contrast] Does the conclusion describe reflection's quality gains the same way as the results section does?

**Reference:** Not exactly. The results section says reflection gives no reliable quality gains as designed, while the conclusion describes reflection as delivering marginal quality gains at substantial latency cost, a softer characterization.

- `22104:22233` These results indicate that, as currently designed, reflection imposes a large computational cost without reliable quality gains.
- `27244:27519` Across a structured DevOps corpus and the multi-hop MuSiQue benchmark, query decomposition improved retrieval quality in the structured domain but reduced ranking precision in the multi-hop setting, and reflection delivered marginal quality gains at substantial latency cost.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The two quoted passages show 'without reliable gains' versus 'marginal gains'.

### e20 [contrast] Is reflection treated as a routing decision by the orchestrator in the same way as decomposition?

**Reference:** No. The orchestrator is described as routing each query to the agents it needs, but the strategy results show reflection does not change routing; it is a post-hoc corrective step applied after the same routing distribution.

- `11381:11471` A central Orchestrator routes each query, invoking only the agents a given query requires.
- `23251:23455` Notably, the distribution is identical between the decomposition-enabled and full-agentic configurations, confirming that reflection operates as a post-hoc corrective step rather than a routing mechanism.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The routing description and the statement that reflection is post-hoc rather than a routing mechanism are both quoted.

### e21 [contrast] How do Naive RAG and the proposed agentic pipeline differ in controlling their retrieval flow?

**Reference:** Naive RAG is a fixed single-pass procedure, which works for isolated lookups but struggles with multi-document reasoning. The agentic pipeline determines its control flow at runtime, using intermediate outputs to decide subsequent actions.

- `2789:2953` This works well for isolated factual lookups, but it struggles with queries that require reasoning across several documents or chaining intermediate results [7, 8].
- `8094:8315` Whereas a Naive pipeline executes a predetermined sequence, an agentic system determines its control flow at runtime, using intermediate outputs to guide subsequent actions and invoking external tools only as needed [39].

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The predetermined sequence versus runtime control flow contrast is supported.

### e22 [causal] Why is decomposition adopted for so few MuSiQue queries despite many queries being complex?

**Reference:** The authors attribute it to a conservative decomposition trigger, and to rule-based heuristics in the orchestrator, so many complex queries that could benefit are not routed to decomposition. The observed distribution therefore reflects these specific rules rather than an optimal policy.

- `23891:24173` Third, the comparatively small share of decomposed queries—even on MuSiQue, where 49% of queries remain in the complex-RAG category—suggests that the decomposition trigger is conservative and that not all complex queries that could benefit are actually routed for decomposition.
- `26461:26629` Fourth, the orchestrator’s routing relies on rule-based heuristics, so the observed strategy distributions reflect those specific rules rather than an optimal policy.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The conservative trigger and the rule-based heuristics are both quoted.

### e23 [causal] Why are the reported latency figures described as relative trends rather than absolute timings?

**Reference:** The generator is a 4-bit quantized Llama model run locally, and the latency was measured in that environment. Because the figures are environment-dependent, they capture relative trends across configurations rather than portable absolute timings.

- `17283:17405` The generative core is Meta’s Llama-3.1-8B-Instruct, quantized to 4-bit GGUF for local inference via llama.cpp [13, 14].
- `26630:26821` Finally, latency figures are environment-dependent and were measured under local quantized inference; they capture relative trends across configurations rather than portable absolute timings.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The quantized local inference and the environment-dependent caveat are both quoted.

### e24 [causal] Why does the paper stress latency so strongly for the DevOps setting?

**Reference:** DevOps and SRE are high-stakes settings where the speed and correctness of documentation lookup can determine incident resolution time. The authors therefore judge a reflection slowdown of about sixfold as likely unacceptable in real-time DevOps use.

- `2160:2441` These limitations are especially consequential in high-stakes operational settings such as DevOps and Site Reliability Engineering (SRE), where the correct interpretation of technical documentation can be the difference between rapid incident resolution and prolonged downtime [3].
- `24450:24624` Accuracy vs. latency: every agentic addition increases response time, with reflection on MuSiQue approaching a sixfold slowdown—likely unacceptable in real-time DevOps use.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The high-stakes DevOps setting and the sixfold-slowdown judgment are both quoted.

### e25 [evolution] How does the proposed decomposition trigger move from its design to the authors' final judgment on it?

**Reference:** The method defines a workflow that detects multi-hop structure, generates ordered sub-queries, retrieves for each, and aggregates the answers. The authors later flag the triggering policy as a high-value target for future optimization.

- `13574:13946` The workflow is: (i) detect multi-hop structure, flagging queries that involve multiple entities or chained dependencies; (ii) generate ordered sub-queries such that earlier results can inform later retrieval; (iii) retrieve independently for each sub-query; and (iv) aggregate the intermediate answers into a single coherent response that aligns with the original intent.
- `24174:24263` This points directly to triggering policy as a high-value target for future optimization.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Conservative' is not in the cited evidence, so it is removed.

### e26 [evolution] How does the open question posed in the introduction get answered in the conclusion?

**Reference:** The introduction says it is unclear how agentic enhancements behave across domains that differ in structure and reasoning demand, since most studies use one domain. The conclusion answers that such enhancements must be applied selectively and with explicit cost awareness rather than uniformly.

- `3309:3549` Interest in this direction has grown quickly, but most studies evaluate agentic RAG on a single domain or benchmark. As a result, it remains unclear how these enhancements behave across domains that differ in structure and reasoning demand.
- `27520:27649` The clear lesson is that agentic enhancements must be applied selectively and with explicit cost awareness rather than uniformly.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The open question in the introduction and the lesson in the conclusion are both quoted.

### e27 [evolution] How does the call for cost-aware orchestration develop from the abstract to the future-work section?

**Reference:** The abstract argues for adaptive, cost-aware orchestration rather than uniformly aggressive reasoning pipelines. Future work makes this concrete, noting that the large latency penalties of agentic components mean orchestration should become explicitly cost-aware.

- `1645:1756` Our findings argue for adaptive, cost-aware orchestration rather than uniformly aggressive reasoning pipelines.
- `28195:28312` Given the large latency penalties measured for agentic components, orchestration should become explicitly cost-aware.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The abstract and future-work passages both call for cost-aware orchestration.

### e28 [local] What chunking configuration is used for the DevOps knowledge base?

**Reference:** Documents are segmented into 600-token chunks with a 100-token overlap, using a token-based semantic chunking strategy.

- `15600:15713` Documents are segmented with a token-based semantic chunking strategy: 600-token chunks with a 100-token overlap.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 600-token chunks with 100-token overlap are directly stated.

### e29 [local] How large is the DevOps knowledge base and how many documentation types does it span?

**Reference:** The knowledge base comprises 80 documents, approximately 10,000 words, spanning six documentation types.

- `17995:18281` The DevOps knowledge base comprises 80 documents (approximately 10,000 words) spanning six documentation types—Standard, Architecture, Runbooks, Incidents, Postmortem, and Onboarding—generated with LLM assistance and validated manually to reflect realistic organizational knowledge.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 80 documents, about 10,000 words and six types are directly stated.

### e30 [local] What is the maximum number of reflection iterations, and why is it capped?

**Reference:** The reflection loop is capped at two iterations to bound cost and latency while balancing potential quality gains against responsiveness.

- `14403:14541` To bound cost and latency, the reflection loop is capped at two iterations, balancing potential quality gains against responsiveness [46].

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

## f-kerno-adrs.txt (30 questions, 1 quotes not found)

> reviewer (claude-opus-5-5): The cap of two iterations and its reason are directly stated.

### f01 [trajectory] ¿Cómo pesa el requisito de un modo de durabilidad relajada en la elección de motor, desde el criterio planteado en ADR-0001 hasta el modelo de commit de ADR-0002?

**Reference:** ADR-0001 dice que Kerno quiere ofrecer un modo de durabilidad relajada sin comprometer la consistencia (ADR-0002), y que ese requisito pesa en la elección. En un LSM esa garantía sale del propio diseño del WAL, lo que favorece al LSM frente al B+ tree.

- `4310:4426` Kerno quiere ofrecer un modo de durabilidad relajada sin comprometer la consistencia (ADR-0002), así que esto pesa.
- `22661:22703` En un LSM sale del propio diseño del WAL.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The claim about CoW sync points and deferred page reuse is not in the evidence. Trimmed to what the quotes support.

### f02 [trajectory] ¿Dónde gana el B+ tree frente al LSM y por qué, aun así, ADR-0001 descarta el B+ tree para los casos de referencia de Kerno?

**Reference:** El B+ tree gana en colas de latencia a escala medium cuando la compactación del LSM coincide con escritura sostenida (memoria de agentes 3,7×, actualizaciones secuenciales 3,0×, inserción secuencial 2,1×, commit de LiveSpec 1,3×) y en lecturas cortas de rangos pequeños. Aun así se descarta, porque los casos de referencia de Kerno (Cauce, memoria de agentes, Revision Engine) son intensivos en escritura.

- `10348:10760` El B+ tree lo tiene en `medium` cuando la compactación del LSM coincide con escritura sostenida (memoria de agentes 3,7×, actualizaciones secuenciales 3,0×, inserción secuencial 2,1×, commit de LiveSpec 1,3×; en `small`, con poca compactación, esas cuatro colas las gana el LSM) y en lecturas cortas de rangos pequeños (listado de módulo 1,5×, recuperación de recuerdos 2,0×, lectura de Cauce 1,3×).
- `14374:14591` pero los casos de referencia de Kerno son intensivos en escritura (Cauce: commits pequeños durables; memoria de agentes: ingesta continua; Revision Engine: cada cambio escribe además una revisión y un `ChangeSet`).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Operational simplicity and open/close are not in the evidence. The latency tails and short range reads are.

### f03 [causal] ¿Por qué el LSM encaja mejor que el B+ tree con el Revision Engine, según el análisis de escritura de ADR-0001?

**Reference:** En un CoW cada commit reescribe la ruta raíz-hoja de cada hoja tocada más metadatos, con uno o dos fsync, así que el techo de commits durables queda por debajo del LSM. En el LSM las revisiones son escrituras secuenciales en WAL/memtable, que son baratas, a costa de una lectura de historial algo más lenta.

- `9470:9653` En un CoW cada commit reescribe la ruta raíz-hoja de cada hoja tocada más metadatos; con dos `fsync` (prototipo) o uno (redb), el techo de commits durables queda por debajo del LSM.
- `12394:12499` Las revisiones son escrituras secuenciales en WAL/memtable (barato); lectura de historial algo más lenta

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 8× figure is not in the evidence.

### f04 [trajectory] ¿Cómo se relacionan las diferencias de rendimiento entre prototipos y motores maduros con la conclusión sobre la arquitectura que se adopta?

**Reference:** ADR-0001 admite que las razones LSM/B+ incluyen también diferencias de madurez (riesgo R-S1). ADR-0012 lo repite como limitación: los prototipos tienen días de desarrollo, y sus diferencias frente a redb/fjall miden también madurez, no solo arquitectura. Por tanto, la ventaja medida no prueba por sí sola la superioridad de una arquitectura.

- `8434:8518` Por eso las razones LSM / B+ incluyen también diferencias de madurez (riesgo R-S1).
- `65909:66040` Los prototipos tienen días de desarrollo; sus diferencias frente a redb/fjall miden también madurez, no solo arquitectura (R-S1).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 'señal orientativa' conclusion is not in the quotes. Trimmed to the acknowledged maturity confound.

### f05 [evolution] ¿Cómo cambió la propuesta de ADR-0007 sobre índices desde su planteamiento hasta su aceptación?

**Reference:** Se propuso un modelo mixto por definición de índice. Al aceptarse, se fijó así: el índice primario es consistente con el commit, y los secundarios son derivados asíncronos con estado (ready/building/stale/failed) y watermark de secuencia.

- `45710:45762` 3. **Mixto por definición de índice** (propuesta).
- `46624:46832` Aceptada la opción recomendada: el índice primario es consistente con el commit; los índices secundarios son derivados asíncronos con estado (`ready`/`building`/`stale`/`failed`) y watermark de secuencia.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The date and the 'forma abierta' claim are unsupported. The core change is supported.

### f06 [evolution] ¿Cómo resolvió el propietario el punto abierto de si los índices de unidades debían ser transaccionales, tras la implementación de E4-03?

**Reference:** E4-03 implementó los índices de unidades como derivados con frescura explícita, porque el acta y la invariante I-23 exigían estado y watermark. El propietario aprobó la implementación de E4-03/E4-04: IDX_PRIMARY e IDX_TYPE siguen siendo derivados, solo los escribe rebuild_index, y tras una escritura query usa la ruta de referencia hasta la siguiente reconstrucción (I-32).

- `48470:48623` E4-03 lo implementa como derivado con frescura explícita, porque el acta (§3, fila E4-03) y la invariante I-23 piden el estado y el watermark para él.
- `49921:50204` Se aprueba la implementación de E4-03/E4-04: los índices de unidades (`IDX_PRIMARY`, `IDX_TYPE`) siguen siendo **derivados** con frescura explícita y solo los escribe `rebuild_index`; tras una escritura, `query` usa la ruta de referencia hasta la siguiente reconstrucción (I-32).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The date and the measured-workload condition are not in the evidence.

### f07 [evolution] ¿Cómo evolucionó la cuestión de si la cabecera del WAL debía sincronizarse, desde detectarla como riesgo hasta su cierre?

**Reference:** Se detectó que la primera trama de cada WAL llevaba synced_len = 0 porque W-1 no sincronizaba la cabecera. Se cerró el 2026-10-31 (M1 de la revisión adversarial): el escritor sincroniza la cabecera en W-1, y WAL-R6 rechaza cualquier trama válida bajo una cabecera rota.

- `80670:80806` - **`synced_len` y cabecera rota (punto 4):** la primera trama de cada WAL lleva `synced_len = 0`, porque W-1 no sincroniza la cabecera.
- `81522:81686` **Cerrado (2026-10-31, M1 de la revisión adversarial):** el escritor sincroniza la cabecera en W-1 y WAL-R6 rechaza cualquier trama válida bajo una cabecera rota.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The loss of a Sync commit is not stated in the evidence.

### f08 [contrast] ¿En qué se diferencian los errores Incompatible y Corruption en el diseño del formato v1?

**Reference:** Incompatible señala una versión o característica desconocida, que no es daño, y no debe presentarse como Corruption. Corruption señala daño. El diseño garantiza que un daño aleatorio nunca se presente como Incompatible y que una incompatibilidad nunca se presente como daño.

- `72777:72871` Una versión o característica desconocida no es daño y no debe presentarse como `Corruption`
- `77056:77164` Un daño aleatorio nunca se presenta como `Incompatible`; una incompatibilidad nunca se presenta como daño.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The ordering of the CRC check before the version check is not in the evidence.

### f09 [causal] ¿Por qué el borrador del formato v1 necesitaba marcar sus bases antes de la congelación, y qué se podía hacer con sus decisiones hasta entonces?

**Reference:** Sin marca, una base creada con el borrador podría abrirse tras la congelación y leerse mal. Hasta la congelación, cada decisión podía revertirse cambiando el borrador y regenerando los vectores (v1.md §15).

- `74202:74285` Sin marca: una base del borrador podría abrirse tras la congelación y leerse mal.
- `77434:77550` Cada decisión se puede revertir antes de la congelación cambiando el borrador y regenerando vectores (v1.md §15).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The rejection after the freeze is not in the evidence. Reframed around what the quotes say.

### f10 [trajectory] ¿Cómo avanza el estado de la especificación del formato desde el planteamiento de ADR-0003 hasta la congelación de v1?

**Reference:** ADR-0003 parte de que M0 no congeló ningún formato: los prototipos solo estaban versionados por magic y un número de versión. El formato concreto se congela al cerrar la Fase 1, y el bit DRAFT se retiró en la congelación del 2026-10-31.

- `28048:28177` M0 no congela ningún formato: los dos prototipos tienen formatos propios, versionados solo por *magic* y un número de versión.
- `27434:27481` el formato concreto se congela al cerrar Fase 1
- `83465:83535` El bit `DRAFT` (punto 10) se retiró en la congelación del 2026-10-31

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Minor unsupported claim about the accepted rules removed.

### f11 [trajectory] ¿Cómo conviven la reserva de espacio para cifrado en el formato v1 y el estado pendiente de ADR-0006?

**Reference:** ADR-0003 exige que el formato reserve desde la versión 1 espacio para nonce y etiqueta, con un bit incompat que indica su uso, aunque ADR-0006 siga pendiente. ADR-0013 consolida esa reserva: activar AEAD no exigirá migrar SST ni MANIFEST y los WAL se crean ya con el bit. Así el formato protege contra una migración futura sin fijar todavía el algoritmo ni la gestión de claves.

- `29897:30126` CRC32C para detectar daño accidental. La integridad frente a manipulación es AEAD (ADR-0006): el formato reserva desde la versión 1 espacio por página/bloque para nonce y etiqueta, y un bit `incompat` indica si están en uso.
- `80447:80538` activar AEAD (ADR-0006) no exigirá migrar SST ni MANIFEST; los WAL se crean ya con el bit.
- `44748:44867` Condiciona ADR-0003 (cabeceras y tamaños), ADR-0009 (borrado criptográfico) y ADR-0005 (claves para tokens cifrados).

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The reservation, the incompat bit, no migration needed and ADR-0006 conditioning ADR-0003 are all supported.

### f12 [trajectory] ¿De qué ADR depende ADR-0009 y cómo lo condiciona ADR-0006?

**Reference:** ADR-0009 depende de ADR-0001, ADR-0004 y ADR-0006. ADR-0006 condiciona el borrado criptográfico de ADR-0009, además de las cabeceras de ADR-0003 y las claves de ADR-0005, así que la supresión queda ligada a la decisión de cifrado.

- `55320:55361` Depende de ADR-0001, ADR-0004 y ADR-0006.
- `44748:44867` Condiciona ADR-0003 (cabeceras y tamaños), ADR-0009 (borrado criptográfico) y ADR-0005 (claves para tokens cifrados).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The evidence shows the dependencies but not explicitly that ADR-0006 is pending. Question reframed.

### f13 [evolution] ¿Cómo evolucionó el modelo de revisiones de ADR-0004 desde la pregunta inicial hasta la decisión de copia completa?

**Reference:** El SAD contemplaba un híbrido con límite de profundidad (opción 3), pero se decidió copia completa: cada revisión guarda su carga útil entera, sin deltas hasta que los benchmarks los justifiquen. El híbrido queda como evolución futura, sujeta a medición y a un ADR nuevo.

- `37765:37865` cada revisión guarda su carga útil entera; no hay deltas hasta que los benchmarks los justifiquen.
- `38413:38538` La opción 3 (híbrido con límite de profundidad) del SAD queda como evolución futura, sujeta a medición y a un ADR nuevo.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The initial question is not in the evidence. Kept the hybrid proposal versus full copy.

### f14 [trajectory] ¿Cómo se traslada la necesidad de suprimir datos desde el planteamiento de ADR-0009 hasta los requisitos del motor de ADR-0001 y el formato de ADR-0013?

**Reference:** ADR-0009 señala que borrar un dato no borra sus revisiones, índices, páginas liberadas ni backups. ADR-0001 registra que el LSM retiene versiones antiguas hasta compactar, por lo que exige range tombstones y compactación dirigida, y ADR-0013 los incorpora en la base de v1 porque añadirlos después exigiría migración.

- `54158:54277` Borrar un dato actual no borra sus revisiones, índices, embeddings, páginas liberadas, segmentos antiguos ni backups.
- `12625:12723` Versiones antiguas en SST/WAL hasta compactar; necesita *range tombstones* y compactación forzada
- `72105:72222` Añadirlas después con un bit `incompat`: cada base creada antes quedaría sin poder suprimir rangos sin migración.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): All three steps are supported by quotes from different ADRs.

### f15 [causal] ¿Por qué el codec por defecto de ADR-0005 permite cerrar ADR-0006 más adelante?

**Reference:** Porque el camino crítico por defecto no contiene material criptográfico, así que ADR-0006 puede cerrarse después sin bloquear las fases 1–3.

- `42342:42470` No hay material criptográfico en el camino crítico por defecto; ADR-0006 puede cerrarse después sin bloquear las fases 1–3.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Only one quote, and it does not mention the random token. Question narrowed to what is supported.

### f16 [trajectory] ¿Cómo llega la experiencia de embeddings de M0 hasta el contrato de artefactos derivados que debe definirse antes de la Etapa 8?

**Reference:** En M0, el workload agent_memory almacenó embeddings de 768 × f32 (3 KiB) como valores normales y midió escritura, lectura y espacio en ambos candidatos. ADR-0018 exige aplicar el contrato de derivados a los índices de la Etapa 4 y a las optimizaciones de la Etapa 6 antes de los embeddings de la Etapa 8.

- `58810:58957` El workload `agent_memory` almacena embeddings de 768 × f32 (3 KiB) como valores normales y mide escritura, lectura y espacio en ambos candidatos.
- `9184:9230` con valores de 1–4 KiB y embeddings de 3 KiB
- `120269:120393` Aplicar el contrato a los índices de la Etapa 4 y a las optimizaciones de la Etapa 6 antes de los embeddings de la Etapa 8.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): ADR-0011 being pending is not in the evidence.

### f17 [evolution] ¿Cómo se modificó la regla de invalidación de cachés de resultados en ADR-0016 entre la propuesta y las garantías definitivas?

**Reference:** La propuesta permitía que una caché de resultados no sirviera lecturas de otra secuencia salvo prueba de que los datos leídos no cambiaron. Las garantías del propietario eliminaron esa excepción: la caché queda ligada a la secuencia de commit de su instantánea, hasta que un ADR posterior defina la prueba.

- `112963:113136` una caché de resultados además registra la secuencia de commit de la instantánea y no sirve lecturas de otra secuencia salvo prueba de que los datos leídos no cambiaron.
- `110980:111213` Una caché de resultados queda ligada a la secuencia de commit de su instantánea y no sirve otra secuencia; se elimina la excepción «salvo prueba de que los datos leídos no cambiaron» hasta que un ADR posterior defina esa prueba

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the proposal and the removal of the exception are quoted.

### f18 [contrast] ¿Por qué ADR-0016 propone la opción 2 de adaptación limitada y no la opción 3 de adaptación general?

**Reference:** La opción 3 tiene mayor superficie de errores, y la especificación prohíbe cambios de esquema autónomos sin política explícita. Por eso se propone la opción 2, sujeta a las garantías G1–G7.

- `108380:108500` Mayor superficie de errores; la especificación prohíbe además cambios de esquema autónomos sin política explícita.
- `112561:112615` Propuesta: opción 2, sujeta a las garantías G1–G7.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Única optimización reversible' and the comparison with the disabled mode are unsupported.

### f19 [trajectory] ¿Cómo se mantuvo la ruta de referencia desde la Etapa 5 hasta la garantía G2 de la adaptación operacional?

**Reference:** La Etapa 5 dejó el planificador determinista, la ruta de referencia y la prueba diferencial planificador/referencia en CI. ADR-0016 exige que la ruta de referencia se mantenga, se compile y se pruebe en CI aunque no se use en producción, como condición de equivalencia.

- `109011:109133` La Etapa 5 dejó el planificador determinista, la ruta de referencia y la prueba diferencial planificador/referencia en CI
- `109966:110024` Se mantiene, se compila y se prueba en CI aunque no se use
- `113453:113563` La equivalencia exige que la ruta de referencia se mantenga y se pruebe en CI aunque no se use en producción.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Recurre ante cualquier duda' is unsupported.

### f20 [trajectory] ¿Cómo evolucionó el alcance de la API pública desde el trait Engine del spike hasta la aceptación de ADR-0010?

**Reference:** El trait Engine del spike carecía de transacciones read-modify-write, cursores, cancelación y errores por categoría. ADR-0010 aceptó una capa genérica sobre TxnEngine de kerno-core, con separación estricta de kerno-storage, y dejó la fachada pública única y la política de semver para cuando exista una API pública estable.

- `56679:56770` carece de transacciones read-modify-write, cursores, cancelación y errores por categoría.
- `57182:57270` genérico sobre `TxnEngine` de `kerno-core`, con separación estricta de `kerno-storage`
- `57655:57808` La fachada pública única (`kerno`, opción 1) y la política de semver se deciden cuando exista una API pública estable; esta aceptación no las fija.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The crate name and 'contrato de comparación' are not in the evidence.

### f21 [trajectory] ¿Qué diferencia hay entre el resultado de la medición de salida de Fase 1 y la condición 1 que ADR-0001 fijó para el LSM?

**Reference:** La condición 1 de ADR-0001 señala como caso a resolver el p99 de escritura en memoria de agentes (3,7× el del B+ tree). La medición de salida de la Fase 1 en la clase de VM de M0 no cumple T1, así que la condición 1 sigue sin cumplirse.

- `13490:13579` (el p99 de escritura en memoria de agentes, 3,7× el del B+ tree, es el caso a resolver).
- `17482:17560` La medición de salida en la clase de VM de M0 **no cumple T1** (condición 1)

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Incumplimientos documentados' is unsupported.

### f22 [trajectory] ¿Cómo se relacionan el objetivo de 2× frente a redb de ADR-0012 y el resultado del B+ tree reportado en ADR-0001?

**Reference:** ADR-0012 fija como objetivo orientativo quedar dentro de 2× del motor maduro de su familia, redb para CoW B+ tree. ADR-0001 indica que el B+ tree queda entre 0,55 y 0,8 veces redb en la mayoría de workloads, lo que está dentro de ese objetivo aunque por debajo de redb en casi todos los casos.

- `65177:65374` el motor elegido debe quedar dentro de 2× del motor maduro de su misma familia (redb para CoW B+ tree, fjall para LSM) en los workloads de M0 a escala `medium`, y documentar dónde no lo consigue.
- `15338:15509` el B+ tree, dentro del objetivo de 2× de redb de ADR-0012 (0,55–0,8× en la mayoría de workloads, a la par en memoria de agentes y por encima en inserción secuencial)

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The 2× target and the 0,55–0,8× result are both quoted.

### f23 [causal] ¿Por qué ADR-0014 rechaza archivar o borrar una unidad con relaciones vivas, y qué espacio deja para cambiar esa decisión?

**Reference:** El rechazo impide que existan relaciones hacia unidades archivadas o borradas, así que la supresión física bajo ADR-0009 no tiene relaciones colgantes que tratar. El propietario lo aprobó como semántica explícita, relajable más adelante a cascada o a relación colgante marcada.

- `101308:101499` Semántica aprobada por el propietario para unidades archivadas o borradas con relaciones vivas: **rechazo** de la operación (relajable más adelante a cascada o relación colgante marcada).
- `99820:100086` Una relación hacia una unidad suprimida queda sujeta a ADR-0009. Con el rechazo de E4-02 no puede existir una relación hacia una unidad archivada o borrada, por lo que la supresión física no tiene relaciones colgantes que tratar mientras se mantenga ese rechazo.

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): 'Sin cambiar el formato' and E4-00 are not in the quotes.

### f24 [evolution] ¿Cómo pasó ADR-0014 de propuesta a decisión aceptada?

**Reference:** En E4-00 el ADR seguía propuesto y se esperaba que la decisión final saliera del prototipo de E4-02. El 2026-11-05 se aceptó la opción 2, registro de relación con índices de adyacencia transaccionales, con la evidencia del prototipo como base.

- `101241:101307` Sigue *Propuesto*. La decisión final sale del prototipo de E4-02.
- `97363:97494` **Opción 2: registro de relación más índices de adyacencia transaccionales**, aceptada el 2026-11-05 con la evidencia de E4-02.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The proposed status and the acceptance date with E4-02 evidence are both quoted.

### f25 [contrast] ¿En qué se diferencian la opción 1 y la opción 2 de ADR-0014 en coste de escritura y en acceso a vecinos?

**Reference:** La opción 1, relaciones como unidades, tiene mínima superficie pero recorrer vecinos exige filtrar todas las relaciones. Las mediciones muestran que además escribe más por relación, porque hereda la maquinaria de la unidad, mientras la opción 2 escribe tres claves pequeñas y resuelve vecinos por rango.

- `85935:86015` Mínima superficie; recorrer vecinos exige un filtro sobre todas las relaciones.
- `92392:92582` La opción 1 escribe *más* porque cada relación hereda la maquinaria de la unidad (cabeza, `REV_META`, `REV_PAYLOAD` y miembro del `ChangeSet`); la opción 2 escribe tres claves pequeñas.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the neighbour-access and write-cost contrasts are quoted.

### f26 [causal] ¿Por qué los índices de adyacencia de relaciones no necesitan watermark, al contrario que los índices de unidades?

**Reference:** ADR-0014 define los índices de adyacencia como primarios: se escriben en el mismo ChangeSet que el registro de la relación. ADR-0007 los mantiene transaccionales, mientras que los índices de unidades son derivados con estado y watermark. Como nunca quedan desfasados, no hace falta una marca de frescura.

- `100320:100494` Los índices de adyacencia son **primarios** (se actualizan en el mismo `ChangeSet`, ADR-0007), no secundarios con estado de frescura: E4-03 no necesita watermark para ellos.
- `50414:50504` Los índices de adyacencia de relaciones no cambian: son transaccionales (ADR-0014, I-21).

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Primary/transactional adjacency indices versus watermark are supported from two places.

### f27 [trajectory] ¿Cómo se traslada el término «conexión» desde la metáfora de ADR-0017 hasta la procedencia obligatoria de las relaciones en ADR-0014?

**Reference:** ADR-0017 asigna el término «conexión» a Edge (ADR-0014) y exige que toda conexión inferida guarde origen y confianza y no se presente como declarada (I-39). ADR-0014 define Edge con procedencia entre sus campos, lo que permite cumplir ese requisito.

- `115101:115141` Las «conexiones» son `Edge` (ADR-0014)
- `85174:85357` La Etapa 4 introduce relaciones tipadas (`Edge`) entre unidades: `edge_id`, `from_unit_id`, `to_unit_id`, `relation_type`, peso y validez temporal opcionales, procedencia y atributos.
- `116575:116668` Toda «conexión» inferida guarda origen y confianza y no se presenta como declarada (I-39).

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): The provenance enum values and validation by writer/decoder are not quoted.

### f28 [trajectory] ¿Cómo se trata la plasticidad en ADR-0017 y en ADR-0016, y por qué no puede mutar las unidades?

**Reference:** ADR-0017 rechazó una unidad con estado dinámico propio, porque las lecturas mutarían datos y se mezclaría aprendizaje con contenido. ADR-0016 traslada la plasticidad a estado derivado separado: las lecturas no mutan datos de usuario y actualizar estadísticas solo escribe en el almacén derivado.

- `115370:115507` Rechazada en la propuesta: haría que las lecturas muten datos (spec §2) y mezclaría aprendizaje operacional con contenido (spec §10).
- `113265:113450` Las lecturas no mutan datos de usuario (spec §2); registrar observaciones o actualizar estadísticas desde una lectura escribe solo en el almacén derivado y nunca bloquea al escritor.

- [x] keep  - [ ] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Both the rejection and the derived-store rule are quoted.

### f29 [trajectory] ¿Cómo se traslada la exigencia de privacidad de ADR-0015 a la trazabilidad de las optimizaciones de ADR-0016?

**Reference:** ADR-0015 sustituye los literales por marcadores tipados antes de calcular una huella BLAKE3 con clave local no exportada, y ningún literal aparece, tampoco en diagnostic. ADR-0016 hereda esa regla: el registro de decisiones sobre optimizaciones sigue las reglas de privacidad de ADR-0015, sin literales ni contenido.

- `106035:106203` Los literales se sustituyen por marcadores tipados antes de calcular la huella; la huella usa BLAKE3 con clave local no exportada, con algoritmo y versión registrados.
- `112337:112420` El registro sigue las reglas de privacidad de ADR-0015 (sin literales ni contenido)
- `104164:104204` Ninguno aparece, tampoco en `diagnostic`

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): G7 and sentinels are not clearly supported. Trimmed.

### f30 [trajectory] ¿Cómo evolucionó la situación de U6 entre el cierre de la Etapa 3 y la re-medición de ADR-0014, y qué se decidió sobre ella?

**Reference:** U6 incumplía al cierre de la Etapa 3 y sigue incumpliendo (K-E3-9) con y sin relaciones tras la re-medición. Se decidió repetir U6 con varias pasadas, con y sin relaciones, cuando se aborde K-E3-9.

- `95969:96057` U6 sigue incumpliendo (K-E3-9), con y sin relaciones, igual que al cierre de la Etapa 3.
- `101036:101109` Repetir U6 (varias pasadas, con y sin relaciones) cuando se aborde K-E3-9

- [ ] keep  - [x] edit  - [ ] drop  - [x] evidence justifies the answer

> reviewer (claude-opus-5-5): Thresholds and 'una pasada no basta' are unsupported.

