# dist artefact hashes

Written by `tools/build-variants.sh`. Do not edit by hand.

The `.wasm` files themselves are gitignored and are rebuilt by that
script. This file is the record of what it built: check the sha256 of
the artefact you are about to register against its row.

| band | imports | exports | size | sha256 |
| --- | ---: | --- | ---: | --- |
| `weather` | 0 | ok | 64348 | `3e171b9ad59e3cc6141f28cfe3b0b0b3d93928030d1f7c655c750bd7a7ff3241` |
| `price` | 0 | ok | 64348 | `ef9f1cdd00be33fa41851597c7ed32d890a41efb203b78b3069b6466762c1f40` |
| `onchain` | 0 | ok | 64348 | `81fedd611ca983ca35be5e5aa7d65d49e754cf642a04378dfe31e4fdb5b5d17b` |
| `label` | 0 | ok | 64413 | `6544aac39a5fd266f991cb8e2daacb552278795d06a1f709d929da6b441a6796` |
| `metadata` | 0 | ok | 64444 | `617b17f1174bcdf1019cba52db8efd6f6c3fcb3a4d3d4d60c7dc074360983e6c` |

Every row above passed: exactly three scored exports, zero imports,
golden-vector agreement, wasmtime/wazero bit-equality, its band's full
test suite, the four Stage 1 gates and the four Stage 2 numbers. A band
that fails any of those is deleted from `dist/` before this file is
written, so a MISSING row means that band must not be uploaded.
