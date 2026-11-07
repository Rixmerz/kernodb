from segment import segment
from validate import validate

DATA = "The ship sailed.\n\nA white whale appeared.\n\nAhab swore revenge.\n\nThe whale sank the ship.\n".encode()
B = segment(DATA)


def seg(i, a, b, phase="development", w=0.5, depth=0.5):
    return {"id": i, "span": [B[a].start, B[b].end], "topic": "t", "phases": [{"phase": phase, "weight": w}], "depth": depth}


def ev(text):
    s = DATA.index(text.encode())
    return {"span": [s, s + len(text.encode())], "quote": text}


def base():
    return {
        "segments": [seg("s1", 0, 0), seg("s2", 1, 1, "clue"), seg("s3", 2, 3, "conclusion")],
        "relations": [
            {"from": "s1", "to": "s2", "kind": "develops", "confidence": 0.9, "evidence": [ev("white whale")]},
            {"from": "s2", "to": "s3", "kind": "resolves", "confidence": 1, "evidence": [ev("whale sank")]},
        ],
        "threads": [{"id": "t1", "topic": "whale", "segments": ["s1", "s2", "s3"]}],
    }


def test_valid_extraction_is_accepted_whole():
    r = validate(DATA, B, base())
    assert len(r.segments) == 3 and len(r.relations) == 2 and len(r.threads) == 1
    assert not r.rejected and r.evidence_mismatch_rate == 0


def test_quote_must_match_bytes_exactly_and_is_not_repaired():
    x = base()
    x["relations"][0]["evidence"][0]["quote"] = "White whale"
    r = validate(DATA, B, x)
    assert r.rejected["evidence_mismatch"] == 1 and r.evidence_mismatch_rate == 0.5
    assert x["relations"][0]["evidence"][0]["quote"] == "White whale"
    # the thread loses its s1→s2 hop, so it is rejected too
    assert r.rejected["thread_hop_without_relation"] == 1 and not r.threads


def test_out_of_range_evidence_span():
    x = base()
    x["relations"][1]["evidence"] = [{"span": [0, len(DATA) + 5], "quote": "x"}]
    assert validate(DATA, B, x).rejected["evidence_mismatch"] == 1


def test_closed_sets_reject_unknown_values():
    x = base()
    x["segments"][0]["phases"][0]["phase"] = "climax"
    x["relations"][1]["kind"] = "causes"
    r = validate(DATA, B, x)
    assert r.rejected["segment_bad_phase"] == 1 and r.rejected["relation_bad_kind"] == 1
    assert r.rejected["relation_unknown_segment"] == 1  # s1 was rejected


def test_weights_in_unit_interval():
    for bad in (1.01, -0.1, True, "0.5"):
        x = base()
        x["segments"][1]["phases"][0]["weight"] = bad
        assert validate(DATA, B, x).rejected["segment_weight_out_of_range"] == 1
    x = base()
    x["segments"][1]["depth"] = 2
    assert validate(DATA, B, x).rejected["segment_weight_out_of_range"] == 1
    x = base()
    x["relations"][0]["confidence"] = 1.5
    assert validate(DATA, B, x).rejected["relation_confidence_out_of_range"] == 1


def test_segment_must_be_block_aligned():
    x = base()
    x["segments"][0]["span"] = [B[0].start + 1, B[0].end]
    assert validate(DATA, B, x).rejected["segment_not_block_aligned"] == 1


def test_relation_needs_evidence_unless_unclassified():
    x = base()
    x["relations"][0]["evidence"] = []
    assert validate(DATA, B, x).rejected["relation_without_evidence"] == 1
    x["relations"][0]["kind"] = "unclassified"
    r = validate(DATA, B, x)
    assert len(r.relations) == 2
    assert r.rejected["thread_hop_without_relation"] == 1  # an unclassified relation is no hop


def test_thread_order_and_cycles():
    x = base()
    x["threads"][0]["segments"] = ["s2", "s1"]
    assert validate(DATA, B, x).rejected["thread_out_of_order"] == 1
    x["threads"][0]["cyclic"] = True
    assert len(validate(DATA, B, x).threads) == 1
    x["threads"][0]["segments"] = ["s1", "s9"]
    assert validate(DATA, B, x).rejected["thread_unknown_segment"] == 1
