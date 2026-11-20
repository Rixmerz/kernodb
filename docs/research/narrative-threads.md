# Experiment design — retrieval by narrative threads

| Field | Value |
|---|---|
| Status | Experiment proposal (approved by the owner as an independent experiment) |
| Date | 2026-10-31 |
| Scope | **Research.** It is not part of the storage core and it is **not a requirement of Stage 3** |
| Depends on | Units, revisions and relations of Stage 3 (read only); no change to the on-disk format |
| Related to | [architecture specification v0.2.0](../spec/architecture-specification-v0.2.0.md); ADR-0017 (unit); invariant I-39 (what is inferred keeps its origin and confidence and is not presented as declared) |

## 1. Summary and scope

**Hypothesis.** For questions whose answer is spread across a document, the appropriate retrieval unit is not the fragment (*chunk*) but a **thematic trajectory**: a sequence, not necessarily contiguous, of segments that develop the same subject along the document.

**What it is.** A **derived and rebuildable** index over the units, revisions and relations of Stage 3. It can be deleted and regenerated from the content without losing information (spec: derived artifacts carry provenance).

**What it is not.**

- It does not touch the WAL, the v1 format or `kerno-storage`; it does not block Stage 3.
- It is not a universal ontology: the narrative phases are a working hypothesis (see §10).
- The "cylinder" is only a **visualisation** of the model; it does not impose any data or query structure.
- An inferred relation is never presented as declared (I-39).

**Expected result.** A report with metrics, ablations and a go/no-go decision (§9). A NO-GO is a valid result and is kept.

## 2. Conceptual model

| Axis (cylinder) | Concept | Definition |
|---|---|---|
| Height | Document position | Order and byte offset of the segment within the revision |
| Sector | Narrative phase | **Weighted** and multiple labels: `problem`, `clue`, `development`, `contrast`, `solution`, `conclusion`, plus `unclassified` |
| Radius | Thematic depth | How much the segment treats a topic in detail, **distinct from the length** (a short paragraph may be dense; a long one, empty) |
| Thread | Trajectory | Ordered sequence of non-contiguous segments. A segment may belong to several threads |

Rules of the model:

1. Phases admit **cycles, branches and several conclusions**; a document may have no `solution` or three. No canonical order is required.
2. A phase label is a weight in `[0, 1]`; the sum need not be 1. Any low weight (threshold `tau_phase`) is resolved as `unclassified`.
3. Depth is estimated from the content (distinct entities, definitions, figures, steps, arguments), not from the number of tokens. Its correlation with the length is evaluated: if it is high (> 0.9), the dimension adds nothing and is discarded (see the ablation in §8).
4. Typed relations between segments: `develops`, `contradicts`, `anticipates`, `resumes`, `exemplifies`, `resolves`, `depends_on`. Each with **confidence** and **textual evidence** (exact offsets). There is an explicit option "no relation / unclassified": the extractor may abstain.
5. A thread is accepted only if each hop between consecutive segments has a relation with evidence. Without evidence there is no hop (see the risk of invented continuity, §10).

## 3. Relation to Stage 3

The index is fed with what Stage 3 already defines and does not add fields to those types.

| Stage 3 input | Use in the experiment |
|---|---|
| `Unit` (document) | Root: a thread index per unit |
| `Revision` (content hash, ADR-0008) | Cache key: `(unit_id, revision_id, extractor_version)`. When the revision changes, the index of that revision becomes obsolete and is rebuilt |
| Relations (Stage 4, when they exist) | Optional. While they do not exist, the narrative relations live only inside the derived index and are not written as `Edge` |
| Provenance | Each segment and relation stores the model, the prompt version and the date |

Rebuilding: `rebuild(unit_id, revision_id)` is a pure function of the content and of the extractor version, except for the non-determinism of the model, which is mitigated by fixing temperature 0 and storing the raw output for auditing. During the experiment the index is stored **outside** `kerno-storage` (JSON Lines files or a local SQLite of the test bench); integrating it into the engine would be a decision after the go (§9).

## 4. Minimal data model (Rust)

Sketch for the experiment's crate (e.g. `experiments/narrative-threads`, outside the engine's workspace). It is illustrative and is not compiled in the repository.

