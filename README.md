# tg-spike

A canonical scoring script for the Telegraph Protocol, written for Track 2, plus
the tooling used to measure it against the protocol's own reference module on
real miner traffic.

The script compiles to a `wasm32-unknown-unknown` module with three exports and
no imports. It scores a miner's answer against a ground truth on relative error,
using a curve built only from arithmetic that IEEE-754 defines exactly, so every
validator host returns identical bits.

```
296 tests   |   cargo test --workspace
```

## Results

Measured against the protocol's compiled reference module under wazero, the
production host. Full method, caveats, and reproduction commands are in
[`docs/EVALUATION.md`](docs/EVALUATION.md).

| Measurement                                                   |     This script |           Reference module |
| ------------------------------------------------------------- | --------------: | -------------------------: |
| Separates an answer 1 cent out from one a million out         |             yes |            no, both 0.0000 |
| Score stability across 3 ground-truth renderings, n=6169      | 99.8% identical |            97.0% identical |
| Quantity extracted from real miner values, n=6169             |            100% |                          — |
| Ranks 2 miners in the order an independent truth gives, n=200 |             yes |  no, ties both at 0.000000 |
| Correlation with real Celsius error, head-to-head set         |            0.68 | `NaN`, it emits a constant |
| Pays 1.0000 for the bare string `USD` against `192.43 USD`    |      no, 0.0000 |                        yes |

The evaluation also reports what it does not show: no known-bad row was reliably
caught under this ABI, the accuracy correlation is structurally circular, and the
ranking rests on 10 paired clusters. Section 7 of the evaluation lists the limits
in full.

## Layout

| Path                    | What it is                                                                                                                                                        |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/eval-script`    | The scoring script. Compiles to the `.wasm` module. No dependencies, no imports, no host calls.                                                                   |
| `crates/host-runner`    | A native stand-in for a validator. Loads the `.wasm` with `wasmtime`, drives the memory ABI, and asserts determinism and bit-equality against the wazero results. |
| `crates/corpus-builder` | Builds the evaluation corpus from the Telegraph daemon feed and the Open-Meteo archive. Caches every HTTP response.                                               |
| `crates/corpus-eval`    | Host-side reduction: prepares corpus rows for scoring, then turns the scored output into the evaluation tables.                                                   |
| `crates/ask-harness`    | Buys Telegraph inference over x402 on Base Sepolia, to obtain head-to-head data the daemon feed cannot supply.                                                    |
| `tools/wazero-runner`   | A Go host using `wazero`, the engine the network runs. Scores modules and writes the result files the Rust side checks against.                                   |
| `tools/org-watcher`     | Polls the `telegraphprotocol` account for the hackathon repository. Unrelated to scoring.                                                                         |

## The ABI

Three exports plus `memory`:

```wat
(memory (export "memory"))
(func (export "alloc")       (param i32)                    (result i32))
(func (export "dealloc")     (param i32 i32))
(func (export "rank_answer") (param i32 i32 i32 i32 i32 i32) (result f32))
```

`rank_answer` takes three `(ptr, len)` pairs — question, ground truth, miner
answer — and returns an `f32` in `[0,1]` where higher is better. The question
argument is present but unread in this version of the ABI; it is kept so a later
version can add question-aware scoring without another ABI break.

Both sides are short texts holding a single value. The protocol standardises a
miner response into one extracted value before `rank_answer` sees it, and the
ground-truth rendering is not specified, so the script reads several renderings
of one number and refuses to guess when a text has more than one meaning.

**Call sequence.** The host calls `alloc(len)` for each non-empty input, writes
the bytes into linear memory, calls `rank_answer`, then calls `dealloc` for each
block. A zero-length input is `ptr=0, len=0` and `alloc` is not called for it.
`alloc` returns 0 to reject — over `MAX_INPUT_BYTES`, invalid layout, or
allocation failure — so the caller must check before using the value as an
address. Address 0 is never a valid block.

The `wasm32-unknown-unknown` build also exports the linker globals `__data_end`
and `__heap_base`. These are rust-lld defaults, not part of the scored ABI; the
protocol's reference module exports them too.

## The score

```
score = t² / (t² + e²)      e = relative error, t = TOLERANCE = 0.03
```

| Relative error |        Score |
| -------------: | -----------: |
|          0.00% | 1.0000000000 |
|          1.00% | 0.9000000000 |
|          3.00% | 0.5000000000 |
|         10.00% | 0.0825688073 |
|       1000.00% | 0.0000089999 |

The curve never reaches exactly 0.0 for a finite error, which is what lets it
rank two wrong answers against each other. `TOLERANCE` is the single-line variant
point: script registration is per-intent, so a per-intent variant changes that one
constant and rebuilds. `0.03` suits a weather temperature intent in Celsius; see
the doc comment on `TOLERANCE` in `crates/eval-script/src/score.rs` for price and
gas suggestions.

## Determinism

Validators commit-reveal their Local Scores and take a stake-weighted median. A
validator whose score deviates by more than 0.15 is penalised, so bit-identical
results across hosts are a correctness requirement rather than a nicety. All 16
golden vectors are bit-identical across `wasmtime` and `wazero`.

The rules that produce that, and why each one is load-bearing:

| Rule                                                           | Why                                                                                                                                                                                                                                        |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Only `+ - * /` in the curve                                    | IEEE-754 defines each as one correctly rounded operation. `ln`, `exp` and `powf` come from a host maths library, where two engines can differ in the last bit.                                                                             |
| Compute in `f64`, narrow once at the `rank_answer` boundary    | A second narrowing point is a second rounding decision, and validators that round differently disagree on a score.                                                                                                                         |
| `BTreeMap`, never `HashMap`                                    | `HashMap` promises no iteration order and its default hasher is randomly seeded.                                                                                                                                                           |
| `f64::total_cmp`, never `partial_cmp`                          | `partial_cmp` returns `None` for NaN, forcing an `unwrap` that can panic. `total_cmp` totally orders every bit pattern.                                                                                                                    |
| `sort_by`, never `sort_unstable`                               | An unstable sort may order equal elements differently between versions.                                                                                                                                                                    |
| NaN never reaches the output                                   | NaN payloads are specification-non-deterministic in WASM. One choke point converts any non-finite or out-of-range value to the worst score.                                                                                                |
| No clock, RNG, file system, network, or environment            | All ambient state that differs per validator. `wasm32-unknown-unknown` imports nothing at all.                                                                                                                                             |
| `MAX_INPUT_BYTES` checked inside `alloc`, before memory growth | A deliberate divergence from the reference module, whose `alloc` returns a usable-looking pointer for an oversize request. Under wazero that pointer traps the module on the next call, making an oversize answer a cheap liveness attack. |

`panic = "abort"` means a panic is a WASM trap and a trap stops the validator, so
every panicking path is removed by construction: no `unwrap`, no `expect`, no
indexing or slicing that can go out of range in library code.

`MAX_INPUT_BYTES` is consensus-relevant. Two validators running scripts with
different caps disagree about which responses are valid and split the median.
`eval-script` is the single definition and `host-runner` imports it rather than
keeping a copy, which fixes the problem inside this workspace only: a real
validator has the `.wasm`, not the Rust source. Exporting the cap as a WASM global,
or specifying it at the protocol level, would fix it properly. Neither is
implemented here.

## Build and run

Rust stable with the `wasm32-unknown-unknown` target, and Go 1.21+ for the wazero
host.

```bash
rustup target add wasm32-unknown-unknown
```

The cross-host check needs the wazero result file to exist first, so run in this
order:

```bash
# 1. Build the script.
cargo build --release --target wasm32-unknown-unknown -p eval-script

