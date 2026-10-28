# KernoDB --- Especificación arquitectónica y guía para Claude Code

**Versión:** 0.2.0\
**Estado:** propuesta de diseño; validar contra el repositorio real
antes de implementar.\
**Propósito:** entregar a Claude Code el contexto del producto, los
principios arquitectónicos y un plan incremental de trabajo.

## 1. Instrucciones obligatorias para Claude Code

No asumas que este documento describe funcionalidades ya implementadas.
Primero inspecciona el repositorio, instrucciones locales, estado de
Git, código, pruebas, dependencias y documentación. No sobrescribas
cambios del usuario ni hagas una reescritura masiva.

Si están disponibles, revisa también estos repositorios de referencia y
reutiliza ideas solo cuando exista una justificación técnica: -
https://github.com/Rixmerz/livespec - https://github.com/Rixmerz/cauce

Entrega primero un diagnóstico con evidencia del repositorio. Distingue
hechos, hipótesis y propuestas. Después propone un plan de tareas
pequeñas con pruebas y criterios de aceptación. Implementa la primera
tarea vertical segura si el entorno permite avanzar; si requiere
aprobación, espera. No afirmes que una función existe sin verificarla y
probarla.

Prioridades, en orden: 1. Integridad y recuperación de datos. 2.
Corrección transaccional y consistencia de índices. 3. Simplicidad y
capacidad de prueba. 4. Trazabilidad y observabilidad. 5. Rendimiento
medido. 6. Adaptación basada en evidencia. 7. Funciones avanzadas de IA.

No dependas de un LLM para operaciones básicas de almacenamiento,
transacciones ni recuperación ante fallos. El sistema debe ser correcto
aunque no haya un modelo de IA disponible.

## 2. Visión del producto

KernoDB busca ser un motor de datos persistente, evolutivo y adaptativo
inspirado en principios funcionales del cerebro. No pretende replicar
literalmente la anatomía cerebral ni afirmar equivalencias biológicas.

**Propuesta de valor:** un motor de datos que conserva la evolución de
la información, representa conexiones entre unidades de conocimiento y
aprende a ejecutar consultas frecuentes de manera más eficiente sin
sacrificar corrección, seguridad ni trazabilidad.

La propuesta debe validarse frente a alternativas consolidadas. Integrar
historial, relaciones, búsqueda semántica y adaptación no es
automáticamente mejor que usar SQLite u otro motor maduro.

### Principios no negociables

-   Los datos confirmados deben sobrevivir a los fallos cubiertos por
    las garantías documentadas.
-   Una optimización no puede cambiar silenciosamente el significado de
    una consulta.
-   El aprendizaje no puede saltarse integridad, autorización ni
    transacciones.
-   La adaptación debe poder inspeccionarse, medirse, desactivarse y
    revertirse.
-   Debe existir una ruta de ejecución correcta aunque no se haya
    aprendido ninguna optimización.
-   Las lecturas no mutan datos por sorpresa.
-   Formato persistente y migraciones versionados.
-   Retención y eliminación consideran revisiones, índices, cachés,
    vectores, logs y backups.
-   Cada mecanismo inspirado en el cerebro necesita un caso de uso y una
    prueba de valor.

## 3. Modelo cerebral como inspiración

La metáfora ayuda a diseñar especialización, conexiones, selección de
rutas, memoria y aprendizaje. No obliga a crear un motor por lóbulo ni
una tabla física por neurona.

  -------------------------------------------------------------------------
  Inspiración funcional     Abstracción en KernoDB  Responsabilidad
  ------------------------- ----------------------- -----------------------
  Función ejecutiva frontal Planificador/ejecutor   Elegir una estrategia
                                                    válida para una
                                                    consulta

  Memoria y asociación      Motor de memoria        Recuperar unidades,
                                                    episodios, revisiones y
                                                    asociaciones

  Integración contextual    Motor                   Combinar señales y
                            contextual/ranking      ordenar resultados

  Procesamiento perceptivo  Adaptadores de          Convertir texto,
                            ingestión               imágenes u otras
                                                    entradas en
                                                    representaciones

  Cerebelo/automatización   Optimizador de rutas    Reutilizar planes
                                                    correctos y reducir
                                                    trabajo repetido

  Hipocampo/consolidación   Pipeline de             Agrupar eventos y crear
                            consolidación           representaciones
                                                    derivadas con
                                                    procedencia

  Selección de acciones     Selector de estrategia  Escoger entre rutas
                                                    candidatas según costo
                                                    y utilidad

  Plasticidad sináptica     Actualización de        Ajustar prioridades
                            estadísticas/pesos      según evidencia
  -------------------------------------------------------------------------

