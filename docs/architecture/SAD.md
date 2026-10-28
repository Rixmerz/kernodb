# KERNO — Documento inicial de arquitectura y plan de implementación

| Campo | Valor |
|---|---|
| Versión | 0.1.0 |
| Fecha | 27 de octubre de 2026 |
| Estado | Baseline inicial para revisión técnica |
| Tipo | Documento de arquitectura de software (SAD) |
| Lenguaje de implementación | Rust |
| Modelo de entrega inicial | Biblioteca embebida y CLI local |

> **Resumen ejecutivo.** Kerno será un motor de datos embebido, persistente y extensible. Su núcleo será un almacén key-value transaccional; sobre él se construirán documentos estructurados, historial de revisiones, índices y, más adelante, recuperación híbrida para IA. La arquitectura prioriza correctitud, recuperación, portabilidad y evolución explícita de los datos. El algoritmo de almacenamiento físico —copy-on-write B+ tree o LSM— permanece pendiente de un ADR basado en una prueba técnica reproducible.

> **Nota de mantenimiento.** Este archivo es la baseline 0.1.0 tal como fue aprobada para revisión. Las decisiones posteriores se registran en [`docs/adr/`](../adr/README.md); cuando un ADR cambie algo de este documento, se publicará una nueva versión del SAD en lugar de editar esta silenciosamente.

## Contenido

