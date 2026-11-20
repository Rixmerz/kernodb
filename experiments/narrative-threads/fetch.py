"""Build the corpus: download (or assemble) each document as UTF-8 plain text and check its hash.

    uv run python fetch.py            # fetch what is missing, verify every hash
    uv run python fetch.py --refresh  # fetch again and verify
    uv run python fetch.py --record   # (maintainer) rewrite sizes and hashes in MANIFEST.json

Every transformation is deterministic, so the same source yields the same bytes and hash.
Texts over 1 MB are git-ignored; this script is how they come back.
"""
from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import subprocess
import sys
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE / "corpus"
MANIFEST = CORPUS / "MANIFEST.json"
REPO = HERE.parent.parent
UA = {"User-Agent": "kerno-narrative-threads-experiment/0.1 (research corpus)"}


def _get(url: str) -> bytes:
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120) as r:
        return r.read()


def normalize(text: str) -> str:
    """LF line ends, no trailing spaces, at most one blank line in a row, one final newline."""
    text = text.replace("\r\n", "\n").replace("\r", "\n").lstrip("﻿")
    lines = [ln.rstrip() for ln in text.split("\n")]
    out = re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip("\n")
    return out + "\n"


def gutenberg(raw: bytes) -> str:
    """The book between Project Gutenberg's START and END markers (the licence header and
    footer are left out; MANIFEST keeps the source URL)."""
    text = raw.decode("utf-8")
    start = re.search(r"\*\*\* ?START OF (THE|THIS) PROJECT GUTENBERG EBOOK[^\n]*\n", text)
    end = re.search(r"\*\*\* ?END OF (THE|THIS) PROJECT GUTENBERG EBOOK", text)
    if not start or not end:
        raise ValueError("Gutenberg markers not found")
    return normalize(text[start.end():end.start()])


class _ArxivText(HTMLParser):
    BLOCK = {"p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "figcaption", "tr", "div", "section", "blockquote"}
    SKIP = {"script", "style", "nav", "header", "footer", "button"}

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.parts: list[str] = []
        self.skip = 0
        self.math = 0
        self.inside = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "article":
            self.inside = True
        if not self.inside:
            return
        if tag in self.SKIP:
            self.skip += 1
        elif tag == "math":
            if not self.skip and not self.math:
                self.parts.append(" " + (a.get("alttext") or "") + " ")
            self.math += 1
        elif tag in self.BLOCK:
            self.parts.append("\n\n")
        elif tag in ("td", "th"):
            self.parts.append(" | ")

    def handle_endtag(self, tag):
        if tag == "article":
            self.inside = False
        if not self.inside:
            return
        if tag in self.SKIP and self.skip:
            self.skip -= 1
        elif tag == "math" and self.math:
            self.math -= 1
        elif tag in self.BLOCK:
            self.parts.append("\n\n")

    def handle_data(self, data):
        if self.inside and not self.skip and not self.math:
            self.parts.append(data)


def arxiv_html(raw: bytes) -> str:
    p = _ArxivText()
    p.feed(raw.decode("utf-8"))
    text = "".join(p.parts)
    paras = [re.sub(r"[ \t ]+", " ", para).strip() for para in re.split(r"\n\s*\n", text)]
    return normalize("\n\n".join(x for x in paras if x))


def repo_files(spec: dict) -> str:
    """Files of this repository at a pinned commit, concatenated in the listed order."""
    chunks = []
    for path in spec["paths"]:
        body = subprocess.run(["git", "-C", str(REPO), "show", f"{spec['commit']}:{path}"],
                              check=True, capture_output=True).stdout.decode("utf-8")
        chunks.append(normalize(body))
    return normalize("\n".join(chunks))


def build(doc: dict) -> bytes:
    kind = doc["source"]["kind"]
    if kind == "gutenberg":
        text = gutenberg(_get(doc["url"]))
    elif kind == "arxiv-html":
        text = arxiv_html(_get(doc["source"]["html"]))
    elif kind == "repo":
        text = repo_files(doc["source"])
    else:
        raise ValueError(f"unknown source kind {kind}")
    return text.encode("utf-8")


def stats(data: bytes) -> dict:
    return {"bytes": len(data), "words": len(data.decode("utf-8").split()),
            "sha256": hashlib.sha256(data).hexdigest()}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--refresh", action="store_true")
    ap.add_argument("--record", action="store_true")
    args = ap.parse_args(argv)
    manifest = json.loads(MANIFEST.read_text())
    bad = 0
    for doc in manifest["documents"]:
        path = CORPUS / doc["file"]
        if args.refresh or args.record or not path.exists():
            path.write_bytes(build(doc))
        got = stats(path.read_bytes())
        if args.record:
            doc.update(got)
        ok = got["sha256"] == doc.get("sha256")
        bad += not ok
        print(f"{'ok ' if ok else 'BAD'} {doc['file']}: {got['bytes']} bytes, {got['words']} words")
    if args.record:
        MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
        return 0
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