Estas correspondencias son hipótesis de diseño, no equivalencias
científicas exactas. Los módulos deben definirse por contratos técnicos
y casos de uso, no por nombres anatómicos.

## 4. Arquitectura conceptual

1.  **Núcleo de datos:** identidad, esquema, estado y ciclo de vida.
2.  **Persistencia transaccional:** almacenamiento físico, journal/WAL o
    alternativa, recuperación y durabilidad.
3.  **Motor de revisiones:** historial, snapshots/deltas, diffs,
    restauración y procedencia.
4.  **Motor de relaciones:** conexiones tipadas entre unidades.
5.  **Índices y recuperación:** clave primaria, filtros, texto y
    vectores cuando proceda.
6.  **Planificador/ejecutor:** selección y ejecución de planes.
7.  **Adaptación operacional:** estadísticas, cachés, planes
    reutilizables e invalidación.
8.  **Consolidación:** artefactos derivados con referencias a sus
    fuentes.
9.  **Seguridad y ciclo de vida:** permisos, cifrado, retención,
    eliminación y auditoría.
10. **SDK/CLI:** API pública, diagnóstico, importación y exportación.

La primera versión puede implementar varias responsabilidades dentro de
pocos módulos. No crear muchos crates o servicios antes de estabilizar
las interfaces.

## 5. Alcance del MVP

### Incluir

-   Motor embebido y local inicialmente.
-   Unidades persistentes con IDs estables.
-   Operaciones CRUD transaccionales.
-   Relaciones tipadas entre unidades.
-   Revisiones consultables, comparación y restauración.
-   Consultas por ID, filtros básicos y relaciones.
-   Planes de consulta y métricas de ejecución.
-   Ruta de referencia correcta y mecanismo de fallback.
-   Una optimización operacional simple que pueda medirse y
    desactivarse.
-   Exportación/importación documentada.
-   Recuperación tras cierres inesperados, dentro de garantías
    explícitas.

### No incluir inicialmente

-   Clúster distribuido o replicación multi-región.
-   SQL completo o compatibilidad total con PostgreSQL.
-   Un motor físico por lóbulo cerebral.
-   Inferencia de LLM dentro del núcleo.
-   Entrenamiento de redes neuronales propias.
-   Búsqueda vectorial aproximada antes de tener benchmarks.
-   Procesamiento multimodal complejo.
-   Cambios de esquema o borrados autónomos sin política explícita.
-   Analítica columnar de gran escala.

## 6. Modelo de datos conceptual

### 6.1 Unit

Una unidad es una entidad persistente: documento, recuerdo, evento,
hecho o recurso. Campos conceptuales:

-   `unit_id`: identidad interna estable.
-   `unit_type`: tipo lógico.
-   `schema_version`: versión de esquema.
-   `current_revision_id`: revisión vigente.
-   `created_at`, `updated_at`.
-   `lifecycle_state`.
-   `metadata`.
-   `content_ref`.

La representación lógica no obliga a guardar todo en una sola estructura
física.

### 6.2 Revision

Campos sugeridos:

-   `revision_id`, `unit_id`, `parent_revision_id`.
-   `commit_sequence`.
-   `content_hash`.
-   `schema_version`, `change_type`.
-   `payload_ref`.
-   `actor_ref` o `source_ref`.
-   `change_set_id` y motivo opcionales.
-   `created_at`.

