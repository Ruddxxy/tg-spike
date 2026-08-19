# Evaluation

A Track 2 canonical script for Telegraph. This document reports what the
script does on real miner traffic, measured against the protocol's
reference scoring module.

Every table below has a command that reproduces it. Every claim carries
its sample size.

**On the reference module.** Every "reference" column in this document is
the protocol's published `word_overlap` module, compiled and run under
wazero. The core team has confirmed that module is a simplified teaching
example, not the production scorer, which ships in the hackathon repo.
Read those columns as a documented baseline, not as the bar a candidate
has to clear. The comparison will be re-run against the real scorer when
it lands; `promotion_gates` already takes the champion as an argument so
that re-run needs no code change.

---

## 1. Numeric separation

Ground truth `192.43`. Six miner answers. Both columns are the value the
compiled `.wasm` module returned under wazero, not a native
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

The reference gives 0.0000 to an answer one cent out and 0.0000 to an
answer a million out, so it cannot separate them. It also gives 0.0000
to `$192.43` and to `192.430`, which are the same number written
differently.

`999999.99` prints as 0.0000000000 at ten decimal places. The exact
value is 3.3339206395588405e-11, bit pattern `0x2e12a09c`. It is not
zero; it is ten orders of magnitude below the one-cent answer. The curve
never returns exactly 0.0 for a finite error, which is what lets it rank
two wrong answers against each other.

Every digit above is an `f32`, because `rank_answer` returns `f32`. The
one-cent row reads 0.9999970198 rather than the 0.9999969994 an `f64`
computation gives. The difference is the narrowing the ABI performs, and
the ABI's value is the one the network sees.

This table, the strategy table, and the cross-branch table all come from
one command sequence, given in full in section 4.

### The curve

`score = t² / (t² + e²)` where `e` is relative error and `t = TOLERANCE
= 0.03`.

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

`TOLERANCE` is the tolerance variant point, and `0.03` suits a weather
temperature intent in Celsius. Script registration is per-intent, so a
per-intent variant is a separate registered binary; the band is a cargo
feature, so the value is folded into the curve at compile time.

Three of the four bands change only that constant. The fourth, `label`,
changes a dispatch rule instead and leaves the constant at the weather
figure — see section 5.3. See the doc comment on `TOLERANCE` in
`crates/eval-script/src/score.rs` for all four, and for why the price
and gas figures are reasoned rather than measured.

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
| ours      |    0.000000 | 0.000000 | 0.000000 |  6169 (100.0%) |
| reference |    0.029502 | 0.000000 | 0.000000 |   5987 (97.0%) |

Every row scores identically across bare, prose, and JSON. The
reference is 97.0% identical, but for a different reason: it scores
0.0000 on almost everything, and a constant is trivially stable.

This was 6159 (99.8%) before the quoted-string rule in section 4.3. The
10 rows that differed were the defect: each had a JSON timestamp whose
hour, 22 or 23, sat closer to the miner's temperature than the real
value did, so the JSON rendering paid a wrong answer up to 0.748 where
the bare rendering paid 0.025.

### 2.3 Accuracy correlation

The correlation reported in this section is substantially circular, and
the circularity is in the corpus rather than in the scorer. The miner's
value comes from `temp_c` in its own response. The ground truth is
`actual_c` from the archive. The error is `|temp_c - actual_c|`. Our
score is a deterministic monotonic function of that same difference.
Correlating the score against the error therefore largely measures that
a monotonic curve is monotonic, so a high number here is close to
arithmetically guaranteed.

Correlation between score and negative absolute error in Celsius:

| miner               |        n |       ours |  reference |
| ------------------- | -------: | ---------: | ---------: |
| bittensor-sn18-zeus |     3941 |     0.9061 |     0.2169 |
| openweathermap      |      323 |     0.8739 |     0.1319 |
| weatherapi          |     1905 |     0.9039 |     0.1962 |
| **all pooled**      | **6169** | **0.9001** | **0.2068** |

One claim does come out of this table, and it is a claim about the
reference rather than about this script. Given the identical real
inputs, the reference scorer returns a median of 0.0000 for all three
miners. It cannot separate a miner 0.1 C out from one 10 C out. Its
correlation is 0.21 not because the relationship is hard to detect but
because it mostly emits a constant. A monotonic curve reaching 0.90
where a constant reaches 0.21 says little about the curve; that the
protocol's own baseline emits a constant on 6,169 rows of its own
traffic is worth knowing on its own.

Our own tracking of the error is not separate evidence, and this
document does not offer it as such — it is the circularity above,
restated. The claim that our scorer survives real traffic rests on
section 2.1 instead, which does not use the correlation at all: 100%
quantity extraction on 6,169 real miner values, across all three
ground-truth renderings, with 100% of rows scoring identically
whichever rendering is used. That is a measurement of parsing, unit
conversion, and rendering robustness on production text, and it stands
whether or not the correlation means anything.

What the correlation does not establish is that our scorer measures
accuracy well. That needs ground truth which does not derive from the
miner's own answer — in particular a truth joined at the question's
location and time rather than at the coordinates the miner returned.
Section 3 has that, on 200 bought asks with an independently geocoded
truth. Read the accuracy claim there, not here, and treat 0.9001 as a
consistency check on this corpus rather than as an accuracy result.

Per-miner accuracy and score:

| miner               |    n | err mean | err med | err p90 | ours mean | ours med | ref mean |  ref med |
| ------------------- | ---: | -------: | ------: | ------: | --------: | -------: | -------: | -------: |
| bittensor-sn18-zeus | 3941 |    1.262 |   0.950 |   2.900 |  0.545913 | 0.543396 | 0.035016 | 0.000000 |
| openweathermap      |  323 |    1.748 |   1.330 |   3.870 |  0.459608 | 0.365015 | 0.012384 | 0.000000 |
| weatherapi          | 1905 |    1.625 |   1.400 |   3.300 |  0.471929 | 0.388383 | 0.020997 | 0.000000 |

Error is in Celsius. The score column uses the bare rendering.

**The per-miner error columns are not a miner ranking.** The samples are
confounded: different cities, times, and forecast horizons, and n ranges
from 323 to 3941.

```
cargo run -p corpus-eval --release -- stats
```

### 2.4 The five known-bad rows

**Zero of the five were reliably caught.** The Miami climate group,
which answered an October 2022 question with an August 2026 forecast,
scored 0.563 against a corpus mean of 0.519 — above average. The Maringá
group, which answered for the wrong continent, scored 0.348, below
average but not because anything detected the error. The remaining three
never reached the scored set. There is no reading of this table in which
the scorer caught a known-bad answer.

| group                        | in corpus | scored | ours mean | ref mean | err mean | penalised?            |
| ---------------------------- | --------: | -----: | --------: | -------: | -------: | --------------------- |
| alphavantage Ethereum        |         0 |      0 |         - |        - |        - | not in the scored set |
| weatherapi Maringa           |        73 |     72 |  0.347687 | 0.013889 |    1.586 | weakly                |
| openweathermap Miami climate |        40 |     40 |  0.563398 | 0.025000 |    0.983 | no                    |
| weatherapi Lisbon            |         2 |      0 |         - |        - |        - | not in the scored set |
| openweathermap moon          |         1 |      0 |         - |        - |        - | not in the scored set |

Corpus mean of our score for comparison: 0.518548, n = 6169.

Row by row:

- Maringa resolved "Maringá PR Brazil" to Brazil, Indiana. It scores
  0.348 against a corpus mean of 0.519 — below average, but only because
  the Indiana forecast happens to be less accurate, not because the
  scorer detected a wrong continent. A wrong-continent answer that
  landed near the right temperature by coincidence would have scored
  well, and nothing in the scorer would have objected.
- Miami climate is that coincidence. It answered an October 2022
  question with an August 2026 forecast and scored 0.563, above the
  corpus mean, with a mean error of 0.983 C — better than average. The
  scorer rewarded it.
- Lisbon (null result) and moon (town Moon, Iran) have no archive ground
  truth, so they were dropped before scoring.
- Ethereum is not in this corpus at all. The corpus builder handles only
  the three weather miners.

The cause is in the corpus construction, and it bounds every submission
rather than only this one. The daemon-feed corpus joined the archive at
the coordinates and valid time the miner itself returned. A miner that
answered for Brazil, Indiana was scored against Indiana's actual
weather. The pair is self-consistent by construction, so the value
comparison is correct and the answer is still wrong. No value-comparing
evaluator can detect that, because the ABI hands the evaluator one
extracted value and one ground-truth value and never shows it the
request parameters — not the location, not the requested timestamp.

**While `rank_answer` receives only `(question, ground_truth,
miner_answer)`, and the truth is joined at miner-supplied coordinates,
no scoring rule can catch these — ours or anyone's.** The limit is in
the inputs, not in the rule applied to them: the evidence that would
distinguish a right answer from a wrong-location one never reaches the
module.

Both fixes sit outside the scoring module. The request parameters could
be added to the ABI, which is the core team's call. Or the truth
pipeline could resolve the location from the question independently of
the answer, which is a corpus change rather than a scoring change.
Section 3 does the second: its truth is geocoded from the question's own
city list and joined at the ask timestamp, so a wrong-location answer
would be caught there. No wrong-location answer occurred in that set, so
the guard is in place but has not yet been tested by a real failure.

```
cargo run -p corpus-eval --release -- knownbad
```

---

## 3. Ranking stability, on bought head-to-head data

The daemon feed cannot rank miners. It routes one miner per question, so
paired comparisons had to be reconstructed from paraphrase clusters and
only 2 survived into the scored set. A flip rate on 2 items is a coin
toss.

So the data was bought instead. 200 paid asks over the Engine's
auto-routed endpoint, 10 cities, 20 asks per city, one fixed query
string per city. 200 of 200 answered, zero failures, 10 of 10 cities
paired. Cost 2.00 USDC on Base Sepolia testnet, every ask settled
on-chain and individually logged.

A pair here is strict: 2 or more distinct miners answered the same query
string. Twenty answers from one miner is not a pair; it is twenty
samples of one miner and it ranks nothing.

### 3.1 The ground truth is independent of the answer

The city list is fixed in the batch plan. Each city is geocoded once
through Open-Meteo, and the geocoder must return the country that was
asked for rather than the first hit — the check that would have caught
the daemon-feed corpus sending "Maringá PR Brazil" to Brazil, Indiana.
The archive is then joined at those coordinates and at the hour nearest
the ask timestamp, which is stamped client-side before the request is
sent.

No coordinate and no join timestamp comes from a miner response. A miner
that answered for the wrong city would now be scored against the right
city's weather and the error would show. This is what section 2.4 says
the daemon-feed corpus could not do.

### 3.2 Per-miner accuracy

| miner          |   n | mean \|e\| | median \|e\| | mean signed e | worst |
| -------------- | --: | ---------: | -----------: | ------------: | ----: |
| OpenWeatherMap | 123 |      1.108 |        1.060 |        +0.536 |  2.09 |
| WeatherAPI     |  77 |      2.244 |        2.500 |        -0.821 |  5.80 |

