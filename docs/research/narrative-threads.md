# Diseño de experimento — recuperación por hilos narrativos

| Campo | Valor |
|---|---|
| Estado | Propuesta de experimento (aprobada por el propietario como experimento independiente) |
| Fecha | 2026-10-31 |
| Alcance | **Investigación.** No forma parte del núcleo de almacenamiento y **no es requisito de la Etapa 3** |
| Depende de | Unidades, revisiones y relaciones de la Etapa 3 (solo lectura); ningún cambio de formato en disco |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md); ADR-0017 (unidad); invariante I-39 (lo inferido guarda origen y confianza y no se presenta como declarado) |

## 1. Resumen y alcance

**Hipótesis.** Para preguntas cuya respuesta está repartida en un documento, la unidad de recuperación adecuada no es el fragmento (*chunk*) sino una **trayectoria temática**: una secuencia, no necesariamente contigua, de segmentos que desarrollan un mismo asunto a lo largo del documento.

**Qué es.** Un índice **derivado y reconstruible** sobre las unidades, revisiones y relaciones de la Etapa 3. Puede borrarse y regenerarse desde el contenido sin perder información (spec: los artefactos derivados llevan procedencia).

**Qué no es.**

- No toca el WAL, el formato v1 ni `kerno-storage`; no bloquea la Etapa 3.
- No es una ontología universal: las fases narrativas son una hipótesis de trabajo (ver §10).
- El «cilindro» es solo una **visualización** del modelo; no impone estructura de datos ni de consulta.
- Una relación inferida nunca se presenta como declarada (I-39).

**Resultado esperado.** Un informe con métricas, ablaciones y una decisión go/no-go (§9). Un NO-GO es un resultado válido y se conserva.

## 2. Modelo conceptual

| Eje (cilindro) | Concepto | Definición |
|---|---|---|
| Altura | Posición documental | Orden y desplazamiento de bytes del segmento dentro de la revisión |
| Sector | Fase narrativa | Etiquetas **ponderadas** y múltiples: `problem`, `clue`, `development`, `contrast`, `solution`, `conclusion`, más `unclassified` |
| Radio | Profundidad temática | Cuánto trata el segmento un tema en detalle, **distinta de la longitud** (un párrafo corto puede ser denso; uno largo, vacío) |
| Hilo | Trayectoria | Secuencia ordenada de segmentos no contiguos. Un segmento puede pertenecer a varios hilos |

Reglas del modelo:

1. Las fases admiten **ciclos, ramas y varias conclusiones**; un documento puede no tener `solution` o tener tres. No se exige un orden canónico.
2. Una etiqueta de fase es un peso en `[0, 1]`; la suma no tiene que ser 1. Todo peso bajo (umbral `tau_phase`) se resuelve como `unclassified`.
3. La profundidad se estima por contenido (entidades distintas, definiciones, cifras, pasos, argumentos), no por número de tokens. Se evalúa su correlación con la longitud: si es alta (> 0,9), la dimensión no aporta y se descarta (ver ablación §8).
4. Relaciones tipadas entre segmentos: `develops`, `contradicts`, `anticipates`, `resumes`, `exemplifies`, `resolves`, `depends_on`. Cada una con **confianza** y **evidencia textual** (desplazamientos exactos). Existe la opción explícita «sin relación / no clasificada»: el extractor puede abstenerse.
5. Un hilo se acepta solo si cada salto entre segmentos consecutivos tiene una relación con evidencia. Sin evidencia no hay salto (ver riesgo de continuidad inventada, §10).

## 3. Relación con la Etapa 3

El índice se alimenta de lo que la Etapa 3 ya define y no añade campos a esos tipos.

| Entrada de Etapa 3 | Uso en el experimento |
|---|---|
| `Unit` (documento) | Raíz: un índice de hilos por unidad |
| `Revision` (hash de contenido, ADR-0008) | Clave de caché: `(unit_id, revision_id, extractor_version)`. Cambia la revisión → el índice de esa revisión queda obsoleto y se reconstruye |
| Relaciones (Etapa 4, cuando existan) | Opcionales. Mientras no existan, las relaciones narrativas viven solo dentro del índice derivado y no se escriben como `Edge` |
| Procedencia | Cada segmento y relación guarda modelo, versión del prompt y fecha |

