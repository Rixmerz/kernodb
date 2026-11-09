"""Phase 3: narrative-thread extraction with Sonnet (narrative-threads.md §5).

The model sees the document as numbered blocks (`segment.numbered`) and answers with
segments as block ranges, relations with verbatim quotes, and threads. The harness turns
that answer into the §5.2 schema without repairing anything:

- a segment's span is `[start of its first block, end of its last block)`; an unknown block
  id or a reversed range leaves the span null, and the validator rejects it;
- an evidence quote is looked up by its exact words (any whitespace between them) inside the
  span of the segment it cites; it is kept with that exact byte range and the bytes found,
  and one that is not found keeps a null span, so the validator counts it as
  `evidence_mismatch` and drops the relation.

The raw answer is kept beside the extraction for audit. Calls are isolated
(`model_call`) and charged to the ledger's `extraction` phase (USD 15); a run stops when
the ledger refuses a call. Long documents are read in windows on block boundaries; a
thread cannot cross a window, which the results report says.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

from ledger import CapReached, Ledger
from model_call import answer_of, ask
from segment import Block, numbered, segment
from validate import KINDS, PHASES, validate

HERE = Path(__file__).resolve().parent
CORPUS = HERE / "corpus"
OUT = HERE / "results" / "extraction"
ALIAS = "sonnet"
WINDOW_BYTES = 300_000
EXTRACTOR_VERSION = "x2"

SCHEMA = {
    "type": "object",
    "properties": {
        "segments": {"type": "array", "items": {"type": "object", "properties": {
            "id": {"type": "string"}, "first_block": {"type": "string"}, "last_block": {"type": "string"},
            "topic": {"type": "string"},
            "phases": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {
                "phase": {"type": "string", "enum": sorted(PHASES)}, "weight": {"type": "number"}},
                "required": ["phase", "weight"]}},
            "depth": {"type": "number"}},
            "required": ["id", "first_block", "last_block", "topic", "phases", "depth"]}},
        "relations": {"type": "array", "items": {"type": "object", "properties": {
            "from": {"type": "string"}, "to": {"type": "string"},
            "kind": {"type": "string", "enum": sorted(KINDS)}, "confidence": {"type": "number"},
            "evidence": {"type": "array", "items": {"type": "object", "properties": {
                "segment": {"type": "string"}, "quote": {"type": "string"}},
                "required": ["segment", "quote"]}}},
            "required": ["from", "to", "kind", "confidence", "evidence"]}},
        "threads": {"type": "array", "items": {"type": "object", "properties": {
            "id": {"type": "string"}, "topic": {"type": "string"},
            "segments": {"type": "array", "items": {"type": "string"}}, "cyclic": {"type": "boolean"}},
            "required": ["id", "topic", "segments"]}},
    },
    "required": ["segments", "relations", "threads"],
}

INSTRUCTIONS = """You index a document for narrative retrieval: a reader will later ask questions whose
answers are spread across distant parts of it, and your index must let a system follow an idea
from where it starts to where it ends. The document is given below as numbered blocks: each
block starts with [bN start:end].

1. Segments: cover every substantive block (skip only boilerplate such as references lists,
   tables of contents, licence text). Group consecutive blocks into segments of one idea, scene or
   argument step (usually 1-5 blocks). Give the first and last block id, a short topic, one or more
   narrative phases with weights in [0,1] (problem, clue, development, contrast, solution,
   conclusion, unclassified) and a depth in [0,1] (how substantive it is).
2. Relations: link segments, near or far apart, wherever the text itself shows the link:
   develops, contradicts, anticipates, resumes, exemplifies, resolves, depends_on. Look in
   particular for links between distant segments (a problem and its later resolution, a clue and
   what it reveals, a claim and its later revision). Each relation needs confidence in [0,1] and
   evidence: short passages copied VERBATIM, character for character, from the segments involved,
   naming the segment each quote is from (no ellipses, no paraphrase). If you cannot quote the
   text that shows a link, do not claim it: use kind "unclassified" or leave it out.
3. Threads: every idea, character, problem or decision the text follows across several segments
   is a thread: an ordered chain in document order (or marked cyclic). Each consecutive pair in a
   thread MUST also appear in your relations with a classified kind (either direction); add the
   relation or shorten the thread. Do not invent continuity: segments that only share a theme
   are not one thread. A segment may be in several threads. Give as many threads as the text
   supports.

Use ids s1, s2, ... for segments and t1, t2, ... for threads. Answer only with the JSON object.