Reglas: - Restaurar una revisión crea una nueva revisión; no reescribe
silenciosamente el pasado. - Usar snapshots/deltas con límites para
evitar cadenas excesivas. - Diferenciar historial lógico de revisiones
del WAL/journal de recuperación física. - Definir visibilidad
transaccional y consistencia de revisiones.

### 6.3 ChangeSet

Agrupa cambios de una operación lógica. Puede contener ID, secuencia de
commit, actor/fuente, motivo, fecha, referencias a revisiones y
metadatos de correlación. Permite responder qué cambió, por qué y como
parte de qué operación.

### 6.4 Edge / relación

Campos sugeridos: `edge_id`, `from_unit_id`, `to_unit_id`,
`relation_type`, peso opcional, validez temporal opcional, timestamps,
procedencia y atributos. No asumir que todas las relaciones son
dirigidas o tienen peso. Diferenciar relaciones declaradas de inferidas.

### 6.5 ExecutionObservation

Registrar, si está habilitado: huella normalizada de consulta, plan,
duración, filas examinadas/devueltas, resultado, cache hit, fallback y
timestamp. Evitar guardar valores sensibles de consultas en métricas por
defecto.

## 7. Identidad, privacidad y seguridad

Separar IDs internos de referencias públicas. UUIDv7 puede servir como
ID interno, pero revela tiempo aproximado y no es un token de seguridad.
Para IDs públicos, usar tokens opacos generados con mecanismos
revisados; la autorización nunca debe depender de que un ID sea difícil
de adivinar. No inventar criptografía propia.

Si hay cifrado en reposo, documentar gestión/rotación de claves y
recuperación ante pérdida. Considerar WAL, snapshots, índices,
embeddings, cachés, logs, exportaciones y backups. La eliminación lógica
y física deben tener semánticas explícitas. Si se promete eliminación
completa, documentar cómo se aplica a derivados y copias de seguridad.

## 8. Persistencia y transacciones

Decisión pendiente: comparar B+ tree con copy-on-write, B+ tree con WAL,
LSM tree y reutilizar un motor embebido consolidado para el prototipo.
Elegir mediante ADR y benchmarks, no por analogía cerebral.

Requisitos mínimos: - Atomicidad y durabilidad acordes con garantías
declaradas. - Consistencia entre unidades, revisiones, relaciones e
índices. - Recuperación tras terminación inesperada. - Errores
explícitos ante fallo de disco o falta de espacio. - Modelo de
aislamiento/concurrencia documentado. - Formato persistente
versionado. - Pruebas de recuperación y propiedades.

Los índices deben corresponder a un estado confirmado. Reconstrucciones
e invalidaciones deben ser explícitas; una consulta no debe devolver
resultados engañosos porque un índice esté incompleto. Si datos e
índices no pueden actualizarse atómicamente, definir recuperación
consistente.

## 9. Planificador y ejecución adaptativa

### Primera versión: determinista

1.  Analizar y validar consulta.
2.  Generar planes candidatos válidos.
3.  Estimar costo con estadísticas disponibles.
4.  Elegir un plan.
5.  Ejecutarlo.
6.  Registrar métricas configurables.
7.  Validar invariantes y usar fallback si procede.

### Ruta de referencia

Mantener una ejecución simple y correcta. Las rutas aprendidas son
optimizaciones opcionales. Cada plan optimizado debe registrar
operación/huella, precondiciones, dependencias de esquema e índices,
estimación de costo, evidencia observada, fecha/versión, invalidadores y
fallback.

### Qué aprender

-   Frecuencia de consultas.
-   Latencia y variabilidad.
-   Datos examinados.
-   Cache hits.
-   Errores, fallbacks y discrepancias.
-   Costo de mantenimiento.
-   Memoria y almacenamiento adicionales.

No reforzar una ruta solo por frecuencia; debe aportar utilidad neta.
Invalidar o revalidar planes cuando cambien datos, esquema, índices o
estadísticas relevantes. Toda optimización debe poder desactivarse
globalmente y por consulta. En modo diagnóstico, explicar por qué se
eligió un plan.

## 10. Plasticidad computacional y automatización

Separar dos aprendizajes:

**Aprendizaje operacional:** optimiza cómo se recuperan/procesan datos
mediante planes reutilizables, índices justificados, cachés, lotes y
rutas compuestas.

**Aprendizaje de conocimiento:** propone relaciones, agrupa eventos,
genera resúmenes, detecta redundancias o contradicciones y vincula
embeddings a revisiones.

No mezclarlos: optimizar una consulta no debe alterar contenido;
consolidar conocimiento produce artefactos derivados con procedencia.

Una conexión puede ganar o perder prioridad según evidencia, pero: - Las
relaciones declaradas por el usuario no deben tratarse como inferidas. -
Las inferencias deben guardar origen y confianza. - No borrar datos
originales por baja frecuencia de acceso. - El olvido de contenido
depende de retención, consentimiento y políticas explícitas. - Debe
poder explicarse por qué una ruta o relación fue priorizada.

Los artefactos derivados registran fuentes/revisiones, método o modelo,
versión, fecha, hash, estado de vigencia y si pueden reconstruirse. Un
resumen nunca reemplaza silenciosamente el original si se requiere
auditoría.

## 11. Índices y recuperación

Orden recomendado: 1. Índice primario. 2. Filtros y relaciones básicas.
3. Índices secundarios justificados por carga. 4. Búsqueda textual, por
ejemplo BM25, si se necesita. 5. Búsqueda vectorial exacta como
baseline. 6. Índice vectorial aproximado solo cuando escala y benchmark
lo justifiquen. 7. Recuperación híbrida combinando filtros, texto,
vectores, relaciones, temporalidad y procedencia.

### Embeddings

Son datos derivados, no la fuente de verdad. Registrar unidad/revisión
de origen, modelo y versión, dimensiones, métrica, chunking, fecha, hash
de entrada y estado de vigencia. Cuando cambia el contenido fuente,
marcar el embedding obsoleto o crear una nueva versión. No mezclar
modelos/dimensiones incompatibles. La inferencia debe ir detrás de
adaptadores o jobs externos, no dentro del núcleo transaccional.

El ranking híbrido debe tener modo diagnóstico y poder explicar sus
señales. No fijar una fórmula universal antes de probarla.

## 12. Historial y procedencia

Políticas posibles: - Sin historial de contenido. - Auditoría
limitada. - Versionado completo con snapshots/deltas.

Operaciones previstas: listar revisiones, comparar, ver procedencia,
restaurar como nueva revisión, consultar ChangeSets e invalidar
derivados obsoletos.

Distinguir información declarada por el usuario, importada, generada por
algoritmo, inferida por modelo y modificada internamente. No presentar
una inferencia como hecho verificado.

## 13. API y organización del código

Primero respetar lenguaje, estructura y convenciones reales del
repositorio. Si ya se usa Rust, módulos posibles a futuro:

-   `kerno-core`
-   `kerno-storage`
-   `kerno-tx`
-   `kerno-data`
-   `kerno-revisions`
-   `kerno-relations`
-   `kerno-index`
-   `kerno-query`
-   `kerno-adaptation`
-   `kerno-cli`

No crear todos como crates desde el día uno. Extraer módulos cuando haya
contratos estables.

La API inicial debería permitir crear/leer/actualizar/archivar unidades;
crear y recorrer relaciones; listar/comparar/restaurar revisiones;
consultar por filtros; inspeccionar el plan; leer estadísticas;
desactivar/limpiar optimizaciones; importar/exportar datos. No diseñar
un lenguaje de consultas complejo antes de validar casos de uso.

## 14. Formato persistente y portabilidad

-   Versionar el formato.
-   Definir migraciones y comportamiento ante versiones desconocidas.
-   Documentar exportación en formato abierto.
-   Verificar integridad tras importación/exportación.
-   Distinguir datos fuente de datos derivados reconstruibles.
-   Probar migraciones con fixtures de versiones previas.
-   No asumir que un cambio de estructura interna es seguro para datos
    existentes.

## 15. Pruebas y calidad

