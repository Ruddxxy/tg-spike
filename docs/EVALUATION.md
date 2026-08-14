# Evaluation

A Track 2 canonical script for Telegraph. This document reports what
the script does on real miner traffic, measured against the protocol's
reference scoring module.

Every table below has a command that reproduces it. Every claim carries
its sample size.

---

## 1. Numeric separation

Ground truth `192.43`. Six miner answers.

| miner answer |         ours | reference | note                       |
| ------------ | -----------: | --------: | -------------------------- |
| `192.43`     | 1.0000000000 |    1.0000 | exact                      |
| `192.44`     | 0.9999969994 |    0.0000 | one cent out               |
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

```
cargo run -p corpus-eval --release -- separation
```

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

This is the central claim: a scorer's output should track real
accuracy. Correlation between score and negative absolute error in
Celsius.

| miner               |        n |       ours |  reference |
| ------------------- | -------: | ---------: | ---------: |
| bittensor-sn18-zeus |     3941 |     0.9061 |     0.2169 |
| openweathermap      |      323 |     0.8739 |     0.1319 |
| weatherapi          |     1905 |     0.9039 |     0.1962 |
| **all pooled**      | **6169** | **0.9001** | **0.2068** |

Per-miner accuracy and score:

| miner               |    n | err mean | err med | err p90 | ours mean | ours med | ref mean |  ref med |
| ------------------- | ---: | -------: | ------: | ------: | --------: | -------: | -------: | -------: |
| bittensor-sn18-zeus | 3941 |    1.262 |   0.950 |   2.900 |  0.545913 | 0.543396 | 0.035016 | 0.000000 |
| openweathermap      |  323 |    1.748 |   1.330 |   3.870 |  0.459608 | 0.365015 | 0.012384 | 0.000000 |
| weatherapi          | 1905 |    1.625 |   1.400 |   3.300 |  0.471929 | 0.388383 | 0.020997 | 0.000000 |

Error is in Celsius. The score column uses the bare rendering.

The reference median is 0.000000 for all three miners. It assigns the
same score to a miner 0.1 C out and a miner 10 C out, which is why its
correlation with real accuracy is 0.21.

**Do not read the per-miner error columns as a miner ranking.** The
samples are confounded: different cities, times, and forecast horizons,
and n ranges from 323 to 3941.

```
cargo run -p corpus-eval --release -- stats
```

### 2.4 The five known-bad rows

| group                        | in corpus | scored | ours mean | ref mean | err mean | penalised?            |
| ---------------------------- | --------: | -----: | --------: | -------: | -------: | --------------------- |
| alphavantage Ethereum        |         0 |      0 |         - |        - |        - | not in the scored set |
| weatherapi Maringa           |        73 |     72 |  0.347687 | 0.013889 |    1.586 | weakly                |
| openweathermap Miami climate |        40 |     40 |  0.563398 | 0.025000 |    0.983 | **no**                |
| weatherapi Lisbon            |         2 |      0 |         - |        - |        - | not in the scored set |
| openweathermap moon          |         1 |      0 |         - |        - |        - | not in the scored set |

Corpus mean of our score for comparison: 0.518548, n = 6169.

**None of the five is reliably caught. Two are not caught at all, and
three are not in the scored set.** The detail matters:

- **Maringa** resolved "Maringá PR Brazil" to Brazil, Indiana. It scores
  0.348 against a corpus mean of 0.519 — below average, but only
  because the Indiana forecast happens to be less accurate, not because
  the scorer detected a wrong continent.
- **Miami climate** answered an October 2022 question with an August
  2026 forecast. It scores **0.563, above the corpus mean**, with a
  mean error of 0.983 C — better than average.
- **Lisbon** (null result) and **moon** (town Moon, Iran) have no
  archive ground truth, so they were dropped before scoring.
- **Ethereum** is not in this corpus at all. The corpus builder handles
  only the three weather miners.

The reason is structural and worth stating plainly. The evaluator
receives one extracted value and one ground-truth value. It never sees
a location or a timestamp. It can only catch a wrong place or a wrong
date if that error produces a wrong _number_ against a correct truth.
In this corpus it does not, because the ground truth was joined at the
coordinates and valid time the **miner itself returned**. A miner that
answered for the wrong city was scored against the truth for that wrong
city. The pair is self-consistent and no value-comparing scorer can see
the error.

That is a limit of the corpus construction as much as of the scorer.
Catching these needs the evaluator to see the request parameters, which
the ABI does not provide.

```
cargo run -p corpus-eval --release -- knownbad
```

---

## 3. Ranking stability

Bootstrap rank-flip: rank miners by mean score, resample items with
replacement 2000 times using a fixed seed, recompute the ranking, and
count how often each adjacent pair swaps. Every miner uses the same
resampled item set in a round, so the comparison is paired.

The item is a paraphrase cluster. A cluster counts only when every
compared miner answered it.

**n = 2 to 5 clusters. This is a very small sample.** The corpus has 40
clusters; 7 hold more than one miner; 5 of those survive into the
scored set; only 2 are answered by all three miners.

