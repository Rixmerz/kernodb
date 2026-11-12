"""Builds the `open` corpus from the committed directory vectors.

Run from crates/kerno-storage/fuzz: python3 -I seed_open.py
Each seed is the files of one small vector directory in the input format of
fuzz_targets/open.rs (name_len u8, name, data_len u32 LE, data); EXPECTED and
LOCK are not part of the image.
"""
import pathlib
import struct

SRC = pathlib.Path("../tests/vectors/v1/dirs")
DST = pathlib.Path("corpus/open")
MAX_BYTES = 16 * 1024  # small seeds only

DST.mkdir(parents=True, exist_ok=True)
for case in sorted(p for p in SRC.iterdir() if p.is_dir()):
    files = [f for f in sorted(case.iterdir()) if f.name not in ("EXPECTED", "LOCK")]
    blob = b"".join(
        bytes([len(f.name)]) + f.name.encode() + struct.pack("<I", f.stat().st_size) + f.read_bytes()
        for f in files
    )
    if len(blob) <= MAX_BYTES:
        (DST / case.name).write_bytes(blob)
