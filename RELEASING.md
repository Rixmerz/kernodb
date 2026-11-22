# Releasing

A release is published only when every item of this gate is done. The gate is a person's decision,
not a script's: nothing here is run for you at publish time.

1. **CI is green** on the commit to publish: `.github/workflows/ci.yml`, the wheels (`wheels.yml`) and
   the npm package (`npm.yml`) workflows.
2. **The pre-release suite is green**, on the built packages, not only on the source tree:
   - `cd bindings/python && maturin develop && pytest` (the conformance suite is
     `tests/test_conformance.py`; it needs `pip install pytest hypothesis`),
   - `cd bindings/node && pnpm install --frozen-lockfile && pnpm build && pnpm test` (the conformance
     suite is `__test__/conformance.test.mts`),
   - `cargo test --workspace --all-features --release`.

   A claim of a README, a doc or a type doc that is not executed by one of these suites is a gap: add
   the test before releasing. Read the number of tests the runner reports, not only its colour.
3. **One real outside integration** is built on the published packages (`pip install --pre kernodb`,
   `npm install @rixmerz/kernodb@alpha`) by someone who works from the READMEs alone, with a task of
   their own. Every place they had to read the source, or guess, is a bug to fix or a doc to correct.
4. **One whole-API review** is done by someone who did not write the code: every public call, error
   and limit of the Rust facade, the Python package and the Node package, read against its docs.

Record the commit, the date, who did 3 and 4 and what they found in `docs/release/`.