Reconstrucción: `rebuild(unit_id, revision_id)` es una función pura del contenido y de la versión del extractor, salvo el no determinismo del modelo, que se mitiga fijando temperatura 0 y guardando la salida cruda para auditoría. Durante el experimento el índice se guarda **fuera** de `kerno-storage` (ficheros JSON Lines o SQLite local del banco de pruebas); integrarlo en el motor sería una decisión posterior al go (§9).

## 4. Modelo de datos mínimo (Rust)

Esbozo para el crate del experimento (p. ej. `experiments/narrative-threads`, fuera del workspace del motor). Es ilustrativo y no está compilado en el repositorio.

```rust
use std::collections::BTreeMap;
use std::ops::Range;

/// Identificadores opacos; `unit` y `revision` vienen de la Etapa 3.
pub struct SegmentId(pub u64);
pub struct ThreadId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Problem,
    Clue,
    Development,
    Contrast,
    Solution,
    Conclusion,
    Unclassified,
}

/// Fase con peso en [0, 1]. Varias por segmento.
pub struct PhaseWeight {
    pub phase: Phase,
    pub weight: f32,
}

/// Rango de bytes exacto sobre el contenido de la revisión.
pub type ByteSpan = Range<u64>;

pub struct NarrativeSegment {
    pub id: SegmentId,
    pub unit_id: [u8; 16],
    pub revision_id: [u8; 16],
    /// Posición documental ("altura"): orden y desplazamiento.
    pub ordinal: u32,
    pub span: ByteSpan,
    /// Fases ponderadas; vacío equivale a `Unclassified`.
    pub phases: Vec<PhaseWeight>,
    /// Profundidad temática ("radio"), en [0, 1]; independiente de `span.len()`.
    pub depth: f32,
    pub topic: String,
    pub provenance: Provenance,
}

pub struct NarrativeThread {
    pub id: ThreadId,
    pub unit_id: [u8; 16],
    pub revision_id: [u8; 16],
    pub topic: String,
    /// Segmentos en orden de lectura del hilo; no contiguos; un segmento
    /// puede aparecer en varios hilos.
    pub segments: Vec<SegmentId>,
    /// Confianza agregada: mínimo de las relaciones que lo sostienen.
    pub confidence: f32,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Develops,
    Contradicts,
    Anticipates,
    Resumes,
    Exemplifies,
    Resolves,
    DependsOn,
    /// El extractor se abstiene: no hay relación demostrable.
    Unclassified,
}

pub struct NarrativeRelation {
    pub from: SegmentId,
    pub to: SegmentId,
    pub kind: RelationKind,
    pub confidence: f32,
    /// Evidencia textual: citas con desplazamientos exactos. Obligatoria
    /// salvo para `Unclassified`.
    pub evidence: Vec<ByteSpan>,
    pub provenance: Provenance,
}

pub struct Provenance {
    pub model: String,
    pub prompt_version: String,
    pub extracted_at_unix_ms: u64,
    /// Marca de origen: siempre `Inferred` (I-39).
    pub origin: Origin,
}

pub enum Origin {
    Inferred,
}

/// Índice derivado; se descarta y se reconstruye por revisión.
pub struct ThreadIndex {
    pub segments: BTreeMap<u64, NarrativeSegment>,
    pub threads: BTreeMap<u64, NarrativeThread>,
    pub relations: Vec<NarrativeRelation>,
}
```

Notas de diseño: `ByteSpan` evita tipos de retorno complejos (clippy `type_complexity`, ver convenciones del repositorio); ninguna estructura exige que las fases sumen 1 ni que los hilos sean contiguos; `Origin` tiene una sola variante a propósito para que lo inferido no se confunda con lo declarado.

## 5. Extracción: prompt y esquema de salida

La extracción corre en dos pasadas para acotar el coste y poder medirlas por separado.

1. **Segmentación y etiquetado**: el documento se entrega con los desplazamientos de byte de cada párrafo/bloque (el texto se numera por el arnés, no por el modelo). El modelo agrupa bloques en segmentos y etiqueta fase, profundidad y tema.
2. **Relaciones e hilos**: con la lista de segmentos (id, tema, fases, resumen de una línea) el modelo propone relaciones y hilos; el arnés verifica la evidencia.

### 5.1 Prompt (esquema; se versiona como `prompt_version`)

