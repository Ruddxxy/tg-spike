# tg-spike — Telegraph Protocol Track 2 toolchain spike

A WASM evaluation script for the Telegraph Protocol, a native validator harness that proves it is
bit-deterministic, and a miner simulator that checks the scoring rule ranks miners correctly.

**Status: FROZEN.** This workspace is a de-risking spike. It was built before the protocol
published its Canonical Scripts. When they are published, expect the ABI and the metric to be
rewritten against the real specification. The value that carries over is the determinism
discipline, the malformed-input matrix, and the simulator harness.

```
167 tests passing   |   cargo test --workspace
```

---

## The three crates

**`crates/eval-script`** — the script itself. It compiles to a `cdylib` for
`wasm32-unknown-unknown`. A validator loads it and calls it with one `(ground_truth,
miner_response)` pair. It returns one Local Score in `[0,1]`. It implements a Brier score and a
normalised log loss. It parses JSON with `serde_json`, holds its own `ln` implementation, and
never calls a host function. It has no clock, no random number generator, no file system access,
and no network access. Every failure returns the worst score rather than a trap.

**`crates/host-runner`** — a native binary that stands in for a validator. It loads the compiled
`.wasm` with the `wasmtime` crate, drives the pointer-and-length memory ABI, and asserts the
properties the protocol needs: 1000 consecutive runs give bit-identical results, a batch score
does not depend on the order of its items, a fixed table of golden vectors matches by bit
equality, and a 27-case malformed-input matrix never traps. It loads the module as raw bytes,
the same way a validator would.

**`crates/miner-sim`** — a simulator that answers a different question: does the scoring rule
rank miners in the right order? It generates synthetic miners whose true quality it controls,
scores every response **through the WASM boundary**, and checks the resulting leaderboard against
invariants. It also separates the two layers the protocol has: the script produces a per-item
score, and the protocol aggregates and applies the ejection rule. It reports a Brier Skill Score
so "accurate" and "skilful" do not get confused.

---

## The ABI

```wat
(memory (export "memory"))
(func (export "alloc")          (param i32)             (result i32))
(func (export "dealloc")        (param i32 i32))
(func (export "score")          (param i32 i32 i32 i32) (result f64))
(func (export "score_log_loss") (param i32 i32 i32 i32) (result f64))
(func (export "score_batch")    (param i32 i32)         (result f64))
```

`alloc` and `dealloc` are the memory protocol. `score` is the export the protocol specified.
`score_log_loss` and `score_batch` are documented extensions: without an export that reaches
them, link-time optimisation removes the hand-rolled `ln` and the summation code, and the spike
would measure nothing.

The `wasm32-unknown-unknown` build also exports the linker symbols `__data_end` and `__heap_base`.
The `wasm32-wasip1` build does not. A validator that compares export sets exactly would see two
different modules.

**Call sequence.** The host calls `alloc(len)` for each input, writes the bytes into linear
memory, calls `score`, then calls `dealloc` for each block.

**`alloc` returns 0 to reject.** It returns 0 when the length is over `MAX_INPUT_BYTES`, when the
layout is invalid, or when the allocator fails. The caller must check the returned value before
it uses it as an address. Address 0 is never a valid block.

Inputs are UTF-8 JSON:

- ground truth: `{"label": 0}` or `{"label": 1}`
- miner response: `{"confidence": <float 0..1>}` — the probability of label 1
- `score_batch`: `[{"ground_truth": {...}, "response": {...}}, ...]`

---

## Score direction: HIGHER IS BETTER

**1.0 is perfect. 0.0 is the worst. Every failure path returns exactly 0.0.**

The source is the Telegraph whitepaper v1.0:

- **Section 7.4, request routing.** `P(route to Miner_m) = Score_m / SUM(Score_j)`. Traffic is
  proportional to the score. If a low score were good, the router would send all traffic to the
  worst miner.
- **Section 4.3, catch-rate promotion.** A script that scores a miner "above 0.70" is described as
  possibly _overscoring_ it. Overscoring means too high, so a high score means good.
- The protocol CTO: a miner that does not respond "scores zero for that round". Zero is the
  punishment, not the reward.

