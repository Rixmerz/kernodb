"""Deterministic evidence validator (narrative-threads.md §5.3).

Every item is accepted as given or rejected and counted: nothing is repaired, clamped
or mapped to a nearby value.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field
from typing import Any

from segment import Block

PHASES = frozenset({"problem", "clue", "development", "contrast", "solution", "conclusion", "unclassified"})
KINDS = frozenset({"develops", "contradicts", "anticipates", "resumes", "exemplifies", "resolves",
                   "depends_on", "unclassified"})


@dataclass
class Report:
    segments: list[dict] = field(default_factory=list)
    relations: list[dict] = field(default_factory=list)
    threads: list[dict] = field(default_factory=list)
    rejected: Counter = field(default_factory=Counter)
    evidence_total: int = 0

    @property
    def evidence_mismatch_rate(self) -> float:
        return self.rejected["evidence_mismatch"] / self.evidence_total if self.evidence_total else 0.0


def _unit(x: Any) -> bool:
    return isinstance(x, (int, float)) and not isinstance(x, bool) and 0.0 <= x <= 1.0


def _span(x: Any, size: int) -> tuple[int, int] | None:
    if (isinstance(x, list) and len(x) == 2 and all(isinstance(v, int) and not isinstance(v, bool) for v in x)
            and 0 <= x[0] < x[1] <= size):
        return x[0], x[1]
    return None


def validate(data: bytes, blocks: list[Block], extraction: dict) -> Report:
    r = Report()
    starts = {b.start: b.ordinal for b in blocks}
    ends = {b.end: b.ordinal for b in blocks}

    seen: set[str] = set()
    for seg in extraction.get("segments") or []:
        span = _span(seg.get("span"), len(data))
        phases = seg.get("phases")
        if not isinstance(seg.get("id"), str) or seg["id"] in seen:
            r.rejected["segment_bad_id"] += 1
        elif span is None:
            r.rejected["segment_span_out_of_range"] += 1
        elif span[0] not in starts or span[1] not in ends or starts[span[0]] > ends[span[1]]:
            r.rejected["segment_not_block_aligned"] += 1
        elif not isinstance(phases, list) or not phases or not all(
                isinstance(p, dict) and p.get("phase") in PHASES for p in phases):
            r.rejected["segment_bad_phase"] += 1
        elif not all(_unit(p.get("weight")) for p in phases) or not _unit(seg.get("depth")):
            r.rejected["segment_weight_out_of_range"] += 1
        else:
            seen.add(seg["id"])
            r.segments.append({**seg, "ordinal": starts[span[0]]})
    ordinal = {s["id"]: s["ordinal"] for s in r.segments}

    for rel in extraction.get("relations") or []:
        evidence = rel.get("evidence") or []
        if rel.get("from") not in ordinal or rel.get("to") not in ordinal:
            r.rejected["relation_unknown_segment"] += 1
            continue
        if rel.get("kind") not in KINDS:
            r.rejected["relation_bad_kind"] += 1
            continue
        if not _unit(rel.get("confidence")):
            r.rejected["relation_confidence_out_of_range"] += 1
            continue
        if not isinstance(evidence, list) or (rel["kind"] != "unclassified" and not evidence):
            r.rejected["relation_without_evidence"] += 1
            continue
        ok = True
        for ev in evidence:
            r.evidence_total += 1
            span = _span(ev.get("span") if isinstance(ev, dict) else None, len(data))
            quote = ev.get("quote") if isinstance(ev, dict) else None
            if span is None or not isinstance(quote, str) or data[span[0]:span[1]] != quote.encode("utf-8"):
                ok = False
        if not ok:
            r.rejected["evidence_mismatch"] += 1
            continue
        r.relations.append(rel)

    linked = {frozenset((x["from"], x["to"])) for x in r.relations if x["kind"] != "unclassified"}
    for th in extraction.get("threads") or []:
        segs = th.get("segments")
        if not isinstance(segs, list) or len(segs) < 2 or any(s not in ordinal for s in segs):
            r.rejected["thread_unknown_segment"] += 1
        elif not th.get("cyclic") and any(ordinal[a] > ordinal[b] for a, b in zip(segs, segs[1:])):
            r.rejected["thread_out_of_order"] += 1
        elif any(frozenset((a, b)) not in linked for a, b in zip(segs, segs[1:])):
            r.rejected["thread_hop_without_relation"] += 1
        else:
            r.threads.append(th)
    return r