All three miners together, n = 2 paired clusters:

| scorer    | rank 1 vs 2 flip | rank 2 vs 3 flip |
| --------- | ---------------: | ---------------: |
| ours      |            25.1% |            24.3% |
| reference |             0.0% |            25.1% |

Pairwise, which uses more clusters:

| pair                         |   n | ours flip | reference flip |
| ---------------------------- | --: | --------: | -------------: |
| zeus vs openweathermap       |   5 |      1.1% |           1.7% |
| zeus vs weatherapi           |   2 |      0.0% |           0.0% |
| openweathermap vs weatherapi |   2 |     24.3% |          25.1% |

**The comparison set is too small to rank these miners.** A 25% flip
rate on two items is a coin toss. The one comparison with any weight is
zeus versus openweathermap at n = 5 clusters and a 1.1% flip rate, and
five items is still not enough to publish a ranking.

This is a property of the corpus, not of either scorer: the three
weather miners rarely answer the same question at the same valid time.
Both scorers face the same limitation, and their flip rates are close
because the sample, not the metric, is the constraint.

```
cargo run -p corpus-eval --release -- rankflip
```

---

## 4. Adversarial results

Every strategy below is a test in
`crates/eval-script/tests/adversarial.rs`. The honest reference is a
miner 10% out, which scores 0.081.

| strategy                 | answer                   |     ours | reference |
| ------------------------ | ------------------------ | -------: | --------: |
| constant word            | `yes`                    | 0.000000 |    0.0000 |
| most common number       | `100`                    | 0.003886 |    0.0000 |
| subset of ground truth   | `malicious`              | 0.250000 |    1.0000 |
| empty                    | ``                       | 0.000000 |    0.0000 |
| control characters       | `\0\1\2`                 | 0.000000 |    0.0000 |
| long padded answer       | `malicious filler…`      | 0.500000 |    0.1250 |
| many candidate numbers   | `1 2 5 … 192.43 …`       | 0.083333 |    0.0833 |
| unit spoof, K value as C | `307.85 C`               | 0.000015 |    0.5000 |
| precision spam           | `192.4300000000001`      | 1.000000 |    0.0000 |
| hedge word               | `about 42`               | 1.000000 |    0.5000 |
| hedged range             | `34 to 36`               | 0.262188 |    0.0000 |
| negation                 | `not malicious`          | 0.000000 |    0.5000 |
| double negation          | `not not malicious`      | 1.000000 |    0.3333 |
| one common token         | `is`                     | 0.500000 |    1.0000 |
| copy question back       | (question verbatim)      | 0.000000 |    0.0000 |
| copy junk question back  | `[direct] 207 -> /price` | 0.000000 |    0.0000 |

Cross-branch farming, where the answer makes the scorer leave the
numeric path:

| ground truth                  | answer                  |     ours | reference |
| ----------------------------- | ----------------------- | -------: | --------: |
| `192.43 USD`                  | `USD`                   | 0.000000 |    1.0000 |
| `34.7 C`                      | `C`                     | 0.000000 |    1.0000 |
| `12 gwei`                     | `gwei`                  | 0.000000 |    1.0000 |
| `192.43 USD`                  | `USD USD USD USD…`      | 0.000000 |    1.0000 |
| `The temperature was 28.9 C.` | `the temperature was C` | 0.000000 |    0.7500 |
| `The temperature was 28.9 C.` | `temperature`           | 0.000000 |    1.0000 |

**The reference pays 1.0000 for the single string `USD`.** A miner that
answers with the unit and nothing else scores a perfect result against
the baseline on every priced intent.

```
cargo run -p corpus-eval --release -- crossbranch
cargo test -p eval-script --test adversarial
```

### 4.1 Strategies that still work

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

### 4.2 A defect this review found in our own scorer

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
cargo run -p corpus-eval --release -- separation
cargo run -p corpus-eval --release -- crossbranch
cargo run -p corpus-eval --release -- renderings
cargo run -p corpus-eval --release -- prepare
(cd tools/wazero-runner && go run . -corpus ../../corpus/eval-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/eval-scores.jsonl)
cargo run -p corpus-eval --release -- parsecov
cargo run -p corpus-eval --release -- stats
cargo run -p corpus-eval --release -- knownbad
cargo run -p corpus-eval --release -- rankflip

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
  multi-miner comparison set is 2 to 5 clusters. That is too small.
- **It does not show that wrong-location or wrong-time answers are
  caught.** Two such groups were measured and neither is penalised,
  for a structural reason given in section 2.4.
- **The correlation of 0.90 is measured against archive-derived truth
  on one intent family**, weather temperature, on 6,169 rows from three
  miners. It is not evidence about price or gas intents.
- **`TOLERANCE = 0.03` was chosen to produce 0.900 at 1% error.** No
  protocol document justifies that value. A different intent needs a
  different one, and the constant exists to make that a one-line change.
- **The corpus is one snapshot** of the daemon feed taken in August 2026. It is not a continuing sample.
