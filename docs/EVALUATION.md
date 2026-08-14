# Evaluation

A Track 2 canonical script for Telegraph. This document reports what
the script does on real miner traffic, measured against the protocol's
reference scoring module.

Every table below has a command that reproduces it. Every claim carries
its sample size.

---

## 1. Numeric separation

Ground truth `192.43`. Six miner answers. Both columns are the value
the compiled `.wasm` module returned under wazero, not a native
recomputation — see section 4 for why that distinction is made and
section 4.1 for the check behind it.

| miner answer |         ours | reference | note                       |
| ------------ | -----------: | --------: | -------------------------- |
| `192.43`     | 1.0000000000 |    1.0000 | exact                      |
| `192.44`     | 0.9999970198 |    0.0000 | one cent out               |
| `$192.43`    | 1.0000000000 |    0.0000 | same number, unit added    |
| `192.430`    | 1.0000000000 |    0.0000 | same number, trailing zero |
| `192.43 USD` | 1.0000000000 |    0.5000 | same number, unit added    |
| `999999.99`  | 0.0000000000 |    0.0000 | a million out              |

The reference gives **0.0000 to an answer one cent out and 0.0000 to an
answer a million out**. It cannot separate them. It also gives 0.0000 to
`$192.43` and to `192.430`, which are the same number written
differently.

`999999.99` prints as 0.0000000000 at ten decimal places. The exact
value is 3.3339206395588405e-11, bit pattern `0x2e12a09c`. It is not
zero; it is ten orders of magnitude below the one-cent answer. The
curve never returns exactly 0.0 for a finite error, which is what lets
it rank two wrong answers against each other.

Every digit above is an `f32`, because `rank_answer` returns `f32`. The
one-cent row reads 0.9999970198 rather than the 0.9999969994 an `f64`
computation gives; the difference is the narrowing the ABI performs,
and the ABI's value is the one the network sees.

This table, the strategy table, and the cross-branch table all come
from one command sequence, given in full in section 4.

### The curve

`score = t² / (t² + e²)` where `e` is relative error and `t =
TOLERANCE = 0.03`.

| relative error |        score |
| -------------: | -----------: |
|          0.00% | 1.0000000000 |
|          0.01% | 0.9999888890 |
|          1.00% | 0.9000000000 |
|          3.00% | 0.5000000000 |
|         10.00% | 0.0825688073 |
|         50.00% | 0.0035870865 |
|       1000.00% | 0.0000089999 |

The curve uses only `+ - * /`. IEEE-754 defines each as a single
correctly rounded operation, so every host returns identical bits. The
module calls no `ln`, `exp`, or `powf`, because those come from a host
maths library and two hosts can differ in the last bit. A one-bit
disagreement between validators is a slashing event.

`TOLERANCE` is the single-line variant point. Script registration is
per-intent, so a per-intent variant changes that one constant and
rebuilds. `0.03` suits a weather temperature intent in Celsius. See the
doc comment on `TOLERANCE` in `crates/eval-script/src/score.rs` for
suggested values for price and gas intents.

---

## 2. Corpus results

7,232 rows of real Telegraph daemon-feed traffic from three weather
miners, with ground truth derived from the Open-Meteo archive. 6,169
rows have ground truth and are scored.

The protocol standardises a miner answer into a single extracted value
before `rank_answer` sees it. The corpus stores the full upstream
response, so the evaluation extracts that single value first, using the
same normaliser that built the corpus.

```
cargo run -p corpus-eval --release -- prepare
cd tools/wazero-runner && go run . -corpus ../../corpus/eval-input.jsonl \
  -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
  -b ../../reference/scoring_module.wasm \
  -out ../../corpus/eval-scores.jsonl
```

Both modules run under wazero, the production host.

### 2.1 Parser coverage on real data

| input set             |    n | strict parse | quantity found |
| --------------------- | ---: | -----------: | -------------: |
| extracted miner value | 6169 |      100.00% |        100.00% |
| ground truth, bare    | 6169 |      100.00% |        100.00% |
| ground truth, prose   | 6169 |        0.00% |        100.00% |
| ground truth, JSON    | 6169 |        0.00% |        100.00% |
| question text         | 6169 |        0.00% |         52.62% |