```rust
use std::collections::BTreeMap;
use std::ops::Range;

/// Opaque identifiers; `unit` and `revision` come from Stage 3.
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

/// Phase with a weight in [0, 1]. Several per segment.
pub struct PhaseWeight {
    pub phase: Phase,
    pub weight: f32,
}

/// Exact byte range over the content of the revision.
pub type ByteSpan = Range<u64>;

pub struct NarrativeSegment {
    pub id: SegmentId,
    pub unit_id: [u8; 16],
    pub revision_id: [u8; 16],
    /// Document position ("height"): order and offset.
    pub ordinal: u32,
    pub span: ByteSpan,
    /// Weighted phases; empty is equivalent to `Unclassified`.
    pub phases: Vec<PhaseWeight>,
    /// Thematic depth ("radius"), in [0, 1]; independent of `span.len()`.
    pub depth: f32,
    pub topic: String,
    pub provenance: Provenance,
}

pub struct NarrativeThread {
    pub id: ThreadId,
    pub unit_id: [u8; 16],
    pub revision_id: [u8; 16],
    pub topic: String,
    /// Segments in reading order of the thread; not contiguous; a segment
    /// may appear in several threads.
    pub segments: Vec<SegmentId>,
    /// Aggregate confidence: minimum of the relations that sustain it.
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
    /// The extractor abstains: no demonstrable relation.
    Unclassified,
}

pub struct NarrativeRelation {
    pub from: SegmentId,
    pub to: SegmentId,
    pub kind: RelationKind,
    pub confidence: f32,
    /// Textual evidence: quotes with exact offsets. Mandatory
    /// except for `Unclassified`.
    pub evidence: Vec<ByteSpan>,
    pub provenance: Provenance,
}

pub struct Provenance {
    pub model: String,
    pub prompt_version: String,
    pub extracted_at_unix_ms: u64,
    /// Origin mark: always `Inferred` (I-39).
    pub origin: Origin,
}

pub enum Origin {
    Inferred,
}

/// Derived index; it is discarded and rebuilt per revision.
pub struct ThreadIndex {
    pub segments: BTreeMap<u64, NarrativeSegment>,
    pub threads: BTreeMap<u64, NarrativeThread>,
    pub relations: Vec<NarrativeRelation>,
}
```

Design notes: `ByteSpan` avoids complex return types (clippy `type_complexity`, see the repository conventions); no structure requires the phases to add up to 1 or the threads to be contiguous; `Origin` has a single variant on purpose, so that what is inferred is not confused with what is declared.

## 5. Extraction: prompt and output schema

The extraction runs in two passes, to bound the cost and to be able to measure them separately.

1. **Segmentation and labelling**: the document is given with the byte offsets of each paragraph/block (the text is numbered by the harness, not by the model). The model groups blocks into segments and labels phase, depth and topic.
2. **Relations and threads**: with the list of segments (id, topic, phases, one-line summary) the model proposes relations and threads; the harness verifies the evidence.

### 5.1 Prompt (schema; versioned as `prompt_version`)

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

### 5.2 Output schema

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

### 5.3 Deterministic validation (in the harness, not in the model)

- The `span` of each segment and of each evidence is within the document, aligned to blocks (segments), and `contenido[span] == quote` byte by byte. A failure discards that relation and is counted as `evidence_mismatch`.
- The segments of a thread exist, are ordered by `ordinal` unless the thread is declared cyclic, and each consecutive hop has an accepted relation.
- Weights and confidences in `[0, 1]`; `kind` and `phase` belong to the closed set (any other value is rejected, not mapped).
- The rate of `evidence_mismatch` per document is a health metric of the extractor (threshold in §9).

## 6. Experimental protocol

### 6.1 Corpus

Between 5 and 6 real documents, with a licence that allows their use, pinned by hash in `docs/research/` or in a data directory ignored by git if they are large. Each type exercises a different weakness:

| # | Type | What it exercises |
|---|---|---|
| 1 | Technical (documentation/RFC/manual of a system) | Dependencies between sections, distant definitions |
| 2 | Research (scientific article) | Statement → method → results → limitations; contrasts |
| 3 | Narrative (story or long chapter) | Clues resolved much later; cycles |
| 4 | Long (> 100 000 words, book or specification) | Scale, cost and latency |
| 5 | Comparative (report that contrasts alternatives) | `contradicts`, `contrast`, several conclusions |
| 6 | Multi-proposal (e.g. a set of ADRs or proposals whose decisions cite each other) | Branches, several conclusions, `depends_on` |

