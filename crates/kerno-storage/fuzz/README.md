# kerno-storage fuzzing (E9A-04)

Coverage-guided fuzzing of the `kerno-storage` readers with `cargo-fuzz`
(libFuzzer). This crate is **outside the Kerno workspace** (it declares its own
`[workspace]`), so the workspace build, its MSRV (1.89) and CI never see it.
It needs a **nightly** toolchain and is **not run in CI**.

## Targets

One per decoder that `tests/struct_vectors.rs` exercises with random bytes
(`decoders_never_panic_on_random_bytes`, 14 kinds), calling the same public
entry points of `kerno_storage::format`, plus one for opening a database.

| target | entry point | fixed context (as the vectors) |
|---|---|---|
| `varint` | `coding::decode_varint` | none |
| `header` | `header::decode_header` | the three file kinds, with and without db id / file number |
| `commit` | `wal::decode_commit` + `Reader::finish` | none |
| `frame` | `wal::decode_frame` | file 1, offset 96, expected seq 1, slot on and off |
| `data_block` | `block::decode_data_block` | file 5, offset 96 |
| `filter_block` | `block::open_block` + `filter::decode_filter` | file 5, offset 2000 |
| `rangedel_block` | `block::open_block` + `rangedel::decode_rangedel` | file 5, offset 1500 |
| `props_block` | `block::open_block` + `props::decode_props` | file 5, offset 1600 |
| `index_block` | `block::open_block` + `sst::decode_index` | file 5, offset 1700 |
| `footer` | `sst::decode_footer` | file 5 |
| `record` | `manifest::decode_record` | generation 1, offset 96, expected edit seq 1 |
| `wal` | `wal::scan_wal` | file 3, expected seq 11 |
| `sst` | `sst::read_sst` | file 42, no `FileMeta`, `last_seq` = max |
| `manifest` | `manifest::replay_manifest` | none |
| `open` | `KernoDb::open` over `SimVfs` | see below |

The contexts are fixed (the vectors use several per decoder); a decoder that
branches on another context (a different offset, expected sequence or
`FileMeta`) is reached only through the in-process tests.

A target fails on a panic. `open` also fails on any error kind other than
`Corruption` or `Incompatible`. Input of `open` is a directory image:
repeated `name_len: u8, name, data_len: u32 LE, data`, restored into an
in-memory `SimVfs` (never the real disk) and opened read-write and read-only
with `Background::Inline`. Names the engine does not own are dropped, and a
read-only open of an image with no `MANIFEST` may return `InvalidArgument`,
because both are documented rejections (REC-13, DIR-2; "no database"), not
findings.

## Install

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

## Run

From `crates/kerno-storage`. New inputs go to a scratch directory first, so
the committed corpus (`fuzz/corpus/<target>/`, seeds from the struct and
directory vectors) is only read:

```sh
mkdir -p /tmp/fuzz-work/varint
cargo +nightly fuzz run varint /tmp/fuzz-work/varint fuzz/corpus/varint -- -max_total_time=60 -timeout=1
# open needs a larger input:
cargo +nightly fuzz run open /tmp/fuzz-work/open fuzz/corpus/open -- -max_total_time=60 -max_len=65536 -timeout=1
```

All targets, one at a time:

```sh
for t in $(cargo +nightly fuzz list); do
  mkdir -p /tmp/fuzz-work/$t
  cargo +nightly fuzz run $t /tmp/fuzz-work/$t fuzz/corpus/$t -- -max_total_time=60 -max_len=65536 -timeout=1 || break
done
```

`fuzz/seed_open.py` rebuilds `corpus/open` from `tests/vectors/v1/dirs`
(`cd fuzz && python3 -I seed_open.py`).

## Reproduce an artifact

A crash is written to `fuzz/artifacts/<target>/`; commit it there.

```sh
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>
```

## Acceptance of the acta: NOT met, still owed

The acta (etapa9-adelanto-acta.md, E9A-04) requires **30 minutes per target,
zero failures and no input slower than 1 s**. The runs done with this change
were **60 seconds per target** (15 targets, 15 minutes in all). That does
**not** meet the threshold; the 30-minute runs remain owed. Command (about
7.5 hours for the 15 targets):

```sh
cd crates/kerno-storage
for t in $(cargo +nightly fuzz list); do
  mkdir -p /tmp/fuzz-work/$t
  cargo +nightly fuzz run $t /tmp/fuzz-work/$t fuzz/corpus/$t -- -max_total_time=1800 -max_len=65536 -timeout=1 || break
done
```

## 60-second runs (nightly 1.102.0 2026-11-10, cargo-fuzz 0.13.2)

No crash, no input over 1 s (`-timeout=1`; slowest unit reported 0 s) in any
target. `cov` is libFuzzer's covered-edge count at the end of the run.

| target | executions | cov | ft |
|---|---:|---:|---:|
| varint | 17,791,940 | 72 | 72 |
| header | 11,102,319 | 126 | 158 |
| commit | 7,515,463 | 231 | 532 |
| frame | 10,021,770 | 213 | 322 |
| data_block | 9,320,381 | 155 | 206 |
| filter_block | 11,697,528 | 131 | 171 |
| rangedel_block | 11,418,383 | 150 | 190 |
| props_block | 10,004,696 | 212 | 291 |
| index_block | 11,274,335 | 160 | 200 |
| footer | 16,788,281 | 66 | 66 |
| record | 10,772,851 | 200 | 245 |
| wal | 795,215 | 328 | 1004 |
| sst | 7,338,340 | 142 | 200 |
| manifest | 763,024 | 655 | 1538 |
| open | 248,941 | 3718 | 10636 |