DOCUMENT:
"""


def windows(blocks: list[Block], size: int = WINDOW_BYTES) -> list[list[Block]]:
    out: list[list[Block]] = []
    cur: list[Block] = []
    for b in blocks:
        if cur and b.end - cur[0].start > size:
            out.append(cur)
            cur = []
        cur.append(b)
    if cur:
        out.append(cur)
    return out


def locate(data: bytes, lo: int, hi: int, quote: str) -> tuple[int, int] | None:
    words = quote.split()
    if not words:
        return None
    m = re.compile(rb"\s+".join(re.escape(w.encode("utf-8")) for w in words)).search(data, lo, hi)
    return (m.start(), m.end()) if m else None


def to_schema(data: bytes, blocks: list[Block], answer: dict, prefix: str) -> dict:
    """The model's answer in the §5.2 schema, ids prefixed by window. Nothing is repaired:
    what cannot be resolved keeps a null span for the validator to reject."""
    by_id = {b.id: b for b in blocks}
    segs, spans = [], {}
    for s in answer.get("segments") or []:
        a, b = by_id.get(s.get("first_block")), by_id.get(s.get("last_block"))
        span = [a.start, b.end] if a and b and a.ordinal <= b.ordinal else None
        sid = f"{prefix}{s.get('id')}"
        spans[sid] = span
        segs.append({"id": sid, "span": span, "topic": s.get("topic"), "phases": s.get("phases"),
                     "depth": s.get("depth")})
    rels = []
    for r in answer.get("relations") or []:
        evidence = []
        for e in r.get("evidence") or []:
            seg_span = spans.get(f"{prefix}{e.get('segment')}")
            found = locate(data, seg_span[0], seg_span[1], e.get("quote") or "") if seg_span else None
            evidence.append({"span": list(found), "quote": data[found[0]:found[1]].decode("utf-8")} if found
                            else {"span": None, "quote": e.get("quote")})
        rels.append({"from": f"{prefix}{r.get('from')}", "to": f"{prefix}{r.get('to')}", "kind": r.get("kind"),
                     "confidence": r.get("confidence"), "evidence": evidence})
    threads = [{"id": f"{prefix}{t.get('id')}", "topic": t.get("topic"), "cyclic": bool(t.get("cyclic")),
                "segments": [f"{prefix}{x}" for x in t.get("segments") or []]} for t in answer.get("threads") or []]
    return {"segments": segs, "relations": rels, "threads": threads}


def run_doc(name: str, ledger: Ledger, pinned: dict) -> None:
    data = (CORPUS / name).read_bytes()
    blocks = segment(data)
    wins = windows(blocks)
    stem = Path(name).stem
    raw_dir = OUT / EXTRACTOR_VERSION / "raw"
    path = OUT / EXTRACTOR_VERSION / f"{stem}.json"
    doc = json.loads(path.read_text()) if path.exists() else {
        "document": name, "sha256": hashlib.sha256(data).hexdigest(), "extractor_version": EXTRACTOR_VERSION,
        "alias": ALIAS, "windows": [], "segments": [], "relations": [], "threads": []}
    done = {w["index"] for w in doc["windows"]}
    for i, win in enumerate(wins):
        if i in done:
            continue
        ledger.authorize("extraction")
        result = ask(ALIAS, SCHEMA, INSTRUCTIONS + numbered(data, win))
        call_id = f"extract:{EXTRACTOR_VERSION}:{name}:w{i}"
        entry = ledger.record("extraction", name, ALIAS, result, call_id)
        ids = entry["model_ids"]
        if pinned.setdefault("ids", ids) != ids:
            raise SystemExit(f"model changed mid-experiment: {pinned['ids']} -> {ids}; stopping")
        if result.get("is_error"):
            raise SystemExit(f"{name} w{i}: the CLI reported an error")
        answer = answer_of(result)
        raw_dir.mkdir(parents=True, exist_ok=True)
        (raw_dir / f"{stem}.w{i}.json").write_text(json.dumps(answer, ensure_ascii=False) + "\n")
        part = to_schema(data, win, answer, f"w{i}.")
        for k in ("segments", "relations", "threads"):
            doc[k] += part[k]
        doc["windows"].append({"index": i, "start": win[0].start, "end": win[-1].end, "blocks": len(win),
                               "call_id": call_id, "model_ids": ids, "charged_usd": entry["charged_usd"]})
        doc["model_ids"] = ids
        path.write_text(json.dumps(doc, ensure_ascii=False) + "\n")
        rep = validate(data, blocks, part)
        print(f"{name} w{i}: {len(rep.segments)} segments, {len(rep.relations)} relations, "
              f"{len(rep.threads)} threads accepted; rejected {dict(rep.rejected)}; "
              f"USD {entry['charged_usd']:.3f}, extraction spent {ledger.spent('extraction'):.3f}", file=sys.stderr)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("docs", nargs="*")
    args = ap.parse_args()
    manifest = json.loads((CORPUS / "MANIFEST.json").read_text())
    ledger, pinned = Ledger(), {}
    for name in args.docs or [d["file"] for d in manifest["documents"]]:
        try:
            run_doc(name, ledger, pinned)
        except CapReached as e:
            print(f"stopped: {e}; partial extraction kept", file=sys.stderr)
            return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
