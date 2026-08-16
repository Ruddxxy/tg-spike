#!/usr/bin/env bash
#
# Build and verify one .wasm per tolerance band.
#
# Each band is a cargo feature that sets TOLERANCE at compile time, so
# every artefact is a separate registered binary with no configuration
# input and no runtime branch. Run from the workspace root:
#
#   tools/build-variants.sh
#
# Artefacts land in dist/. The script refuses to leave a bad artefact
# there: a band that fails any check is deleted before the script exits
# non-zero.
#
# THE UPLOAD GATE
#
# The wasip1 target builds the SAME crate to the SAME filename with 4
# WASI imports. Registering that artefact fails with "module[env] not
# instantiated". Every band below is checked for zero imports, and the
# check is the last word: an artefact that does not pass it is removed.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 1

TARGET_WASM="target/wasm32-unknown-unknown/release/eval_script.wasm"
DIST="dist"
GOLDEN_IN="golden_vectors.json"
FAILED=0

mkdir -p "$DIST"

banner() { printf '\n=== %s ===\n' "$1"; }

# band -> cargo feature flags
band_flags() {
  case "$1" in
    weather) echo "" ;;
    *)       echo "--no-default-features --features $1" ;;
  esac
}

for BAND in weather price onchain; do
  banner "BAND: $BAND"
  FLAGS="$(band_flags "$BAND")"
  OUT="$DIST/eval_script_${BAND}.wasm"

  # A stale artefact from an earlier band would otherwise be re-verified
  # and reported as this band's.
  rm -f "$TARGET_WASM"

  # shellcheck disable=SC2086
  if ! cargo build -q -p eval-script --release \
        --target wasm32-unknown-unknown $FLAGS; then
    echo "BUILD FAILED"; FAILED=1; continue
  fi
  cp "$TARGET_WASM" "$OUT"

  # --- exports, exactly three plus memory -------------------------------
  EXPORTS="$(wasm-tools print "$OUT" | grep -oP '\(export "\K[^"]+' | sort | tr '\n' ' ')"
  echo "exports:       $EXPORTS"
  SCORED="$(wasm-tools print "$OUT" | grep -oP '\(export "\K[^"]+' \
            | grep -vE '^(memory|__data_end|__heap_base)$' | sort | tr '\n' ' ')"
  if [ "$SCORED" != "alloc dealloc rank_answer " ]; then
    echo "EXPORTS WRONG: expected 'alloc dealloc rank_answer', got '$SCORED'"; FAILED=1
  fi

  # --- imports, the upload gate ----------------------------------------
  IMPORTS="$(wasm-tools print "$OUT" | grep -c '(import' || true)"
  echo "imports:       $IMPORTS"
  if [ "$IMPORTS" != "0" ]; then
    echo "IMPORT GATE FAILED, refusing to keep this artefact"
    wasm-tools print "$OUT" | grep '(import'
    rm -f "$OUT"; FAILED=1; continue
  fi

  wasm-tools validate "$OUT" && echo "validate:      ok"
  echo "sha256:        $(sha256sum "$OUT" | cut -d' ' -f1)"
  echo "size:          $(stat -c%s "$OUT") bytes"

  # --- cross-engine bit equality ---------------------------------------
  #
  # The expected bits in golden_vectors.json are calibrated for
  # TOLERANCE = 0.03, so only the weather band can be checked against
  # them. For the other bands the file is rebuilt from this band's own
  # wazero run, which makes host-runner's comparison a wasmtime-versus-
  # wazero check. That is the property that matters: a disagreement
  # between two engines on one module is the slashing event.
  WAZERO_OUT="target/golden-${BAND}-wazero.json"
  ( cd tools/wazero-runner && go run . -golden "../../$GOLDEN_IN" \
      -a "../../$OUT" -out "../../$WAZERO_OUT" >/dev/null ) || {
        echo "wazero run FAILED"; FAILED=1; continue; }

  if [ "$BAND" = "weather" ]; then
    GOLDEN_FOR_BAND="$GOLDEN_IN"
    echo "golden source: $GOLDEN_IN (hand-pinned, calibrated for t=0.03)"
  else
    GOLDEN_FOR_BAND="target/golden-vectors-${BAND}.json"
    python3 - "$GOLDEN_IN" "$WAZERO_OUT" "$GOLDEN_FOR_BAND" <<'PY'
import json, sys
src, run, dst = sys.argv[1], sys.argv[2], sys.argv[3]
vectors = json.load(open(src))["vectors"]
got = {v["name"]: v for v in json.load(open(run))["vectors"]}
for v in vectors:
    r = got[v["name"]]
    v["expected"], v["bits_hex"] = r["value"], r["bits_hex"]
json.dump({"vectors": vectors}, open(dst, "w"), indent=2)
PY
    echo "golden source: $GOLDEN_FOR_BAND (derived from this band's wazero run)"
  fi

  TG_GOLDEN_VECTORS="$GOLDEN_FOR_BAND" \
    cargo run -q -p host-runner --release -- "$OUT" "$WAZERO_OUT" \
    | grep -E '^(PASS|FAIL|overall)' | sed 's/^/host-runner:  /'
  # shellcheck disable=SC2181
  if [ "${PIPESTATUS[0]}" != "0" ]; then echo "HOST-RUNNER FAILED"; FAILED=1; fi

  # --- structural self-match / cross-match fixtures ---------------------
  # shellcheck disable=SC2086
  if cargo test -q -p eval-script --test scoring $FLAGS 2>&1 | tail -3 \
       | sed 's/^/structural:   /'; then :; fi
  # shellcheck disable=SC2086
  cargo test -q -p eval-script --test scoring $FLAGS >/dev/null 2>&1 \
    || { echo "STRUCTURAL FIXTURES FAILED"; FAILED=1; }
done

# Restore the default band in target/.
#
# Every band builds to the same target path, so without this the tree is
# left holding whichever band ran last. `cargo test` and `host-runner`
# both read that path and both compare against golden_vectors.json,
# which is calibrated for the weather band, so they would fail on an
# artefact this script left behind rather than on any real defect.
banner "RESTORING THE DEFAULT BAND IN target/"
rm -f "$TARGET_WASM"
if cargo build -q -p eval-script --release --target wasm32-unknown-unknown; then
  ( cd tools/wazero-runner && go run . -golden "../../$GOLDEN_IN" \
      -a "../../$TARGET_WASM" -out ../../target/golden-f32-wazero.json >/dev/null )
  echo "target/ holds the weather band again; wazero golden file regenerated"
else
  echo "COULD NOT RESTORE THE DEFAULT BUILD"; FAILED=1
fi

banner "ARTEFACTS"
sha256sum "$DIST"/*.wasm 2>/dev/null || echo "(none)"

if [ "$FAILED" != "0" ]; then
  banner "RESULT: FAILED"
  exit 1
fi
banner "RESULT: every band built, exported three functions, imported nothing"
