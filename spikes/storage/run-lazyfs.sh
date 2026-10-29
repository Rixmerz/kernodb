#!/usr/bin/env bash
# Phase 1 exit item (phase1-design.md §6.6, R-84): scenario A of the engine on
# a LazyFS mount, with the unsynced pages of the mount thrown away between the
# writer and the checker. LazyFS keeps its own page cache and writes it to the
# backing directory only on fsync, so clearing it loses exactly what the engine
# never made durable, on a real file system.
#
# Steps: build LazyFS at a pinned commit, mount it, run a control that shows
# the mount does lose unsynced data, then for `sync` and `buffered` run the
# writer (tests/lazyfs.rs), clear the cache and run the checker.
#
# Exit code: 0 the run passed, 2 the run could not be made (no FUSE, a missing
# tool, a failed mount): the exit item stays pending, 1 anything failed.
# The last line of the log is `RESULT: PASS`, `RESULT: PENDING (<why>)` or
# `RESULT: FAIL`.
#
# Environment overrides (defaults in brackets):
#   LAZYFS_REPO  [https://github.com/dsrhaslab/lazyfs]
#   LAZYFS_REF   [fa7d32ee9710d55a7e529ac3c375f6616563c78b]  full commit hash; checked after checkout
#   LAZYFS_SRC   [<work dir>/lazyfs]   an existing checkout of LAZYFS_REF to reuse
#   WORK         [mktemp -d]           scratch directory: backing root, mount, state, FIFOs
#   OUT          [spikes/storage/results/<date>-f1]   where lazyfs.log is stored
set -euo pipefail
cd "$(dirname "$0")/../.."

LAZYFS_REPO="${LAZYFS_REPO:-https://github.com/dsrhaslab/lazyfs}"
LAZYFS_REF="${LAZYFS_REF:-fa7d32ee9710d55a7e529ac3c375f6616563c78b}"
OUT="${OUT:-spikes/storage/results/$(date +%Y-%m-%d)-f1}"
WORK="${WORK:-$(mktemp -d)}"
LAZYFS_SRC="${LAZYFS_SRC:-$WORK/lazyfs}"

ROOT="$WORK/root" # backing directory of the mount
MNT="$WORK/mnt"
STATE="$WORK/state" # outside the mount: what the writer promised
CONF="$WORK/lazyfs.toml"
FAULTS_FIFO="$WORK/faults.fifo"
DONE_FIFO="$WORK/faults-done.fifo"

mkdir -p -- "$OUT"
exec > >(tee -a -- "$OUT/lazyfs.log") 2>&1

RESULT="FAIL"
cleanup() {
  local rc=$?
  trap - EXIT
  if mountpoint -q -- "$MNT" 2>/dev/null; then
    fusermount3 -u -- "$MNT" || echo "warning: could not unmount $MNT"
  fi
  echo "RESULT: $RESULT"
  # Only a directory this script created is removed.
  if [[ -z "${KEEP_WORK:-}" && "$WORK" == "${TMPDIR:-/tmp}"/* ]]; then
    rm -rf -- "${WORK:?}"
  fi
  exit "$rc"
}
trap cleanup EXIT

pending() {
  RESULT="PENDING ($1)"
  echo "The LazyFS exit item stays pending: $1"
  exit 2
}

echo "== LazyFS scenario A, $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "kerno commit: $(git rev-parse HEAD)"
uname -a

# 1. Prerequisites. A missing one makes the run pending, never a pass.
[[ -r /dev/fuse && -w /dev/fuse ]] || pending "/dev/fuse is not available to this user"
for tool in fusermount3 cmake g++ git cargo mountpoint timeout; do
  command -v "$tool" >/dev/null || pending "$tool is not installed"
done
if [[ "$(id -u)" != 0 ]] && ! grep -qs '^user_allow_other' /etc/fuse.conf; then
  pending "user_allow_other is not set in /etc/fuse.conf (LazyFS mounts with allow_other)"
fi
[[ "$LAZYFS_REF" =~ ^[0-9a-f]{40}$ ]] || { echo "LAZYFS_REF must be a full commit hash"; exit 1; }

# 2. LazyFS at the pinned commit, built from source.
if [[ ! -d "$LAZYFS_SRC/.git" ]]; then
  git clone --quiet -- "$LAZYFS_REPO" "$LAZYFS_SRC"
fi
git -C "$LAZYFS_SRC" checkout --quiet --detach "$LAZYFS_REF"
[[ "$(git -C "$LAZYFS_SRC" rev-parse HEAD)" == "$LAZYFS_REF" ]] || { echo "LazyFS checkout is not $LAZYFS_REF"; exit 1; }
echo "lazyfs commit: $LAZYFS_REF"
(cd "$LAZYFS_SRC/libs/libpcache" && ./build.sh) || pending "libpcache did not build"
(cd "$LAZYFS_SRC/lazyfs" && ./build.sh) || pending "lazyfs did not build"

# 3. Mount. mount-lazyfs.sh exits 0 even when it fails, so the mount point and
# the FIFO are the only proof.
mkdir -p -- "$ROOT" "$MNT" "$STATE"
cat > "$CONF" <<EOF
[faults]
fifo_path="$FAULTS_FIFO"
fifo_path_completed="$DONE_FIFO"
[cache]
apply_eviction=false
[cache.simple]
custom_size="1gb"
blocks_per_page=1
[filesystem]
log_all_operations=false
logfile="$WORK/lazyfs.log"
EOF
(cd "$LAZYFS_SRC/lazyfs" && ./scripts/mount-lazyfs.sh -c "$CONF" -m "$MNT" -r "$ROOT" -s)
for _ in $(seq 1 60); do
  mountpoint -q -- "$MNT" && [[ -p "$FAULTS_FIFO" ]] && break
  sleep 0.5
done
mountpoint -q -- "$MNT" && [[ -p "$FAULTS_FIFO" ]] || pending "LazyFS did not mount"
echo "mounted on $MNT over $(df -T -- "$ROOT" | awk 'NR==2 {print $2}')"

# Throws away every unsynced page and waits for LazyFS to say it did.
clear_cache() {
  timeout 10 tee -- "$FAULTS_FIFO" >/dev/null <<<"lazyfs::clear-cache"
  timeout 30 head -n 1 -- "$DONE_FIFO" | grep -q finished
  echo "cache cleared"
}

# 4. Control: the mount loses an unsynced write and keeps a synced one. Without
# this a green run could mean that nothing was ever lost.
printf 'unsynced' > "$MNT/control-unsynced"
printf 'synced' > "$MNT/control-synced"
sync --data -- "$MNT/control-synced"
clear_cache
[[ ! -s "$MNT/control-unsynced" ]] || { echo "control: an unsynced write survived the cache clear"; exit 1; }
[[ "$(cat -- "$MNT/control-synced")" == synced ]] || { echo "control: a synced write was lost"; exit 1; }
echo "control: unsynced data lost, synced data kept"

# 5. Scenario A in both durability modes.
cargo test --release -p kerno-storage --test lazyfs --no-run
for mode in sync buffered; do
  echo "== mode $mode"
  for phase in write check; do
    if [[ "$phase" == check ]]; then
      clear_cache
    fi
    KERNO_LAZYFS_DB="$MNT/db-$mode" KERNO_LAZYFS_STATE="$STATE" KERNO_LAZYFS_MODE="$mode" \
      cargo test --release -p kerno-storage --test lazyfs -- --ignored --exact "lazyfs_scenario_a_$phase" --nocapture
  done
done

RESULT="PASS"
