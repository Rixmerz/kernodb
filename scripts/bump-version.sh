#!/usr/bin/env bash
# Sets every published version of the repository from one value to another, in one step.
# Usage: scripts/bump-version.sh 1.0.0-alpha.1 1.0.0-alpha.2
# Cargo.toml files take the SemVer form; Python takes its PEP 440 form from Cargo.toml
# through maturin (1.0.0-alpha.2 is published as 1.0.0a2). Run from the repository root.
set -euo pipefail
from="${1:?from version}"
to="${2:?to version}"
files=(
  Cargo.toml
  bindings/node/Cargo.toml bindings/node/package.json
  bindings/python/Cargo.toml
  clients/typescript/package.json clients/typescript/package-lock.json
  integrations/vercel-ai/package.json integrations/vercel-ai/package-lock.json
)
for f in "${files[@]}"; do
  if grep -q "\"$from\"\|= \"$from\"\|\"version\": \"$from\"" "$f"; then
    sed -i.bak "s/\"$from\"/\"$to\"/g" "$f" && rm -f "$f.bak"
    echo "updated $f"
  else
    echo "unchanged $f (no $from in it)"
  fi
done
# Refresh the lock files so --locked builds keep working.
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace
(cd bindings/python && (cargo update -p kerno-python --offline >/dev/null 2>&1 || cargo update -p kerno-python))
(cd bindings/node && (cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace))
# Projects outside the workspace that pin the crates in their own lock file (not published).
for d in examples/*/ crates/kerno-storage/fuzz/; do
  [ -f "$d/Cargo.lock" ] && (cd "$d" && (cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null 2>&1)) && echo "refreshed ${d}Cargo.lock"
done
echo "now: git grep -n \"$from\" to check nothing was left behind"