Pruebas necesarias según etapa: - Unitarias e integración. -
Consistencia de unidades, revisiones, relaciones e índices. -
Recuperación tras fallos simulados. - Property-based tests. - Fuzzing de
parsers/entradas externas. - Migraciones y exportación/importación. -
Concurrencia según garantías soportadas. - Benchmarks reproducibles.

Invariantes iniciales: 1. El estado actual apunta a una revisión válida.
2. Restaurar no destruye el historial posterior. 3. Índices y planes no
contradicen el estado confirmado. 4. Plan adaptativo y plan de
referencia producen resultados equivalentes. 5. Invalidar una
optimización no altera datos. 6. Una transacción fallida no deja cambios
parciales visibles fuera del contrato. 7. Los artefactos derivados
conservan procedencia relevante. 8. La eliminación respeta la política
documentada.

Comparar contra alternativas adecuadas, incluyendo SQLite u otros
motores embebidos pertinentes. Medir latencias p50/p95/p99, throughput,
disco, memoria, costo de revisiones, relaciones, filtros y mejora neta
adaptativa. Documentar hardware, dataset, comandos y parámetros. No
afirmar superioridad sin resultados reproducibles.

## 16. Plan de trabajo

### Etapa 0 --- Diagnóstico

Mapear repositorio, pruebas, dependencias, funciones reales, riesgos y
decisiones pendientes. Entregar tabla de estado y plan priorizado.

**Aceptación:** evidencia del repositorio; separar hechos, hipótesis y
propuestas.

### Etapa 1 --- Contratos mínimos

Definir unidad, revisión, relación, transacción y consulta; registrar
ADR relevantes.

**Aceptación:** contratos comprensibles, ejemplos/pruebas y ningún tipo
sin consumidor conocido.

### Etapa 2 --- Persistencia y transacciones

Implementar o validar almacenamiento, recuperación y modelo
transaccional.

**Aceptación:** pruebas de recuperación pasan y garantías documentadas
coinciden con las probadas.

### Etapa 3 --- Unidades, revisiones y procedencia

CRUD, ChangeSets, comparación y restauración.

**Aceptación:** reconstruir evolución y restaurar sin destruir
revisiones posteriores.

### Etapa 4 --- Relaciones e índices

Relaciones tipadas, filtros e índices justificados.

**Aceptación:** resultados coinciden con ruta de referencia.

### Etapa 5 --- Planificador y observabilidad

Planes, métricas, explicación de elección y fallback.

**Aceptación:** el plan puede inspeccionarse; logs minimizan datos
sensibles.

### Etapa 6 --- Primera plasticidad operacional

Implementar una optimización pequeña (por ejemplo, caché o plan
reutilizable) y compararla con modo adaptativo desactivado.

**Aceptación:** mejora medible, equivalencia de resultados e
invalidación probada.

### Etapa 7 --- Memoria asociativa

Recuperación por relaciones, contexto y procedencia; vector exacto como
baseline si corresponde.

**Aceptación:** calidad de recuperación evaluable y fuentes rastreables.

### Etapa 8 --- Consolidación y embeddings

Jobs/adaptadores para artefactos derivados, versionado y obsolescencia.

**Aceptación:** cada derivado se vincula a sus fuentes y puede
invalidarse/reconstruirse.

### Etapa 9 --- Endurecimiento

Seguridad, migraciones, exportación/importación, documentación,
benchmarks y pruebas de estrés.

**Aceptación:** límites y garantías documentados; fallos críticos
resueltos o explícitamente bloqueantes.

Claude debe saltarse etapas ya implementadas y probadas; no rehacer
trabajo por seguir el orden del documento.

## 17. ADRs requeridas

Crear `docs/adr/` o usar la estructura existente. Evaluar: 1.
Lenguaje/toolchain. 2. Construir o reutilizar motor físico; B+ tree,
copy-on-write, WAL o LSM. 3. Transacciones/concurrencia. 4. Formato
persistente y migraciones. 5. Snapshots/deltas y retención. 6.
Representación física de relaciones. 7. API pública y lenguaje de
consultas. 8. IDs internos y públicos. 9. Seguridad/cifrado. 10. Índices
textuales y vectoriales. 11. Privacidad de observabilidad. 12.
Adaptación e invalidación. 13. Semántica de unidad neuronal. 14.
Versionado y reconstrucción de derivados.