# 2. Produce the wazero-side golden file with the Go host.
(cd tools/wazero-runner && go run . -golden ../../golden_vectors.json \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -out ../../target/golden-f32-wazero.json)

# 3. Drive the module as a wasmtime validator would. Asserts determinism and
#    bit-equality with the wazero results.
cargo run -p host-runner --release

# 4. Everything else.
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Skipping step 2 does not stop `host-runner`; its cross-host section fails on
purpose and prints the exact command to fix it. Missing cross-host evidence must
not silently pass.

`cargo run -p host-runner --release -- --show-cross-host` prints only the
cross-engine table. `host-runner` also takes an optional module path and an
optional wazero result path, so the `wasm32-wasip1` build can be driven too:

```bash
cargo build --release --target wasm32-wasip1 -p eval-script
cargo run -p host-runner --release -- \
   target/wasm32-wasip1/release/eval_script.wasm target/golden-f32-wazero.json
```

## Reproducing the evaluation

`corpus/` is not committed — it is 362 MB, almost all of it the raw daemon-feed
responses kept verbatim so the scoring tools see the exact bytes the daemon
recorded. Every table in `docs/EVALUATION.md` is regenerated from scratch by the
commands below, and section 6 of that document lists the full sequence.

```bash
# Build the corpus. Fetches the daemon feed and the Open-Meteo archive, then
# caches every response, so a second run makes zero network requests.
cargo run -p corpus-builder

# Build the protocol's reference module to compare against.
git clone --depth 1 https://github.com/telegraphprotocol/telegraph-examples /tmp/tgref
(cd /tmp/tgref/wasm-scoring-module/rust-module && \
   cargo build --release --target wasm32-unknown-unknown)
mkdir -p reference && cp \
   /tmp/tgref/wasm-scoring-module/rust-module/target/wasm32-unknown-unknown/release/scoring_module.wasm \
   reference/

# Score the corpus with both modules under wazero, then reduce to tables.
cargo run -p corpus-eval --release -- prepare
(cd tools/wazero-runner && go run . -corpus ../../corpus/eval-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm -out ../../corpus/eval-scores.jsonl)
cargo run -p corpus-eval --release -- stats
cargo run -p corpus-eval --release -- parsecov
cargo run -p corpus-eval --release -- knownbad
```

The head-to-head set in section 3 is bought, not fetched, and costs 2.00 testnet
USDC on Base Sepolia. `crates/ask-harness` runs `dry-run`, then `once`, then
`probe`, then `batch`, and each step gates the next. The signer refuses any chain
other than Base Sepolia, reads its key from an environment variable, and never
writes it to disk, to the response cache, or to a log line. The budget cap is a
hard refusal rather than a warning.