Error is in Celsius against the archive actual at the geocoded city and
the ask hour. OpenWeatherMap is about twice as accurate, running
slightly warm; WeatherAPI runs cold.

### 3.3 The scorers, against that accuracy

| scorer    | OpenWeatherMap | WeatherAPI | ranks them correctly? |
| --------- | -------------: | ---------: | --------------------- |
| ours      |         0.3792 |     0.2455 | yes                   |
| reference |       0.000000 |   0.000000 | no — it cannot rank   |

Our score puts OpenWeatherMap above WeatherAPI, which is the order the
independent accuracy measurement gives. The reference assigns 0.000000
to both, on all 200 rows, under all three ground-truth renderings.

Correlation between score and negative absolute error, on this set:

| miner          |   n |   ours | reference |
| -------------- | --: | -----: | --------: |
| OpenWeatherMap | 123 | 0.8631 |   **NaN** |
| WeatherAPI     |  77 | 0.6880 |   **NaN** |
| **all pooled** | 200 | 0.6789 |   **NaN** |

The reference's correlation is `NaN`. It is not a low number; there is
no number. A correlation divides by the standard deviation of each
series, and a scorer that emits the same constant for every input has a
standard deviation of zero. There is nothing to correlate.

### 3.4 Ranking stability

Bootstrap rank-flip over the 10 paired clusters, 2000 resamples, fixed
seed, one shared index set per round:

| scorer    | rank 1 vs 2 flip rate |
| --------- | --------------------: |
| ours      |                 19.9% |
| reference |                  0.0% |

**n = 10 clusters is small.** A 19.9% flip rate means the ordering holds
in about four resamples out of five, which is suggestive and not
settled. It is five times the paired data the daemon-feed corpus
produced, and it is still not enough to publish a miner ranking. The
reference's 0.0% is not stability: two miners tied at exactly 0.000000
never swap because neither ever moves.

### 3.5 Limits of this measurement

"Accuracy" here means agreement with Open-Meteo, which is a reanalysis
model rather than station observations, while WeatherAPI is
station-derived. A methodology gap between a model and a station network
would systematically favour whichever miner is closer to Open-Meteo's
own model, and that miner is not necessarily the more accurate one in
the world. The gap is largest at Dubai, where WeatherAPI reported 35.5 C
against an archive actual of 41.3 C.

To check whether those gaps were location errors rather than measurement
differences, the coordinates WeatherAPI returned were read back for its
three worst cities — Dubai, Singapore and Nairobi — and all three were
correct. These are genuine measurement disagreements. Reading a miner's
coordinates for that check is diagnosis; it never feeds the ground-truth
join.

The correlation also remains structurally circular. The score is a
monotonic function of the same difference the correlation measures, so a
high value is close to arithmetically guaranteed, exactly as section 2.3
states. What changed is not the correlation but the error measurement
underneath it: the truth is now independent of the miner's claimed
location and time, so the per-miner error figures in 3.2 can be trusted
in a way the daemon-feed corpus could not support. The correlation is
not new evidence.

### 3.6 A routing finding that bounds every Track 2 submission

The node's registry lists 4 miners as active for `WEATHER_CHECK` and 5
for `WEATHER_FORECAST`. Across 200 auto-routed asks, 3 of the 5 were
served zero requests:

| miner_id | name                  | asks of 200 |
| -------- | --------------------- | ----------: |
| 211      | OpenWeatherMap        |         123 |
| 212      | WeatherAPI            |          77 |
| 18       | Zeus (Bittensor SN18) |           0 |
| 0        | Lacre-Meteo           |           0 |
| 64173    | OathCast Weather      |           0 |

The router selects from a narrower set than the registry advertises.
This is not a scoring question, but it bounds what any Track 2
evaluation can claim: **a submission can only be shown to rank the
miners the router actually reaches.** A two-miner comparison is the most
this network currently permits through the auto-routed endpoint,
whatever the registry says. Diagnosing why would mean calling
`/engine/v1/ask/{miner_id}` directly, which targets a named miner and is
out of bounds under hackathon rule 04.

```
cargo run -p ask-harness -- batch --plan     # writes the city list, spends nothing
cargo run -p corpus-eval --release -- geocode
cargo run -p ask-harness -- batch --budget 300
cargo run -p corpus-eval --release -- headtohead
cargo run -p corpus-eval --release -- prepare corpus/head-to-head.jsonl corpus/h2h-input.jsonl
(cd tools/wazero-runner && go run . -corpus ../../corpus/h2h-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm -out ../../corpus/h2h-scores.jsonl)
cargo run -p corpus-eval --release -- stats corpus/h2h-scores.jsonl
cargo run -p corpus-eval --release -- rankflip corpus/h2h-scores.jsonl
```

The head-to-head corpus is emitted in the same shape as the daemon-feed
corpus, so `prepare` consumes it unchanged: 200 rows read, 200 written,
no drops.

---

## 4. Adversarial results

Both columns below are measured by running the compiled `.wasm` modules
under wazero — ours and the protocol's reference module — through the
same harness path that produced the corpus columns in section 2. Neither
number comes from a reimplementation.

That distinction matters because a reimplementation is not the thing it
reimplements. A reference column built from a native Rust copy of the
published `word_overlap` would be a claim about the copy, however
faithful the copy is; section 4.1 gives the check that keeps the copy
out of every published figure.

