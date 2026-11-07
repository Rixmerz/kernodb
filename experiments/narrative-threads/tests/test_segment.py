import json
from pathlib import Path

from segment import segment

CORPUS = Path(__file__).resolve().parent.parent / "corpus"


def test_blocks_are_exact_byte_ranges():
    data = "# Título\n\nPrimer párrafo,\ncon dos líneas.  \n\n\n  Segundo — ñ.\n## Sub\nTercero\n".encode()
    blocks = segment(data)
    texts = [data[b.start:b.end].decode() for b in blocks]
    assert texts == ["# Título", "Primer párrafo,\ncon dos líneas.", "Segundo — ñ.", "## Sub", "Tercero"]
    assert [b.kind for b in blocks] == ["heading", "paragraph", "paragraph", "heading", "paragraph"]
    assert [b.ordinal for b in blocks] == list(range(5))


def test_crlf_and_empty():
    assert segment(b"") == []
    data = b"a\r\nb\r\n\r\nc"
    assert [data[b.start:b.end] for b in segment(data)] == [b"a\r\nb", b"c"]


def test_deterministic_and_ordered_on_corpus():
    for doc in json.loads((CORPUS / "MANIFEST.json").read_text())["documents"]:
        path = CORPUS / doc["file"]
        if not path.exists():
            continue
        data = path.read_bytes()
        blocks = segment(data)
        assert blocks == segment(data)
        assert all(a.end <= b.start for a, b in zip(blocks, blocks[1:]))
        for b in blocks:
            chunk = data[b.start:b.end]
            assert chunk == chunk.strip() and chunk and b"\n\n" not in chunk