Document 6 may be the repository's own `docs/adr/` set as a self-consumption case, provided it is not the only one of the "technical" type.

### 6.2 Test questions

Per document, 10–15 questions (total 50–80), each with a **hand-annotated answer** and the minimum list of byte ranges that support it. Requirement: at least 70 % of the questions need scattered information (≥ 2 passages separated by non-relevant content).

Categories:

1. **Scattered**: the answer combines 2–4 distant passages ("how was the problem raised in the introduction resolved?").
2. **Contrast**: "which two positions oppose each other and how do they differ?".
3. **Cause/dependency tracking**: "what must be met before X?".
4. **Evolution**: "how does the position on X change along the document?".
5. **Local control**: the answer fits in one passage (≈ 20 %); detects whether the threads make worse what already worked.
6. **Unanswerable**: ≈ 10 %; measures how much "noise" the system invents.

The questions and annotations are written by a person other than whoever tunes the extractor, and they are frozen (hash) **before** running any system. A second annotator validates a 20 % sample and the agreement is reported.

### 6.3 Compared systems (baselines)

Same embedding model, same final context budget (K tokens delivered to the reader) and same reader model in all of them.

| System | Description |
|---|---|
| **B1 — chunk + vector** | Fixed-size chunks with overlap; top-k by similarity |
| **B2 — vector + context expansion** | As B1, but each hit is expanded to contiguous neighbours (± n chunks or the containing section) up to the budget |
| **T — threads** | The query retrieves seed segments (vector over segments) and is expanded following threads and relations; it is ordered by position and cut at the budget |

Fair configuration: the hyperparameters of B1 and B2 (chunk size, k, n) are tuned on a development subset (≈ 20 % of the questions) with the same effort as that of T; the remaining 80 % is for evaluation and is not touched. The extraction cost of T (tokens and time, once per revision) is reported separately and also amortised per query.

### 6.4 Metrics

All computed over byte ranges against the annotations; they are reported per document and overall, with a confidence interval (*bootstrap*) given the small size.

| Metric | Definition |
|---|---|
| **Precision** | Retrieved bytes that fall in annotated ranges / retrieved bytes |
| **Coverage** | Annotated ranges covered (≥ 50 % of their length) / annotated ranges. The "full coverage of the question" (all the ranges) is also reported |
| **Correct connections** | For T: fraction of the thread hops and relations used whose pair of segments is annotated as connected and whose type matches (type and existence are reported separately). For B1/B2, not applicable |
| **Noise** | Retrieved bytes outside any relevant range / delivered budget; plus the rate of answers given to "unanswerable" questions |
| **Latency** | p50/p95 of the query (without extraction) and construction cost per document. Machine at rest, ≥ 30 repetitions, same hardware (a lesson of the project: a median close to the threshold under variable load is not a pass) |
| **Answer quality** (secondary) | Correctness of the reader's answer, evaluated blindly by a person on a sample; it avoids optimising only retrieval |

### 6.5 Procedure

1. Freeze corpus, questions and annotations (hashes).
2. Build the indexes (B1/B2 and T) and record the cost and the rate of `evidence_mismatch`.
3. Run the development subset; tune B1/B2/T.
4. Freeze the configuration and run the evaluation set once per system (and three times to estimate the variation of the extractor).
5. Run the ablations (§8) on the same evaluation set.
6. Report with tables, examples of failures of each system and the decision of §9.

## 7. What counts as a "correct thread"

For the qualitative analysis, a thread is reviewed by hand on a sample (≥ 30 threads): each hop is marked as *justified* (the evidence sustains it), *plausible without evidence* or *invented*. The rate of invented hops is the direct measure of the risk of invented continuity and enters the criterion of §9.

## 8. Ablations

The evaluation is repeated removing one component each time, always against the full T:

| Ablation | What is removed | What it answers |
|---|---|---|
| **A1 — no sectors** | The phases are ignored; threads and relations are kept; the expansion does not use phase weights | Does the phase ontology add value or are relations enough? |
| **A2 — no depth** | `depth` is discarded (replaced by length or by a constant) | Does depth, distinct from length, improve the selection or the order? |
| **A3 — no threads** | Only segments + vector (no expansion by thread or by relation) | Does the thread structure add anything over better segmentation? |
| A4 (optional) — no mandatory evidence | Hops without a verified quote are accepted | Quantifies the cost in noise of invented continuity |

