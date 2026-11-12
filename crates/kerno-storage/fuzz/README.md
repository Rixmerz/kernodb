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

## Acceptance of the acta: met on 2026-11-12

The acta (etapa9-adelanto-acta.md, E9A-04) requires **30 minutes per target,
zero failures and no input slower than 1 s**. The 30-minute runs were done on
2026-11-12 on the local 4-CPU machine, four targets at a time, each with:

```sh
cargo +nightly fuzz run <target> <work>/<target> corpus/<target> -- -max_total_time=1800 -max_len=65536 -timeout=1
```

Every target ran its 1,801 seconds and ended with no crash, no timeout and no
slow unit. The machine was shared with other builds during the runs, so the
execution counts are lower than an idle machine would give.

| target | executions in 30 min | failures |
|---|---:|---:|
| varint | 405,996,980 | 0 |
| header | 211,566,339 | 0 |
| commit | 135,577,990 | 0 |
| frame | 219,033,334 | 0 |
| data_block | 103,763,480 | 0 |
| filter_block | 125,316,472 | 0 |
| rangedel_block | 195,665,581 | 0 |
| props_block | 221,177,481 | 0 |
| index_block | 75,895,281 | 0 |
| footer | 222,420,953 | 0 |
| record | 200,338,530 | 0 |
| wal | 5,698,047 | 0 |
| sst | 206,800,162 | 0 |
| manifest | 7,420,267 | 0 |
| open | 4,959,653 | 0 |

The working corpora of these runs were not committed; the committed corpus is
the one of the 60-second runs below.

## Stage 9 run on the final code (E9-14): 2026-11-12

Package E9-14 of the Stage 9 acta (`docs/architecture/etapa9-acta.md`, H-7)
repeats the 30-minute run on the code of `kerno-storage` as Stage 9 leaves it
(last change to `crates/kerno-storage/src`: commit `be27e74`; tree at
`cca7ef6`). Same command and limits as above, four targets at a time, with
nightly 1.102.0 and cargo-fuzz 0.13.2. Each run started from the committed
corpus and wrote to a working directory outside the repository.

Every target ran its 1,801 seconds and ended with no crash, no timeout and no
slow unit; `artifacts/` holds no file. No input was added to the committed
corpus, since none failed.

| target | executions in 30 min | cov | ft | failures |
|---|---:|---:|---:|---:|
| varint | 427,959,903 | 72 | 72 | 0 |
| header | 296,364,723 | 130 | 181 | 0 |
| commit | 207,625,874 | 231 | 570 | 0 |
| frame | 313,543,961 | 249 | 460 | 0 |
| data_block | 198,919,045 | 188 | 288 | 0 |
| filter_block | 262,139,947 | 160 | 230 | 0 |
| rangedel_block | 175,649,868 | 197 | 293 | 0 |
| props_block | 242,636,008 | 254 | 383 | 0 |
| index_block | 183,972,923 | 197 | 271 | 0 |
| footer | 499,398,250 | 67 | 67 | 0 |
| record | 310,873,073 | 263 | 355 | 0 |
| wal | 4,877,579 | 335 | 985 | 0 |
| sst | 198,623,700 | 156 | 228 | 0 |
| manifest | 6,676,172 | 662 | 1,616 | 0 |
| open | 4,546,088 | 3,838 | 11,053 | 0 |

Limits of this run, stated as they are:

- The acta asks for the machine to itself (section 3.2). It was not: other
  package builds and tests ran beside both batches (load average about 4 on
  4 CPUs), so the counts are lower than an idle machine would give.
- `wal`, `manifest` and `open` do file work per input and reached between 4.5
  and 6.7 million executions, two orders of magnitude below the pure decoders.
  Thirty minutes is thin for them; a longer run of those three is the natural
  next step.

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