| N.º | Apartado |
|---|---|
| 1 | [Propósito y alcance](#1-propósito-y-alcance) |
| 2 | [Decisiones confirmadas y abiertas](#2-decisiones-confirmadas-y-decisiones-abiertas) |
| 3 | [Principios de arquitectura](#3-principios-de-arquitectura) |
| 4 | [Despliegue y concurrencia](#4-modelo-de-despliegue-y-concurrencia) |
| 5 | [Modelo lógico de datos](#5-modelo-lógico-de-datos) |
| 6 | [Arquitectura de módulos](#6-arquitectura-lógica-de-módulos) |
| 7 | [Almacenamiento, transacciones y recuperación](#7-almacenamiento-físico-transacciones-y-recuperación) |
| 8 | [Revision Engine](#8-revision-engine-historial-diferencias-y-procedencia) |
| 9 | [Índices y motor de consultas](#9-índices-y-motor-de-consultas) |
| 10 | [Embeddings y recuperación híbrida](#10-embeddings-y-recuperación-híbrida-para-ia) |
| 11 | [Seguridad, privacidad y ciclo de vida](#11-seguridad-privacidad-y-ciclo-de-vida) |
| 12 | [Backups, diagnóstico y operación](#12-backups-diagnóstico-y-operación-local) |
| 13 | [Esquemas, formato y compatibilidad](#13-esquemas-formato-y-compatibilidad) |
| 14 | [Organización del repositorio](#14-organización-propuesta-del-repositorio) |
| 15 | [ADR propuestos](#15-architecture-decision-records-adr) |
| 16 | [Plan de implementación](#16-plan-de-implementación) |
| 17 | [Primer hito: M0](#17-primer-hito-m0--decisión-de-almacenamiento-basada-en-evidencia) |
| 18 | [Calidad, pruebas y definición de terminado](#18-calidad-pruebas-y-definición-de-terminado) |
| 19 | [Riesgos y mitigaciones](#19-riesgos-y-mitigaciones) |
| 20 | [Decisiones y tareas inmediatas](#20-decisiones-y-tareas-inmediatas) |
| 21 | [Referencias técnicas](#21-referencias-técnicas) |
| A | [Glosario](#apéndice-a-glosario) |

## 1. Propósito y alcance

Este documento convierte las decisiones iniciales de Kerno en una base técnica para implementar, probar y revisar el proyecto. Define el propósito, los límites de la primera versión, los módulos, las garantías que el motor debe ofrecer, las decisiones aún abiertas y el plan de ejecución.

Kerno no se considerará listo por sumar funcionalidades, sino por demostrar garantías mediante pruebas reproducibles. La primera prioridad es que los datos confirmados puedan recuperarse correctamente, que las transacciones tengan semántica clara y que los formatos puedan evolucionar sin perder datos.

### 1.1 Visión

Kerno es un motor de datos embebido, local-first, con persistencia transaccional, evolución versionada y una ruta de extensión hacia recuperación híbrida para aplicaciones de IA.

La propuesta de valor no es implementar todos los modelos de bases de datos, sino coordinar de forma consistente:

- almacenamiento y consultas por clave;
- documentos estructurados y sus índices;
- revisiones, diferencias y procedencia de cambios;
- búsqueda textual y vectorial como extensiones;
- controles de acceso, cifrado, retención y exportación;
- recuperación ante fallos y migración de formatos.

### 1.2 Objetivos del producto

1. Ofrecer un motor embebido usable desde una aplicación Rust y mediante una CLI.
2. Persistir datos en disco y definir garantías de commit y recuperación verificables.
3. Mantener un núcleo key-value pequeño y extensible; los documentos serán una capa lógica superior.
4. Separar el journal de recuperación del historial permanente de revisiones y de la auditoría.
5. Permitir consultar y comparar revisiones de datos cuando la política de historial lo requiera.
6. Facilitar índices secundarios y recuperación híbrida sin acoplar el motor a un proveedor de IA.
7. Facilitar exportación, migraciones, diagnóstico, backups y pruebas de integridad.
8. Diseñar seguridad y privacidad como capacidades explícitas, sin prometer cumplimiento automático de una ley.

### 1.3 Casos de uso de referencia

| Caso | Carga representativa | Qué evalúa |
|---|---|---|
| LiveSpec | símbolos, dependencias, hashes, cambios y relaciones | consultas por clave, índices, trazabilidad y evolución |
| Cauce | tareas, intentos, resultados, eventos y memorias | escrituras transaccionales, historial y recuperación |
| Memoria de agentes | documentos, recuerdos, procedencia, vigencia y embeddings | historial, filtros, búsqueda textual/vectorial y eliminación de datos derivados |

Estos casos serán workloads de evaluación, no dependencias del motor ni modelos que deban imponerse a todos los usuarios.

### 1.4 No objetivos de la primera versión

- Servidor de red, servicio gestionado o protocolo remoto.
- Replicación, consenso, sharding o despliegue distribuido.
- SQL completo y su parser; la primera API será tipada y programática.
- Motor columnar para analítica masiva.
- Lenguaje propio de consultas de grafos.
- Ejecución de modelos de IA dentro del motor.
- Agente autónomo que decida qué recordar; esa lógica corresponde a la aplicación.
- Ramas y fusiones de historial al estilo Git.
- Garantía de que Kerno, por sí solo, hace legalmente conforme a cualquier aplicación.

## 2. Decisiones confirmadas y decisiones abiertas

### 2.1 Decisiones confirmadas

| ID | Decisión |
|---|---|
| D-01 | Implementación en Rust. |
| D-02 | Entrega inicial como biblioteca local y CLI; sin servidor de red. |
| D-03 | Un escritor y varios lectores concurrentes dentro del mismo proceso. |
| D-04 | Núcleo key-value; documentos estructurados por encima. |
| D-05 | UUIDv7 para identidad interna; representación pública configurable. |
| D-06 | El historial de revisiones es independiente del journal de recuperación. |
| D-07 | Índice primario primero; índices secundarios, texto y vectores después. |
| D-08 | Embeddings mediante adaptadores externos; su generación no forma parte del núcleo. |
| D-09 | Cifrado estándar, gestión de claves y políticas de ciclo de vida como capacidades del producto. |
| D-10 | Formato persistente versionado, migraciones documentadas y exportación independiente del formato físico. |
| D-11 | Comparar copy-on-write B+ tree y LSM mediante ADR antes de congelar el almacenamiento físico. |

### 2.2 Decisiones que todavía no deben asumirse como resueltas

- Elección definitiva del motor físico: B+ tree copy-on-write o LSM.
- Protocolo exacto de commit, sincronización a disco y recuperación.
- Representación binaria de páginas/registros y compatibilidad entre versiones.
- Modelo concreto de MVCC, snapshots de lectura y bloqueo entre procesos.
- Formato de revisiones, frecuencia de snapshots y algoritmo de deltas.
- Estrategia predeterminada de identificador público: token aleatorio con mapeo o token cifrado reversible.
- Algoritmos, límites y administración de claves para cifrado en reposo.
- Serialización documental, normalización y hashing canónico.
- Semántica de índices derivados y cómo declarar su frescura.
- Semántica de eliminación y retención frente a revisiones, backups y derivados.
- API pública estable, semver y política de deprecación.

Cada tema deberá resolverse en un Architecture Decision Record (ADR) antes de que sus supuestos queden codificados de forma difícil de revertir.

## 3. Principios de arquitectura

1. **Correctitud antes que velocidad.** Un benchmark alto no compensa pérdida de datos, commits ambiguos o índices incoherentes.
2. **Núcleo pequeño.** Las funcionalidades avanzadas se implementan en capas separadas.
3. **Una fuente de verdad.** Los índices, embeddings y otros derivados nunca sustituyen al contenido fuente.
4. **Recuperación explícita.** Cada estado persistente debe tener reglas de recuperación, validación y diagnóstico.
5. **Historia con semántica.** WAL, revisiones y auditoría son mecanismos distintos, con ciclos de vida diferentes.
6. **Construcción reproducible.** Formatos, hashes, migraciones y pruebas deben producir resultados deterministas donde corresponda.
7. **Seguridad sin criptografía casera.** Se utilizan algoritmos y bibliotecas consolidados; Kerno define la integración y el manejo de claves.
8. **Medir antes de especializar.** HNSW, LSM, B+ tree, grafo u otras estructuras se justifican por workloads y métricas.
9. **Portable y exportable.** El usuario no debe quedar cautivo del formato físico interno.
10. **Políticas explícitas.** Retención, auditoría, consistencia eventual y exposición de identificadores tienen configuración y límites documentados.

## 4. Modelo de despliegue y concurrencia

### 4.1 Despliegue inicial

Kerno se ejecutará embebido en el proceso consumidor y expondrá una biblioteca Rust. Una CLI local permitirá crear, inspeccionar, validar, exportar y diagnosticar bases. No habrá un servidor ni autenticación de red en el MVP.

La biblioteca debe ofrecer APIs claras para abrir/cerrar la base, iniciar transacciones, leer/escribir, consultar estado y exportar datos. La CLI utilizará la misma biblioteca pública, no una segunda implementación del motor.

### 4.2 Concurrencia acordada

- Un escritor activo por base de datos.
- Varios lectores concurrentes dentro del mismo proceso.
- Lecturas sobre un snapshot coherente según el modelo de aislamiento que se decida en ADR.
- Detección de doble apertura incompatible del mismo directorio/archivo.
- Ninguna promesa inicial de escritura multi-proceso, servidor remoto o replicación.

El modelo interno puede usar MVCC, copy-on-write, locks de lectura o una combinación; debe resolverse de forma consistente con el motor físico escogido. No debe inferirse que «un escritor» elimina la necesidad de atomicidad, aislamiento o manejo de cierres inesperados.

## 5. Modelo lógico de datos

### 5.1 Entidades principales

- **Database:** conjunto de archivos, metadatos y configuración de una instancia.
- **Namespace/Collection:** agrupación lógica para claves, documentos y políticas.
- **Record:** pareja clave/valor con metadatos de identidad y revisión.
- **Document:** representación estructurada encima del valor; JSON será el primer formato conveniente, pero no debe confundirse con el formato físico.
- **Revision:** versión lógica de un registro.
- **ChangeSet:** agrupación de cambios que pertenecen a una operación lógica.
- **IndexDefinition:** definición versionada de un índice.
- **DerivedArtifact:** embedding, fragmento, representación de búsqueda u otro dato calculado a partir de una revisión fuente.

### 5.2 Identidad

Cada registro tendrá un `internal_id` basado en UUIDv7. UUIDv7 ofrece orden temporal aproximado, pero no será el orden transaccional de Kerno: el orden de commits se representará mediante una secuencia monotónica asignada por la base.

La representación pública será intercambiable mediante una abstracción `IdentifierCodec`. Las opciones iniciales a evaluar son:

- token aleatorio opaco persistido y mapeado a `internal_id`;
- token cifrado autenticado y versionado que permita recuperar el ID interno.

La primera opción añade una correspondencia almacenada; la segunda añade complejidad de gestión criptográfica y rotación de claves. El token será estable para cada registro. No se cifrarán selectivamente bits del UUIDv7 ni se inventará una transformación propia. Un identificador opaco no sustituye la autorización.

### 5.3 Esquemas

La primera versión permitirá documentos estructurados sin exigir un esquema rígido global. Más adelante podrán definirse validación por colección, tipos de campos, índices declarativos y versiones de esquema. Las migraciones de esquema estarán separadas de las migraciones del formato físico.

## 6. Arquitectura lógica de módulos

| Módulo | Responsabilidad | Dependencias prohibidas o límites |
|---|---|---|
| `kerno-core` | tipos fundamentales, errores, configuración y contratos | no depender de proveedor de IA ni de CLI |
| `kerno-storage` | páginas/registros, archivos, formato físico, lectura/escritura | no decidir políticas de negocio |
| `kerno-transactions` | commit, aislamiento, writer coordination y recuperación | no depender de embeddings |
| `kerno-documents` | serialización y consultas documentales | no ser el único formato de almacenamiento físico |
| `kerno-revisions` | historial, deltas, snapshots y restauración | independiente del WAL de recuperación |
| `kerno-indexes` | índice primario, índices secundarios y estado de reconstrucción | los índices deben ser reconstruibles desde datos fuente |
| `kerno-security` | interfaces de claves, cifrado, identificadores públicos y políticas | no implementar algoritmos criptográficos propios |
| `kerno-embeddings` | contratos de adaptador, versiones y jobs derivados | sin dependencias obligatorias de red/modelo en el core |
| `kerno-cli` | administración, diagnóstico, exportación e inspección | reutilizar la API de la biblioteca |

Esta estructura es lógica, no una obligación de crear nueve crates desde el primer día. Se recomienda comenzar con pocos crates y separarlos cuando sus contratos sean estables.

### 6.1 Flujo de consulta

1. La aplicación llama a la API con una clave interna o identificador público.
2. La capa pública valida y resuelve el identificador, cuando corresponda.
3. La transacción establece el snapshot de lectura.
4. El motor consulta el índice primario y recupera el registro.
5. Las capas documental o de búsqueda aplican filtros y transformaciones requeridas.
6. La salida aplica la política de representación y evita exponer metadatos internos innecesarios.

### 6.2 Flujo de escritura

1. Validar la operación, límites y política aplicable.
2. Serializar el valor y calcular los metadatos de integridad.
3. Registrar la modificación en el mecanismo transaccional definido por el ADR de almacenamiento.
4. Mantener atomicidad entre contenido, revisión y estructuras transaccionales críticas.
5. Confirmar de acuerdo con la política de durabilidad configurada.
6. Marcar índices derivados asíncronos como pendientes, si procede, y exponer su watermark/estado.

No se confirma una escritura antes de cumplir la garantía de durabilidad prometida por la configuración. La documentación distinguirá, por ejemplo, confirmación con sincronización forzada y modos menos estrictos, si llegan a ofrecerse.

## 7. Almacenamiento físico, transacciones y recuperación

### 7.1 Alternativas principales

**Alternativa A: B+ tree copy-on-write.** Potencialmente atractiva para una base embebida generalista, rangos, snapshots y consultas de estado. Deben estudiarse fragmentación, amplificación de escritura, gestión de páginas y recuperación de raíces/commits.

**Alternativa B: LSM tree.** Potencialmente atractiva para ingestión y escrituras sostenidas. Introduce gestión de memtables, archivos ordenados, tombstones, compactación y amplificación de lectura/escritura.

La selección queda pendiente de ADR-0001 y no debe basarse solo en popularidad o en resultados publicados por terceros.

### 7.2 Recuperación transaccional

El diseño tendrá que identificar inequívocamente el último commit válido, detectar registros truncados y recuperar un estado coherente. La implementación puede utilizar WAL o un protocolo transaccional apropiado al motor elegido; el concepto de «WAL» no se impondrá si la arquitectura finalmente usa un mecanismo diferente.

Se definirán expresamente:

- qué significa que una transacción está confirmada;
- cuándo se sincroniza el archivo y qué devuelve el API si falla la sincronización;
- cómo se detectan escrituras parciales;
- cómo se resuelven cierres antes, durante y después de commit;
- cómo se recupera una actualización interrumpida del formato;
- qué herramienta permite verificar o exportar una base dañada sin sobrescribirla automáticamente.

### 7.3 Journal, historial y auditoría

| Sistema | Propósito | Retención |
|---|---|---|
| Journal de recuperación | atomicidad y recuperación tras fallos | solo el tiempo requerido por el protocolo, salvo otra necesidad documentada |
| Revision Engine | reconstruir y comparar estados históricos | configurable según política |
| Auditoría | registrar operaciones y contexto de seguridad | configurable y minimizada |

Estos sistemas pueden compartir secuencias de commit o metadatos, pero no compartirán implícitamente la misma política de retención.

## 8. Revision Engine: historial, diferencias y procedencia

### 8.1 Objetivo

Permitir reconstruir versiones anteriores de los datos, comparar dos revisiones y registrar el contexto de cambios sin convertir el journal de recuperación en un historial permanente.

### 8.2 Modelo de revisión

Una revisión podrá incluir:

- `revision_id` y `record_id`;
- `parent_revision` o referencia equivalente;
- `commit_sequence`;
- hash del contenido canónico, cuando sea aplicable;
- versión de esquema;
- referencia a snapshot o delta;
- tipo de cambio: creación, modificación, borrado lógico o restauración;
- actor, fuente, razón o referencia a tarea, si están disponibles y permitidos por la política.

La ausencia de actor o razón debe ser válida; no se inventará contexto.

### 8.3 Almacenamiento de revisiones

Se evaluará un modelo híbrido de snapshots completos y deltas estructurales. Se fijarán límites de profundidad y tamaño para evitar una cadena de deltas demasiado costosa. Los snapshots podrán reconstruirse/compactarse sin alterar la semántica del historial.

Restaurar una revisión producirá una nueva revisión equivalente al contenido histórico seleccionado; no borrará silenciosamente revisiones posteriores.

### 8.4 Políticas de historial

- **none:** no conserva versiones lógicas; se mantienen las garantías necesarias para recuperación transaccional.
- **audit:** conserva eventos/metadatos definidos, no necesariamente el contenido completo anterior.
- **versioned:** conserva revisiones consultables y comparables dentro de la retención configurada.

El ciclo de vida del historial debe considerar datos personales, backups, índices, embeddings, cachés y snapshots. No se promete inmutabilidad ilimitada ni que toda eliminación pueda reflejarse instantáneamente en cada soporte secundario.

## 9. Índices y motor de consultas

### 9.1 Secuencia de implementación

| Orden | Estructura | Caso principal | Condición de entrada |
|---|---|---|---|
| 1 | índice primario | búsqueda por clave/ID | MVP |
| 2 | índices secundarios ordenados | igualdad, rangos, ordenación y filtros | tras validar transacciones y almacenamiento |
| 3 | índices compuestos | combinaciones de predicados frecuentes | respaldado por benchmarks |
| 4 | índice invertido con ranking textual | consultas por términos y relevancia | fase de búsqueda |
| 5 | búsqueda vectorial exacta | referencia de similitud y pruebas de recall | primera extensión IA |
| 6 | HNSW u otro ANN | baja latencia a escala | solo tras medir el coste de búsqueda exacta |
| 7 | relaciones/grafo especializado | recorridos de relaciones | solo si workloads lo justifican |
| — | almacenamiento columnar | agregaciones analíticas masivas | fuera de la dirección inicial |

No se crearán todos los índices automáticamente. Cada índice consume disco, memoria y trabajo de mantenimiento; la definición será explícita y, a largo plazo, podrá informar al optimizador de consultas.

### 9.2 Consistencia de índices

Los índices necesarios para que una transacción se observe coherentemente se actualizarán en la misma frontera atómica. Los índices costosos y reconstruibles podrán ser derivados y asíncronos, pero tendrán un estado visible (`ready`, `building`, `stale`, `failed`) y una secuencia de sincronización.

Una consulta debe declarar o heredar la política de frescura. Si el índice está atrasado, Kerno debe aplicar una estrategia documentada, como esperar, consultar una ruta exacta alternativa o devolver un estado de no disponibilidad para esa consulta; no debe fingir frescura.

### 9.3 API de consultas

El MVP tendrá API tipada para claves y operaciones de documentos. SQL no entra en el alcance inicial. Se diseñarán desde el principio límites de consulta, paginación por cursor, cancelación y errores tipados. Un futuro `EXPLAIN` mostrará índices usados, filtros, costes observados y si intervino una estructura derivada.

## 10. Embeddings y recuperación híbrida para IA

### 10.1 Separación de responsabilidades

Kerno almacenará vectores y ofrecerá contratos de búsqueda; no descargará modelos, no exigirá acceso a internet y no generará embeddings en el núcleo. Un adaptador opcional permitirá proveedores locales o externos.

### 10.2 Metadatos de un embedding

Como mínimo: ID del documento, revisión fuente exacta, perfil de embedding, proveedor/modelo y versión, dimensiones, tipo numérico, métrica de distancia, perfil de fragmentación, estado y marca de sincronización.

### 10.3 Ciclo de vida

- **Cambio de documento:** marcar el embedding anterior como obsoleto.
- **Cambio de modelo o fragmentación:** crear un nuevo perfil y permitir reconstrucción gradual.
- **Fallo del proveedor:** conservar la fuente, marcar el trabajo fallido y permitir reintentos idempotentes.
- **Borrado:** aplicar la política también a vectores, chunks, índices y colas pendientes.
- **Cambio de índice:** poder reconstruirlo desde embeddings fuente que sigan siendo válidos.

### 10.4 Búsqueda híbrida

La evolución prevista es búsqueda textual (p. ej. BM25) + búsqueda vectorial + filtros estructurados. La fusión de resultados (p. ej. Reciprocal Rank Fusion) será una opción de capa de consulta, no una obligación del almacenamiento físico. Primero se implementará búsqueda vectorial exacta como baseline; los índices aproximados deberán medirse contra ella en recall, latencia, memoria y disco.

## 11. Seguridad, privacidad y ciclo de vida

### 11.1 Límites de responsabilidad

La biblioteca embebida ofrece mecanismos y contratos; la aplicación anfitriona sigue siendo responsable de la identidad de sus usuarios, la autorización de negocio y la base jurídica para tratar datos. Un futuro servidor Kerno necesitará controles de autenticación, TLS, aislamiento de tenants y administración propios.

### 11.2 Identificadores públicos

`IdentifierCodec` será intercambiable. El token público será estable, versionado y validado criptográficamente si se elige un formato cifrado reversible. La gestión de claves y la rotación se definirán antes de publicar ese formato. No se asumirá que la dificultad para adivinar un ID concede autorización.

### 11.3 Cifrado en reposo

Se utilizarán bibliotecas criptográficas consolidadas y cifrado autenticado cuando corresponda. Las claves maestras no se guardarán junto a los datos cifrados. Se definirá qué archivos quedan protegidos: datos, journal, snapshots, temporales y backups. El borrado criptográfico se considerará una estrategia dependiente del diseño de claves, no una promesa universal.

### 11.4 Auditoría

Los eventos podrán registrar actor/referencia, operación, recurso, resultado, timestamp, secuencia y correlación. No se registrará automáticamente el contenido completo de documentos, secretos, tokens ni payloads personales. La auditoría tendrá límites, redacción y política de retención propios.

### 11.5 Privacidad por diseño

Se diseñarán mecanismos para exportación, rectificación, supresión, clasificación y retención. El motor facilitará aplicar políticas, pero no decidirá automáticamente la base legal, la finalidad o las excepciones de conservación. Los perfiles normativos mapearán requisitos a controles y pruebas; no equivaldrán a certificación o cumplimiento jurídico automático.

## 12. Backups, diagnóstico y operación local

Kerno debe ofrecer utilidades para:

- crear backups consistentes sin copiar archivos en un estado ambiguo;
- verificar integridad y compatibilidad de versión;
- restaurar a un directorio separado y validar el resultado;
- exportar datos a un formato lógico documentado;
- inspeccionar versiones del formato y migraciones pendientes;
- mostrar estadísticas de archivos, índices, revisiones y espacio;
- inspeccionar el estado de índices derivados y reconstruirlos;
- diagnosticar daños sin ejecutar reparación destructiva de forma automática.

Una restauración de backup será una operación probada, no una promesa basada únicamente en la existencia de un archivo de copia.

## 13. Esquemas, formato y compatibilidad

Kerno tendrá tres ejes de versión separados:

1. **Formato físico:** páginas, registros, cabeceras y commits.
2. **Esquema lógico:** estructura y restricciones de los documentos.
3. **Índices/datos derivados:** algoritmos, versiones y parámetros de construcción.

Toda migración tendrá precondiciones, pasos de ejecución, validación posterior y una ruta documentada de restauración desde un backup. Se definirá qué versiones puede abrir cada release y cuándo requiere migración explícita.

Los hashes del contenido necesitan una serialización canónica cuando deban ser comparables entre ejecuciones o versiones. Para JSON se evaluará el uso de RFC 8785 (JCS), teniendo en cuenta sus restricciones sobre tipos numéricos y Unicode; no se aplicará una normalización ad hoc silenciosa.

La exportación lógica tendrá un formato versionado e independiente de las páginas internas. El usuario podrá migrar datos sin tener que entender el árbol físico, los segmentos LSM o la codificación privada del motor.

## 14. Organización propuesta del repositorio

```text
kerno/
├── Cargo.toml
├── crates/
│   ├── kerno-core/
│   ├── kerno-storage/
│   ├── kerno-transactions/
│   ├── kerno-documents/
│   ├── kerno-revisions/
│   ├── kerno-indexes/
│   ├── kerno-security/
│   ├── kerno-embeddings/     # opcional en etapa inicial
│   └── kerno-cli/
├── docs/
│   ├── architecture/
│   ├── adr/
│   ├── file-format/
│   ├── security/
│   ├── testing/
│   └── roadmap/
├── tests/
│   ├── recovery/
│   ├── transactions/
│   ├── revisions/
│   ├── corruption/
│   └── workloads/
└── benchmarks/
```

La estructura final de crates se revisará después de que los contratos se estabilicen. El primer prototipo debe preferir simplicidad y pocas dependencias, no una fragmentación prematura.

## 15. Architecture Decision Records (ADR)

Los ADR describen decisiones duraderas y sus consecuencias. Los siguientes son propuestos; su estado inicial es *Propuesto / pendiente de aprobación*, excepto los puntos explícitamente confirmados en la sección 2. El estado vigente de cada uno se mantiene en [`docs/adr/`](../adr/README.md).

**ADR-0001 — Motor de almacenamiento físico.** Estado: Pendiente.
Pregunta: ¿B+ tree copy-on-write o LSM?
Opciones: implementación propia de B+ tree copy-on-write; implementación propia de LSM; evaluar una tercera opción solo si la evidencia la justifica.
Propuesta: comparar un prototipo de cada enfoque con workloads iguales, incluyendo punto, rango, actualización, historial y recuperación.
Criterios: correctitud, latencia p50/p95/p99, throughput, tiempo de recuperación, RAM máxima, bytes escritos/tamaño de datos y complejidad operacional. No fijar un ganador por benchmarks externos.
Consecuencia: esta decisión debe cerrarse antes de congelar el formato físico.

**ADR-0002 — Modelo de transacciones y durabilidad.** Estado: Pendiente.
Pregunta: ¿qué protocolo define commit, sincronización, snapshots y recuperación?
Opciones: WAL con protocolo de commit; copy-on-write con raíz/manifest de commit; variante apropiada al motor seleccionado.
Criterio de aprobación: matriz de fallos que define cada punto de interrupción y el estado esperado después de reabrir. La API debe documentar qué garantías ofrece cada modo de durabilidad.

**ADR-0003 — Formato físico y versionado.** Estado: Pendiente.
Pregunta: ¿cómo se versionan cabeceras, páginas, registros, checksums y commits?
Propuesta: versionado explícito desde la primera base; rechazo seguro de versiones no compatibles; exportación lógica independiente.
Criterios: detección de truncamientos/corrupción, compatibilidad probada y migración con interrupción simulada.

**ADR-0004 — Historial de revisiones.** Estado: Pendiente.
Pregunta: ¿qué representa una revisión y cuándo usar snapshot/delta?
Propuesta: Revision Engine separado del journal; ChangeSet opcional; snapshot + delta con límite de profundidad; restauración que crea una nueva revisión.
Criterios: reconstrucción exacta, diff determinista y retención configurable.

**ADR-0005 — Identificadores públicos.** Estado: Pendiente.
Pregunta: token aleatorio con mapeo o token cifrado reversible.
Propuesta: abstracción `IdentifierCodec`; comparar el coste de persistir una correspondencia frente a la gestión de claves, rotación, longitud y validación.
Criterios: estabilidad del token, rechazo de manipulación, soporte de versionado y claridad de la separación entre identificación y autorización.

**ADR-0006 — Cifrado y gestión de claves.** Estado: Pendiente.
Pregunta: alcance del cifrado de archivos y forma de administrar claves.
Propuesta: algoritmos estándar y biblioteca auditada; claves fuera del directorio de datos; interfaz de proveedor de claves y estrategia de rotación definida.
Criterios: datos, journals, temporales y backups cubiertos; pérdida de clave documentada; pruebas de integridad y rotación.

**ADR-0007 — Consistencia y reconstrucción de índices.** Estado: Pendiente.
Pregunta: qué índices participan en transacciones y cuáles son derivados.
Propuesta: índice primario consistente con el commit; índices secundarios críticos con frontera atómica clara; índices costosos con estado de frescura y reconstrucción.
Criterios: los índices reconstruidos producen resultados equivalentes a un índice nuevo sobre los mismos datos.

**ADR-0008 — Serialización documental y hashes.** Estado: Pendiente.
Pregunta: JSON inicial, soporte binario y canonicalización para hashing.
Propuesta: separar formato lógico y físico; evaluar RFC 8785 para JSON cuando sus restricciones sean aceptables; versionar el algoritmo de hash.
Criterios: resultados reproducibles en los casos soportados y pruebas de compatibilidad.

**ADR-0009 — Supresión, retención y backups.** Estado: Pendiente.
Pregunta: cómo coordinar borrado lógico/físico, revisiones, índices, embeddings y backups.
Propuesta: políticas explícitas por colección; operaciones de supresión conscientes de datos derivados; documentación de ventanas de expiración de backups.
Criterios: una prueba de extremo a extremo demuestra qué se elimina inmediatamente, qué queda sujeto a compactación y qué plazo siguen los backups.

**ADR-0010 — API pública y límites de crates.** Estado: Pendiente.
Pregunta: qué tipos y errores son públicos y cuáles módulos son internos.
Propuesta: API pequeña, tipada y estable; no hacer públicos los detalles de páginas o del motor físico; módulos separados solo cuando sus contratos estén claros.
Criterios: ejemplo de aplicación externa que compila; guía de errores; política de deprecación y semver.

**ADR-0011 — Perfil de vector y recuperación híbrida.** Estado: Pendiente.
Pregunta: qué contratos comunes definen embeddings, métricas, filtros y frescura de índices.
Propuesta: vectores exactos primero; metadatos de modelo y revisión fuente; generación mediante adaptadores; ANN solo con evidencia.
Criterios: comparación con baseline exacto, verificación de dimensiones/métrica y prueba de invalidación tras actualizar/borrar el documento.

**ADR-0012 — Metodología de benchmarks y umbrales de salida.** Estado: Pendiente.
Pregunta: cómo medir sin declarar éxito a partir de una única prueba sintética.
Propuesta: datasets y scripts reproducibles; misma máquina, mismos datos y durabilidad equivalente; reportar p50/p95/p99, throughput, RAM, disco y recuperación.
Criterios: resultados repetibles y publicación de entorno, versión, configuración y limitaciones.

## 16. Plan de implementación

### Fase 0 — Spike arquitectónico y decisiones (1–2 semanas)

Objetivo: resolver la incertidumbre de almacenamiento antes de congelar el formato.

Entregables:

- ADR-0001 con decisión documentada o recomendación explícita de cambiar el enfoque.
- prototipos pequeños y comparables para B+ tree copy-on-write y LSM, sin pretender que sean motores de producción;
- harness de benchmarks y datasets sintéticos/representativos;
- matriz de fallos y garantías de commit propuestas;
- ADR iniciales de persistencia, transacciones, formato e identidad;
- lista de invariantes y riesgos.

Salida: decisión revisada por evidencia. Si el prototipo no justifica complejidad propia, registrar un no-go temporal y reconsiderar el alcance antes de escribir el motor completo.

### Fase 1 — Núcleo persistente (3–5 semanas, estimación inicial)

- apertura/cierre y bloqueo compatible con un proceso;
- almacenamiento por clave/valor;
- commits y recuperación;
- comprobación de integridad;
- pruebas de corrupción y fallos.

Salida: motor experimental que demuestra invariantes de persistencia y pasa la matriz inicial de recuperación.

### Fase 2 — Transacciones y documentos (3–5 semanas)

- API tipada y operaciones por lote;
- documento estructurado sobre key-value;
- aislamiento de lecturas concurrentes;
- índice primario y filtros básicos;
- CLI inicial.

Salida: biblioteca local que un proyecto de prueba externo puede usar sin acceder a detalles internos.

### Fase 3 — Versionado (3–5 semanas)

- Revision Engine, ChangeSet, snapshots y deltas;
- history/diff/restore;
- políticas none/audit/versioned;
- límites de reconstrucción y retención.

Salida: historial reproducible con atomicidad documentada respecto al registro actual.

### Fase 4 — Seguridad y ciclo de vida (3–5 semanas)

- `IdentifierCodec` y política inicial de IDs públicos;
- cifrado y proveedor de claves;
- auditoría minimizada;
- exportación, supresión y backups;
- migraciones y diagnóstico.

Salida: revisión de seguridad y pruebas de ciclo de vida completadas para el alcance embebido.

### Fase 5 — Búsqueda e IA (4–6 semanas)

- índices secundarios;
- búsqueda textual;
- vectores exactos y adaptador de embeddings;
- actualización/invalidez de derivados;
- ranking híbrido inicial.

Salida: benchmark de recuperación híbrida y pruebas de frescura/supresión.

### Fase 6 — Beta endurecida (4–8 semanas)

- fuzzing y pruebas extendidas de fallos;
- compatibilidad de formato/migraciones;
- documentación de operación y recuperación;
- benchmarks comparativos con SQLite y motores de referencia;
- pruebas con workloads de LiveSpec, Cauce y memoria de agentes.

Salida: beta documentada con limitaciones, matriz de compatibilidad y criterios explícitos de uso recomendado.

Las estimaciones son rangos iniciales para una persona con desarrollo regular. No son garantías de calendario. Un prototipo básico puede aparecer antes; una beta segura y fiable puede requerir seis a doce meses o más, y la preparación para sistemas críticos requiere validación adicional.

## 17. Primer hito: M0 — decisión de almacenamiento basada en evidencia

### 17.1 Objetivo

No escribir aún el motor completo. Elegir el enfoque de almacenamiento y registrar las garantías que la implementación deberá satisfacer.

### 17.2 Alcance

- Prototipo mínimo de acceso por clave y escritura persistente para cada alternativa candidata.
- Benchmarks de lecturas/escrituras por clave, actualizaciones, escaneos por rango y patrón mixto.
- Estimación de RAM, tamaño en disco y tiempo de reapertura/recuperación.
- Prototipo de versionado de cambios a nivel de diseño, sin construir todavía el Revision Engine completo.
- Pruebas de interrupciones simuladas en puntos clave del protocolo candidato.
- Comparación con una alternativa existente pertinente como referencia, registrando que no se compararán cifras si las garantías de durabilidad son distintas.

### 17.3 Workloads mínimos

1. **Punto:** insert, get, update y delete con claves aleatorias.
2. **Append-heavy:** inserción secuencial y sucesivas actualizaciones.
3. **Mixto:** predominio de lecturas y fracción de escrituras definida en el benchmark.
4. **Rango:** lectura ordenada de intervalos de claves.
5. **Reapertura:** tiempo para reabrir y reconstruir el índice/estado.
6. **Historial simulado:** varias revisiones por registro para estimar el coste futuro del versionado.

Las pruebas deben incluir datasets que quepan en memoria y datasets mayores si el entorno lo permite. El tamaño exacto se registra como parámetro, no como una condición universal.

### 17.4 Método de medición

- Misma máquina, sistema operativo, compilación y configuración para cada candidato.
- Separar calentamiento de mediciones y ejecutar varias repeticiones.
- Reportar throughput, latencias p50/p95/p99, RAM máxima, tamaño en disco y tiempo de recuperación.
- Registrar versiones, flags, datasets, configuración de sync y límites conocidos.
- No comparar durabilidad fuerte de una opción con durabilidad relajada de otra como si fueran equivalentes.
- No establecer un umbral de velocidad antes de medir el baseline; definir objetivos después de conocer los resultados.

### 17.5 Criterios de aceptación

M0 se considera terminado únicamente si:

1. ADR-0001 selecciona un enfoque o documenta por qué ninguna alternativa debe implementarse todavía.
2. Se publica un informe reproducible con código, comandos, configuración y resultados.
3. Existe una matriz de puntos de fallo y estado esperado tras recuperación.
4. Se documenta el significado exacto de commit y durabilidad propuestos.
5. Las opciones se comparan también por complejidad, recuperación, compactación, rangos y compatibilidad futura con Revision Engine.
6. Se identifican las decisiones que dependen de la elección del almacenamiento y cuáles siguen desacopladas.
7. No hay una dependencia funcional obligatoria de IA, red o proveedor externo.

### 17.6 Pruebas mínimas de M0

- La misma secuencia de operaciones produce el mismo estado lógico en cada candidato.
- Insert/update/delete/reopen recupera el estado esperado en ejecuciones normales.
- El prototipo detecta o rechaza entradas/archivos truncados según el diseño ensayado.
- Las pruebas de fallo se ejecutan en puntos predefinidos y se comparan con la matriz esperada.
- Los benchmarks se pueden repetir mediante un único comando documentado.
- Los resultados incluyen al menos una medición de rango, una de recuperación y una de memoria/disco.

M0 no certifica la corrección del futuro motor; reduce el riesgo de una decisión arquitectónica temprana y deja los primeros contratos listos para implementarse.

## 18. Calidad, pruebas y definición de terminado

### 18.1 Pirámide de pruebas

- Unitarias de serialización, claves, índices y políticas.
- Pruebas de propiedades sobre secuencias aleatorias de operaciones.
- Pruebas de integración de transacciones y recuperación.
- Crash testing: interrupción de procesos en puntos de escritura y commit.
- Fuzzing de entradas, parser documental y formato binario.
- Tests de compatibilidad/migración desde versiones previas.
- Benchmarks con workloads definidos y resultados versionados.

### 18.2 Definition of Done

Una funcionalidad no se considera terminada hasta que:

- el contrato de API y errores esté documentado;
- se hayan definido los invariantes afectados;
- las pruebas normales y de fallo relevantes pasen;
- se hayan evaluado efectos sobre formato, migración, historial, índices y backups;
- se hayan evaluado consecuencias para seguridad y privacidad;
- las métricas de rendimiento estén disponibles cuando corresponda;
- los ADR relacionados se actualicen;
- la CLI/documentación muestren el comportamiento real, sin garantías no probadas.

### 18.3 Release gates

No publicar una versión beta si existe una pérdida reproducible de una operación confirmada bajo la configuración de durabilidad prometida, un camino de recuperación no probado, una migración destructiva sin backup validado o una discrepancia conocida entre índices e información fuente que se oculte al consumidor.

## 19. Riesgos y mitigaciones

| Riesgo | Consecuencia | Mitigación |
|---|---|---|
| Alcance excesivo | No terminar el núcleo | MVP acotado y extensiones por evidencia |
| Elegir mal B+ tree/LSM | Rehacer formato y recuperación | M0 con ADR y prototipos comparables |
| Historial demasiado costoso | Alto uso de disco o lecturas lentas | snapshots, límites de deltas, retención y benchmarks |
| Índices derivados obsoletos | Resultados incorrectos | estado explícito, watermark y reconstrucción |
| Cifrado mal diseñado | Pérdida de datos o falsa seguridad | bibliotecas consolidadas, gestión de claves y revisión específica |
| Borrado incompleto | Retención no intencional | política que abarque revisiones, índices y backups |
| Formato sin versionado | Migraciones peligrosas | versión desde la primera base y exportación lógica |
| Benchmarks poco representativos | Decisiones equivocadas | workloads de LiveSpec, Cauce y memoria de agentes |
| API inestable | Coste de mantenimiento | superficie pública pequeña, semver y ADR |
| Promesas legales excesivas | Riesgo contractual y reputacional | mapear controles a requisitos; revisión jurídica del caso de uso |

## 20. Decisiones y tareas inmediatas

El siguiente orden de trabajo será:

1. Crear el repositorio, la licencia elegida y las convenciones de contribución; no copiar implementación de otros proyectos sin revisar licencias.
2. Añadir este documento como baseline de arquitectura y crear la carpeta `docs/adr/`.
3. Escribir ADR-0001 y ADR-0002 como borradores con hipótesis y criterios de decisión.
4. Construir el harness de pruebas y benchmark de M0.
5. Ejecutar el spike B+ tree frente a LSM y registrar los resultados reproducibles.
6. Aprobar el formato y la semántica de commit antes de crear bases de datos de usuarios.
7. Empezar la implementación del núcleo persistente únicamente después de cerrar M0.

**Fuera de la primera tanda de commits.** No empezar por HNSW, embeddings, SQL, un lenguaje de grafos, un servidor de red ni un sistema de agentes. Tampoco comprometer compatibilidad del formato antes de que existan pruebas de apertura, recuperación y migración.

## 21. Referencias técnicas

Las referencias siguientes son bases de estudio; no sustituyen los ADR ni obligan a copiar sus arquitecturas.

1. RFC 9562 — Universally Unique IDentifiers (UUIDs). Define UUIDv7 y su campo temporal. <https://www.rfc-editor.org/rfc/rfc9562.html>
2. RFC 8785 — JSON Canonicalization Scheme (JCS). Referencia para una representación JSON determinista en usos de hashing/firma, sujeta a las limitaciones del estándar. <https://www.rfc-editor.org/rfc/rfc8785.html>
3. redb — Rust embedded database. Referencia de arquitectura para base embebida, árboles B+ copy-on-write y transacciones. <https://www.redb.org/>
4. RocksDB Wiki — Overview / Architecture. Referencia para el enfoque LSM, SSTables, memtables y compactación. <https://github.com/facebook/rocksdb/wiki/RocksDB-Overview>
5. NIST SP 800-88 Rev. 2 — Guidelines for Media Sanitization. Referencia para políticas de sanitización y borrado criptográfico, según aplicabilidad. <https://csrc.nist.gov/pubs/sp/800/88/r2/final>
6. OWASP Application Security Verification Standard (ASVS). Catálogo de requisitos verificables para aplicaciones y servicios que puedan construirse sobre Kerno. <https://asvs.owasp.org/>

## Apéndice A. Glosario

- **ADR:** registro breve de una decisión de arquitectura, su contexto y consecuencias.
- **ANN:** búsqueda aproximada de vecinos más cercanos.
- **BM25:** función de ranking para recuperación textual.
- **ChangeSet:** conjunto de cambios que forman una operación lógica.
- **Delta:** representación de diferencias respecto de una revisión anterior.
- **Embedding:** vector generado por un modelo a partir de un contenido.
- **LSM tree:** estructura que organiza escrituras en memoria y segmentos ordenados en disco, con compactación.
- **MVCC:** control de concurrencia que permite mantener versiones visibles para distintos snapshots de lectura.
- **Snapshot:** representación estable de un estado en un punto determinado.
- **WAL:** journal que registra operaciones antes de aplicar/confirmar cambios, según un protocolo de recuperación.
- **Watermark:** secuencia que identifica hasta qué commit o revisión una estructura derivada está actualizada.