Every input the ABI actually receives yields a quantity. The prose and
JSON renderings correctly fail the _strict_ single-value parser and
succeed under the lenient scanner, which is the intended split.

The question text is advisory and is never required. The 47.38% of
questions with no quantity are questions with no number in them, for
example `"What is the weather in Tokyo?"`. Eight distinct shapes cover
all of them.

```
cargo run -p corpus-eval --release -- parsecov
```

### 2.2 Score stability across the three ground-truth renderings

The real ground-truth format is undisclosed. A score that changes with
the rendering is a defect, because the miner does not choose the
rendering.

Per-row spread, highest score minus lowest across the three renderings,
n = 6169:

| scorer    | mean spread |   median |      p90 | rows identical |
| --------- | ----------: | -------: | -------: | -------------: |
| ours      |    0.000447 | 0.000000 | 0.000000 |   6159 (99.8%) |
| reference |    0.029502 | 0.000000 | 0.000000 |   5987 (97.0%) |

99.8% of rows score identically across bare, prose, and JSON. The
reference is 97.0% identical, but for a different reason: it scores
0.0000 on almost everything, and a constant is trivially stable.

### 2.3 Accuracy correlation

**Read this paragraph before the number below it.** The correlation
reported here is substantially circular, and the circularity is in the
corpus, not in the scorer. The miner's value comes from `temp_c` in its
own response. The ground truth is `actual_c` from the archive. The
error is `|temp_c - actual_c|`. Our score is a deterministic monotonic
function of that same difference. Correlating the score against the
error therefore largely measures that a monotonic curve is monotonic.
A high number here is close to arithmetically guaranteed, and a
reviewer should treat it as such.

Correlation between score and negative absolute error in Celsius:

| miner               |        n |       ours |  reference |
| ------------------- | -------: | ---------: | ---------: |
| bittensor-sn18-zeus |     3941 |     0.9061 |     0.2169 |
| openweathermap      |      323 |     0.8739 |     0.1319 |
| weatherapi          |     1905 |     0.9039 |     0.1962 |
| **all pooled**      | **6169** | **0.9001** | **0.2068** |

Given that caveat, here is what the number does and does not support.

**What survives scrutiny.** One thing, and it is a claim about the
reference rather than about us. Given the identical real inputs, the
reference scorer returns a median of **0.0000 for all three miners**.
It cannot separate a miner 0.1 C out from one 10 C out. Its correlation
is 0.21 not because the relationship is hard to detect but because it
mostly emits a constant. A monotonic curve reaching 0.90 where a
constant reaches 0.21 says little about the curve; that the protocol's
own baseline emits a constant on 6,169 rows of its own traffic is worth
knowing on its own.

Our own tracking of the error is NOT separate evidence, and this
document does not offer it as such. It is the circularity above,
restated. The claim that our scorer survives real traffic rests on
**section 2.1** instead, which does not use the correlation at all:
100% quantity extraction on 6,169 real miner values, across all three
ground-truth renderings, with 99.8% of rows scoring identically
whichever rendering is used. That is a measurement of parsing, unit
conversion, and rendering robustness on production text, and it stands
whether or not the correlation means anything.

**What does not survive scrutiny.** This is not independent evidence
that our scorer measures accuracy well. Establishing that needs ground
truth that does not derive from the miner's own answer — in particular
a truth joined at the question's location and time rather than at the
coordinates the miner returned. Building that is the ask-harness work
in Wave 5, and **it is not done**. Until it is, treat 0.9001 as a
consistency check, not as an accuracy result.

Per-miner accuracy and score:

| miner               |    n | err mean | err med | err p90 | ours mean | ours med | ref mean |  ref med |
| ------------------- | ---: | -------: | ------: | ------: | --------: | -------: | -------: | -------: |
| bittensor-sn18-zeus | 3941 |    1.262 |   0.950 |   2.900 |  0.545913 | 0.543396 | 0.035016 | 0.000000 |
| openweathermap      |  323 |    1.748 |   1.330 |   3.870 |  0.459608 | 0.365015 | 0.012384 | 0.000000 |
| weatherapi          | 1905 |    1.625 |   1.400 |   3.300 |  0.471929 | 0.388383 | 0.020997 | 0.000000 |

Error is in Celsius. The score column uses the bare rendering.

**Do not read the per-miner error columns as a miner ranking.** The
samples are confounded: different cities, times, and forecast horizons,
and n ranges from 323 to 3941.

```
cargo run -p corpus-eval --release -- stats
```

### 2.4 The five known-bad rows

**Zero of the five were reliably caught.** The Miami climate group,
which answered an October 2022 question with an August 2026 forecast,
scored **0.563 against a corpus mean of 0.519 — above average**. The
Maringá group, which answered for the wrong continent, scored 0.348,
below average but not because anything detected the error. The
remaining three never reached the scored set. There is no reading of
this table in which the scorer caught a known-bad answer.

| group                        | in corpus | scored | ours mean | ref mean | err mean | penalised?            |
| ---------------------------- | --------: | -----: | --------: | -------: | -------: | --------------------- |
| alphavantage Ethereum        |         0 |      0 |         - |        - |        - | not in the scored set |
| weatherapi Maringa           |        73 |     72 |  0.347687 | 0.013889 |    1.586 | weakly                |
| openweathermap Miami climate |        40 |     40 |  0.563398 | 0.025000 |    0.983 | **no**                |
| weatherapi Lisbon            |         2 |      0 |         - |        - |        - | not in the scored set |
| openweathermap moon          |         1 |      0 |         - |        - |        - | not in the scored set |

Corpus mean of our score for comparison: 0.518548, n = 6169.

Row by row:

- **Maringa** resolved "Maringá PR Brazil" to Brazil, Indiana. It scores
  0.348 against a corpus mean of 0.519 — below average, but only
  because the Indiana forecast happens to be less accurate, not because
  the scorer detected a wrong continent. A wrong-continent answer that
  landed near the right temperature by coincidence would have scored
  well, and nothing in the scorer would have objected.
- **Miami climate** is that coincidence. It answered an October 2022
  question with an August 2026 forecast and scored **0.563, above the
  corpus mean**, with a mean error of 0.983 C — better than average. The
  scorer rewarded it.
- **Lisbon** (null result) and **moon** (town Moon, Iran) have no
  archive ground truth, so they were dropped before scoring.
- **Ethereum** is not in this corpus at all. The corpus builder handles
  only the three weather miners.

**Why, and why it is a protocol observation rather than an excuse.**
Wave 2 joined the archive at the coordinates and valid time the **miner
itself returned**. A miner that answered for Brazil, Indiana was scored
against Indiana's actual weather. The pair is self-consistent by
construction, so the value comparison is correct and the answer is
still wrong. No value-comparing evaluator can detect that, because the
ABI hands the evaluator one extracted value and one ground-truth value
and never shows it the request parameters — not the location, not the
requested timestamp.

This is worth putting in front of the core team because it bounds what
any Track 2 submission can do. **While `rank_answer` receives only
`(question, ground_truth, miner_answer)`, and the truth is joined at
miner-supplied coordinates, no scoring rule can catch these — ours or
anyone's.** The limit is in the inputs, not in the rule applied to
them: the evidence that would distinguish a right answer from a
wrong-location one never reaches the module.

Two things would change that, and both sit outside the scoring module.
The request parameters could be added to the ABI, which is the core
team's call. Or the truth pipeline could resolve the location from the
question independently of the answer, which is a corpus change and is
what the Wave 5 ask-harness is built to do. It is not done, so this
document claims nothing on that front.

```
cargo run -p corpus-eval --release -- knownbad
```

---

## 3. Ranking stability: not measurable on this corpus

**No ranking result is published here, because the data cannot support
one.**