Cada ADR contiene contexto, alternativas, decisión, consecuencias,
riesgos y condiciones para reabrirla.

## 18. Riesgos y mitigaciones

  -----------------------------------------------------------------------
  Riesgo                              Mitigación
  ----------------------------------- -----------------------------------
  Construir todos los motores         MVP vertical y etapas
  simultáneamente                     

  Metáfora cerebral sin beneficio     Caso de uso y benchmark para cada
  real                                mecanismo

  Aprendizaje cambia resultados       Ruta de referencia, equivalencia y
                                      fallback

  Historial costoso                   Snapshots/deltas, retención y
                                      medición

  Índices o embeddings obsoletos      Versionado, invalidación y
                                      reconstrucción

  Exceso de módulos                   Modularizar tras estabilizar
                                      contratos

  Reescribir código funcional         Diagnóstico y pruebas antes de
                                      cambiar

  Motor propio inferior a soluciones  Benchmark temprano y posibilidad de
  maduras                             reutilizar

  Borrado/consolidación destructiva   Originales, procedencia y política
                                      explícita

  Datos sensibles en logs             Minimización y redacción

  Afirmaciones de rendimiento sin     Benchmarks reproducibles
  evidencia                           
  -----------------------------------------------------------------------

## 19. Definition of Done

Una tarea no está terminada solo porque compila. Debe incluir código,
pruebas pertinentes, ejecución real de pruebas, errores/límites
documentados, ausencia de regresiones, dependencias justificadas y
resumen exacto de lo realizado. No ocultar fallos ni reducir pruebas
para conseguir un build verde.

## 20. Primera tarea de Claude Code

Empieza con diagnóstico, no con una reescritura.

1.  Inspecciona el repositorio, instrucciones y estado de Git.
2.  Identifica lenguaje, módulos, tests, CI y estado real del código.
3.  Revisa documentación de KernoDB y los repositorios `livespec` y
    `cauce` si son accesibles.
4.  Compara implementación y especificación.
5.  Produce una tabla: área, estado real, evidencia, brecha, riesgo y
    siguiente acción.
6.  Identifica las cinco decisiones técnicas más importantes no
    justificadas.
7.  Recomienda la siguiente tarea vertical más pequeña que entregue
    valor.
8.  Implementa solo esa tarea si está permitido; en caso contrario,
    espera aprobación.
9.  Al terminar informa archivos modificados, decisiones, pruebas y
    resultados reales, limitaciones y siguiente paso.

No ajustes ciegamente el código a este documento. Si el código revela
una solución mejor, explica la razón y registra una ADR o actualiza la
especificación.

## 21. Glosario

-   **Unidad:** entidad persistente de información.
-   **Revisión:** estado versionado de una unidad.
-   **ChangeSet:** conjunto de cambios de una operación lógica.
-   **Edge:** relación tipada entre unidades.
-   **Planificador:** selecciona una estrategia de ejecución.
-   **Ruta de referencia:** ejecución simple usada como base de
    corrección.
-   **Plasticidad computacional:** ajuste de pesos/estadísticas según
    evidencia.
-   **Consolidación:** creación de una representación derivada con
    procedencia.
-   **Artefacto derivado:** índice, embedding, resumen o caché
    reconstruible.
-   **Obsolescencia:** derivado que ya no corresponde a sus fuentes
    actuales.
-   **ADR:** registro de decisión arquitectónica.

## 22. Resumen ejecutivo

Construir KernoDB en este orden: persistencia e integridad; unidades,
revisiones y relaciones; consultas e índices; planificación y
observabilidad; plasticidad operacional; recuperación híbrida;
consolidación y adaptadores de IA.

La metáfora cerebral solo es útil si conduce a contratos, pruebas y
mejoras medibles. La meta no es una base de datos que parezca un
cerebro, sino un motor de datos capaz de aprender a trabajar mejor sin
perder integridad, trazabilidad ni comportamiento explicable.
