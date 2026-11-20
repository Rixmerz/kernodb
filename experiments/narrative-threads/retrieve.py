"""The three retrieval strategies, under one context budget.

Units are the segmenter's blocks. Each strategy returns an ordered list of block ordinals whose
text fits in `BUDGET_BYTES` (a block that does not fit is skipped, the next is tried), so the
three are compared on the same amount of context:

- `vector`: blocks by cosine similarity to the question;
- `expanded`: the top `SEEDS` blocks, each followed by its neighbours (±`RADIUS`), then the
  vector ranking;
- `threads`: the top `SEEDS` blocks; for each, the segments that hold it, and the segments
  reachable from them through accepted threads and accepted classified relations (one hop),
  ranked by their best block's similarity, each segment's blocks added in similarity order;
  then the vector ranking. Only what `validate` accepted is used.

Embeddings are local (fastembed, `MODEL`), cached per document under `results/index/`
keyed by the document's SHA-256 and the model name. No model API call is made here.
"""
from __future__ import annotations

import hashlib
import json
from collections.abc import Callable
from pathlib import Path

import numpy as np

from segment import Block, segment
from validate import validate

HERE = Path(__file__).resolve().parent
MODEL = "BAAI/bge-small-en-v1.5"
BUDGET_BYTES = 6000
SEEDS = 3
RADIUS = 1
CONFIG = {"model": MODEL, "budget_bytes": BUDGET_BYTES, "seeds": SEEDS, "radius": RADIUS}

_model = None


def _embedder():
    global _model
    if _model is None:
        from fastembed import TextEmbedding
        _model = TextEmbedding(MODEL)
    return _model


def block_vectors(name: str, data: bytes, blocks: list[Block]) -> np.ndarray:
    sha = hashlib.sha256(data).hexdigest()
    path = HERE / "results" / "index" / f"{Path(name).stem}.{MODEL.replace('/', '_')}.{sha[:16]}.npy"
    if path.exists():
        return np.load(path)
    texts = [data[b.start:b.end].decode("utf-8") for b in blocks]
    vecs = np.array(list(_embedder().embed(texts)), dtype=np.float32)
    path.parent.mkdir(parents=True, exist_ok=True)
    np.save(path, vecs)
    return vecs


def query_vector(question: str) -> np.ndarray:
    return np.array(next(iter(_embedder().query_embed([question]))), dtype=np.float32)


def _fill(order, blocks: list[Block], budget: int) -> list[int]:
    out, seen, used = [], set(), 0
    for i in order:
        if i in seen:
            continue
        seen.add(i)
        size = blocks[i].end - blocks[i].start
        if used + size <= budget:
            out.append(i)
            used += size
    return out


def vector(sims: np.ndarray, blocks: list[Block], graph=None, budget: int = BUDGET_BYTES) -> list[int]:
    return _fill(np.argsort(-sims, kind="stable").tolist(), blocks, budget)


def expanded(sims: np.ndarray, blocks: list[Block], graph=None, budget: int = BUDGET_BYTES) -> list[int]:
    ranked = np.argsort(-sims, kind="stable").tolist()
    order = []
    for s in ranked[:SEEDS]:
        order += [s] + [j for d in range(1, RADIUS + 1) for j in (s - d, s + d) if 0 <= j < len(blocks)]
    return _fill(order + ranked, blocks, budget)


class Graph:
    """The accepted extraction of one document, over block ordinals."""

    def __init__(self, data: bytes, blocks: list[Block], extraction: dict) -> None:
        rep = validate(data, blocks, extraction)
        start = {b.start: b.ordinal for b in blocks}
        end = {b.end: b.ordinal for b in blocks}
        self.members = {s["id"]: list(range(start[s["span"][0]], end[s["span"][1]] + 1)) for s in rep.segments}
        self.of_block: dict[int, list[str]] = {}
        for sid, ords in self.members.items():
            for o in ords:
                self.of_block.setdefault(o, []).append(sid)
        self.near: dict[str, set[str]] = {sid: set() for sid in self.members}
        for r in rep.relations:
            if r["kind"] != "unclassified":
                self.near[r["from"]].add(r["to"])
                self.near[r["to"]].add(r["from"])
        for t in rep.threads:
            for sid in t["segments"]:
                self.near[sid].update(x for x in t["segments"] if x != sid)
        self.report = rep


def threads(sims: np.ndarray, blocks: list[Block], graph: Graph, budget: int = BUDGET_BYTES) -> list[int]:
    ranked = np.argsort(-sims, kind="stable").tolist()
    order: list[int] = []
    for s in ranked[:SEEDS]:
        order.append(s)
        reach: set[str] = set()
        for sid in graph.of_block.get(s, []):
            reach.add(sid)
            reach |= graph.near[sid]
        segs = sorted(reach, key=lambda sid: -max(sims[o] for o in graph.members[sid]))
        for sid in segs:
            order += sorted(graph.members[sid], key=lambda o: -sims[o])
    return _fill(order + ranked, blocks, budget)


STRATEGIES: dict[str, Callable] = {"vector": vector, "expanded": expanded, "threads": threads}


def load(name: str, extraction_path: Path | None) -> tuple[bytes, list[Block], np.ndarray, Graph | None]:
    data = (HERE / "corpus" / name).read_bytes()
    blocks = segment(data)
    graph = Graph(data, blocks, json.loads(extraction_path.read_text())) if extraction_path else None
    return data, blocks, block_vectors(name, data, blocks), graph