The honest comparison throughout is a miner 10% out, which scores 0.081.
Every row is also a test in `crates/eval-script/tests/adversarial.rs`.

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

Three rows state a repeat count, because the reference score depends on
it. The reference divides by the answer token count, so a padded
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

`crates/corpus-eval/src/baseline.rs` holds a native copy of the
reference `word_overlap`. It produces no number in this document.
`adversarial-report` recomputes every row with it and prints the
disagreements:

```
=== COMPILED MODULE VERSUS NATIVE COPY ===
all 28 rows agree to within 1e-6
the native word_overlap copy is faithful to the shipped module
```

All 28 rows agree, so the copy is faithful — but nothing published here
depends on that being true, because every figure in sections 1 and 4
comes from the compiled module.

Both tables are generated from `crates/corpus-eval/src/adversarial.rs`,
which is the single source for sections 1 and 4: a row that is not in
that file is not in this report.

The harness also scores each case three times, under three ground-truth
fields holding identical text, and the report fails if the three scores
differ. That is a free determinism check on the harness itself.

The `separation` and `crossbranch` subcommands print native tables as a
fast development view. They print a banner saying so, and their numbers
are not the published ones.

### 4.2 Strategies that still work

**The prose farm is the largest one, and the ABI is the reason.** The
node extracts a single answer text before `rank_answer` sees it, and for
a miner whose `signal_mapping.label_field` names a container it builds
that text with an LLM converter. Both weather miners now serving traffic
are in that group: miner 211's `label_field` is `weather`, an array, and
miner 212's is `current`, an object. So the module receives a SENTENCE,
carrying several numbers, and nothing in `(question, ground_truth,
miner_answer)` says which quantity was asked for.

The scorer keeps the best match over every candidate, so an answer that
states the WRONG value still scores well whenever some other quantity in
the same sentence sits near the right one. Against a ground truth of
28.1 C, with an honest miner 10% out scoring 0.0831:

| miner answer                                              | asserts |  score |
| --------------------------------------------------------- | ------: | -----: |
| `Wind 28.1 kph, temperature 34.9 C.`                      |    34.9 | 0.5000 |
| `It feels like 28.1 C, actual temperature 34.9 C.`        |    34.9 | 0.5000 |
| `The temperature is 34.9 C, up from 28.1 C this morning.` |    34.9 | 0.5000 |
| `Tokyo: 34.9C, feels like 28.1C, humidity 62%.`           |    34.9 | 0.3333 |

Every one of those is wrong and every one clears the honest bar by four
to six times. In the first row `kph` is not a known unit, so `28.1`
parses as dimensionless and is then read in the ground truth's unit —
the wrong-quantity number is promoted to a temperature.

The anti-spray divisor is the only brake and it is the wrong instrument.
It counts numbers, so it charges an honest verbose answer exactly as it
charges a farm. The most detailed CORRECT answer in the fixture set,
`Tokyo: 28.1C, feels like 30.2C, wind 11.2 kph, humidity 62%.`, scores
**0.2500 — below every wrong answer above**. Verbose honesty is punished
harder than a wrong answer that mentions the right number in passing.

**The converter output is cached, so this is repeatable rather than
random.** Every validator scoring a given response reads the same
string, and the same string again on a re-score. A farm that works once
works every time and for every validator at once, which makes the prose
farm materially more exploitable than a per-validator sampling would be.
It also means a miner can test a phrasing offline and know exactly what
it will earn.

```
cargo run -p eval-script --example prose_probe -- --emit
(cd tools/wazero-runner && go run . -corpus ../../corpus/prose-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/prose-scores.jsonl)
cargo run -p eval-script --example prose_probe -- --show
```

#### A rule that was tested and rejected

One intent-agnostic fix was implemented and measured: when the answer
holds several candidates, prefer the one whose surrounding words overlap
the GROUND TRUTH's words. It uses no domain vocabulary — no weather
terms, no unit list — only the truth's own tokens, so it would carry to
a gas price or a stock price unchanged.

It was dropped, because it failed on both sides at once. Scores below
are the compiled module under wazero, ground truth in its prose
rendering, with a three-word window each side:

| answer                                                         | correct? | rule off |    rule on |
| -------------------------------------------------------------- | -------- | -------: | ---------: |
| `Tokyo: 28.1C, feels like 30.2C, wind 11.2 kph, humidity 62%.` | yes      |   0.2500 | **0.0347** |
| `The temperature in Tokyo is 28.1 C. Yesterday it was 31.4 C.` | yes      |   0.5000 | **0.0306** |
| `Wind 28.1 kph, temperature 34.9 C.`                           | no       |   0.5000 |     0.5000 |
| `temperature 34.9 C, wind 28.1 kph`                            | no       |   0.5000 |     0.5000 |

Two honest answers fell **below** the 0.0831 honest bar, and the two
attacks it was built to stop did not move. The causes are structural
rather than tuning:

- The unit letter `C` is vocabulary the ground truth also contains, and
  it sits beside every temperature candidate. In the first row the wrong
  candidate `30.2` wins 2 hits to 1 purely by sitting between two `C`s.
- Adjacent clauses put both candidates inside each other's windows. In
  `Wind 28.1 kph, temperature 34.9 C.` both numbers score 2 hits, the
  tie falls back to best-match, and the farm survives. Narrowing the
  window to separate them loses the signal instead: in the second row
  `temperature` is already outside the correct candidate's window at
  three words, which is why that row breaks.

No window size satisfies both, so the approach is not rescued by
tuning. The rule is not in the shipped module.

