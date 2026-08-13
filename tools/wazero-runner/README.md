# wazero-runner

A Go host driver for the Telegraph Track 2 scoring module ABI, built on
[wazero](https://github.com/tetratelabs/wazero) v1.12.0. This tool matches
the production Telegraph node host: pure Go, no CGo, no wasmtime.

Go version used: **go1.25.0** (`/usr/local/go`, `go version go1.25.0
linux/amd64`). wazero v1.12.0 requires Go 1.25 or newer, so `go 1.23` in
`go.mod` does not build; this module pins `go 1.25.0`.

## The ABI

```
alloc(size: i32) -> i32
dealloc(ptr: i32, size: i32)
rank_answer(q_ptr, q_len, gt_ptr, gt_len, ma_ptr, ma_len) -> f32
```

A zero length string is passed as `ptr=0, len=0`. `alloc` is not called for
it.

## Build

```sh
cd tools/wazero-runner
go build -o wazero-runner .
```

## Mode 1: compare (the default)

Loads two wasm modules, scores the same input triple through both, and
prints the two scores side by side with the raw f32 bit pattern of each
score in hex. If one module traps or errors, its row shows the failure
text and the other module's score still prints.

```sh
go run . \
  -a /path/to/module_a.wasm \
  -b /path/to/module_b.wasm \
  -q "What is the capital of France?" \
  -gt "Paris is the capital of France." \
  -ma "The capital of France is Paris"
```

## Mode 2: matrix

Runs a built in set of 8 edge case inputs through two wasm modules and
prints a differential table: case, reference score, our score, delta or
notes. `-a` is our module, `-b` is the reference module.

```sh
go run . -matrix \
  -a /path/to/eval_script.wasm \
  -b /path/to/scoring_module.wasm
```

Row 1 is the doc example (question "What is the capital of France?",
ground truth "Paris is the capital of France.", miner answer "The capital
of France is Paris"); the reference module must score it 0.8333. Rows 7
(invalid UTF-8 miner answer) and 8 (2 MiB miner answer) print extra detail
below the table: the alloc pointer, the module memory size in pages before
and after alloc, whether the host memory write succeeds, and the exact
error text of any failure, for each of the question, ground truth, and
miner answer fields, for both modules.

Every row runs with a fresh wazero runtime and a fresh module instance per
module, so one row's trap or panic cannot affect the next row.

## Mode 3: golden

Scores every vector in a golden vector JSON file through one wasm module
and writes a result JSON file with the f32 bit pattern of each score. A
Rust host runner reads this file and checks bit equality against its own
wasmtime results.

```sh
go run . -golden ../../golden_vectors.json \
  -a /path/to/eval_script.wasm \
  -out /path/to/wazero_golden_result.json
```

For each vector, `question` is the empty string (`ptr=0, len=0`),
`ground_truth` is the vector's `ground_truth` field, and `miner_answer` is
the vector's `response` field.

Output shape:

```json
{
  "runner": "wazero",
  "wasm_path": "<the -a path as given>",
  "wasm_sha256": "<lowercase hex sha256 of the wasm file bytes>",
  "vectors": [{ "name": "...", "bits_hex": "0x3f800000", "value": 1.0 }]
}
```

`bits_hex` is the IEEE-754 f32 bit pattern from `math.Float32bits`,
lowercase, `0x` prefixed, always 8 hex digits.

Golden mode stops on the first error, because a bit equality report with a
gap in it is not evidence. The error message names the failing vector.

## Code quality

```sh
gofmt -l .      # prints nothing
go vet ./...    # clean
```
