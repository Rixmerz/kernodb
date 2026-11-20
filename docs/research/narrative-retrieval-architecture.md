# Architecture report — narrative retrieval in Kerno

| Field | Value |
|---|---|
| Status | Draft for the owner (no code, no format or ADR changes) |
| Date | 2026-11-07 |
| Base | `main` at `6505562` (Stage 5 closed, #24 fixed) |
| Sources reviewed | `docs/research/narrative-threads.md`, `docs/roadmap/README.md`, ADR-0004, 0007, 0008, 0009, 0011, 0014, 0016, 0017, 0018, `docs/architecture/invariants.md`, SAD, `crates/kerno-units/src` |

## 1. Diagnosis of the current state

### 1.1 Implemented (with tests)

| Piece | Where | What it contributes to narrative retrieval |
|---|---|---|
| Unit and revision with a content hash; the payload is opaque bytes | `kerno-units` (`types.rs`, `codec.rs`); ADR-0008 accepted | The original text is the source of truth and has an exact identity `(unit_id, revision_id, content_hash)`: it serves as the key of a derived datum and to verify quotes byte by byte |
| History and retention of revisions | `history.rs`, `retention.rs`; ADR-0004 | An old revision stays readable until retention removes it: a narrative index may remain valid for the revision it describes |
| Typed relations with provenance `Declared`/`Imported`/`Algorithmic`/`Inferred` and confidence | `db.rs` (`Relation`, `relations_of`), `codec.rs`; ADR-0014 accepted; I-39 verified | Inferred content can no longer be presented as declared. **But** a relation joins two *units* (`from`, `to`: `UnitId`), not two ranges of a revision, and it survives new revisions |
| Derived index with a watermark and states `Ready`/`Stale`/`Failed`/`Building` | `index.rs`; I-23, I-24 | The freshness pattern that a narrative index must copy |
| Deterministic planner with reference path, fingerprint and observations | `planner.rs`, `fingerprint.rs`, `observation.rs`; Stage 5 | Reference path and differential test (I-32). It is **access planning**, not comprehension: it must not be mixed with the narrative |

### 1.2 Only documented

- **Experiment design** (`narrative-threads.md`), complete: conceptual model (height, sector, radius, thread), illustrative data model in Rust, two-pass extraction, deterministic validation of evidence, protocol with B1/B2/T, metrics, ablations A1–A4, go/no-go criterion and risks. Status: approved as an independent experiment, not run.
- **Surrounding contracts, not accepted:** ADR-0011 (vectors and hybrid retrieval, *pending*), ADR-0017 (unit semantics, *proposed*), ADR-0018 (versioning and rebuilding of derived data, *proposed*).
- **Future stages:** Stage 7 (associative memory: retrieval by relations, exact vector search, ranking with diagnostic mode), Stage 8 (common registry of derived data, embeddings outside the core).
- **Pending invariants:** I-25 (embedding → exact revision and profile), I-38 (derived → sources, method and version), I-31 (the core needs no network or model).

### 1.3 Missing

1. **No line of code** for the experiment: no harness, corpus, questions or baselines.
2. **The roadmap does not mention narrative retrieval.** It lives only in `docs/research/`. It is the main risk that the project loses it: no stage consumes or requires it.
3. **Addressing by ranges.** Kerno has no object that points to a byte range of a revision, which is the minimum unit of a segment and of an evidence.
4. **Registry of derived data (Stage 8)** and **embeddings/vectors (ADR-0011)**: the two prerequisites for integrating anything into the engine.
5. **AI adapter interface** (LLM, embeddings): it does not exist, not even as a contract.
6. **Owner decisions** open in §11 of the design: go/no-go thresholds, models, corpus, cost budget.

### 1.4 A mismatch to fix now

Narrative relations (`develops`, `contradicts`…) are between **ranges of a specific revision** and are valid only for that revision. The relations of `kerno-units` are between **units** and persist across revisions. Storing narrative relations as `kerno-units` relations would be wrong: an edit of the document would leave relations alive over text that no longer exists. The current design already avoids this ("they are not written as `Edge`"); the architecture must turn it into a rule.

## 2. Recommended architecture

### 2.1 Proposed decision

**A retrieval subsystem outside the core, in two steps:**

1. **Now:** the experiment as an **independent program outside the engine's workspace** (`experiments/narrative-threads/`), which only reads revisions and stores its index in its own files. It is what the design already said, and it is the only thing that allows measuring without committing anything.
2. **Only after a GO:** a **crate of its own** (e.g. `kerno-narrative`) that depends on `kerno-units` and on the registry of derived data of Stage 8, never the other way round. The models come in through **adapters** that live in separate processes or crates. The engine does not know the crate.

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

### 2.2 Discarded alternatives

| Alternative | Why not |
|---|---|
| **Module of `kerno-units`** | It puts AI semantics into the storage layer. It drags model dependencies into the core (clashes with I-31 and D-08). It forces changing the v1 format to store segments. And it ties the core's pace to an experiment that may come out NO-GO |
| **Part of the planner / operational adaptation** | The planner decides *how to read* data already identified and must give the same result as the reference path (I-32). Narrative retrieval decides *what is relevant*, and its result changes on purpose. Mixing them would break the differential test and would confuse ADR-0016 with comprehension |
| **Separate engine with its own storage** | It duplicates the source of truth, loses the exact provenance, the suppression (ADR-0009) and the invalidation by revision. Kerno would be reduced to a blob store |
| **Subsystem inside the core, but isolated** | Better than a module, but it still compiles with the engine and pushes to touch the format. There is no gain over a separate crate that depends on the core |

The independent crate that uses Kerno as storage is, in practice, the option "separate engine that uses Kerno", but without its own store: it keeps everything as derived data of Kerno.

## 3. Relation to the rest of Kerno

| Piece | Relation |
|---|---|
| **Embeddings / ADR-0011** | Segments are embedded like any derived datum (I-25): vector + source revision + model profile. They serve to find **seed segments**. ADR-0011 must be decided before integrating; the experiment may use brute force in memory |
| **Relations (ADR-0014)** | Narrative relations live **inside the derived index**, never as `kerno-units` relations. Future exception, to be decided in an ADR: projecting as an `Inferred` relation between units a link *between documents* (e.g. an ADR that resolves another), with its confidence and evidence |
| **Secondary indexes (ADR-0007)** | The narrative index copies the state pattern `Ready`/`Stale`/`Failed`/`Building`, but its "watermark" is the revision and the extractor profile, not the commit sequence |
| **Associative memory (Stage 7)** | It is the natural consumer. The Stage 7 retrieval (exact vector + relations + temporal context) is the **fallback**. The narrative is one more **signal and expander** of its ranking, explainable in its diagnostic mode |
| **Registry of derived data (Stage 8, ADR-0018)** | Prerequisite for the integration: the narrative index is registered there with its sources, method and version (I-38) |
| **Planner** | None. The planner's observations do not train the narrative and the narrative does not change plans. If a narrative retrieval is ever cached, that cache is operational adaptation and is governed by ADR-0016, separately |

## 4. Data model

Part of the sketch of `narrative-threads.md` §4 (segment, thread, relation, provenance, `Origin::Inferred`), with these additions:

```rust
/// Exact identity of what was analysed. The index key.
pub struct Source {
    pub unit_id: UnitId,
    pub revision_id: RevisionId,
    pub content_hash: [u8; 32],   // that of the revision (ADR-0008)
}

/// Everything whose change forces a rebuild.
pub struct ExtractorProfile {
    pub segmenter_version: u32,     // how the blocks are numbered (deterministic)
    pub model: String,              // exact model id, never an alias
    pub prompt_version: String,
    pub schema_version: u32,        // JSON schema of the output
    pub validator_version: u32,     // deterministic evidence rules
    pub embedder: Option<String>,   // embeddings profile of the segments
}

pub enum IndexStatus { Ready, Stale, Failed { reason: String }, Building }

pub struct NarrativeIndex {
    pub source: Source,
    pub profile: ExtractorProfile,
    pub status: IndexStatus,
    pub raw_output_hash: [u8; 32],  // raw output stored for auditing (ADR-0018)
    pub windows: Vec<WindowSpan>,   // windows used in long documents
    pub segments: Vec<NarrativeSegment>,
    pub threads: Vec<NarrativeThread>,
    pub relations: Vec<NarrativeRelation>,
    pub rejected: RejectionCounts,  // evidence_mismatch, unknown kind…
}
```

Rules:

- Every range (`span`) is in bytes over the revision of `source`. An evidence is accepted only if `contenido[span] == quote` byte by byte.
- A relation without an accepted evidence does not exist; a thread without an accepted relation at each hop does not exist.
- Everything is `Inferred`; none of this is shown as declared (I-39).
- Nothing is stored in the v1 format during the experiment. After a GO, the storage follows the Stage 8 decision on derived data.

## 5. Flows

### 5.1 Indexing (external job, per revision)

1. Read the Kerno revision and check its `content_hash`.
2. **Deterministic segmenter** (code, not a model): blocks with their byte range.
3. If the document does not fit in the model's context: overlapping windows of blocks, plus a merge pass between windows that can only propose relations with evidence in both.
4. **Pass 1 (LLM):** group blocks into segments; phases with weight, depth, theme.
5. **Pass 2 (LLM):** relations and threads over the list of segments.
6. **Deterministic validation:** ranges, quotes byte by byte, closed sets, weights in [0, 1], thread hops with a relation. What does not pass is discarded and counted; it is never repaired.
7. Embed segments with the embeddings adapter.
8. Register the derived datum with `Source`, `ExtractorProfile`, raw output and status `Ready`.

### 5.2 Retrieval

1. Query → vector (embeddings adapter) and **narrative intent** (classifier: scattered, contrast, cause, evolution, local, unanswerable). The intent only weights; it never filters.
2. **Baseline always:** Stage 7 retrieval (exact vector + relations + filters).
3. If there is a `Ready` index for the snapshot's revision: seed segments → relevant sectors → threads that contain them → expansion through relations, with the preceding and following context of each segment.
4. Assemble in document order, keeping **several branches, contradictions and conclusions** as separate paths, up to the token budget.
5. Each returned fragment carries its provenance: range, revision, thread, relation and the confidence that brought it (Stage 7 diagnostic mode).
6. **Fallback:** if there is no index, it is `Stale`/`Failed`, or the expander fails, the baseline is returned and the reason is recorded.

## 6. Versioning and invalidation

| Change | Effect |
|---|---|
| New revision of the document | The index of the previous revision **remains valid for it** (snapshots and history). There is no index for the new one: `Stale` until rebuilt. An old index is never "patched" with the new revision |
| Change of any field of `ExtractorProfile` | All the indexes of that profile become `Stale`; they are rebuilt on demand or in batch. Two profiles may coexist during the migration, and one is chosen per query |
| Retention removes a revision | Its narrative index is deleted with it |
| Suppression (ADR-0009) | Deletes segments, quotes, embeddings, raw output and relations between documents that name it, in the same logical step as the source |
| Relation between documents (if it exists) | Obsolete as soon as either side changes |
| Non-determinism of the model | The hash of the output is stored, not only of the input (ADR-0018). Rebuilding may give another index: it is a new version, not an error |

Invalidation is by **exact identity**, not by time nor by heuristic: either `(source, profile)` matches, or the index is not used.

## 7. Adapters: interchangeable models without coupling the core

Four ports, each a small contract, implemented outside the core:

| Port | Input → output | Identity it declares |
|---|---|---|
| `Segmenter` | text → blocks with range | version (deterministic, no model) |
| `NarrativeExtractor` | blocks → JSON of the schema | model, prompt, schema |
| `Embedder` | text → vector | model, dimension, metric, normalisation |
| `IntentClassifier` (optional) | query → intent weights | model or rules |

- The **validator** is not a port: it is fixed code of Kerno, so that no model decides which evidence is valid.
- Each adapter may be a process (JSON over stdin/stdout or local HTTP). Changing provider means changing adapter and, therefore, profile: it invalidates what was built with the previous one.
- The core does not compile against any adapter (I-31). Without adapters, retrieval keeps working with whatever does not require a model.

## 8. Risks and costs

| Risk | Mitigation |
|---|---|
| Invented continuity | Mandatory byte-by-byte evidence, explicit abstention, manual review of hops, ablation A4 |
| Phase ontology that does not fit (reference, tables, code) | Multiple weights, `unclassified`, ablation A1; ontology per document type if needed |
| Depth that only reflects length | Measure correlation; ablation A2 |
| Threads that add nothing over a good segmentation | Ablation A3; promote only the segmentation if that is what wins |
| Extraction cost | Long document (~100 000 words ≈ 130 000 tokens) × 2 passes + overlapping windows: on the order of 300 000–400 000 input tokens per revision. Reported as a total and amortised per query. It is only re-extracted when the revision or the profile changes |
| Query latency | Expansion bounded by budget; p95 criterion ≤ 2× B2 |
| Model drift and non-determinism | Exact profile, temperature 0, raw output, three runs |
| Evaluation leakage | Questions frozen by hash before running, a different annotator |
| Pull towards the core | Outside the workspace until a GO; integration only via ADR |
| **Losing the idea in the roadmap** | Add it as a research track with an owner and hook points (§10) |

## 9. Validation experiment

The protocol of `narrative-threads.md` §6–§9 is kept, with these adjustments:

1. **Corpus (6 real documents, licence verified, pinned by hash):** technical (RFC or manual), scientific article, story or long chapter, document > 100 000 words, comparative report, and Kerno's `docs/adr/` set as a self-consumption case.
2. **Questions:** 10–15 per document (50–80), ≥ 70 % scattered; 20 % of local control and 10 % unanswerable; annotated with their ranges by a person other than whoever tunes the extractor; 20 % reviewed by a second annotator.
3. **Systems, with the same embedder, reader and token budget:**
   - **B1:** fixed chunks + top-k vector.
   - **B2:** B1 + context expansion (neighbours or the containing section). It is the baseline to beat.
   - **T:** vector seeds over segments + expansion through threads and relations.
   - **Ablations A1–A4** over T.
4. **Metrics:** precision and coverage by bytes, full coverage of the question, correct connections, noise, invented answers to unanswerable questions, latency p50/p95 (≥ 30 repetitions, machine at rest), construction cost, `evidence_mismatch`; bootstrap CIs.
5. **Decision:** the go/no-go of §9 of the design, with thresholds that you confirm **before** measuring.
6. **Location:** `experiments/narrative-threads/`, outside the engine's workspace. It may use its own dependencies because it is not compiled with Kerno.

## 10. Impact on the roadmap and required ADRs

**Roadmap (proposal, without touching stages in progress):**

- Add a **research track R-1 "Narrative retrieval"** in parallel, which blocks no stage. Dependencies: it reads revisions (Stage 3, done). Output: go/no-go report.
- Hook its possible integration into **Stage 7** (consumer, as a ranking signal with fallback to the baseline) and **Stage 8** (registry of derived data), without changing their current acceptance criteria.

**ADR:**

| ADR | When | Content |
|---|---|---|
| Decide **ADR-0018** (derived data) | Before any integration | Common contract for derived data that the narrative index will use |
| Decide **ADR-0011** (vectors) | Before Stage 7 | Vector seeds and exact baseline |
| New **ADR-0019 "Narrative retrieval as a derived datum"** | Only after a GO | Crate outside the core, index per `(revision, profile)`, narrative relations outside ADR-0014, mandatory fallback, provenance in the answer |
| New **ADR-0020 "AI adapter ports"** | With Stage 8 (embeddings need it too) | Contracts of `Segmenter`, `NarrativeExtractor`, `Embedder`; profiles; no adapter in the core |

No accepted ADR and not the v1 format is changed. ADR-0016 stays aside: it is operational adaptation and governs nothing of this.

## 11. Short answer

Yes, the current architecture is heading in the right direction for this capability. The text is already the verifiable source of truth. Inferred relations are no longer confused with declared ones. Derived data already has a freshness pattern. And the experiment is already well designed.

There are three gaps to correct so as not to lose the idea:

1. It is not in the roadmap.
2. There is no way to name a range of a revision.
3. Its integration prerequisites (ADR-0018, ADR-0011) are still undecided.

Decisions that fall to you to start the experiment: go/no-go thresholds, extraction and reading models, corpus and its licence, and cost budget.