```text
Eres un anotador. Recibes un documento dividido en bloques con su rango de
bytes [inicio, fin). Devuelve SOLO JSON conforme al esquema.

Reglas:
- Un segmento es un rango contiguo de bloques. No cortes dentro de un bloque.
- "phases" admite varias fases con peso 0..1. Si ninguna encaja, usa
  "unclassified". El documento puede no seguir un orden problema→conclusión.
- "depth" mide cuánto se profundiza en el tema (0..1), NO la longitud.
- Una relación exige "evidence": citas literales con su rango de bytes
  EXACTO tomado de los bloques. Si no puedes citar, usa kind="unclassified".
- No inventes continuidad: dos segmentos relacionados solo por el mismo
  tema, sin relación demostrable en el texto, NO forman parte del mismo hilo.
- Un segmento puede estar en varios hilos.
```

### 5.2 Esquema de salida

```json
{
  "segments": [
    {
      "id": "s1",
      "span": [0, 0],
      "topic": "string",
      "phases": [{"phase": "problem|clue|development|contrast|solution|conclusion|unclassified", "weight": 0.0}],
      "depth": 0.0
    }
  ],
  "relations": [
    {
      "from": "s1",
      "to": "s4",
      "kind": "develops|contradicts|anticipates|resumes|exemplifies|resolves|depends_on|unclassified",
      "confidence": 0.0,
      "evidence": [{"span": [0, 0], "quote": "string"}]
    }
  ],
  "threads": [
    {"id": "t1", "topic": "string", "segments": ["s1", "s4", "s9"]}
  ]
}
```

### 5.3 Validación determinista (en el arnés, no en el modelo)

- `span` de cada segmento y de cada evidencia está dentro del documento, alineado a bloques (segmentos) y `contenido[span] == quote` byte a byte. Un fallo descarta esa relación y se cuenta como `evidence_mismatch`.
- Los segmentos de un hilo existen, están ordenados por `ordinal` salvo que el hilo se declare cíclico, y cada salto consecutivo tiene una relación aceptada.
- Pesos y confianzas en `[0, 1]`; `kind` y `phase` pertenecen al conjunto cerrado (cualquier otro valor se rechaza, no se mapea).
- La tasa de `evidence_mismatch` por documento es una métrica de salud del extractor (umbral en §9).

## 6. Protocolo experimental

### 6.1 Corpus

Entre 5 y 6 documentos reales, con licencia que permita su uso, fijados por hash en `docs/research/` o en un directorio de datos ignorado por git si son grandes. Cada tipo ejerce una debilidad distinta:

| # | Tipo | Qué ejerce |
|---|---|---|
| 1 | Técnico (documentación/RFC/manual de un sistema) | Dependencias entre secciones, definiciones lejanas |
| 2 | Investigación (artículo científico) | Planteamiento → método → resultados → limitaciones; contrastes |
| 3 | Narrativo (relato o capítulo largo) | Pistas que se resuelven mucho después; ciclos |
| 4 | Largo (> 100 000 palabras, libro o especificación) | Escala, coste y latencia |
| 5 | Comparativo (informe que contrasta alternativas) | `contradicts`, `contrast`, varias conclusiones |
| 6 | Multipropuesta (p. ej. conjunto de ADR o propuestas con decisiones que se citan entre sí) | Ramas, varias conclusiones, `depends_on` |

El documento 6 puede ser el propio conjunto `docs/adr/` del repositorio como caso de autoconsumo, siempre que no sea el único del tipo «técnico».

### 6.2 Preguntas de prueba

Por documento, 10–15 preguntas (total 50–80), cada una con **respuesta anotada a mano** y la lista mínima de rangos de bytes que la sostienen. Requisito: al menos el 70 % de las preguntas necesitan información dispersa (≥ 2 pasajes separados por contenido no relevante).

Categorías:

1. **Dispersa**: la respuesta combina 2–4 pasajes lejanos («¿cómo se resolvió el problema planteado en la introducción?»).
2. **Contraste**: «¿qué dos posturas se oponen y en qué difieren?».
3. **Seguimiento de causa/dependencia**: «¿qué hay que cumplir antes de X?».
4. **Evolución**: «¿cómo cambia la posición sobre X a lo largo del documento?».
5. **Control local**: la respuesta cabe en un pasaje (≈ 20 %); detecta si los hilos empeoran lo que ya funcionaba.
6. **Sin respuesta**: ≈ 10 %; mide cuánto «ruido» inventa el sistema.

Las preguntas y anotaciones las escribe una persona distinta de quien ajusta el extractor, y se congelan (hash) **antes** de ejecutar ningún sistema. Un segundo anotador valida una muestra del 20 % y se reporta el acuerdo.

### 6.3 Sistemas comparados (baselines)

Mismo modelo de embeddings, mismo presupuesto de contexto final (K tokens entregados al lector) y mismo modelo lector en todos.