The script computes a loss internally and converts once, at the boundary: `score = 1.0 - loss`.
The batch path converts **per item and then averages**. It does not average and then convert.
Those are equal in exact arithmetic but not in floating point, because the batch path sorts into a
total order and uses Kahan summation, and the sorted order of `{1 - L}` is the reverse of `{L}`.

---

## Determinism rules, and why each one exists

Validators commit-reveal their Local Scores and take a stake-weighted median. A validator whose
score deviates from the median by more than 0.15 is penalised. So bit-identical results across
hosts are a correctness requirement, not a nicety.

| Rule                                                             | Why                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Hand-rolled `ln`, never `f64::ln`**                            | A libm transcendental can be a host-provided import and can differ between WASM hosts. `+ - * /` and `sqrt` are IEEE-754 exact in the WASM specification, so an `ln` built only from those is reproducible by construction. Measured error against std: **3 ULP** across the full f64 range, including subnormals. Cost: 388 bytes. |
| **Sort into a total order, then Kahan-sum**                      | Floating point addition is not associative, so a plain sum depends on the order the host supplies items in. Sorting removes the order dependence; Kahan reduces the accumulated rounding error. Sorting alone leaves precision on the table; Kahan alone is not order-independent. Both are needed.                                 |
| **`BTreeMap`, never `HashMap`**                                  | A `HashMap` does not promise an iteration order, and its default hasher is randomly seeded. A `BTreeMap` gives the same order on every host and every run.                                                                                                                                                                          |
| **`f64::total_cmp`, never `partial_cmp`**                        | `partial_cmp` returns `None` for NaN, which forces an `unwrap` that can panic. `total_cmp` is a total order over every f64 bit pattern, so no tie is left to chance and no sort can panic.                                                                                                                                          |
| **`sort_by`, never `sort_unstable`**                             | An unstable sort may order equal elements differently between implementations or versions.                                                                                                                                                                                                                                          |
| **NaN never reaches the output**                                 | NaN bit patterns are specification-non-deterministic in WASM. Two hosts can produce different NaN payloads for the same operation. A single choke point converts any non-finite or out-of-range value to the worst score.                                                                                                           |
| **No clock, no RNG, no file system, no network, no environment** | Every one of these is ambient state that differs per validator. `wasm32-unknown-unknown` is the chosen target precisely because it imports nothing at all.                                                                                                                                                                          |

Verified: the same input produces bit-identical `f64` results across `wasm32-unknown-unknown` and
`wasm32-wasip1` — two targets, two linkers, two instantiation paths.

---

## Consensus-relevant constants

`MAX_INPUT_BYTES` (1 MiB) is **consensus-relevant**. Two validators running scripts with different
caps will disagree about which miner responses are valid, produce different Local Scores for an
identical payload, and split the stake-weighted median. Whitepaper section 5.3 Category C
penalises the validators for that divergence, not the script author.

`eval-script` is the single definition. `host-runner` imports
`eval_script::MAX_INPUT_BYTES` rather than keeping its own copy.

**This fixes the problem for this workspace only.** A real validator has the `.wasm` binary, not
the Rust source, and cannot import a Rust constant. Two candidate production fixes:

1. **Export the cap as a WASM global** that the host reads from the module after instantiation.
   The cap then travels with the artifact it belongs to and cannot drift.
2. **Have the protocol specify the cap**, and require every canonical script to honour it. The cap
   becomes part of the consensus rules rather than a property of one script.

This is an open question for the protocol, not a defect in this crate. Neither fix is implemented
here.

---

## Known-open items

Carried forward verbatim so a future session does not re-derive them.

**1. f64 resolution is asymmetric above 0.5.** The `1.0 - loss` conversion spends precision at the
wrong end. f64 has 4,602,678,819,172,646,912 representable values in `[0.0, 0.5)` but only
4,503,599,627,370,496 in `[0.5, 1.0]` — **1,022x more resolution at the bad end**. Any
`|confidence - label| < 7.45e-9` now scores exactly 1.0, so near-perfect miners become
bit-identical. Routing is proportional to score, so discrimination at the _top_ is what sets
traffic share. This does not break consensus, because it is deterministic. It does reduce the
protocol's ability to separate the best miners.