Interpretation: if A3 ≈ T, the value comes from the segmentation and not from the threads; if A1 ≈ T, the phases are superfluous and the model is simplified; if A2 ≈ T, the radius is eliminated.

## 9. Go/no-go criterion for promoting it to first-class retrieval

The decision is taken with the frozen evaluation set. **GO** only if **all** these conditions are met; the numerical threshold is confirmed by the owner **before** measuring (as in ADR-0012), and the following are proposed values:

1. **Relevant improvement**: on scattered-information questions, T beats B2 (the best baseline) in full coverage of the question by ≥ 15 percentage points with the lower bound of the 95 % CI > 0, **without** lowering the precision by more than 3 points.
2. **No local regression**: on local-control questions, T is not worse than B2 by more than 3 points.
3. **Reliable connections**: ≥ 80 % of the hops reviewed by hand are *justified* and ≤ 5 % are *invented*; `evidence_mismatch` ≤ 5 %.
4. **Bounded noise**: the noise of T does not exceed that of B2 by more than 5 points and, on unanswerable questions, it does not invent more than B2.
5. **Contribution of the structure**: A3 stays below T on the metric of point 1 with the same CI criterion; if not, the threads are not promoted (only the segmentation might be promoted).
6. **Affordable cost**: query latency p95 ≤ 2× that of B2 and construction cost amortisable according to a budget fixed by the owner; the rebuild of a long document (doc. 4) finishes within the fixed time.
7. **Stability**: the variation between three runs of the extractor is ≤ 5 points on the metric of point 1.

**NO-GO**: (1), (2) or (3) is not met. **Intermediate result**: (1)–(3) are met but not (5) or (6): it is reported and, where appropriate, only the useful component is promoted (e.g. the segmentation or the expansion by relations), not "threads" as a concept.

A GO does not integrate anything by itself: it opens a proposal (ADR) with the integration design, which respects that the index is derived, rebuildable and marked as inferred (I-39), and that it does not change the v1 format.

## 10. Risks

| Risk | Description | Mitigation / signal |
|---|---|---|
| **Invented continuity** | The model joins passages by thematic affinity and fabricates a trajectory that the author did not write | Mandatory textual evidence verified byte by byte; "unclassified" option; manual review of hops (§7); ablation A4; explicit instruction in the prompt |
| **Rigid sectors** | The phase ontology does not fit technical documents, tables or reference material (non-narrative) and forces false labels | Multiple weights, `unclassified`, cycles and several conclusions; ablation A1; report the rate of `unclassified` per document type; the ontology is not universal and may be replaced by document type |
| **Depth ≈ length** | The radius ends up correlating with size and adds nothing | Measure the correlation; ablation A2 |
| **Evaluation leakage** | Tuning the extractor while looking at the questions | Freeze questions and annotations beforehand; different annotator; separate development subset |
| **Weak baselines** | Comparing with a badly tuned B1 inflates the result | Same tuning effort; B2 as the main reference |
| **Small sample** | 5–6 documents do not generalise | Bootstrap CIs; report by type; do not extrapolate beyond the types tested |
| **Non-determinism / model drift** | Changes in the model alter the index | Model and prompt version in the provenance; three runs; temperature 0; raw output stored |
| **Extraction cost** | Long documents are expensive | Report the cost separately and amortised; extraction by windows with overlap in doc. 4 |
| **Coupling with the core** | The experiment conditions Stage 3 | It lives outside the engine's workspace, only reads revisions, with no format changes; no Stage 3 deliverable depends on it |
| **Anthropomorphism** | Reading the metaphor of the cylinder or of the brain as semantics | The cylinder is a visualisation; the terms are defined by technical contract (ADR-0017) |

## 11. Deliverables and open decisions

**Deliverables:** frozen corpus and questions (with hashes), extraction and validation harness, derived index, report with tables and the verdict of §9.

**Open decisions for the owner:**

- Confirm or adjust the thresholds of §9 before measuring.
- Choose the extraction model and the reader model (and their cost budget).
- Approve the concrete corpus and the licence of each document.
- Acceptable construction cost budget for point 6 of §9.