| Sistema | Descripción |
|---|---|
| **B1 — chunk + vector** | Chunks de tamaño fijo con solapamiento; top-k por similitud |
| **B2 — vector + expansión de contexto** | Como B1, pero cada acierto se expande a vecinos contiguos (± n chunks o sección contenedora) hasta el presupuesto |
| **T — hilos** | La consulta recupera segmentos semilla (vector sobre segmentos) y se expande siguiendo hilos y relaciones; se ordena por posición y se corta al presupuesto |

Configuración justa: los hiperparámetros de B1 y B2 (tamaño de chunk, k, n) se ajustan sobre un subconjunto de desarrollo (≈ 20 % de las preguntas) con el mismo esfuerzo que el de T; el 80 % restante es de evaluación y no se toca. El coste de extracción de T (tokens y tiempo, una vez por revisión) se informa aparte y también amortizado por consulta.

### 6.4 Métricas

Todas calculadas sobre rangos de bytes frente a las anotaciones; se informan por documento y global, con intervalo de confianza (*bootstrap*) dado el tamaño reducido.

| Métrica | Definición |
|---|---|
| **Precisión** | Bytes recuperados que caen en rangos anotados / bytes recuperados |
| **Cobertura** | Rangos anotados cubiertos (≥ 50 % de su longitud) / rangos anotados. Se informa también la cobertura «completa de la pregunta» (todos los rangos) |
| **Conexiones correctas** | Para T: fracción de saltos de hilo y relaciones usados cuya pareja de segmentos está anotada como conectada y cuyo tipo coincide (se informa tipo y existencia por separado). Para B1/B2, no aplica |
| **Ruido** | Bytes recuperados fuera de todo rango relevante / presupuesto entregado; más la tasa de respuestas dadas en preguntas «sin respuesta» |
| **Latencia** | p50/p95 de consulta (sin extracción) y coste de construcción por documento. Máquina en reposo, ≥ 30 repeticiones, mismo hardware (lección del proyecto: una mediana cercana al umbral bajo carga variable no es un pase) |
| **Calidad de respuesta** (secundaria) | Corrección de la respuesta del lector, evaluada ciegamente por persona sobre una muestra; evita optimizar solo recuperación |

### 6.5 Procedimiento

1. Congelar corpus, preguntas y anotaciones (hashes).
2. Construir los índices (B1/B2 y T) y registrar coste y tasa de `evidence_mismatch`.
3. Ejecutar el subconjunto de desarrollo; ajustar B1/B2/T.
4. Congelar configuración y ejecutar el conjunto de evaluación una sola vez por sistema (y tres veces para estimar variación del extractor).
5. Ejecutar ablaciones (§8) sobre el mismo conjunto de evaluación.
6. Informe con tablas, ejemplos de fallos de cada sistema y la decisión del §9.

## 7. Qué cuenta como «hilo correcto»

Para el análisis cualitativo, un hilo se revisa a mano en una muestra (≥ 30 hilos): cada salto se marca como *justificado* (la evidencia lo sostiene), *plausible sin evidencia* o *inventado*. La tasa de saltos inventados es la medida directa del riesgo de continuidad inventada y entra en el criterio del §9.

## 8. Ablaciones

Se repite la evaluación quitando un componente cada vez, siempre contra T completo:

| Ablación | Qué se quita | Qué responde |
|---|---|---|
| **A1 — sin sectores** | Se ignoran las fases; hilos y relaciones se conservan; la expansión no usa pesos de fase | ¿Aporta valor la ontología de fases o basta con relaciones? |
| **A2 — sin profundidad** | Se descarta `depth` (se sustituye por longitud o por constante) | ¿La profundidad distinta de la longitud mejora la selección o el orden? |
| **A3 — sin hilos** | Solo segmentos + vector (sin expansión por hilo ni por relación) | ¿Aporta algo la estructura de hilos sobre segmentar mejor? |
| A4 (opcional) — sin evidencia obligatoria | Se aceptan saltos sin cita verificada | Cuantifica el coste en ruido de la continuidad inventada |

Interpretación: si A3 ≈ T, el valor viene de la segmentación y no de los hilos; si A1 ≈ T, las fases sobran y se simplifica el modelo; si A2 ≈ T, se elimina el radio.

## 9. Criterio go/no-go para promoverlo a recuperación de primera clase

