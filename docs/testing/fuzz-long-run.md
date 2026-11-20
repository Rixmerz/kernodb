# Long fuzzing run of kerno-storage (OPS-13)

Row OPS-13 of [COMPLETION-PLAN.md](../COMPLETION-PLAN.md) asks for a longer fuzzing campaign
than the weekly 60-second CI job and the 30-minute Stage 9 runs (E9-14). The Stage 9 runs
were on a loaded machine (load about 4 on 4 CPUs), and the targets that do file work per
input (`wal`, `manifest` and `open`) reached only 4.5 to 6.7 million executions.

[`scripts/fuzz-long-run.sh`](../../scripts/fuzz-long-run.sh) runs that campaign. The targets
and how to install the tools are in
[`crates/kerno-storage/fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md).

## What the script does

- Builds every target first, so a build failure stops the run before the machine time starts.
- Runs the targets one at a time with the command of the fuzz README:
  `cargo +nightly fuzz run <target> <work>/<target> fuzz/corpus/<target> -- -max_total_time=<secs> -max_len=65536 -timeout=1 -print_final_stats=1`.
  New inputs go to a working directory outside the repository; the committed corpus is
  only read.
- Appends `uptime` to `load.log` before the build, when the first target starts, every
  10 minutes and when it ends. The first line shows the machine as the run found it; the
  next lines carry the build's load for a few minutes.
- Writes `counts.tsv`: target, seconds, executions (libFuzzer's
  `stat::number_of_executed_units`), `cov`, `ft` and the exit code. `<target>.log` keeps
  each target's full output.
- Records a crash and goes on to the next target, so an unattended run is not lost to one
  failure. It exits with 1 at the end if any target failed and names them; the failing
  input is in `crates/kerno-storage/fuzz/artifacts/<target>/`.

The script's header lists its environment overrides (`scripts/fuzz-long-run.sh --help`).

## The owner's run

This run needs an idle machine, which only the owner can provide (decision 7 of the
completion plan). The defaults are the plan's campaign: `wal`, `manifest` and `open` for
8 h each, and the other 12 targets for 30 min each. That is **about 30 hours** in total.

```sh
OUT="$HOME/kerno-fuzz-long-run" scripts/fuzz-long-run.sh
```

Requirements for the run to count:

- **Load average below 1 for the whole run**, as the row says, shown by `load.log`. A
  running fuzzer is one busy process and adds about 1.0 to the load average by itself, so
  the logged value sits near 1 even on an idle machine. Read the log with that in mind;
  nothing else should be running. How to read the threshold is the owner's call.
- **Every target at or above its E9-14 execution count**, the table "Stage 9 run on the
  final code (E9-14)" of the fuzz README.
- **No crash.** A crash is fixed, and its input from `fuzz/artifacts/<target>/` is added
  to `fuzz/corpus/<target>/`.

Record the result as a section "Long run (date)" in `crates/kerno-storage/fuzz/README.md`,
with the table from `counts.tsv`, the toolchain versions, the commit of the code, and
`load.log` attached.

## Running in Docker

Owner decision, 2026-11-19: the fuzz campaign may run in Docker on dedicated CPU cores
while other work uses the remaining cores. **This applies to fuzzing only.** Benchmarks
(U2, R1 recovery, SQLite, U6) still need a quiet machine with load below 1.

Start it from the repository root (the image is built on first use: pinned nightly,
cargo-fuzz 0.13.2, all targets built, non-root user):

```sh
docker/fuzz/run.sh                  # cores 13-15, results in ./fuzz-results/<date>
CPUS=4-6 RESULTS=/data/fuzz docker/fuzz/run.sh
docker/fuzz/run.sh wal manifest     # only these targets
```

The container gets `--cpuset-cpus=$CPUS` and `--cpus=<number of cores>`. `run.sh` starts
one `scripts/fuzz-long-run.sh` per core, each under `taskset` on its own core, and gives
the targets to the cores by planned time: with the default three cores each takes one of
`wal`, `manifest` and `open` (8 h) and four of the 12 short targets (2 h), about 10 h in
total. `LONG_TARGETS`, `LONG_SECS`, `SHORT_SECS` and `MONITOR_SECS` pass through.

**Criterion for fuzzing:** each fuzz process runs pinned to its own dedicated core
(instead of host-wide load average below 1). Dedicated means nothing else of the host is
scheduled on those cores: keep them out of other containers, builds and `taskset` jobs.
Consider also isolating them from the kernel scheduler (`isolcpus`), which `run.sh` does
not do.

### Reading the results

Everything is under the results directory:

- `plan.txt`: which targets each core runs.
- `monitor.log`: one line every 10 min, on the host: UTC time, `/proc/loadavg`
  (`load=1min 5min 15min`, the host's, expected near the number of busy cores),
  `cpuN=<percent>` for each pinned core over a 5 s sample, `temp_max` (the hottest
  `/sys/class/thermal` zone, `n/a` if unreadable) and the execution count of every
  target so far (the last libFuzzer status line, or the final stats when it has finished).
- `core-<N>/counts.tsv`, `<target>.log`, `load.log`: as in "What the script does".
- `corpus/<target>/`: the working corpus; `artifacts/<target>/`: crashing inputs.

A healthy run shows each `cpuN` near 100 % in every line.

### When the run is inconclusive

Record the run as **inconclusive**, not as passed or failed, when:

- a pinned core was not dedicated: a `cpuN` well below 100 % for a stretch while the
  target was running, or other work known to have run on it;
- the machine throttled: a high `temp_max` with falling exec counts, or the kernel log
  reporting thermal or power throttling.

An inconclusive run is repeated, it does not count towards OPS-13. **Exec counts must still
reach the E9-14 counts** of the fuzz README; a dedicated, unthrottled run below them
fails the row the same as on an idle machine. A crash is a failure either way.

## Smoke run: 2026-11-19

A short check that the script works end to end. It is **not** the campaign: 20 s per
target, on a shared machine. Its counts must not be compared with E9-14.

- Machine: 14 CPUs, shared with other builds. The load average fell from 55 to 9 during
  the run (`load.log` below). This smoke run predates the `uptime` line written before
  the build.
- Toolchain: no nightly is installed here and there is no rustup, so the run used the
  system's stable `rustc 1.99.1` with `RUSTC_BOOTSTRAP=1` and `TOOLCHAIN=` (plain
  `cargo fuzz`), and cargo-fuzz 0.13.2. The owner's run uses nightly, the default.
- Code: the tree at commit `3a54bbc`.

```sh
TOOLCHAIN= RUSTC_BOOTSTRAP=1 LONG_SECS=20 SHORT_SECS=20 LOAD_INTERVAL=30 \
  OUT=/tmp/kerno-fuzz-smoke/out WORK=/tmp/kerno-fuzz-smoke/work scripts/fuzz-long-run.sh
```

It took 5 min 34 s including the build, and exited with 0.

| target | seconds | executions | cov | ft | exit |
|---|---:|---:|---:|---:|---:|
| commit | 20 | 217,452 | 228 | 488 | 0 |
| data_block | 20 | 1,354,344 | 153 | 226 | 0 |
| filter_block | 20 | 1,958,730 | 128 | 190 | 0 |
| footer | 20 | 1,855,317 | 65 | 67 | 0 |
| frame | 20 | 1,359,988 | 192 | 270 | 0 |
| header | 20 | 2,345,449 | 112 | 141 | 0 |
| index_block | 20 | 640,750 | 154 | 197 | 0 |
| manifest | 20 | 146,581 | 624 | 1,349 | 0 |
| open | 20 | 42,682 | 3,712 | 10,493 | 0 |
| props_block | 20 | 1,313,251 | 189 | 249 | 0 |
| rangedel_block | 20 | 2,088,309 | 153 | 216 | 0 |
| record | 20 | 1,723,930 | 183 | 223 | 0 |
| sst | 20 | 1,898,291 | 117 | 132 | 0 |
| varint | 20 | 2,953,690 | 72 | 73 | 0 |
| wal | 20 | 254,375 | 302 | 780 | 0 |

`load.log`, first and last lines:

```text
2026-11-19T18:35:03Z  15:35:03 up 59 min,  1 user,  load average: 55.31, 43.90, 33.60
2026-11-19T18:40:28Z  15:40:28 up  1:04,  1 user,  load average: 9.80, 23.60, 27.99
```

The failure path was checked too: `scripts/fuzz-long-run.sh varint no_such_target` wrote
`no_such_target 5 ? ? ? 1` to `counts.tsv`, named the target, and exited with 1. A
duration of `0` is refused with exit 2.

**Trap:** `crates/kerno-storage/fuzz/Cargo.lock` records the workspace crates at the
workspace version (`1.0.0-alpha.1` since the alpha preparation), so any change of that
version makes a `cargo fuzz` build rewrite it. Refresh it with
`cargo metadata --manifest-path crates/kerno-storage/fuzz/Cargo.toml --format-version 1 --offline > /dev/null`.
The smoke run's change to it was discarded. Commit the refreshed lock file together with the
owner's run, or before it.
