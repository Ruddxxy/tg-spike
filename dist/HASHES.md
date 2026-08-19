# dist artefact hashes

Written by `tools/build-variants.sh`. Do not edit by hand.

The `.wasm` files themselves are gitignored and are rebuilt by that
script. This file is the record of what it built: check the sha256 of
the artefact you are about to register against its row.

| band | imports | exports | size | sha256 |
| --- | ---: | --- | ---: | --- |
| `weather` | 0 | ok | 64418 | `1bbd8532952b49fce7cc638cea0c21dc730c7285fca68771125867ebbbfacd47` |
| `price` | 0 | ok | 64418 | `421b59831cd0f2fe8133f0d9da02b5eb98d71fb91fbf281fc2e9eb7940fe0109` |
| `onchain` | 0 | ok | 64418 | `2cee712117573657aea3fb037b511b27ff9ab07643013cd2f827d49e0a4ba737` |
| `label` | 0 | ok | 64483 | `8bf7bdf46ec49c781f70c75a53b52d0a7f83afc62f9a5e2889fbd0045c3d27a0` |

Every row above passed: exactly three scored exports, zero imports,
golden-vector agreement, wasmtime/wazero bit-equality, its band's full
test suite, the four Stage 1 gates and the four Stage 2 numbers. A band
that fails any of those is deleted from `dist/` before this file is
written, so a MISSING row means that band must not be uploaded.