**2. `ResponseKind::correct` is ambiguous at exactly `confidence == 0.5`.** In the simulator, an
item with a very small signal drives `u.powf(gamma)` to underflow, and the certainty lands on
exactly 0.5. At that value the reported confidence carries no information about which label the
miner predicted. This is reachable and occurs several times in a 5,000-item data set. It is not
theoretical. Any future code that treats `correct` as a function of the sign of
`confidence - 0.5` will be wrong on low-signal items.

**3. Ejection cannot reorder survivors.** The protocol's ejection rule removes a miner from the
routing pool. It never moves another miner. This is a mathematical guarantee, not a measurement:
the leaderboard sorts each miner on its own mean score, which does not depend on which other
miners are in the pool, so filtering rows before sorting cannot reorder the rows that remain. Only
the rank _numbers_ compress. The comparison output is therefore empty by design. It would stop
being true only if a future scoring rule made one miner's score depend on the others.

**4. The consensus-constant question** in the section above.

---

## Traps that will bite the next person

**The `u32`-pointer ABI is unsound to round-trip on 64-bit native.** `alloc` returns
`ptr as u32`, which truncates a real 64-bit heap address. This is correct on `wasm32`, where a
linear-memory address never needs more than 32 bits. It is a hard crash on a native target.
The native tests therefore **only exercise the error paths** — pointer overflow, a null pointer
with a non-zero length, a zero length, and an over-cap length — all of which return before any
raw memory read happens. Never "simplify" a native test by feeding an `alloc` result back into
`dealloc` or a read.

**Never hold a `&mut [u8]` memory view across an `alloc`.** `alloc` can grow linear memory, and
growth invalidates any data view the host already took. `write_bytes` re-fetches
`memory.data_mut(&mut store)` on every call and never stores a slice. Hoisting that fetch out of
a loop reintroduces silent memory corruption, not a clean error.

**`panic = "abort"` means `catch_unwind` is not a safety net.** There is no net. A panic is a WASM
trap, and a trap stops the validator. Every panicking path has to be removed by construction:
no `unwrap`, no `expect`, no indexing, no slicing that can go out of range. The library code holds
none of these; the tests may.

**`{"confidence": 1e400}` is valid JSON that parses to `+Inf`.** JSON puts no bound on the
exponent. It is syntactically well-formed and semantically lethal. A `is_nan()` check alone does
not catch it; the `is_finite()` check is load-bearing.

**`strip = true` and `twiggy` are mutually exclusive.** The release profile strips the name
section, so `twiggy` can only print `code[N]`. To read a size profile, rebuild with
`--config 'profile.release.strip="debuginfo"'` into a separate target directory.

**Seed hygiene.** The data set generator and the miner response generator use the same PRNG and
each draw 2 values per item. If both are seeded with the same value, the miner's correctness draw
lands on that item's own signal draw, and correctness becomes an anti-correlated function of
difficulty. Everything compiles, every test passes, and the leaderboard looks plausible. This
happened once. `ResponseSeed` now prevents it at compile time: `Dataset` cannot make one, and
there is no `From<u64>`.

---

## Build and run

```bash
# Build the script for the target the workspace uses.
cargo build --release --target wasm32-unknown-unknown -p eval-script

# Drive it as a validator would, and assert determinism.
cargo run -p host-runner --release

# Rank synthetic miners through the WASM boundary.
cargo run -p miner-sim --release

# Everything.
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`host-runner` takes an optional path argument, so the `wasm32-wasip1` module can be driven too:

```bash
cargo build --release --target wasm32-wasip1 -p eval-script
cargo run -p host-runner --release -- target/wasm32-wasip1/release/eval_script.wasm
```

`miner-sim` exits non-zero when an invariant fails. One invariant does fail on the skewed data
set, on purpose: a miner that is calibrated against the item signal but blind to the class base
rate is genuinely a worse forecaster than one that always predicts the majority class. The Brier
Skill Score table shows that **9 of 11 archetypes have negative skill** on that data set — they
are worse than a forecaster that knows only the base rate. That is a real property of a proper
scoring rule without a skill baseline, not a defect in the script.

---

## Language

Every comment, doc comment, and error message follows **ASD-STE100 Simplified Technical English**:
short active-voice sentences, one instruction per sentence, one word with one meaning, no idioms.
Keep it that way.
