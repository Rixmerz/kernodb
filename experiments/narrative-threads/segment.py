"""Deterministic block segmenter.

A block is a paragraph (text between blank lines) or a heading line (a Markdown `#` line).
Each block carries its exact byte range `[start, end)` over the UTF-8 document, so that
`data[start:end]` is the block, without its surrounding whitespace.
"""
from __future__ import annotations

import re
from dataclasses import dataclass

_HEADING = re.compile(rb"^#{1,6} ")


@dataclass(frozen=True)
class Block:
    ordinal: int
    start: int
    end: int
    kind: str  # "heading" | "paragraph"

    @property
    def id(self) -> str:
        return f"b{self.ordinal}"


def _lines(data: bytes):
    pos = 0
    for line in data.splitlines(keepends=True):
        yield pos, line
        pos += len(line)


def segment(data: bytes) -> list[Block]:
    """Blocks in document order. Pure: the same bytes always give the same blocks."""
    spans: list[tuple[int, int, str]] = []
    cur: list[int] | None = None  # [start, end] of the paragraph being built

    def close():
        nonlocal cur
        if cur is not None:
            spans.append((cur[0], cur[1], "paragraph"))
            cur = None

    for pos, line in _lines(data):
        body = line.rstrip(b"\r\n")
        stripped = body.strip()
        if not stripped:
            close()
            continue
        lead = len(body) - len(body.lstrip())
        trail = len(body) - len(body.rstrip())
        start, end = pos + lead, pos + len(body) - trail
        if _HEADING.match(stripped):
            close()
            spans.append((start, end, "heading"))
        elif cur is None:
            cur = [start, end]
        else:
            cur[1] = end
    close()
    return [Block(i, s, e, k) for i, (s, e, k) in enumerate(spans)]


def numbered(data: bytes, blocks: list[Block]) -> str:
    """The document as the extractor receives it: each block with its byte range."""
    return "\n\n".join(f"[{b.id} {b.start}:{b.end}]\n{data[b.start:b.end].decode('utf-8')}" for b in blocks)
