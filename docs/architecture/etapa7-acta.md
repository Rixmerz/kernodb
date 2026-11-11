# Acta de entrada de la Etapa 7: memoria asociativa

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Aprobada** el 2026-11-11 (decisión delegada por el propietario; revocable) |
| Acto | Propuesta de alcance y de los paquetes E7-00 a E7-06 |
| Documentos | [Roadmap, Etapa 7](../roadmap/README.md#etapa-7--memoria-asociativa) · [cierre de la Etapa 6](etapa6-cierre.md) · [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) (pendiente) · [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) (propuesto) · [invariantes](invariants.md) · spec v0.2.0 §11, §12, §16 |

## 0. Estado

Esta acta está **aprobada** (§8). Los WP son secuenciales, como en las Etapas 3 a 6.

## 1. Condición de inicio

| Condición | Estado |
|---|---|
| Etapas 4 y 5 cerradas (roadmap: recorridos de relaciones y explicación de planes disponibles) | Cumplida: [cierre de la Etapa 4](etapa4-cierre.md) y [cierre de la Etapa 5](etapa5-cierre.md), aprobados |
| Etapa 6 cerrada | Cumplida: [cierre](etapa6-cierre.md) aprobado por delegación el 2026-11-11; la caché de resultados queda desactivada por defecto |
| ADR-0011 (vectores y recuperación híbrida) | *Pendiente*. Se decide en E7-00 (§7, punto 1) |
| ADR-0018 (versionado de derivados) | *Propuesto*. No hace falta: esta etapa no persiste derivados (§2.3) |
| Experimento de hilos narrativos | Cerrado con NO-GO ([informe](../../experiments/narrative-threads/results/REPORT.md)); no se integra |

## 2. Alcance

### 2.1 Objetivos

1. **Consulta asociativa**: a partir de una o varias unidades semilla, recuperar unidades relacionadas siguiendo relaciones tipadas a más de un salto, con cotas obligatorias (profundidad, número de resultados y presupuesto de entradas visitadas).
2. **Contexto temporal**: restringir el recorrido por `commit_seq` (relaciones y revisiones creadas dentro de una ventana) y responder sobre una instantánea.
3. **Procedencia**: filtrar por la procedencia de la relación (`Declared`, `Imported`, `Algorithmic`, `Inferred`) y por el actor o la fuente del `ChangeSet` que la creó.
4. **Rastreabilidad**: cada resultado lleva su rastro: las relaciones del camino que lo alcanzó, la revisión de cada unidad y la secuencia de commit de cada paso.
5. **Ranking determinista con modo diagnóstico** que explica cuánto aporta cada señal.
6. **Puntuación vectorial exacta** como señal opcional y línea base (§7, punto 1).
7. Calidad de recuperación medida con un conjunto de consultas y respuestas esperadas **fijado antes de escribir el código de recuperación**.

### 2.2 Punto de partida

`kerno-units` tiene relaciones declaradas con procedencia obligatoria (ADR-0014), el índice de adyacencia y `UnitSnapshot::relations_of` a un salto, con su ruta de referencia (`reference_relations_of`, I-32). Cada relación guarda `created_seq`; el `ChangeSet` guarda actor, fuente y motivo. El planificador explica su plan (Etapa 5) y las observaciones no llevan literales fuera de `diagnostic` (I-37). No existe recorrido a varios saltos, ni ranking, ni vectores.

### 2.3 Fuera de alcance

- Índice vectorial aproximado (ANN): solo con un benchmark que lo justifique (roadmap).
- Generar embeddings, resúmenes o relaciones inferidas: Etapa 8, detrás de adaptadores (I-31).
- **Persistir vectores u otros derivados**: exige ADR-0018 y ADR-0009; es de la Etapa 8. Ningún cambio del formato persistente en esta etapa.
- Búsqueda textual (BM25).
- Una fórmula de ranking universal (spec §11): los pesos son configuración, con un valor por defecto medido.
- Usar la caché de resultados de la Etapa 6 en consultas asociativas.
- Recuperación narrativa (experimento cerrado con NO-GO).

## 3. Paquetes de trabajo

Todos son secuenciales. «Hecho» incluye `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, la suite del WP y la suite completa en verde, el conteo de pruebas antes y después, y cada prueba nueva vista fallar por mutación.

| WP | Contenido | Depende de | Aceptación |
|---|---|---|---|
| **E7-00** Puertas de entrada | Esta acta aprobada. ADR-0011 decidido según §7, punto 1. **Conjunto de evaluación congelado** (§6.1): corpus, consultas, respuestas esperadas con grado de relevancia y `MANIFEST` con SHA-256, comprometido antes de cualquier código de recuperación. Métricas de §6 implementadas y probadas con respuestas fijas. Líneas base B0 y B1 medidas sobre el conjunto | Etapa 6 cerrada | Conjunto y umbrales comprometidos; una prueba comprueba los SHA-256; ningún WP posterior arranca antes |
| **E7-01** Modelo de consulta y ruta de referencia | Tipo `AssocQuery` (semillas, tipos de relación, dirección, profundidad máxima, límite de resultados, presupuesto de visitas, ventana de `commit_seq`, filtro de procedencia). Cotas obligatorias con máximos duros. Ruta de referencia: recorrido en anchura sobre `reference_relations_of`, con orden determinista. Cada resultado con su rastro | E7-00 | Vectores fijos de consultas y resultados; una consulta sin cotas se rechaza; ciclos y relaciones repetidas no duplican resultados; agotar el presupuesto se informa y no se confunde con «no hay más» |
| **E7-02** Ejecución por índice en el planificador | El recorrido usa el índice de adyacencia como camino del planificador; su respuesta se comprueba (como `check_relations`) y cualquier fallo cae a la ruta de referencia con causa registrada. Explicación del plan y observaciones sin literales | E7-01 | Prueba diferencial con semilla contra la ruta de referencia en los dos motores (I-32); inyección de respuestas falsas del índice; centinelas de I-37 sobre las consultas asociativas |
| **E7-03** Contexto temporal y procedencia | Ventana de `commit_seq` y filtros de procedencia, actor y fuente aplicados en ambas rutas. Rastro verificable: cada relación y revisión citada existe en la instantánea | E7-02 | I-40 (§4): prueba que recorre cada resultado y encuentra sus fuentes; una relación inferida nunca aparece como declarada en el rastro (I-39); el reloj no ordena nada (I-26) |
| **E7-04** Ranking y modo diagnóstico | Señales: distancia en saltos, tipo de relación, recencia por `commit_seq`, clase de procedencia y, si se suministran vectores, similitud exacta. Puntuación determinista; empates resueltos por identificador. El modo diagnóstico devuelve la aportación de cada señal. Vectores: los entrega quien llama, en memoria; se comprueban dimensión y métrica; fuerza bruta exacta | E7-03 | I-41 (§4): la suma de las aportaciones es la puntuación; el mismo estado y la misma consulta dan el mismo orden; dimensiones o métricas incompatibles se rechazan; con el ranking apagado el orden es el de la ruta de referencia |
| **E7-05** Medición | Calidad sobre el conjunto congelado frente a los umbrales de §6.2. Rendimiento según §6.3. Salida cruda comprometida | E7-04 | Mediciones comprometidas; ningún umbral ni respuesta esperada cambia después de E7-00 |
| **E7-06** Cierre | Revisión adversarial. Promoción de I-40 e I-41. Actualización del roadmap y del SAD | todos | Criterios de §4 cumplidos o registrados como no afirmados; informe sin bloqueantes abiertos |

Camino crítico: E7-00 → E7-01 → E7-02 → E7-03 → E7-04 → E7-05 → E7-06.

## 4. Criterios de aceptación

Del [roadmap, Etapa 7](../roadmap/README.md#etapa-7--memoria-asociativa) y de spec §11 y §12:

| Criterio | Invariante | Evidencia prevista | Estado |
|---|---|---|---|
| Calidad evaluable con un conjunto fijado antes de medir | — | E7-00 (conjunto congelado), E7-05 | pendiente |
| Toda respuesta rastreable a sus fuentes | **I-40** (nuevo): cada resultado de una consulta asociativa cita las relaciones y revisiones que lo justifican, y todas existen en la instantánea consultada | E7-03 | pendiente |
| El ranking explica sus señales | **I-41** (nuevo): la puntuación es determinista y el modo diagnóstico da la aportación de cada señal, cuya suma es la puntuación | E7-04 | pendiente |
| Equivalencia con la ruta de referencia | I-32 | Prueba diferencial (E7-02) | pendiente |
| Una relación inferida no se presenta como declarada | I-39 | E7-03 | pendiente |
| Sin literales fuera de `diagnostic` | I-37 | Centinelas (E7-02, E7-04) | pendiente |
| El núcleo no necesita red ni modelo | I-31 | Revisión de dependencias (E7-06): los vectores los entrega quien llama | pendiente |
| Sin ANN antes de un benchmark que lo justifique | — | Fuera de alcance (§2.3) | cumplido por alcance |

## 5. Riesgos

| ID | Riesgo | Tratamiento propuesto |
|---|---|---|
| K-E3-8, K-E3-9 | U2 en `history`; U6 con retención concurrente | Sin cambios; esta etapa no cambia la escritura |
| H-1 (heredado) | Las comprobaciones en consulta no garantizan completitud | La equivalencia se demuestra con la prueba diferencial |
| **Nuevo** | El conjunto de evaluación se ajusta a la implementación | Se congela con SHA-256 antes de escribir la recuperación; las respuestas esperadas salen del relato con el que se genera el corpus, no del código de `kerno-units`; el generador vive en `kerno-testkit`, que no depende de `kerno-units` |
| **Nuevo** | Un conjunto sintético no representa un uso real | Se declara como límite en el cierre; los umbrales miden esta implementación sobre este conjunto, no calidad en producción |
| **Nuevo** | El recorrido a varios saltos crece sin cota en nodos muy conectados | Cotas obligatorias y presupuesto de visitas; agotarlo se informa en el resultado |
| **Nuevo** | El ranking no mejora al orden simple | La etapa cierra con el resultado negativo documentado y el orden por defecto queda en «saltos, luego identificador» (§7, punto 3) |
| **Nuevo** | La señal vectorial fija supuestos de ADR-0011 difíciles de revertir | Los vectores solo existen en memoria y los entrega quien llama; nada se escribe en el formato |

Política de merge: la [puerta de corrección de la Fase 1](phase1-design.md#71-puerta-de-corrección-adr-0012-umbral-1-c-25-bloqueante) es bloqueante. Los incumplimientos de rendimiento se registran como riesgos y no bloquean. Los umbrales de calidad deciden el ranking por defecto, no el merge.

## 6. Conjunto de evaluación y umbrales

### 6.1 Conjunto de evaluación (se congela en E7-00)

- **Corpus**: generado con semilla fija a partir de un relato de memoria de agente (sesiones, tareas, decisiones, hechos, fuentes), con relaciones de las cuatro clases de procedencia y relaciones distractoras. Escala de calidad: unas 2 000 unidades. Escala de rendimiento: 20 000.
- **Consultas**: al menos 60, repartidas en cuatro clases (al menos 12 por clase): varios saltos, temporal, procedencia y combinada.
- **Respuestas esperadas**: derivadas del relato, con grado de relevancia (0 a 3), no de la definición de la consulta.
- **Líneas base**: B0, filtro por tipo (consulta existente); B1, vecinos a un salto en el orden del índice.

### 6.2 Calidad (sobre las 60 consultas, corte en 10)

- Recall@10 de la consulta asociativa con ranking ≥ 0,90.
- nDCG@10 ≥ 0,80.
- nDCG@10 al menos un 10 % mejor que B1 y que el orden simple «saltos, luego identificador».
- En ninguna clase de consulta el nDCG@10 queda por debajo del de B1.
- Rastro: 100 % de los resultados con rastro verificable (I-40). Este no es un umbral de calidad, es un criterio de corrección.

### 6.3 Rendimiento (informativo, salvo la primera línea)

- Con 20 000 unidades y profundidad 2, el p50 del recorrido por índice no es peor que el de la ruta de referencia.
- Se informan p50, p99 y entradas visitadas por profundidad (1 a 3). Sin umbral: no hay medición previa de la que partir.
- T1–T10 y U1–U6 no cambian; esta etapa no escribe en el motor.

## 7. Puntos que el propietario debe decidir

1. **ADR-0011.** Propuesta: aceptar la opción 1 (búsqueda exacta primero) con este límite para la Etapa 7: los vectores los entrega quien llama y viven en memoria; se fijan los contratos de dimensión y métrica (coseno y producto escalar); almacenar vectores como derivados ligados a su revisión fuente (I-25) queda para la Etapa 8 con ADR-0018.
2. **Conjunto de evaluación.** Propuesta: el de §6.1, sintético y con semilla, congelado en E7-00. No se reutiliza el corpus del experimento de hilos narrativos (texto y modelos externos).
3. **Ranking por defecto.** Propuesta: activado por defecto solo si cumple §6.2; si no, orden «saltos, luego identificador» y el ranking queda como opción.
4. **Umbrales de §6**, fijados antes de medir.
5. **Cotas duras.** Propuesta: profundidad máxima 4, 1 000 resultados y 100 000 entradas visitadas por consulta; quien llama puede bajarlas, no subirlas.

El orden de seguridad frente a IA (roadmap, decisión pendiente 1) no bloquea esta etapa: nada de lo que entrega se persiste como derivado. Debe estar decidido antes de la Etapa 8.

## 8. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-11 | **Aprobada** | El propietario delegó las decisiones («lo mejor y más correcto, toma tú las riendas») y pidió arrancar con esta acta. Decisiones de §7: (1) ADR-0011, opción 1 con vectores en memoria entregados por quien llama; persistencia en la Etapa 8; (2) conjunto sintético de §6.1, congelado en E7-00; (3) ranking por defecto solo si cumple §6.2; (4) umbrales de §6 tal como están; (5) cotas duras de profundidad 4, 1 000 resultados y 100 000 visitas. El propietario puede revocar cualquiera |