One common token against a short ground truth scores 0.5. `is` against
`is malicious` is one shared token out of two. Dividing by the union
kills the reference's 1.0000 but cannot push a one-of-two overlap below
one half. It decays as the ground truth grows: 0.167 against a six-token
truth. A fix needs rarity weighting, which needs a corpus the module
cannot carry. Test:
`known_weakness_one_common_token_against_a_short_ground_truth`.

Repeated-word padding scores 0.5. Token sets deduplicate, so padding
with one repeated word adds exactly one distinct token. Padding with
distinct words falls below 0.05.

A prose ground truth carrying a date is still farmable. The
quoted-string rule in section 4.3 is scoped to JSON-shaped text, so
`2026` against `The temperature at 2026-08-10T12:00 was 28.9 C.` still
scores 1.0. No prose rendering in the corpus carries a date, so the rule
was not widened to skip date-shaped runs anywhere: that needs a date
grammar, which would risk eating a real negative such as `-5 C`, and a
grammar tuned until the numbers improve is not defensible.

That same rule made one case worse. A JSON ground truth with no quantity
in value position, such as `{"time":"2026-08-10T12:00"}`, now yields no
truth value at all and falls to the text branch, so the scaffolding
answer `time` scores 0.333 where it scored 0.000 before. Against that,
the unrelated answer `12 gwei` falls from 1.000 to 0.000 on the same
truth. Such a truth carries no quantity, so the text branch is the
module's designed treatment, but the scaffolding row is a real
regression and is recorded as one.

Three rows in the table above read as profitable but are not attacks:
precision spam, hedge word, and double negation all score 1.0 because
they are the correct answer in an unusual form. `not not malicious`
means `malicious`.

### 4.3 Defects the corpus found in this scorer

The adversarial suite tested only inputs written by hand. Running the
scorer against real corpus renderings found three live defects:

1. **Echoing a junk question earned 0.1357.** The question `[direct] 207
-> /price` contains `207`, so an echo of it parsed as a _number_ and
   went to the numeric branch, where 207 against 192.43 scores 0.1357.
   The copied-question defence ran only in the text branch and never saw
   it. The check now runs before dispatch and covers every branch.

   **That fix was necessary and it was not sufficient.** The check fired
   on a Jaccard overlap above 0.99, and Jaccard falls when the answer
   grows, so an attacker escaped it by growing the answer: the echo with
   ONE word appended scored 0.135687 again — the whole of the defect,
   back for one token. A threshold on a similarity the attacker can
   dilute cannot hold.

   The rule now has two halves and an answer must meet both: it gives
   back EVERY token of the question, and everything it adds beyond the
   question is FOREIGN to the ground truth. The first half reads
   RECALL of the question's tokens, which divides by the question's
   size and therefore does not move when the answer is padded — the
   same reason the substitution charge reads recall. The second half is
   what keeps an honest answer safe: an answer that repeats the
   question and then answers it carries the payload, and the payload is
   a token the truth holds. Padding cannot escape either half, because
   every filler word is foreign to the truth by construction.

   Measured: the verbatim echo, the echo with one through ten words
   appended, prefixed, suffixed and interleaved all score 0.0000, while
   `the temperature in tokyo is 34.7 C` against the question
   `temperature in tokyo` scores 1.0. Tests:
   `padding_an_echo_of_the_question_never_escapes_the_check`,
   `an_honest_answer_that_repeats_the_question_is_not_an_echo`,
   `a_question_that_is_its_own_answer_still_scores`.

2. **A prose ground truth could be farmed for 0.667, and a JSON ground
   truth scored a correct answer 0.000.** Both came from one root cause:
   the scorer asked "does the whole string parse as one value?" instead
   of "does the ground truth contain a quantity?". A prose truth fell to
   token overlap, so returning the scaffolding words without the number
   paid 0.667 while an honest miner 10% out earned 0.081 — the farm paid
   eight times better than real work. A JSON truth has no whitespace, so
   number extraction found nothing and a correct answer scored zero.

   The fix for both is the dispatch rule in `score_answer`: when the
   ground truth carries a quantity and the answer carries none, the
   score is 0.0, because the miner did not supply what was asked for.

