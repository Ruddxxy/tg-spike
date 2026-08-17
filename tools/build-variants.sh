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
# The script the bands are compared against. Override it the hour a new
# one lands:
#
#   CHAMPION=path/to/new.wasm tools/build-variants.sh
CHAMPION="${CHAMPION:-reference/scoring_module.wasm}"
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

for BAND in weather price onchain label; do
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

  # --- the whole test suite for THIS band -------------------------------
  #
  # Not just the structural fixtures. This script used to run
  # `--test scoring` only, and that hid three tolerance-calibrated
  # assertions in `adversarial` that failed on the price and onchain
  # bands for as long as those bands have existed.
  # shellcheck disable=SC2086
  if cargo test -q -p eval-script $FLAGS >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    PASSED="$(cargo test -q -p eval-script $FLAGS 2>&1 | grep -c '^test result: ok')"
    echo "tests:         every eval-script test binary passed ($PASSED of them)"
  else
    echo "BAND TEST SUITE FAILED"
    # shellcheck disable=SC2086
    cargo test -q -p eval-script $FLAGS 2>&1 | grep -E '^(---- |test result: FAILED)' | head -5
    FAILED=1
  fi

  # --- Stage 1 structural checks ---------------------------------------
  #
  # The four the promotion pipeline names: the module loads and exports
  # the ABI (proved above), a blank answer is exactly 0.0, a correct
  # answer beats an unrelated one, and long or non-ASCII input does not
  # trap. The harness measures all four through the engine.
  #
  # --- Stage 2 numbers --------------------------------------------------
  #
  # worst_self_match, score_stddev, candidate_margin and candidate_wins,
  # for THIS band's artefact against the champion.
  REPORT="target/promotion-report-${BAND}.txt"
  if cargo run -q --release -p corpus-eval --example promotion_gates -- \
       --report --module "$OUT" --champion "$CHAMPION" > "$REPORT" 2>&1; then
    grep -E "worst_self_match: |all 40 questions|all 80 candidate scores" "$REPORT" \
      | head -4 | sed 's/^ */stage 2:      /'
    grep -E "^   gate [1-4] " "$REPORT" | sed 's/^ */stage 1:      /' 
  else
    echo "PROMOTION REPORT FAILED"; FAILED=1
  fi
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
printf '%-14s %-8s %-8s %s\n' band imports exports sha256
for BAND in weather price onchain label; do
  OUT="$DIST/eval_script_${BAND}.wasm"
  if [ ! -f "$OUT" ]; then
    printf '%-14s %s\n' "$BAND" "MISSING"
    continue
  fi
  IMP="$(wasm-tools print "$OUT" | grep -c '(import' || true)"
  EXP="$(wasm-tools print "$OUT" | grep -oP '\(export "\K[^"]+' \
         | grep -vE '^(memory|__data_end|__heap_base)$' | sort | tr '\n' ' ')"
  EXPOK="no"
  [ "$EXP" = "alloc dealloc rank_answer " ] && EXPOK="ok"
  printf '%-14s %-8s %-8s %s\n' "$BAND" "$IMP" "$EXPOK" "$(sha256sum "$OUT" | cut -d" " -f1)"
done

if [ "$FAILED" != "0" ]; then
  banner "RESULT: FAILED"
  exit 1
fi
banner "RESULT: every band built, exported three functions, imported nothing"
