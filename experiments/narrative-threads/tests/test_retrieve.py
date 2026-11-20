import numpy as np

from retrieve import Graph, expanded, threads, vector
from segment import segment

DATA = "\n\n".join(f"Block number {i} text." for i in range(10)).encode()
B = segment(DATA)
SIZE = B[0].end - B[0].start


def seg(i, a, b):
    return {"id": i, "span": [B[a].start, B[b].end], "topic": "t",
            "phases": [{"phase": "development", "weight": 1}], "depth": 1}


def ev(i):
    t = f"Block number {i}".encode()
    s = DATA.index(t)
    return {"span": [s, s + len(t)], "quote": t.decode()}


GRAPH = Graph(DATA, B, {
    "segments": [seg("s1", 0, 0), seg("s2", 8, 9)],
    "relations": [{"from": "s1", "to": "s2", "kind": "resolves", "confidence": 1, "evidence": [ev(0), ev(9)]}],
    "threads": []})
SIMS = np.array([1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1], dtype=np.float32)


def test_budget_is_equal_and_respected():
    for f in (vector, expanded, threads):
        out = f(SIMS, B, GRAPH, budget=3 * SIZE)
        assert len(out) == 3 and len(set(out)) == 3


def test_vector_takes_top_similarity():
    assert vector(SIMS, B, budget=3 * SIZE) == [0, 1, 2]


def test_threads_follow_relation_to_distant_segment():
    out = threads(SIMS, B, GRAPH, budget=3 * SIZE)
    assert out[0] == 0 and 9 in out and 8 in out


def test_expanded_adds_neighbours():
    sims = SIMS.copy()
    sims[5] = 2.0
    out = expanded(sims, B, budget=3 * SIZE)
    assert out == [5, 4, 6]
