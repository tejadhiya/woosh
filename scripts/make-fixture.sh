#!/usr/bin/env bash
# Known-answer fixture tree for the engine suite and the sweep benchmark.
#
# Every branch here exists to pin down a specific behaviour that is easy to get
# silently wrong -- see the comments. Sizes are chosen so the totals are exact.
set -euo pipefail

OUT="${1:-.fixture}"
SCALE="${2:-1}"   # multiply the bulk tree for benchmarking

rm -rf "$OUT"
mkdir -p "$OUT"
echo "building fixture at $OUT (scale ${SCALE}x)"

# --- venvs under deliberately odd names -------------------------------------
# Detection must be structural, never name-based.
for name in .venv venv myenv mynenv custom-env; do
  d="$OUT/projects/py-$name/$name"
  mkdir -p "$d/bin" "$d/lib/python3.13/site-packages"
  printf 'home = /opt/homebrew/bin\nversion = 3.13.1\n' > "$d/pyvenv.cfg"
  printf '#!/bin/sh\n' > "$d/bin/activate"
  : > "$d/bin/python3"
  for pkg in requests-2.31.0 numpy-1.26.4 flask-3.0.0; do
    di="$d/lib/python3.13/site-packages/$pkg.dist-info"
    mkdir -p "$di"
    printf 'Metadata-Version: 2.1\nName: %s\nVersion: %s\n\nlong body\n' \
      "${pkg%-*}" "${pkg##*-}" > "$di/METADATA"
  done
  mkdir -p "$d/lib/python3.13/site-packages/payload"
  dd if=/dev/zero of="$d/lib/python3.13/site-packages/payload/blob.bin" \
     bs=1024 count=2048 status=none
done

# A venv with no METADATA at all: the folder-name fallback must still find it.
di="$OUT/projects/py-.venv/.venv/lib/python3.13/site-packages/orphan-9.9.9.dist-info"
mkdir -p "$di"

# --- conda-style environment -------------------------------------------------
mkdir -p "$OUT/projects/conda-proj/env/conda-meta"
: > "$OUT/projects/conda-proj/env/conda-meta/history"

# --- node_modules per package manager ---------------------------------------
for mgr in npm pnpm yarn bun; do
  p="$OUT/projects/js-$mgr"
  mkdir -p "$p/node_modules/.bin" "$p/node_modules/left-pad"
  case "$mgr" in
    npm)  : > "$p/package-lock.json" ;;
    pnpm) : > "$p/pnpm-lock.yaml" ;;
    yarn) : > "$p/yarn.lock" ;;
    bun)  : > "$p/bun.lockb" ;;
  esac
  dd if=/dev/zero of="$p/node_modules/left-pad/index.js" bs=1024 count=3072 status=none
done

# Yarn PnP: nodeLinker pnp means there is no real node_modules to report.
mkdir -p "$OUT/projects/js-pnp/node_modules"
printf 'nodeLinker: pnp\n' > "$OUT/projects/js-pnp/.yarnrc.yml"
dd if=/dev/zero of="$OUT/projects/js-pnp/node_modules/stub.bin" bs=1024 count=64 status=none

# --- hardlink farm: the dedup case ------------------------------------------
# Two "projects" sharing one store by hardlink. Summing them naively double-counts.
store="$OUT/store"
mkdir -p "$store"
dd if=/dev/zero of="$store/shared-blob.bin" bs=1024 count=4096 status=none
for i in 1 2; do
  d="$OUT/projects/linked-$i/node_modules/pkg"
  mkdir -p "$d"
  : > "$OUT/projects/linked-$i/pnpm-lock.yaml"
  ln "$store/shared-blob.bin" "$d/blob.bin"
done

# --- sparse file: allocated vs apparent size --------------------------------
# Apparent 64MB, allocated ~0. Using apparent size here would over-report wildly.
mkdir -p "$OUT/sparse"
dd if=/dev/zero of="$OUT/sparse/thin.img" bs=1 count=0 seek=64m status=none 2>/dev/null || \
  printf '' > "$OUT/sparse/thin.img"

# --- symlink escape: must be refused, never followed ------------------------
mkdir -p "$OUT/traps"
ln -sfn /System/Library "$OUT/traps/escape-absolute"
ln -sfn ../../.. "$OUT/traps/escape-relative"
ln -sfn "$OUT/traps/loop-b" "$OUT/traps/loop-a" 2>/dev/null || true
ln -sfn "$OUT/traps/loop-a" "$OUT/traps/loop-b" 2>/dev/null || true

# --- cargo target gate -------------------------------------------------------
# With Cargo.toml -> claimed. Without -> must be ignored.
mkdir -p "$OUT/projects/rs-real/target/debug"
printf '[package]\nname = "x"\n' > "$OUT/projects/rs-real/Cargo.toml"
dd if=/dev/zero of="$OUT/projects/rs-real/target/debug/bin" bs=1024 count=6144 status=none
mkdir -p "$OUT/projects/not-rust/target"
dd if=/dev/zero of="$OUT/projects/not-rust/target/data.bin" bs=1024 count=6144 status=none

# --- regenerable caches ------------------------------------------------------
for c in __pycache__ .pytest_cache .mypy_cache .ruff_cache .next .turbo; do
  mkdir -p "$OUT/projects/caches/$c"
  dd if=/dev/zero of="$OUT/projects/caches/$c/data.bin" bs=1024 count=6144 status=none
done

# --- build output that must be pruned but NOT reported ----------------------
for d in dist build; do
  mkdir -p "$OUT/projects/output/$d"
  dd if=/dev/zero of="$OUT/projects/output/$d/app.js" bs=1024 count=8192 status=none
done

# --- large files and package bundles ---------------------------------------
# Fake photos library package: reported as ONE advisory unit, never descended into.
mkdir -p "$OUT/projects/media/MyPhotos.photoslibrary/resources"
dd if=/dev/zero of="$OUT/projects/media/MyPhotos.photoslibrary/database.sqlite" bs=1024 count=8192 status=none
dd if=/dev/zero of="$OUT/projects/media/MyPhotos.photoslibrary/resources/thumb.jpg" bs=1024 count=2048 status=none

# Large files: .dmg installer and .mp4 video (8MB each, above test threshold 5MB)
dd if=/dev/zero of="$OUT/projects/media/sample_installer.dmg" bs=1024 count=8192 status=none
dd if=/dev/zero of="$OUT/projects/media/vacation.mp4" bs=1024 count=8192 status=none

# A sparse large file (apparent 1GB, alloc ~0)
dd if=/dev/zero of="$OUT/projects/media/huge_sparse.vmdk" bs=1 count=0 seek=1G status=none 2>/dev/null || \
  printf '' > "$OUT/projects/media/huge_sparse.vmdk"

# --- bulk tree for the benchmark --------------------------------------------
bulk="$OUT/bulk"
for s in $(seq 1 "$SCALE"); do
  for a in $(seq 0 9); do
    for b in $(seq 0 9); do
      d="$bulk/s$s/d$a/d$b"
      mkdir -p "$d"
      for f in $(seq 0 39); do : > "$d/f$f"; done
    done
  done
done

echo "fixture ready:"
echo "  dirs:  $(find "$OUT" -type d | wc -l | tr -d ' ')"
echo "  files: $(find "$OUT" -type f | wc -l | tr -d ' ')"
echo "  du:    $(du -sh "$OUT" | cut -f1)"
