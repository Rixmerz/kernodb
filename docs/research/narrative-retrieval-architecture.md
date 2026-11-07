# Informe arquitectónico — recuperación narrativa en Kerno

| Campo | Valor |
|---|---|
| Estado | Borrador para el propietario (sin código, sin cambios de formato ni de ADR) |
| Fecha | 2026-11-07 |
| Base | `main` en `6505562` (Etapa 5 cerrada, #24 corregido) |
| Fuentes revisadas | `docs/research/narrative-threads.md`, `docs/roadmap/README.md`, ADR-0004, 0007, 0008, 0009, 0011, 0014, 0016, 0017, 0018, `docs/architecture/invariants.md`, SAD, `crates/kerno-units/src` |

## 1. Diagnóstico del estado actual

### 1.1 Implementado (con pruebas)

| Pieza | Dónde | Lo que aporta a la recuperación narrativa |
|---|---|---|
| Unidad y revisión con hash de contenido; la carga útil son bytes opacos | `kerno-units` (`types.rs`, `codec.rs`); ADR-0008 aceptado | El texto original es la fuente de verdad y tiene una identidad exacta `(unit_id, revision_id, content_hash)`: sirve de clave para un derivado y para verificar citas byte a byte |
| Historial y retención de revisiones | `history.rs`, `retention.rs`; ADR-0004 | Una revisión antigua sigue siendo legible hasta que la retención la quita: un índice narrativo puede seguir siendo válido para la revisión que describe |
| Relaciones tipadas con procedencia `Declared`/`Imported`/`Algorithmic`/`Inferred` y confianza | `db.rs` (`Relation`, `relations_of`), `codec.rs`; ADR-0014 aceptado; I-39 verificada | Lo inferido ya no puede presentarse como declarado. **Pero** una relación une dos *unidades* (`from`, `to`: `UnitId`), no dos rangos de una revisión, y sobrevive a nuevas revisiones |
| Índice derivado con marca de agua y estados `Ready`/`Stale`/`Failed`/`Building` | `index.rs`; I-23, I-24 | El patrón de frescura que un índice narrativo debe copiar |
| Planificador determinista con ruta de referencia, huella y observaciones | `planner.rs`, `fingerprint.rs`, `observation.rs`; Etapa 5 | Ruta de referencia y prueba diferencial (I-32). Es **planificación de acceso**, no comprensión: no debe mezclarse con lo narrativo |

### 1.2 Solo documentado

- **Diseño del experimento** (`narrative-threads.md`), completo: modelo conceptual (altura, sector, radio, hilo), modelo de datos en Rust ilustrativo, extracción en dos pasadas, validación determinista de evidencias, protocolo con B1/B2/T, métricas, ablaciones A1–A4, criterio go/no-go y riesgos. Estado: aprobado como experimento independiente, sin ejecutar.
- **Contratos que lo rodean, sin aceptar:** ADR-0011 (vectores y recuperación híbrida, *pendiente*), ADR-0017 (semántica de la unidad, *propuesto*), ADR-0018 (versionado y reconstrucción de derivados, *propuesto*).
- **Etapas futuras:** Etapa 7 (memoria asociativa: recuperación por relaciones, búsqueda vectorial exacta, ranking con modo diagnóstico), Etapa 8 (registro común de derivados, embeddings fuera del núcleo).
- **Invariantes pendientes:** I-25 (embedding → revisión y perfil exactos), I-38 (derivado → fuentes, método y versión), I-31 (el núcleo no necesita red ni modelo).

### 1.3 Falta

1. **Ninguna línea de código** del experimento: ni arnés, ni corpus, ni preguntas, ni baselines.
2. **El roadmap no menciona la recuperación narrativa.** Solo vive en `docs/research/`. Es el riesgo principal de que el proyecto la pierda: ninguna etapa la consume ni la exige.
3. **Direccionamiento por rangos.** Kerno no tiene ningún objeto que apunte a un rango de bytes de una revisión, que es la unidad mínima de un segmento y de una evidencia.
4. **Registro de derivados (Etapa 8)** y **embeddings/vectores (ADR-0011)**: los dos prerrequisitos de integrar algo en el motor.
5. **Interfaz de adaptadores de IA** (LLM, embeddings): no existe ni como contrato.
6. **Decisiones del propietario** abiertas en §11 del diseño: umbrales go/no-go, modelos, corpus, presupuesto de coste.

### 1.4 Un desajuste que hay que fijar ya

Las relaciones narrativas (`develops`, `contradicts`…) son entre **rangos de una revisión concreta** y valen solo para esa revisión. Las relaciones de `kerno-units` son entre **unidades** y persisten entre revisiones. Guardar relaciones narrativas como relaciones de `kerno-units` sería incorrecto: una edición del documento dejaría vivas relaciones sobre texto que ya no existe. El diseño actual ya lo evita («no se escriben como `Edge`»); la arquitectura debe convertirlo en regla.

## 2. Arquitectura recomendada

### 2.1 Decisión propuesta

**Un subsistema de recuperación fuera del núcleo, en dos pasos:**

1. **Ahora:** el experimento como **programa independiente fuera del workspace del motor** (`experiments/narrative-threads/`), que solo lee revisiones y guarda su índice en ficheros propios. Es lo que ya decía el diseño, y es lo único que permite medir sin comprometer nada.
2. **Solo tras un GO:** un **crate propio** (p. ej. `kerno-narrative`) que depende de `kerno-units` y del registro de derivados de la Etapa 8, nunca al revés. Los modelos entran por **adaptadores** que viven en procesos o crates aparte. El motor no conoce el crate.

```
              ┌───────────────────────────── fuera del núcleo ─────────────────────────────┐
consulta ──▶  │ recuperación (Etapa 7)  ──▶  expansor narrativo (opcional) ──▶ contexto       │
              │        │  baseline exacto + relaciones          │ lee índice derivado         │
              │        ▼                                        ▼                             │
              │  adaptador de embeddings              índice narrativo (derivado)            │
              │                                       ◀── job de extracción ◀── adaptador LLM │
              └──────────────────────┬──────────────────────────┬──────────────────────────────┘
                                     │ lee revisiones            │ registra derivado (Etapa 8)
              ┌──────────────────────▼──────────────────────────▼──────────────┐
              │ núcleo: kerno-units (unidades, revisiones, relaciones, índices) │
              │         kerno-storage (motor)        — sin red, sin modelos (I-31) │
              └──────────────────────────────────────────────────────────────────┘
```

### 2.2 Alternativas descartadas

| Alternativa | Por qué no |
|---|---|
| **Módulo de `kerno-units`** | Mete semántica de IA en la capa de almacenamiento. Arrastra dependencias de modelos hacia el núcleo (choca con I-31 y D-08). Obliga a cambiar el formato v1 para guardar segmentos. Y ata el ritmo del núcleo a un experimento que puede salir NO-GO |
| **Parte del planificador / adaptación operacional** | El planificador decide *cómo leer* datos ya identificados y debe dar el mismo resultado que la ruta de referencia (I-32). La recuperación narrativa decide *qué es relevante*, y su resultado cambia a propósito. Mezclarlos rompería la prueba diferencial y confundiría ADR-0016 con comprensión |
| **Motor separado con su propio almacenamiento** | Duplica la fuente de verdad, pierde la procedencia exacta, la supresión (ADR-0009) y la invalidación por revisión. Kerno quedaría reducido a un almacén de blobs |
| **Subsistema dentro del núcleo, pero aislado** | Mejor que un módulo, pero sigue compilándose con el motor y presiona para tocar el formato. No hay ganancia frente a un crate aparte que dependa del núcleo |

El crate independiente que usa Kerno como almacenamiento es, en la práctica, la opción «motor separado que usa Kerno», pero sin almacén propio: guarda todo como derivado de Kerno.

## 3. Relación con el resto de Kerno

| Pieza | Relación |
|---|---|
| **Embeddings / ADR-0011** | Los segmentos se embeben como cualquier derivado (I-25): vector + revisión fuente + perfil de modelo. Sirven para encontrar **segmentos semilla**. ADR-0011 debe decidirse antes de integrar; el experimento puede usar fuerza bruta en memoria |
| **Relaciones (ADR-0014)** | Las relaciones narrativas viven **dentro del índice derivado**, nunca como relaciones de `kerno-units`. Excepción futura, a decidir en un ADR: proyectar como relación `Inferred` entre unidades un vínculo *entre documentos* (p. ej. un ADR que resuelve otro), con su confianza y evidencia |
| **Índices secundarios (ADR-0007)** | El índice narrativo copia el patrón de estados `Ready`/`Stale`/`Failed`/`Building`, pero su «marca de agua» es la revisión y el perfil del extractor, no la secuencia de commit |
| **Memoria asociativa (Etapa 7)** | Es el consumidor natural. La recuperación de la Etapa 7 (vector exacto + relaciones + contexto temporal) es el **fallback**. Lo narrativo es una **señal y un expansor** más de su ranking, explicable en su modo diagnóstico |
| **Registro de derivados (Etapa 8, ADR-0018)** | Prerrequisito de la integración: el índice narrativo se registra allí con sus fuentes, método y versión (I-38) |
| **Planificador** | Ninguna. Las observaciones del planificador no entrenan lo narrativo y lo narrativo no cambia planes. Si un día se cachea una recuperación narrativa, esa caché es adaptación operacional y la gobierna ADR-0016, aparte |

## 4. Modelo de datos

Parte del esbozo de `narrative-threads.md` §4 (segmento, hilo, relación, procedencia, `Origin::Inferred`), con estos añadidos:

```rust
/// Identidad exacta de lo que se analizó. Clave del índice.
pub struct Source {
    pub unit_id: UnitId,
    pub revision_id: RevisionId,
    pub content_hash: [u8; 32],   // el de la revisión (ADR-0008)
}

/// Todo lo que, al cambiar, obliga a reconstruir.
pub struct ExtractorProfile {
    pub segmenter_version: u32,     // cómo se numeran los bloques (determinista)
    pub model: String,              // id exacto del modelo, nunca un alias
    pub prompt_version: String,
    pub schema_version: u32,        // esquema JSON de salida
    pub validator_version: u32,     // reglas deterministas de evidencia
    pub embedder: Option<String>,   // perfil de embeddings de los segmentos
}

pub enum IndexStatus { Ready, Stale, Failed { reason: String }, Building }

pub struct NarrativeIndex {
    pub source: Source,
    pub profile: ExtractorProfile,
    pub status: IndexStatus,
    pub raw_output_hash: [u8; 32],  // salida cruda guardada para auditoría (ADR-0018)
    pub windows: Vec<WindowSpan>,   // ventanas usadas en documentos largos
    pub segments: Vec<NarrativeSegment>,
    pub threads: Vec<NarrativeThread>,
    pub relations: Vec<NarrativeRelation>,
    pub rejected: RejectionCounts,  // evidence_mismatch, kind desconocido…
}
```

Reglas:

- Todo rango (`span`) es de bytes sobre la revisión de `source`. Una evidencia se acepta solo si `contenido[span] == quote` byte a byte.
- Una relación sin evidencia aceptada no existe; un hilo sin relación aceptada en cada salto no existe.
- Todo es `Inferred`; nada de esto se muestra como declarado (I-39).
- Nada se guarda en el formato v1 durante el experimento. Tras un GO, el almacenamiento sigue la decisión de la Etapa 8 sobre derivados.

## 5. Flujos

### 5.1 Indexación (job externo, por revisión)

1. Leer la revisión de Kerno y comprobar su `content_hash`.
2. **Segmentador determinista** (código, no modelo): bloques con su rango de bytes.
3. Si el documento no cabe en el contexto del modelo: ventanas solapadas por bloques, más una pasada de unión entre ventanas que solo puede proponer relaciones con evidencia en ambas.
4. **Pasada 1 (LLM):** agrupar bloques en segmentos; fases con peso, profundidad, tema.
5. **Pasada 2 (LLM):** relaciones e hilos sobre la lista de segmentos.
6. **Validación determinista:** rangos, citas byte a byte, conjuntos cerrados, pesos en [0, 1], saltos de hilo con relación. Lo que no pasa se descarta y se cuenta; nunca se repara.
7. Embeber segmentos con el adaptador de embeddings.
8. Registrar el derivado con `Source`, `ExtractorProfile`, salida cruda y estado `Ready`.

### 5.2 Recuperación

1. Consulta → vector (adaptador de embeddings) e **intención narrativa** (clasificador: dispersa, contraste, causa, evolución, local, sin respuesta). La intención solo pondera; nunca filtra.
2. **Baseline siempre:** recuperación de la Etapa 7 (vector exacto + relaciones + filtros).
3. Si hay índice `Ready` para la revisión del snapshot: segmentos semilla → sectores relevantes → hilos que los contienen → expansión por relaciones, con contexto anterior y posterior de cada segmento.
4. Ensamblar en orden documental, conservando **varias ramas, contradicciones y conclusiones** como recorridos separados, hasta el presupuesto de tokens.
5. Cada fragmento devuelto lleva su procedencia: rango, revisión, hilo, relación y confianza que lo trajo (modo diagnóstico de la Etapa 7).
6. **Fallback:** si no hay índice, está `Stale`/`Failed`, o el expansor falla, se devuelve el baseline y se registra por qué.

## 6. Versionado e invalidación

| Cambio | Efecto |
|---|---|
| Nueva revisión del documento | El índice de la revisión anterior **sigue siendo válido para ella** (snapshots e historial). Para la nueva no hay índice: `Stale` hasta reconstruir. Nunca se «parchea» un índice viejo con la nueva revisión |
| Cambio de cualquier campo de `ExtractorProfile` | Todos los índices de ese perfil quedan `Stale`; se reconstruyen bajo demanda o en lote. Pueden convivir dos perfiles durante la migración, y se elige uno por consulta |
| Retención quita una revisión | Su índice narrativo se borra con ella |
| Supresión (ADR-0009) | Borra segmentos, citas, embeddings, salida cruda y relaciones entre documentos que la nombren, en el mismo paso lógico que la fuente |
| Relación entre documentos (si existe) | Obsoleta en cuanto cambia cualquiera de los dos lados |
| No determinismo del modelo | Se guarda el hash de la salida, no solo de la entrada (ADR-0018). Reconstruir puede dar otro índice: es una versión nueva, no un error |

La invalidación es por **identidad exacta**, no por tiempo ni por heurística: o `(source, profile)` coincide, o el índice no se usa.

## 7. Adaptadores: modelos intercambiables sin acoplar el núcleo

Cuatro puertos, cada uno un contrato pequeño, implementados fuera del núcleo:

| Puerto | Entrada → salida | Identidad que declara |
|---|---|---|
| `Segmenter` | texto → bloques con rango | versión (determinista, sin modelo) |
| `NarrativeExtractor` | bloques → JSON del esquema | modelo, prompt, esquema |
| `Embedder` | texto → vector | modelo, dimensión, métrica, normalización |
| `IntentClassifier` (opcional) | consulta → pesos de intención | modelo o reglas |

- El **validador** no es un puerto: es código fijo de Kerno, para que ningún modelo decida qué evidencia es válida.
- Cada adaptador puede ser un proceso (JSON por stdin/stdout o HTTP local). Cambiar de proveedor es cambiar de adaptador y, por tanto, de perfil: invalida lo construido con el anterior.
- El núcleo no compila contra ningún adaptador (I-31). Sin adaptadores, la recuperación sigue funcionando con lo que no requiere modelo.

## 8. Riesgos y costes

| Riesgo | Mitigación |
|---|---|
| Continuidad inventada | Evidencia byte a byte obligatoria, abstención explícita, revisión manual de saltos, ablación A4 |
| Ontología de fases que no encaja (referencia, tablas, código) | Pesos múltiples, `unclassified`, ablación A1; ontología por tipo de documento si hace falta |
| Profundidad que solo refleja longitud | Medir correlación; ablación A2 |
| Hilos que no aportan sobre una buena segmentación | Ablación A3; promover solo la segmentación si es lo que gana |
| Coste de extracción | Documento largo (~100 000 palabras ≈ 130 000 tokens) × 2 pasadas + ventanas con solape: del orden de 300 000–400 000 tokens de entrada por revisión. Se informa total y amortizado por consulta. Solo se reextrae al cambiar la revisión o el perfil |
| Latencia de consulta | Expansión acotada por presupuesto; criterio p95 ≤ 2× B2 |
| Deriva y no determinismo del modelo | Perfil exacto, temperatura 0, salida cruda, tres ejecuciones |
| Fuga de evaluación | Preguntas congeladas por hash antes de ejecutar, anotador distinto |
| Arrastre hacia el núcleo | Fuera del workspace hasta un GO; integración solo vía ADR |
| **Pérdida de la idea en el roadmap** | Añadirla como pista de investigación con dueño y puntos de enganche (§10) |

## 9. Experimento de validación

Se mantiene el protocolo de `narrative-threads.md` §6–§9, con estos ajustes:

1. **Corpus (6 documentos reales, licencia verificada, fijados por hash):** técnico (RFC o manual), artículo científico, relato o capítulo largo, documento > 100 000 palabras, informe comparativo, y el conjunto `docs/adr/` de Kerno como caso de autoconsumo.
2. **Preguntas:** 10–15 por documento (50–80), ≥ 70 % dispersas; 20 % de control local y 10 % sin respuesta; anotadas con sus rangos por una persona distinta de quien ajusta el extractor; 20 % revisado por un segundo anotador.
3. **Sistemas, con el mismo embedder, lector y presupuesto de tokens:**
   - **B1:** chunks fijos + vector top-k.
   - **B2:** B1 + expansión de contexto (vecinos o sección contenedora). Es el baseline a batir.
   - **T:** semillas por vector sobre segmentos + expansión por hilos y relaciones.
   - **Ablaciones A1–A4** sobre T.
4. **Métricas:** precisión y cobertura por bytes, cobertura completa de la pregunta, conexiones correctas, ruido, respuestas inventadas en preguntas sin respuesta, latencia p50/p95 (≥ 30 repeticiones, máquina en reposo), coste de construcción, `evidence_mismatch`; IC por bootstrap.
5. **Decisión:** el go/no-go del §9 del diseño, con umbrales que confirmas **antes** de medir.
6. **Lugar:** `experiments/narrative-threads/`, fuera del workspace del motor. Puede usar dependencias propias porque no se compila con Kerno.

## 10. Impacto en el roadmap y ADR necesarios

**Roadmap (propuesta, sin tocar etapas en curso):**

- Añadir una **pista de investigación R-1 «Recuperación narrativa»** en paralelo, que no bloquea ninguna etapa. Dependencias: lee revisiones (Etapa 3, hecha). Salida: informe go/no-go.
- Enganchar su posible integración a la **Etapa 7** (consumidor, como señal del ranking con fallback al baseline) y a la **Etapa 8** (registro de derivados), sin cambiar sus criterios de aceptación actuales.

**ADR:**

| ADR | Cuándo | Contenido |
|---|---|---|
| Decidir **ADR-0018** (derivados) | Antes de cualquier integración | Contrato común de derivados que el índice narrativo usará |
| Decidir **ADR-0011** (vectores) | Antes de la Etapa 7 | Semillas por vector y baseline exacto |
| Nuevo **ADR-0019 «Recuperación narrativa como derivado»** | Solo tras un GO | Crate fuera del núcleo, índice por `(revisión, perfil)`, relaciones narrativas fuera de ADR-0014, fallback obligatorio, procedencia en la respuesta |
| Nuevo **ADR-0020 «Puertos de adaptadores de IA»** | Con la Etapa 8 (también lo necesitan los embeddings) | Contratos de `Segmenter`, `NarrativeExtractor`, `Embedder`; perfiles; ningún adaptador en el núcleo |

No se cambia ningún ADR aceptado ni el formato v1. ADR-0016 queda al margen: es adaptación operacional y no gobierna nada de esto.

## 11. Respuesta corta

Sí, la arquitectura actual va en la dirección correcta para esta capacidad. El texto ya es la fuente de verdad verificable. Las relaciones inferidas ya no se confunden con las declaradas. Los derivados ya tienen un patrón de frescura. Y el experimento ya está bien diseñado.

Hay tres ausencias que hay que corregir para no perder la idea:

1. No figura en el roadmap.
2. No hay forma de nombrar un rango de una revisión.
3. Sus prerrequisitos de integración (ADR-0018, ADR-0011) siguen sin decidir.

Decisiones que te tocan para arrancar el experimento: umbrales del go/no-go, modelos de extracción y lectura, corpus y su licencia, y presupuesto de coste.