Ranking miners requires head-to-head data: the same question, at the
same valid time, answered by more than one miner. The Telegraph daemon
feed does not produce it. The protocol routes each question to one
miner, so the feed records exactly one answer per question and there is
nothing to compare directly. Paired comparisons had to be reconstructed
after the fact from paraphrase clusters — groups of questions that ask
the same thing in different words at the same valid time.

That reconstruction yields almost nothing. The corpus holds 40
clusters. 7 hold more than one miner. 5 of those survive into the
scored set, and only 2 are answered by all three miners. A bootstrap
flip rate computed on 2 items is a coin toss, and reporting it to one
decimal place would give it a precision it does not have.

This is a property of the routing, not of either scorer, and both
scorers face it equally. **It is worth reporting to the core team on
its own account**: a network whose feed is a single-miner-per-question
log cannot be used to compare miners retrospectively, however good the
scoring module is. Comparative evaluation needs either a feed that
records the losing candidates, or a client that asks the same question
repeatedly and lets routing spread it. The Wave 5 ask-harness targets
the second; it has not produced data yet.

The method itself is implemented and tested in
`crates/corpus-eval/src/bootstrap.rs` — paired resampling with a fixed
seed, one shared index set per round. It is correct and it becomes
usable the moment real head-to-head data exists. It is simply not run
against n = 2.

---

## 4. Adversarial results

**Both columns below are measured by running the compiled `.wasm`
modules under wazero** — ours and the protocol's reference module —
through the same harness path that produced the corpus columns in
section 2. Neither number comes from a reimplementation.

That distinction matters. An earlier draft of this table built the
reference column from a native Rust copy of the published
`word_overlap`. The copy is faithful, and section 4.1 shows the check
that proves it, but "we reimplemented their scorer and ours beats it"
is a claim a reviewer should not have to take on trust.

The honest comparison throughout is a miner 10% out, which scores
0.081. Every row is also a test in
`crates/eval-script/tests/adversarial.rs`.

| strategy                 | ground truth                 | answer                           |     ours | reference |
| ------------------------ | ---------------------------- | -------------------------------- | -------: | --------: |
| constant word            | `192.43`                     | `yes`                            | 0.000000 |    0.0000 |
| most common number       | `192.43`                     | `100`                            | 0.003886 |    0.0000 |
| subset of ground truth   | `high risk malicious binary` | `malicious`                      | 0.250000 |    1.0000 |
| empty                    | `192.43`                     | (empty)                          | 0.000000 |    0.0000 |
| control characters       | `192.43`                     | `\0\1\2`                         | 0.000000 |    0.0000 |
| long padded answer       | `malicious`                  | `malicious` + `filler` × 200     | 0.500000 |    0.0050 |
| many candidate numbers   | `192.43`                     | 14 numbers, one of them `192.43` | 0.071429 |    0.0714 |
| unit spoof, K value as C | `34.7 C`                     | `307.85 C`                       | 0.000015 |    0.5000 |
| precision spam           | `192.43`                     | `192.4300000000001`              | 1.000000 |    0.0000 |
| hedge word               | `42`                         | `about 42`                       | 1.000000 |    0.5000 |
| hedged range             | `35`                         | `34 to 36`                       | 0.262188 |    0.0000 |
| negation                 | `malicious`                  | `not malicious`                  | 0.000000 |    0.5000 |
| double negation          | `malicious`                  | `not not malicious`              | 1.000000 |    0.3333 |
| one common token         | `is malicious`               | `is`                             | 0.500000 |    1.0000 |
| copy question back       | `34.7 C`                     | the question verbatim            | 0.000000 |    0.0000 |
| copy junk question back  | `192.43`                     | `[direct] 207 -> /price`         | 0.000000 |    0.0000 |

Cross-branch farming, where the answer makes the scorer leave the
numeric path:

| ground truth                  | answer                  |     ours | reference |
| ----------------------------- | ----------------------- | -------: | --------: |
| `192.43 USD`                  | `USD`                   | 0.000000 |    1.0000 |
| `34.7 C`                      | `C`                     | 0.000000 |    1.0000 |
| `12 gwei`                     | `gwei`                  | 0.000000 |    1.0000 |
| `192.43 USD`                  | `USD` × 8               | 0.000000 |    1.0000 |
| `The temperature was 28.9 C.` | `the temperature was C` | 0.000000 |    0.7500 |
| `The temperature was 28.9 C.` | `temperature`           | 0.000000 |    1.0000 |

**The reference pays 1.0000 for the single string `USD`.** A miner that
answers with the unit and nothing else scores a perfect result against
the baseline on every priced intent.

Three rows state a repeat count, because the reference score depends
on it. The reference divides by the ANSWER token count, so a padded
answer's reference score is a function of how much padding it carries
and means nothing unless the count is stated: 200 repeats of `filler`
gives 0.0050, 7 repeats would give 0.1250. Our score is 0.500000 at any
repeat count, because a token set removes the duplicates.

```
cargo run -p corpus-eval --release -- adversarial-emit
(cd tools/wazero-runner && go run . -corpus ../../corpus/adversarial-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/adversarial-scores.jsonl)
cargo run -p corpus-eval --release -- adversarial-report
cargo test -p eval-script --test adversarial
```

### 4.1 The native copy is a test oracle, not a source

`crates/corpus-eval/src/baseline.rs` still holds a native copy of the
reference `word_overlap`. It no longer produces any number in this
document. `adversarial-report` recomputes every row with it and prints
the disagreements:

```
=== COMPILED MODULE VERSUS NATIVE COPY ===
all 28 rows agree to within 1e-6
the native word_overlap copy is faithful to the shipped module
```

All 28 rows agree, so the earlier draft's reference column was not
wrong about the reference module's behaviour. Three published numbers
nonetheless changed, none of them for that reason:

- The **spray** row was transcribed from a 12-number answer while the
  test sprays 14 numbers: 0.083333 becomes 0.071429.
- The **padding** row was transcribed from a 7-repeat padding while the
  case pads 200 times: the reference falls from 0.1250 to 0.0050.
- The **one cent out** row in section 1 carried an `f64` value,
  0.9999969994, for a result the ABI returns as `f32`. It is
  0.9999970198.

All three are now generated from
`crates/corpus-eval/src/adversarial.rs`, which is the single source for
every table in sections 1 and 4 — a row that is not in that file is not
in this report.

The harness also scores each case three times, under three
ground-truth fields holding identical text, and the report fails if the
three scores differ. That is a free determinism check on the harness
itself.

The `separation` and `crossbranch` subcommands still print native
tables, as a fast development view. They now print a banner saying so
and are not listed in section 6, because their numbers are not the
published ones.

### 4.2 Strategies that still work

Three, and a defect this review found in our own scorer.

**A JSON ground truth can be farmed with a date part.** A JSON
rendering carries a timestamp, so it holds several numbers. The scorer
cannot know which is the wanted value, so it takes the best match. The
answer `2026` scores **1.000000** against
`{"temperature_2m":28.9,"time":"2026-08-10T12:00"}`. The same answer
scores below 0.001 against the bare and prose renderings. Dividing by
the ground-truth number count instead would punish an honest miner for
a rendering it does not control. Test:
`known_weakness_a_json_truth_can_be_farmed_with_a_date_part`.

**One common token against a short ground truth scores 0.5.** `is`
against `is malicious` is one shared token out of two. Dividing by the
union kills the reference's 1.0000 but cannot push a one-of-two overlap
below one half. It decays as the ground truth grows: 0.167 against a
six-token truth. A fix needs rarity weighting, which needs a corpus the
module cannot carry. Test:
`known_weakness_one_common_token_against_a_short_ground_truth`.

**Repeated-word padding scores 0.5.** Token sets deduplicate, so
padding with one repeated word adds exactly one distinct token. Padding
with distinct words falls below 0.05.

Three rows in the table above read as profitable but are not attacks:
precision spam, hedge word, and double negation all score 1.0 because
they **are the correct answer** in an unusual form. `not not malicious`
means `malicious`.

