# Test suite timings

Measured with `cargo test --workspace --release --no-fail-fast`, every output line stamped, on a
14-thread development machine that was shared with other jobs (load average 5 to 10 during the
runs). "Finished in" is the time libtest reports for one test binary, without compilation. Treat
differences under about 10% as noise.

## Before and after

| Test binary (crate) | Before | After | What changed |
| --- | ---: | ---: | --- |
| `compaction` (kerno-storage) | 18.1 s | 4.5 s | The four seeds of `concurrent_compactions_preserve_model` run side by side: their injected delays are real time. Each seed still asserts the model, the snapshot after close and the reopen; the "two compactions at once" check takes the maximum over the seeds. |
| `operations` (kerno-server) | 10.0 s | 0.5 s | The silent-TLS-handshake test waited for the real 10 s server timeout. `KERNO_SERVER_HEADER_TIMEOUT_MS` (default 10000, see `docs/server.md`) lets the test use 500 ms. It still asserts that the server hangs up. |
| `derived_measure` (kerno-units) | 9.7 s | 3.9 s | The four measurements of `quality_over_the_frozen_set` (quality and equalities, for both engines) share only the read-only set and run side by side; the reports are checked as before. |
| `range_delete` (kerno-storage) | 13.5 s | 7.1 s | The six crash sweeps of `range_delete_crash_at_every_point` are independent (each point builds its own device) and run side by side. Same kinds, durabilities and filter. |
| Python `test_max_batch_ops` | 0.2 s | 0.2 s | The raised cap now uses 12 000 and a batch of 10 001 operations (one above the default cap of 10 000) instead of 60 000 and 50 000. It asserts the same two things: the default cap refuses, and a raised cap lets a larger batch through. |
| All Rust test binaries, summed | 256 s | 222 s | |
| First test binary to last, wall clock | 260 s | 227 s | |

The full Python suite is 43 tests in 0.4 s and the Node suite is 48 tests in 0.2 s (1.7 s with
`tsc --noEmit`); neither is a bottleneck.

## Slowest binaries left (after)

| Binary (kerno-storage unless noted) | Time | Why it is slow |
| --- | ---: | --- |
| `shared` | 27 s | Crash and fault sweeps "at every I/O point" for several modes, all run in parallel; they are CPU-bound, the longest single test is 3.7 s alone. |
| `dir_vectors` | 18.6 s | One test, `dir_vectors_exhaustive_bit_flips`, flips every bit of every byte of seven small databases (about 350 000 opens, 150 s of CPU). |
| `shared_ranges` | 16 s | Same sweeps as `shared`, with range deletions. |
| `ext_failure` | 15.7 s | Same sweeps with extended failure kinds, once inline and once with threads. |
| `assoc_reference` (kerno-units) | 11.7 s | Breadth-first oracle over the 64 frozen queries. |

## What was tried and not kept

* `dir_vectors`: splitting the bit-flip and truncation sweeps into 1024-offset jobs spread over a
  worker per core made the binary slower (23 to 25 s against 17.4 to 18.6 s, interleaved runs). The
  sweeps are CPU-bound and the two tests already fill the machine, so the existing thread per case
  stays.
* `assoc_reference` `frozen_corpus_traces_are_verifiable`: running the 64 queries in four threads does
  not compile, because the generic `E::Snapshot` is not `Sync`. Left as it was.

## No opt-in tests

No test was moved behind `#[ignore]` or an environment variable, so the CI workflow is unchanged.
The tests that were shrunk (the Python batch size, the TLS timeout) keep their assertions.

## Reproducing

Per-test times of one binary: `RUSTC_BOOTSTRAP=1 target/release/deps/<binary>-<hash> -Z unstable-options --report-time`.
