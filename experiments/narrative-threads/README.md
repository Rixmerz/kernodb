# Narrative-thread retrieval experiment

Validation experiment for `docs/research/narrative-threads.md` and
`docs/research/narrative-retrieval-architecture.md` §9: does retrieving by narrative
threads (segments linked by evidence-backed relations) find the evidence a question needs
better than plain vector search?

It lives outside the Cargo workspace and is a Python project managed with uv. It touches no
crate, `Cargo.toml` or ADR.

## Phase 1 (this commit): no model calls

| File | What |
|---|---|
| `corpus/` | six real documents as UTF-8 text, and `MANIFEST.json` (URL, author, title, licence and its evidence, version, acquisition date, bytes, words, SHA-256) |
| `fetch.py` | rebuilds every corpus file deterministically and checks its hash; files over 1 MB are git-ignored and come back through it |
| `segment.py` | deterministic block segmenter: paragraphs and heading lines with exact byte ranges `[start, end)` |
| `validate.py` | deterministic evidence validator of §5.3: exact byte quotes, closed sets of kinds and phases, weights in [0, 1], every thread hop backed by an accepted relation; rejected items are counted, never repaired |
| `ledger.py` | the cost ledger and its hard caps, for the phases that call models |

### Corpus

| # | Type | Document | Licence |
|---|---|---|---|
| a | technical / architectural | Kerno `docs/architecture/SAD.md` | this repository's own docs |
| b | research, hypotheses and evidence | arXiv 2609.00063v1, *Medical Causal Hypothesis Verification with Large Language Models* | CC BY 4.0 |
| c | narrative with flashbacks | Doyle, *A Study in Scarlet* (Gutenberg #244) | public domain (USA) |
| d | long (>100k words), recurring themes | Melville, *Moby Dick* (Gutenberg #2701) | public domain (USA) |
| e | comparative, alternatives and contradictions | arXiv 2606.05658v1, *Agent-Orchestrated Adaptive RAG: A Comparative Study on Structured and Multi-Hop Retrieval* | CC BY 4.0 |
| f | several proposals and conclusions | Kerno `docs/adr/0001..0018`, concatenated | this repository's own docs |

The CC BY texts are converted from arXiv's HTML (math as its LaTeX alt text, figures
dropped); attribution is in `MANIFEST.json`. Project Gutenberg's header and footer are
stripped. Gutenberg updates its files now and then: if a hash no longer matches, the
source changed, and the experiment's frozen hash is the one in the manifest.

### Cost ledger

`Ledger.authorize(phase)` before each call refuses it once the phase's cap or the total is
reached; `Ledger.record(...)` after it appends to `results/ledger.jsonl` the phase,
document, alias, the model ids exactly as `modelUsage` reports them, input/output/cache
tokens, the CLI's `total_cost_usd`, and locks' cost of the same `session_id`'s transcript
(`locks.usage`, read in a separate process; locks' store is never opened). The larger of
the two counts against the caps; when locks cannot find the transcript its figure is `null`
and the CLI's is used. A call can overshoot a cap; the next one is refused.

## Reproduce

```bash
cd experiments/narrative-threads
uv run python fetch.py          # downloads what is missing, checks every SHA-256
uv run --group dev pytest -q
```

## Owner's parameters (verbatim)

- Extraction model: Claude Sonnet. Answer model: Claude Haiku. Evaluation: independent human. Exact model ids are pinned and never changed mid-experiment.
- 30 questions per document (180 in total), each with a reference answer and the byte ranges that justify it.
- Baselines: plain vector search, vector search with context expansion, and narrative-thread retrieval.
- GO thresholds:
  - evidence coverage at least 10% better than the best baseline;
  - segment precision no more than 5 percentage points worse;
  - thread coherence at least 80% positive in human evaluations;
  - justifiable relations at least 85%;
  - answers without invented evidence at least 95%;
  - reproducibility: same version and configuration, with traceable results.
- Budget: USD 25 (15 extraction, 7 queries, 3 repetitions and eval). Stop at the limit and present partial results.
