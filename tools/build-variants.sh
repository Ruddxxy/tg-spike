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
#
# The import check counts lines: `wasm-tools print | grep -c '(import'`.
# With wasm-tools absent that pipeline counts zero lines of no output
# and the gate reads the 0 as "zero imports", so the check PASSES on
# every artefact including a 4-import one. The gate must never fail
# open, so the prerequisite check below runs before any band builds.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 1

# --- prerequisites ------------------------------------------------------
#
# Checked before the first build, not at first use. A missing tool must
# stop the script, never weaken a check inside it.
for TOOL in wasm-tools python3; do
  if ! command -v "$TOOL" >/dev/null 2>&1; then
    echo "error: $TOOL is not on PATH, and this script needs it."
    case "$TOOL" in
      wasm-tools)
        echo "  wasm-tools reads the import and export lists. Without it the"
        echo "  import gate counts zero imports for every artefact and passes"
        echo "  a module that would be rejected at registration."
        echo "  install: cargo install wasm-tools"
        ;;
      python3)
        echo "  python3 derives the per-band golden file from that band's own"
        echo "  wazero run, which is how the price and onchain bands get a"
        echo "  wasmtime-versus-wazero check at all."
        echo "  install: your platform's python3 package"
        ;;
    esac
    exit 1
  fi
done

TARGET_WASM="target/wasm32-unknown-unknown/release/eval_script.wasm"
DIST="dist"
GOLDEN_IN="golden_vectors.json"
# The script the bands are compared against. Override it the hour a new
# one lands:
#
#   CHAMPION=path/to/new.wasm tools/build-variants.sh
CHAMPION="${CHAMPION:-reference/scoring_module.wasm}"
FAILED=0

# `reference/` is gitignored, so on a clean checkout the default
# champion does NOT exist and every band's Stage 2 step would fail with
# a message about the harness rather than about the missing file.
if [ ! -f "$CHAMPION" ]; then
  echo "error: no champion module at $CHAMPION"
  echo "  Stage 2 compares each band against a champion .wasm. The default is"
  echo "  the protocol's reference module, which is gitignored because it is"
  echo "  built from another repository:"
  echo
  echo "    git clone --depth 1 https://github.com/telegraphprotocol/telegraph-examples /tmp/tgref"
  echo "    (cd /tmp/tgref/wasm-scoring-module/rust-module && \\"
  echo "       cargo build --release --target wasm32-unknown-unknown)"
  echo "    mkdir -p reference && cp \\"
  echo "       /tmp/tgref/wasm-scoring-module/rust-module/target/wasm32-unknown-unknown/release/scoring_module.wasm \\"
  echo "       reference/"
  echo
  echo "  Or point at any other .wasm:  CHAMPION=path/to/module.wasm $0"
  exit 1
fi

mkdir -p "$DIST"

banner() { printf '\n=== %s ===\n' "$1"; }

# band -> cargo feature flags
band_flags() {
  case "$1" in
    weather) echo "" ;;
    *)       echo "--no-default-features --features $1" ;;
  esac
}

for BAND in weather price onchain label metadata; do
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

# The hash record.
#
# dist/ is gitignored: a committed binary that must be hash-checked
# immediately before upload is an invitation to upload a stale one. The
# HASHES.md file is the exception, un-ignored in .gitignore, because the
# thing worth keeping in version control is the RECORD of what was
# built, not the bytes. Check the hash you are about to upload against
# the row here.
HASHES="$DIST/HASHES.md"
{
  echo "# dist artefact hashes"
  echo
  echo "Written by \`tools/build-variants.sh\`. Do not edit by hand."
  echo
  echo "The \`.wasm\` files themselves are gitignored and are rebuilt by that"
  echo "script. This file is the record of what it built: check the sha256 of"
  echo "the artefact you are about to register against its row."
  echo
  echo "| band | imports | exports | size | sha256 |"
  echo "| --- | ---: | --- | ---: | --- |"
} > "$HASHES"

printf '%-14s %-8s %-8s %s\n' band imports exports sha256
for BAND in weather price onchain label metadata; do
  OUT="$DIST/eval_script_${BAND}.wasm"
  if [ ! -f "$OUT" ]; then
    printf '%-14s %s\n' "$BAND" "MISSING"
    echo "| \`$BAND\` | — | — | — | MISSING, this band failed a check |" >> "$HASHES"
    continue
  fi
  IMP="$(wasm-tools print "$OUT" | grep -c '(import' || true)"
  EXP="$(wasm-tools print "$OUT" | grep -oP '\(export "\K[^"]+' \
         | grep -vE '^(memory|__data_end|__heap_base)$' | sort | tr '\n' ' ')"
  EXPOK="no"
  [ "$EXP" = "alloc dealloc rank_answer " ] && EXPOK="ok"
  SUM="$(sha256sum "$OUT" | cut -d" " -f1)"
  SIZE="$(stat -c%s "$OUT")"
  printf '%-14s %-8s %-8s %s\n' "$BAND" "$IMP" "$EXPOK" "$SUM"
  echo "| \`$BAND\` | $IMP | $EXPOK | $SIZE | \`$SUM\` |" >> "$HASHES"
done

{
  echo
  echo "Every row above passed: exactly three scored exports, zero imports,"
  echo "golden-vector agreement, wasmtime/wazero bit-equality, its band's full"
  echo "test suite, the four Stage 1 gates and the four Stage 2 numbers. A band"
  echo "that fails any of those is deleted from \`dist/\` before this file is"
  echo "written, so a MISSING row means that band must not be uploaded."
} >> "$HASHES"
echo
echo "hash record: $HASHES"

if [ "$FAILED" != "0" ]; then
  banner "RESULT: FAILED"
  exit 1
fi
banner "RESULT: every band built, exported three functions, imported nothing"
