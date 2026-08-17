# dist artefact hashes

Written by `tools/build-variants.sh`. Do not edit by hand.

The `.wasm` files themselves are gitignored and are rebuilt by that
script. This file is the record of what it built: check the sha256 of
the artefact you are about to register against its row.

| band | imports | exports | size | sha256 |
| --- | ---: | --- | ---: | --- |
| `weather` | 0 | ok | 64620 | `85608d410d75c44fbd6679f70ff00bb3272441d8cd772f92d7b6ae11477ab894` |
| `price` | 0 | ok | 64620 | `72736dad244fa04d6d79e0e76e399d472bef38d68af36477acdc4893d1562903` |
| `onchain` | 0 | ok | 64620 | `04903822729a4ed5ac2362df73043324c02ed62c5c45d2a4688a4dd068c2a70f` |
| `label` | 0 | ok | 64685 | `bc1dbc4c0607bef2c939e181d699f0729aa5ec57839cfedc3f1238a9456fab52` |

Every row above passed: exactly three scored exports, zero imports,
golden-vector agreement, wasmtime/wazero bit-equality, its band's full
test suite, the four Stage 1 gates and the four Stage 2 numbers. A band
that fails any of those is deleted from `dist/` before this file is
written, so a MISSING row means that band must not be uploaded.