Se decide con el conjunto de evaluación congelado. **GO** solo si se cumplen **todas** estas condiciones; el umbral numérico lo confirma el propietario **antes** de medir (como ADR-0012), y lo siguiente son valores propuestos:

1. **Mejora relevante**: en preguntas de información dispersa, T supera a B2 (el mejor baseline) en cobertura completa de la pregunta en ≥ 15 puntos porcentuales con el límite inferior del IC al 95 % > 0, **sin** bajar la precisión más de 3 puntos.
2. **Sin regresión local**: en preguntas de control local, T no es peor que B2 en más de 3 puntos.
3. **Conexiones fiables**: ≥ 80 % de los saltos revisados a mano son *justificados* y ≤ 5 % son *inventados*; `evidence_mismatch` ≤ 5 %.
4. **Ruido acotado**: el ruido de T no supera el de B2 en más de 5 puntos y en preguntas sin respuesta no inventa más que B2.
5. **Aportación de la estructura**: A3 queda por debajo de T en la métrica del punto 1 con el mismo criterio de IC; si no, no se promueven los hilos (podría promoverse solo la segmentación).
6. **Coste asumible**: latencia de consulta p95 ≤ 2× la de B2 y coste de construcción amortizable según un presupuesto fijado por el propietario; la reconstrucción de un documento largo (doc. 4) termina en el tiempo fijado.
7. **Estabilidad**: la variación entre tres ejecuciones del extractor es ≤ 5 puntos en la métrica del punto 1.

**NO-GO**: no se cumple (1), (2) o (3). **Resultado intermedio**: se cumplen (1)–(3) pero no (5) o (6): se informa y se promueve, en su caso, solo el componente útil (p. ej. segmentación o expansión por relaciones), no «hilos» como concepto.

Un GO no integra nada por sí mismo: abre una propuesta (ADR) con el diseño de integración, que respeta que el índice es derivado, reconstruible y marcado como inferido (I-39), y que no cambia el formato v1.

## 10. Riesgos

| Riesgo | Descripción | Mitigación / señal |
|---|---|---|
| **Continuidad inventada** | El modelo une pasajes por afinidad temática y fabrica una trayectoria que el autor no escribió | Evidencia textual obligatoria y verificada byte a byte; opción «no clasificada»; revisión manual de saltos (§7); ablación A4; instrucción explícita en el prompt |
| **Sectores rígidos** | La ontología de fases no encaja en documentos técnicos, tablas o referencia (no narrativos) y fuerza etiquetas falsas | Pesos múltiples, `unclassified`, ciclos y varias conclusiones; ablación A1; informar tasa de `unclassified` por tipo de documento; la ontología no es universal y puede reemplazarse por tipo de documento |
| **Profundidad ≈ longitud** | El radio acaba correlacionando con el tamaño y no aporta | Medir correlación; ablación A2 |
| **Fuga de evaluación** | Ajustar el extractor mirando las preguntas | Congelar preguntas y anotaciones antes; anotador distinto; subconjunto de desarrollo separado |
| **Baselines débiles** | Comparar con un B1 mal ajustado infla el resultado | Mismo esfuerzo de ajuste; B2 como referencia principal |
| **Muestra pequeña** | 5–6 documentos no generalizan | IC por *bootstrap*; informar por tipo; no extrapolar fuera de los tipos probados |
| **No determinismo / deriva del modelo** | Cambios del modelo alteran el índice | Versión de modelo y prompt en la procedencia; tres ejecuciones; temperatura 0; salida cruda guardada |
| **Coste de extracción** | Documentos largos son caros | Informar coste aparte y amortizado; extracción por ventanas con solape en el doc. 4 |
| **Acoplamiento con el núcleo** | Que el experimento condicione la Etapa 3 | Vive fuera del workspace del motor, solo lee revisiones, sin cambios de formato; ningún entregable de Etapa 3 depende de él |
| **Antropomorfismo** | Leer la metáfora del cilindro o del cerebro como semántica | El cilindro es visualización; los términos se definen por contrato técnico (ADR-0017) |

## 11. Entregables y decisiones abiertas

**Entregables:** corpus y preguntas congeladas (con hashes), arnés de extracción y validación, índice derivado, informe con tablas y el veredicto del §9.

**Decisiones abiertas para el propietario:**

- Confirmar o ajustar los umbrales del §9 antes de medir.
- Elegir el modelo de extracción y el modelo lector (y su presupuesto de coste).
- Aprobar el corpus concreto y la licencia de cada documento.
- Presupuesto de coste de construcción aceptable para el punto 6 del §9.