3. **Every number inside a JSON ground truth was a free match target.**
   The lenient scan of
   `{"temperature_2m":28.9,"time":"2026-08-10T12:00"}` produced seven
   candidates — `2` from the key name `temperature_2m`, the real `28.9`,
   then `2026`, `-8`, `-10`, `12` and `0` from the timestamp — and the
   scorer keeps the BEST match over every pair. So the answers `2026`,
   `12`, `2` and `0` each scored 1.000000 against that rendering while
   scoring under 0.003 against the bare and prose renderings of the same
   truth. It also broke a registration structural check: the unrelated
   answer `12 gwei` matched the `12` of `T12:00` and scored 1.0000,
   tying the correct answer `28.9`, so the self-match did not strictly
   beat the cross-match. On the 6,169-row corpus the same defect paid 10
   rows up to 0.748 for an answer the bare rendering scored 0.025.

   The fix is a syntax rule, in `scan_truth_values`: **in a ground truth
   shaped like a JSON document, a number that sits inside a quoted
   string is text — a key name or an ISO timestamp — and is not a
   candidate match target, unless the quoted string IS a value.** A
   quoted string is a value when the whole string parses under
   `parse_value`, the module's existing strict reader. That admits
   `"28.9"`, `"28.9 C"`, `"28.9C"`, `" -5.2 "`, `"$192.43"`,
   `"12 gwei"` and `"2026"`; `parse_value` already handles the
   surrounding whitespace and the unit suffix, so the rule needs no
   trimming and no unit list of its own.

   **An earlier version of this clause tested a weaker property and it
   was a farm.** It also admitted a string that CONTAINS one numeric run
   standing on its own, so that a value wrapped in a sentence
   (`"28.9 C in Paris"`) would keep its number. "Is a value" and
   "contains a number" are different properties, and the second one
   admits every string carrying an incidental number. Against a truth
   whose real value is `28.1`, with an honest miner 10% out earning
   0.0831:

   | ground truth                                  | answer | before |      now |
   | --------------------------------------------- | ------ | -----: | -------: |
   | `{"status":"HTTP 200","temperature_2m":28.1}` | `200`  | 1.0000 | 0.000024 |
   | `{"summary":"3 alerts active",…}`             | `3`    | 1.0000 | 0.001127 |
   | `{"note":"revision 4",…}`                     | `4`    | 1.0000 | 0.001222 |
   | `{"station":"KJFK 12",…}`                     | `12`   | 1.0000 | 0.002734 |
   | `{"city":"Paris 2026",…}`                     | `2026` | 1.0000 | 0.000000 |
   | `{"window":"6 hours",…}`                      | `6`    | 1.0000 | 0.001453 |

   Six of seven realistic shapes paid a WRONG answer a perfect score.
   The correct answer `28.1` still scores 1.0000 on every one of them.

   **There is no middle ground, and the strict rule has a price.**
   `"28.9 C in Paris"` and `"200 OK"` have the same shape — a value,
   then words — so a rule that admits the first admits the second, and
   `"3 alerts active"`, `"6 hours"` and `"2 of 3"` with it. The
   difference between them is meaning, not syntax. So a JSON truth whose
   ONLY quantity sits inside a sentence in a quoted string now carries
   no quantity and falls to the text branch, where the correct answer
   scores 0.3333 rather than 1.0000. That is recorded as a cost, and
   `a_number_inside_a_quoted_phrase_is_not_a_match_target` asserts both
   halves of it.

   The two shapes the original rule was written to reject are the two
   the corpus produces, `"temperature_2m"`, whose `2` is glued between
   `_` and `m`, and `"2026-08-10T12:00"`, which holds five numbers.
   Both are still rejected.

   The rule never looks at a number's magnitude, so it cannot be aimed
   at a chosen value. It applies to the GROUND TRUTH only: the answer
   keeps the lenient scan, because the answer's number count is the
   anti-spray divisor and dropping quoted numbers from that count would
   make a spray of quoted numbers cheaper than a spray of bare ones.

   Measured effect: the `2026` farm falls from 1.000000 to 0.000000188,
   the `12 gwei` cross-match from 1.000000 to 0.002625, the correct
   answer `28.9` stays 1.000000, all six cross-branch rows stay
   0.000000, and score stability across the three renderings rises from
   99.8% to 100% of 6,169 rows (section 2.2). Tests:
   `a_json_truth_cannot_be_farmed_with_a_date_or_key_part`,
   `a_json_truth_self_match_beats_an_unrelated_cross_match`,
   `a_quoted_value_in_a_json_truth_still_scores`.

---

## 5. The promotion gates

The node records four numbers when it compares a candidate script
against a champion: `worst_self_match`, `score_stddev`,
`candidate_margin` and `candidate_wins`. The weather corpus cannot
measure any of them. It holds one intent family and every ground truth
is a quantity, so it says nothing about a URL verdict or a CVE severity.

The benchmark for this section is 40 (question, good answer, bad answer)
triples over intent families outside the corpus: URL scans, SSL grades,
CVE severities, sentiment labels, translations in five scripts, fact
checks, chat completions, and a short numeric tail. Every ground truth
appears in one of the three renderings a validator may send: bare, prose
from the response converter, and JSON. Every good answer is a plausible
miner output rather than a copy of the truth, because a benchmark of
byte-identical good answers measures the exact-match short circuit and
nothing else.

```
cargo run --release -p corpus-eval --example promotion_gates -- --report
```

Every score comes from the compiled module under wazero. The harness
also scores each row through the native library and refuses to print a
number the two do not agree on.

### 5.1 The results, weather band

| gate               | result                          |
| ------------------ | ------------------------------- |
| `worst_self_match` | 1.0000 on all 40, floor is 0.75 |
| `score_stddev`     | 0.4201 over 80 candidate scores |
| `candidate_margin` | 0.56                            |
| `candidate_wins`   | 34/40                           |

The champion in that run is the compiled reference module. It is a
teaching example rather than the production scorer, so those columns
are a baseline and not a target; the harness takes the champion as an
argument for that reason:

```
cargo run --release -p corpus-eval --example promotion_gates -- \
  --report --module dist/eval_script_weather.wasm --champion path/to/other.wasm
```

### 5.2 Self-match did not survive a doubled space, and now does

**This section reports a defect that is FIXED. Both columns below are
historical; the current value of every cell in the right-hand column is
1.0000.**

`worst_self_match` reached 1.0000 through the exact-match short circuit,
which needs BYTE equality. The same gate with one doubled space in the
answer — the same words, the same numbers, the same meaning — scored
0.5000 on two of the 40:

| question | truth                                               | self-match | doubled space, BEFORE | doubled space, NOW |
| -------- | --------------------------------------------------- | ---------: | --------------------: | -----------------: |
| q09      | `CVE-2021-44228 has a severity rating of CRITICAL.` |     1.0000 |                0.5000 |             1.0000 |
| q36      | `INVOICE 2024-001`                                  |     1.0000 |                0.5000 |             1.0000 |

