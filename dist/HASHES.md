# dist artefact hashes

Written by `tools/build-variants.sh`. Do not edit by hand.

The `.wasm` files themselves are gitignored and are rebuilt by that
script. This file is the record of what it built: check the sha256 of
the artefact you are about to register against its row.

| band | imports | exports | size | sha256 |
| --- | ---: | --- | ---: | --- |
| `weather` | 0 | ok | 64810 | `b101aa8e5870a329bbdc9a6602b0df29e274431ae86414e1b2a6671a49fcb3f2` |
| `price` | 0 | ok | 64810 | `58c7f38f44419a6ba0d1d5a97b1d402a04d04f8fe223dd6401798dd696bfa4f4` |
| `onchain` | 0 | ok | 64810 | `a8df404386ee07db87407c72d14c641c7f1baded28583011fb109197cd1769c9` |
| `label` | 0 | ok | 64875 | `08edbdb6159475185ce4449c80ecf641557f531302f098daf70d390991204679` |

Every row above passed: exactly three scored exports, zero imports,
golden-vector agreement, wasmtime/wazero bit-equality, its band's full
test suite, the four Stage 1 gates and the four Stage 2 numbers. A band
that fails any of those is deleted from `dist/` before this file is
written, so a MISSING row means that band must not be uploaded.
