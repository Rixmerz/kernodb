# Releasing Kerno binaries

Pushing a tag `v*` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml). It builds `kerno` (`crates/kerno-cli`) and `kerno-mcp` (`crates/kerno-mcp`) in release mode, packages one archive per target and attaches them to a GitHub release.

| Target | Runner | Archive |
|---|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` | `.tar.gz` |
| `x86_64-apple-darwin` | `macos-15-intel` | `.tar.gz` |
| `aarch64-apple-darwin` | `macos-latest` | `.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `.zip` (best effort until decided, see below) |

Each runner is native for its target, so no cross linker is involved. The `ubuntu-24.04-arm` runner is free only for public repositories; on a private one, use a paid larger runner or cross-compile.

Each archive is named `kerno-<tag>-<target>` and holds `kerno`, `kerno-mcp`, `LICENSE-APACHE`, `LICENSE-MIT` and `README.md`. The release also carries one `<archive>.sha256` per archive and a combined `SHA256SUMS`.

## Steps

1. Make sure `main` is green (CI workflow) and holds what you want to ship.
2. Update `version` in `[workspace.package]` of `Cargo.toml` if the release should report a new one, commit it and merge it. The workflow does not compare the tag with it.
3. Tag the commit and push the tag:

   ```sh
   git tag v1.0.0-alpha.1
   git push origin v1.0.0-alpha.1
   ```

   A tag with a hyphen (`v1.0.0-rc.1`) is published as a pre-release.
4. Watch the **Release** run in the Actions tab. The `release` job starts only when every `build` job has finished.
5. Open the release, check the notes (they are generated from the merged pull requests) and that every archive and `SHA256SUMS` are attached.

If a build fails, fix it on `main`, delete the tag (`git push --delete origin v1.0.0-alpha.1`, `git tag -d v1.0.0-alpha.1`) and tag again. If the `release` job already created the release, delete it in the GitHub UI first.

## Verifying a download

```sh
sha256sum -c kerno-v1.0.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz.sha256   # Linux
shasum -a 256 -c kerno-v1.0.0-alpha.1-aarch64-apple-darwin.tar.gz.sha256   # macOS
```

On Windows: `(Get-FileHash kerno-v1.0.0-alpha.1-x86_64-pc-windows-msvc.zip).Hash` and compare it with the `.sha256` file. The checksums prove the archive is the one the workflow produced; they are not a signature.

## Known limitation: Windows

`kerno-core`'s `StdVfs` now compiles for Windows (`seek_read`/`seek_write` behind `cfg(windows)` in `crates/kerno-core/src/vfs/std_fs.rs`), and the CI job `windows` (`.github/workflows/ci.yml`) builds `kerno` and `kerno-mcp` and runs the `kerno-core` and `kerno-storage` tests on `windows-latest`. Whether Windows is in the 1.0 binary set is still the owner's decision (row ENG-03 of [the completion plan](COMPLETION-PLAN.md)). Until then the Windows entry keeps `experimental: true` and `continue-on-error`: its failure does not block the release, which then ships without a `.zip` and without that line in `SHA256SUMS`. If Windows is kept and the `windows` job is green, remove `experimental: true` from the matrix entry, and the `continue-on-error` line with its `ponytail:` comment, so a Windows failure blocks the release again; do the same in `wheels.yml` and `npm.yml`. If Windows is out, drop the entry from the three workflows and say so here.

## Libraries: crates.io, PyPI and npm

The libraries are versioned with the binaries: `[workspace.package] version` in `Cargo.toml` (1.0.0-alpha.1), the `version` of the four path entries `kerno-core`, `kerno-storage`, `kerno-units` and `kerno` in `[workspace.dependencies]`, `version` in `bindings/python/Cargo.toml` (the wheel takes it from there) and in `bindings/node/Cargo.toml` and `bindings/node/package.json`. Change them together, then refresh the lock files (`cargo metadata --format-version 1 > /dev/null` at the root, in `bindings/python` and in `bindings/node`) and rebuild the Node module (`pnpm build` in `bindings/node`), whose generated `bindings/node/binding.js` carries the version.

**crates.io.** Only `kerno-core`, `kerno-storage`, `kerno-units` and `kerno` set `publish = true`; the workspace default stays `false`. The CI job `publish-dry-run` packages and builds the four on every push:

```sh
cargo publish --dry-run -p kerno-core -p kerno-storage -p kerno-units -p kerno
```

The owner publishes by running the same command without `--dry-run`, logged in with `cargo login`, from a clean checkout of the tagged commit. Publishing `kerno-storage` and `kerno-units` does not put them under the promise of [compatibility.md](compatibility.md) §4.

**PyPI.** [`.github/workflows/wheels.yml`](../.github/workflows/wheels.yml) runs on a `v*` tag and on demand (`workflow_dispatch`). It builds `abi3-py39` wheels for the targets of the table above and an sdist, installs each wheel on its runner and runs `pytest bindings/python/tests`. On a tag, its `publish` job uploads them through PyPI trusted publishing from the GitHub environment `pypi`. Before the first tag the owner registers the trusted publisher on PyPI (project `kernodb`, workflow `wheels.yml`, environment `pypi`) and creates the `pypi` environment with themself as required reviewer; without that approval nothing is uploaded.

**npm.** [`.github/workflows/npm.yml`](../.github/workflows/npm.yml) runs on a `v*` tag and on demand. It builds the native module for the same targets with napi-rs and runs `pnpm test` on each runner. On a tag, its `publish` job moves each module into its platform package (`napi create-npm-dirs`, `napi artifacts`) and runs `npm publish --provenance --access public`, whose `prepublishOnly` script (`napi prepublish -t npm`) publishes the platform packages first. Before the first tag the owner creates the GitHub environment `npm`, with themself as required reviewer and an npm automation token as its secret `NPM_TOKEN`.

To check either workflow without publishing, run it from the Actions tab (`workflow_dispatch`): it builds and tests, and its `publish` job is skipped.

## Checking the workflow without releasing

The workflow runs only on a tag, so to check a change to it without cutting a release:

```sh
actionlint .github/workflows/release.yml
cargo build --release --locked -p kerno-cli -p kerno-mcp
```

The first validates the YAML and the expressions; the second is the exact build step, for the host target.