Both truths carry two numbers, and the anti-spray divisor counted both
of them against an answer that simply repeated the truth. Nothing in
this repository controls how the node builds the answer of a self-match
check. If anything normalises or re-renders the truth first, that is a
hard rejection at the 0.75 floor.

The divisor now counts only the answer numbers the truth does NOT hold,
and only for an answer that gives back EVERY quantity the truth holds.
The second half of that rule is not decoration. Without it the wrong
answer `INVOICE 2024-002` kept the `2024` for free and scored 1.0000,
level with the right one. With it, that answer misses the `001` target,
pays for both of its numbers, and scores 0.5000.

Across all 174 vectors of the benchmark, exactly two moved:
`q09-selfws` and `q36-selfws`, both 0.5000 to 1.0000. No corpus row can
move, because no corpus miner answer holds more than one number, so the
divisor was already 1 on every one of the 6,169 rows.

The spray it charges for is unchanged, in kind and in degree: five
numbers with one right pays 0.200, which is what it paid before the
quoting rule existed. The exemption needs the answer to hold every
number the truth holds AND no number it does not, and a spray fails the
second half by construction, so it never reaches the exemption at all.
An answer with no number never reaches the divisor either, so the farm
that returns the words around a value keeps its 0.0.

### 5.3 A truth that carries a number it was not asked for

Five of the 40 rows score 0.0000 for BOTH candidates, so they give the
node nothing to compare:

| question | ground truth                                                |   good |    bad |
| -------- | ----------------------------------------------------------- | -----: | -----: |
| q02      | `{"verdict":"phishing","confidence":0.97}`                  | 0.0000 | 0.0000 |
| q09      | `CVE-2021-44228 has a severity rating of CRITICAL.`         | 0.0000 | 0.0000 |
| q10      | `{"cve":"CVE-2021-44228","severity":"critical","cvss":9.8}` | 0.0000 | 0.0000 |
| q14      | `{"label":"negative","score":0.88}`                         | 0.0000 | 0.0000 |
| q23      | `{"verdict":"false","sources":3}`                           | 0.0000 | 0.0000 |

A sixth row, q07 `{"grade":"A","protocol":"TLS 1.3"}`, was in this table
until the quoted-span rule was tightened to admit only a string that IS
a value. `"TLS 1.3"` names a protocol version, so under the looser rule
its `1.3` was a quantity, the truth carried a number, and rule 6 zeroed
the correct answer `A`. It now scores 0.1667 against 0.0000 and is a
win in every band. See section 4.3.

Dispatch rule 6 is the cause: the truth holds a quantity, the answer
holds none, so the answer scores 0.0. In these rows the quantity is a
confidence, a CVSS score or a protocol version, and the wanted answer is
a word.

One row is worse than a tie. On q22 the truth
`Partly true. The programme reduced transmission by 40%.` paid 0.0000
for the correct `partly true` and 0.0036 for the wrong `60%`. A wrong
number outranked a right label.

The `label` band changes that one rule and nothing else. Measured on the
same benchmark, six vectors move and all six are `good` answers rising
off the floor:

| question | weather band | label band |
| -------- | -----------: | ---------: |
| q02 good |       0.0000 |     0.2000 |
| q09 good |       0.0000 |     0.1429 |
| q10 good |       0.0000 |     0.1429 |
| q14 good |       0.0000 |     0.2000 |
| q22 good |       0.0000 |     0.2500 |
| q23 good |       0.0000 |     0.2500 |

No `bad` answer moves, so nothing on this benchmark is paid that was not
paid before. The same holds over the full 19,482-vector set: the two
bands differ on 21 vectors, every one of them a case where the truth
carries a number and the answer carries none — rule 6's exact
precondition — and every one raised from exactly 0.0000.

| band    | `worst_self_match` | `score_stddev` | `candidate_margin` | `candidate_wins` |
| ------- | -----------------: | -------------: | -----------------: | ---------------: |
| weather |             1.0000 |         0.4201 |               0.56 |            34/40 |
| price   |             1.0000 |         0.4218 |               0.57 |            34/40 |
| onchain |             1.0000 |         0.4215 |               0.53 |            34/40 |
| label   |             1.0000 |         0.4117 |               0.59 |            40/40 |

The `label` figures are NOT MEASURED in the sense the weather tolerance
is. They come from this 40-row benchmark, which this repository wrote.
No live traffic and no corpus stands behind them.

The band has a price. Rule 6 is what stops an answer that gives back the
scaffolding of a truth and none of its value, and this band relaxes it.
On a weather truth the band pays 0.667 for `the temperature was C`,
against 0.0831 for an honest miner 10 percent out. It must never be
registered for an intent whose answer is a quantity, and
`crates/eval-script/tests/label_band.rs` asserts both the gain and the
price.

### 5.4 Stage 1, and what a long answer used to cost

All four Stage 1 gates pass on all four bands: the module loads and
answers all 174 vectors, a blank and a whitespace-only answer both score
exactly 0.0000, a correct answer beats an unrelated one on 40 of 40
questions, and all 14 long, emoji and non-ASCII cases return without a
trap.

The last gate asks only that a long answer does not crash. Ours did not
crash, and it was still a defect: the check for a digit in front of an
exponent marker walked the whole text in front of every `e`, so the cost
of one call grew with the square of the answer length, which the miner
chooses up to the 1 MiB cap.

Measured under wazero, one `rank_answer` call, fastest of three:

|       answer size |           before |     after | speedup |
| ----------------: | ---------------: | --------: | ------: |
|             8 KiB |        61,987 us |    462 us |    134x |
|            16 KiB |       246,555 us |    880 us |    280x |
|            32 KiB |       987,097 us |  1,729 us |    571x |
|            64 KiB |     4,012,775 us |  3,264 us |   1229x |
| 1 MiB (projected) | 1,043,221,173 us | 44,274 us |  23563x |

Cost per doubling of the input: 4.0x before, 1.9x after. The projected
row extends each column along its own measured curve; at the cap that is
about 17 minutes before the change and about 44 ms after it.

The root cause was confirmed rather than assumed: the same 64 KiB text
with a single digit in front of it, which makes the prefix search stop
at byte 0, cost 2,047 us instead of 1,055,783 us natively. The fix keeps
one accumulator instead of rescanning, so the predicate is the same at
every index and all 174 vectors are bit-identical across it.

### 5.5 What the Unicode gates cost

Three limits are real and none of them is a bug to be fixed without a
consensus decision:

- `МОСКВА` against `москва` scores 0.0000 while `CRITICAL` against
  `critical` scores 1.0000. Case folding is ASCII only, because a
  Unicode fold table changes with the Unicode version and two validators
  on different tables is a slashing event. The cost is that a correct
  answer differing only in case scores zero in every non-Latin script.
- CJK has no spaces, so `你好世界` is one token. Partial credit is
  impossible: exact or zero, nothing between. The same holds for
  Japanese and Thai.
- A token is capped at 32 BYTES, which is 32 ASCII characters but about
  10 Devanagari ones. Two different Hindi words sharing a 10-character
  prefix score 1.0000. That is a false positive rather than a farm, since
  it needs the truth's own prefix, but a wrong answer can be paid in
  full.

---

## 6. Determinism

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

## 7. Reproduction

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

# the head-to-head set, section 3. This SPENDS 2.00 testnet USDC.
cargo run -p ask-harness -- batch --plan          # city list, spends nothing
cargo run -p corpus-eval --release -- geocode     # truth coordinates, free
cargo run -p ask-harness -- batch --budget 300
cargo run -p corpus-eval --release -- headtohead
cargo run -p corpus-eval --release -- prepare \
   corpus/head-to-head.jsonl corpus/h2h-input.jsonl
(cd tools/wazero-runner && go run . -corpus ../../corpus/h2h-input.jsonl \
   -a ../../target/wasm32-unknown-unknown/release/eval_script.wasm \
   -b ../../reference/scoring_module.wasm \
   -out ../../corpus/h2h-scores.jsonl)
cargo run -p corpus-eval --release -- stats corpus/h2h-scores.jsonl
cargo run -p corpus-eval --release -- rankflip corpus/h2h-scores.jsonl

# section 5, the promotion gates. Scores every row through the compiled
# module under wazero and compares against a champion .wasm.
cargo run --release -p corpus-eval --example promotion_gates -- --report
cargo run --release -p corpus-eval --example promotion_gates -- \
  --report --module dist/eval_script_label.wasm --champion reference/scoring_module.wasm

# section 5.4, the cost ladder. --measure runs once per version of the
# code, so the before column needs the pre-fix source checked out.
cargo run --release -p corpus-eval --example promotion_gates -- --measure
cargo run --release -p corpus-eval --example promotion_gates -- --measure --after
cargo run --release -p corpus-eval --example promotion_gates -- --table

# every band, with its Stage 1 gates, its Stage 2 numbers and its hash
tools/build-variants.sh

# verification
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo test -p eval-script --no-default-features --features label
cargo run -p host-runner --release
```

The corpus is rebuilt with `cargo run -p corpus-builder`. It caches
every HTTP response, so a rerun makes zero network requests.

---

## 8. What this evaluation does not show

- **It does not settle a miner ranking.** Section 3 ranks two miners on
  10 paired clusters with a 19.9% bootstrap flip rate. That is
  suggestive, not settled, and it covers only the 2 miners the router
  actually reached out of 5 registered active.
- **It does not show that wrong-location or wrong-time answers are
  caught.** Zero of five known-bad groups were caught, and one scored
  above the corpus mean. See section 2.4 for why no value-comparing
  module can catch them under this ABI.
- **The correlation of 0.90 is substantially circular** — the score is a
  monotonic function of the same difference the correlation measures —
  and it is measured against archive-derived truth on one intent family,
  weather temperature, on 6,169 rows from three miners. It is not
  evidence about price or gas intents, and it is not an independent
  accuracy result. See section 2.3.
- **`TOLERANCE = 0.03` was chosen to produce 0.900 at 1% error.** No
  protocol document justifies that value. A different intent needs a
  different one, and the constant exists to make that a one-line change.
- **"Accuracy" in section 3 means agreement with Open-Meteo**, a
  reanalysis model, not station observations. The methodology gap may
  systematically favour whichever miner is closer to that model.
- **It does not show that a prose answer is scored on the right
  quantity.** The node hands the module one converted sentence, and the
  three-string ABI never names the quantity that was asked for. Four
  wrong answers in section 4.2 score 0.33 to 0.50 against an honest bar
  of 0.0831, and the most detailed correct answer scores 0.2500. The
  converter output is cached, so the effect is repeatable for every
  validator rather than sampled.
- **The corpus is one snapshot** of the daemon feed taken in August 2026,
  and the head-to-head set is one 28-minute window on 15 August 2026.
  Neither is a continuing sample.
