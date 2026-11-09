from extract import to_schema
from segment import segment
from validate import validate

DATA = "The ship sailed.\n\nA white whale\nappeared.\n\nAhab swore revenge.\n\nThe whale sank the ship.\n".encode()
B = segment(DATA)


def answer(quote="white whale appeared"):
    return {
        "segments": [{"id": "s1", "first_block": "b0", "last_block": "b1", "topic": "t",
                      "phases": [{"phase": "problem", "weight": 0.5}], "depth": 0.5},
                     {"id": "s2", "first_block": "b3", "last_block": "b3", "topic": "t",
                      "phases": [{"phase": "conclusion", "weight": 1}], "depth": 0.5}],
        "relations": [{"from": "s1", "to": "s2", "kind": "resolves", "confidence": 0.8,
                       "evidence": [{"segment": "s1", "quote": quote}]}],
        "threads": [{"id": "t1", "topic": "whale", "segments": ["s1", "s2"]}],
    }


def test_block_ranges_and_wrapped_quotes_become_exact_spans():
    x = to_schema(DATA, B, answer(), "w0.")
    r = validate(DATA, B, x)
    assert len(r.segments) == 2 and len(r.relations) == 1 and len(r.threads) == 1 and not r.rejected
    ev = x["relations"][0]["evidence"][0]
    assert DATA[ev["span"][0]:ev["span"][1]] == b"white whale\nappeared"


def test_quote_outside_its_segment_or_changed_is_rejected_not_repaired():
    for q in ("whale sank", "white whales appeared"):
        r = validate(DATA, B, to_schema(DATA, B, answer(q), "w0."))
        assert r.rejected["evidence_mismatch"] == 1 and r.rejected["thread_hop_without_relation"] == 1


def test_unknown_or_reversed_block_range_is_rejected():
    a = answer()
    a["segments"][1]["first_block"] = "b9"
    a["segments"][0]["first_block"], a["segments"][0]["last_block"] = "b1", "b0"
    r = validate(DATA, B, to_schema(DATA, B, a, "w0."))
    assert r.rejected["segment_span_out_of_range"] == 2