### 4.3 A defect this review found in our own scorer

Wave 3 tested only inputs written by hand. Running the scorer against
real corpus renderings found two live defects:

1. **Echoing a junk question earned 0.1357.** The question `[direct] 207
-> /price` contains `207`, so an echo of it parsed as a _number_ and
   went to the numeric branch, where 207 against 192.43 scores 0.1357.
   The copied-question defence ran only in the text branch and never saw
   it. The check now runs before dispatch and covers every branch.

2. **A prose ground truth could be farmed for 0.667, and a JSON ground
   truth scored a correct answer 0.000.** Both came from one root cause:
   the scorer asked "does the whole string parse as one value?" instead
   of "does the ground truth contain a quantity?". A prose truth fell to
   token overlap, so returning the scaffolding words without the number
   paid 0.667 while an honest miner 10% out earned 0.081 — the farm paid
   eight times better than real work. A JSON truth has no whitespace, so
   number extraction found nothing and a correct answer scored zero.

Both are fixed and pinned by tests. The fix is the dispatch rule in
`score_answer`: when the ground truth carries a quantity and the answer
carries none, the score is 0.0, because the miner did not supply what
was asked for.

---

## 5. Determinism

The network can run either wasmtime or wazero. Two honest validators on
different engines must never disagree.

All 16 golden vectors are bit-identical across wasmtime and wazero.

```
$ wasm-tools print target/wasm32-unknown-unknown/release/eval_script.wasm | grep -c '(import'
0
```

Zero imports. The module needs nothing from the host beyond linear
memory. Function exports are exactly `alloc`, `dealloc`, `rank_answer`.

```
cargo run -p host-runner --release
```

---

## 6. Reproduction

```
# build both modules
cargo build -p eval-script --release --target wasm32-unknown-unknown
git clone --depth 1 https://github.com/telegraphprotocol/telegraph-examples /tmp/tgref
(cd /tmp/tgref/wasm-scoring-module/rust-module && cargo build --release --target wasm32-unknown-unknown)
mkdir -p reference && cp /tmp/tgref/wasm-scoring-module/rust-module/target/wasm32-unknown-unknown/release/scoring_module.wasm reference/

# tables
cargo run -p corpus-eval --release -- renderings
cargo run -p corpus-eval --release -- prepare
(cd tools/wazero-runner && go run . -corpus ../../corpus/eval-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/eval-scores.jsonl)
cargo run -p corpus-eval --release -- parsecov
cargo run -p corpus-eval --release -- stats
cargo run -p corpus-eval --release -- knownbad

# the adversarial tables, through the compiled modules
cargo run -p corpus-eval --release -- adversarial-emit
(cd tools/wazero-runner && go run . -corpus ../../corpus/adversarial-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/adversarial-scores.jsonl)
cargo run -p corpus-eval --release -- adversarial-report

# verification
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo run -p host-runner --release
```

The corpus is rebuilt with `cargo run -p corpus-builder`. It caches
every HTTP response, so a rerun makes zero network requests.

---

## 7. What this evaluation does not show

- **It does not show that our scorer ranks miners better.** The
  multi-miner comparison set is 2 to 5 clusters, so no ranking result
  is reported at all. See section 3.
- **It does not show that wrong-location or wrong-time answers are
  caught.** Zero of five known-bad groups were caught, and one scored
  above the corpus mean. See section 2.4 for why no value-comparing
  module can catch them under this ABI.
- **The correlation of 0.90 is substantially circular** — the score is
  a monotonic function of the same difference the correlation measures
  — and it is measured against archive-derived truth on one intent
  family, weather temperature, on 6,169 rows from three miners. It is
  not evidence about price or gas intents, and it is not an
  independent accuracy result. See section 2.3.
- **`TOLERANCE = 0.03` was chosen to produce 0.900 at 1% error.** No
  protocol document justifies that value. A different intent needs a
  different one, and the constant exists to make that a one-line change.
- **The corpus is one snapshot** of the daemon feed taken in August 2026. It is not a continuing sample.
