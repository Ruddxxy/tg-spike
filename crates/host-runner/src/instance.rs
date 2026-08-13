//! This module has the wasmtime glue. It loads the eval-script WASM module
//! and drives the memory ABI.
//!
//! # WASI note
//!
//! This module always builds a WASI preview1 (p1) linker, even when the
//! loaded module is a plain `wasm32-unknown-unknown` build that imports
//! nothing from WASI. This is a simpler design than a true two-path split.
//!
//! A true two-path split needs two different `Store<T>` generic types: one
//! for a plain module (`Store<()>`) and one for a WASI module
//! (`Store<HostState>`). A `ScriptInstance` would then need an enum over
//! two sets of `TypedFunc` handles, or boxed calls, for every ABI method.
//! That adds real complexity for no behavior change: a module with zero
//! WASI imports links and runs the same way against a WASI-capable linker
//! as it does against a plain linker, because `Linker::instantiate` only
//! resolves the imports the module actually asks for. Unused linker
//! entries are inert.
//!
//! This code still reports, per module, whether it asked for WASI imports
//! at all (see [`ScriptInstance::instantiation_path`]), so the report is
//! honest about what each `.wasm` file needed.

use std::path::Path;

use anyhow::{bail, Context, Result};
use wasmtime::{Engine, ExternType, Instance, Linker, Memory, Module, Store, TypedFunc};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

/// This turns a `wasmtime::Result` into an `anyhow::Result` with an added
/// message.
///
/// The `wasmtime::Error` type in this version does not implement
/// `std::error::Error`. Because of that, `anyhow::Context` does not apply
/// to `Result<T, wasmtime::Error>` directly. This helper does the same job
/// by hand: it turns the wasmtime error into text and wraps it in a new
/// `anyhow::Error`.
trait WasmtimeResultExt<T> {
    fn wasm_context(self, msg: impl std::fmt::Display) -> Result<T>;
}

impl<T> WasmtimeResultExt<T> for wasmtime::Result<T> {
    fn wasm_context(self, msg: impl std::fmt::Display) -> Result<T> {
        self.map_err(|e| anyhow::anyhow!("{msg}: {e}"))
    }
}

/// A fixed wall clock. It always returns the UNIX epoch.
///
/// A validator must not give the script a real wall clock. A real clock is
/// a source of non-determinism: two validators that run the same script at
/// different times could get different results. This clock removes that
/// risk. It always answers the same way, no matter when the host runs it.
struct FixedWallClock;

impl wasmtime_wasi::HostWallClock for FixedWallClock {
    fn resolution(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }

    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_secs(0)
    }
}

/// A fixed monotonic clock. It always returns zero.
///
/// Like the wall clock, a real monotonic clock is a source of
/// non-determinism across runs and across machines. This clock removes
/// that risk by always answering zero.
struct FixedMonotonicClock;

impl wasmtime_wasi::HostMonotonicClock for FixedMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }

    fn now(&self) -> u64 {
        0
    }
}

/// This is the state that the `Store` holds for one script instance.
///
/// It holds the WASI preview1 context. The context is empty: no stdio, no
/// env vars, no preopened directories, and fixed clocks. This is a
/// determinism requirement, not a convenience. A validator must give the
/// script zero ambient authority. A script with access to real time, real
/// files, or real environment data could return a different score on
/// different validators, or leak host data.
///
/// One gap: this build does not override the random number generator. A
/// full override needs the `rand_core` crate to name the `Rng` trait that
/// `WasiCtxBuilder::secure_random`/`insecure_random` require. That crate is
/// outside the fixed dependency list for this crate. In practice this gap
/// is safe here: the eval-script crate does pure JSON parsing and math, and
/// its `Cargo.toml` does not turn on `serde_json`'s `preserve_order`
/// feature, so its JSON map type is a `BTreeMap`, not a hash map that needs
/// a random seed. The script has no reason to call the WASI random import.
struct HostState {
    wasi: WasiP1Ctx,
}

/// This says which linker path a module needed at load time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstantiationPath {
    /// The module had no WASI preview1 imports.
    Plain,
    /// The module had at least one WASI preview1 import.
    Wasi,
}

impl std::fmt::Display for InstantiationPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstantiationPath::Plain => write!(f, "plain (no WASI imports found)"),
            InstantiationPath::Wasi => write!(f, "WASI preview1 (module asks for WASI imports)"),
        }
    }
}

/// This says what happened when the host asked the module for a block
/// of linear memory.
///
/// See [`ScriptInstance::write_bytes`] for the call that returns this
/// type. The point of this type is to make a rejected allocation
/// impossible to mistake for a real address. A magic-number check for
/// 0 does not give that safety; this type does, because the caller
/// must match both arms before it can use a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocOutcome {
    /// The module gave a block. The pointer is never 0. The length is
    /// the length the caller asked for.
    Granted(u32, u32),
    /// The module said no. Its `alloc` export returned 0. This is the
    /// module's own length cap, checked before any memory read and
    /// before any memory grow. This is not a wasmtime error.
    Rejected,
}

/// This is the result of one full call to `rank_answer`, through the
/// whole write/call/free cycle.
///
/// A validator report needs more than the score value. It also needs
/// to know whether the module's own `alloc` cap rejected the question,
/// the ground truth, or the miner answer before `rank_answer` ran at
/// all. See [`ScriptInstance::rank_answer_outcome`].
#[derive(Debug, Clone, Copy)]
pub struct RankAnswerOutcome {
    /// The score. This is `0.0` when `alloc_rejected` is `true`.
    pub value: f32,
    /// `true` when the module's `alloc` cap rejected the question, the
    /// ground truth, or the miner answer before the host called
    /// `rank_answer`. `false` means `rank_answer` itself produced
    /// `value`. A `false` result can still carry a `value` of `0.0`,
    /// for the module's own reasons, such as invalid JSON. Read
    /// `value` and `alloc_rejected` together to tell which defence
    /// fired.
    pub alloc_rejected: bool,
}

/// This holds one loaded and instantiated eval-script module.
///
/// It wraps the `Store`, `Instance`, `Memory`, and the typed function
/// handles for the ABI. Use [`ScriptInstance::load`] to build one from a
/// `.wasm` file path.
pub struct ScriptInstance {
    store: Store<HostState>,
    memory: Memory,
    alloc_fn: TypedFunc<u32, u32>,
    dealloc_fn: TypedFunc<(u32, u32), ()>,
    rank_answer_fn: TypedFunc<(u32, u32, u32, u32, u32, u32), f32>,
    instantiation_path: InstantiationPath,
    export_names: Vec<String>,
    function_export_names: Vec<String>,
    file_size_bytes: u64,
    wasm_sha256: String,
}

impl ScriptInstance {
    /// This loads a `.wasm` file and makes it ready to call.
    ///
    /// It reads the file, compiles it, checks its imports, builds a WASI
    /// preview1 linker with an empty context, and instantiates the module.
    /// It then finds the three ABI exports: `alloc`, `dealloc`, and
    /// `rank_answer`. It returns an error if any step fails.
    pub fn load(wasm_path: &Path) -> Result<Self> {
        let wasm_bytes = std::fs::read(wasm_path)
            .with_context(|| format!("cannot read wasm file at {}", wasm_path.display()))?;
        let file_size_bytes = wasm_bytes.len() as u64;
        let wasm_sha256 = crate::golden::sha256_hex(&wasm_bytes);

        let engine = Engine::default();
        let module = Module::new(&engine, &wasm_bytes).wasm_context(format!(
            "cannot compile wasm module {}",
            wasm_path.display()
        ))?;

        let export_names: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();
        // This finds only the function exports, not `memory` or a
        // global. Section 1 of the host report checks this narrower
        // list against the published ABI surface: exactly `alloc`,
        // `dealloc`, and `rank_answer`. `memory` and any global export
        // are expected, and they are not functions, so they must not
        // count against that check.
        let function_export_names: Vec<String> = module
            .exports()
            .filter(|e| matches!(e.ty(), ExternType::Func(_)))
            .map(|e| e.name().to_string())
            .collect();

        let instantiation_path = if module
            .imports()
            .any(|i| i.module() == "wasi_snapshot_preview1")
        {
            InstantiationPath::Wasi
        } else {
            InstantiationPath::Plain
        };

        // Build an empty WASI context. No stdio, no env vars, no preopened
        // directories, no inherited network. This gives the script zero
        // ambient authority. See the `HostState` doc comment above for the
        // one known gap (random number generator).
        let wasi_ctx = WasiCtxBuilder::new()
            .wall_clock(FixedWallClock)
            .monotonic_clock(FixedMonotonicClock)
            .build_p1();

        let mut linker: Linker<HostState> = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)
            .wasm_context("cannot add WASI preview1 imports to linker")?;

        let mut store = Store::new(&engine, HostState { wasi: wasi_ctx });

        let instance: Instance = linker
            .instantiate(&mut store, &module)
            .wasm_context(format!(
                "cannot instantiate wasm module {}",
                wasm_path.display()
            ))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .context("module has no exported linear memory named 'memory'")?;

        let alloc_fn = instance
            .get_typed_func::<u32, u32>(&mut store, "alloc")
            .wasm_context("module has no 'alloc' export with the right type")?;
        let dealloc_fn = instance
            .get_typed_func::<(u32, u32), ()>(&mut store, "dealloc")
            .wasm_context("module has no 'dealloc' export with the right type")?;
        let rank_answer_fn = instance
            .get_typed_func::<(u32, u32, u32, u32, u32, u32), f32>(&mut store, "rank_answer")
            .wasm_context("module has no 'rank_answer' export with the right type")?;

        Ok(Self {
            store,
            memory,
            alloc_fn,
            dealloc_fn,
            rank_answer_fn,
            instantiation_path,
            export_names,
            function_export_names,
            file_size_bytes,
            wasm_sha256,
        })
    }

    /// This gives the linker path that this module needed.
    pub fn instantiation_path(&self) -> InstantiationPath {
        self.instantiation_path
    }

    /// This gives the list of every export name found in the module,
    /// of any kind: function, memory, or global.
    pub fn export_names(&self) -> &[String] {
        &self.export_names
    }

    /// This gives the list of function export names found in the
    /// module. It does not include `memory` or any global export.
    pub fn function_export_names(&self) -> &[String] {
        &self.function_export_names
    }

    /// This gives the size of the `.wasm` file in bytes.
    pub fn file_size_bytes(&self) -> u64 {
        self.file_size_bytes
    }

    /// This gives the lowercase hex SHA-256 hash of the `.wasm` file
    /// bytes this instance loaded.
    ///
    /// This is the same hash the `wazero-runner` golden mode writes
    /// into its own result file. The cross host check in this crate
    /// compares this hash against that file's hash, to stop a stale
    /// wazero result file from faking agreement with a different
    /// `.wasm` build.
    pub fn wasm_sha256(&self) -> &str {
        &self.wasm_sha256
    }

    /// This gives the current size of the module linear memory, in
    /// pages.
    ///
    /// One page is 64 KiB, fixed by the WASM specification. Tests use
    /// this method to prove that a rejected `alloc` call does not grow
    /// linear memory.
    pub fn memory_size_pages(&mut self) -> u64 {
        self.memory.size(&mut self.store)
    }

    /// This writes bytes into the module's linear memory.
    ///
    /// It calls `alloc` to get a block, then copies `data` into that
    /// block. It returns [`AllocOutcome::Granted`] with the pointer and
    /// length of the new block.
    ///
    /// The module's own `alloc` function rejects a request over its
    /// length cap. It signals a rejection by returning the address 0,
    /// and it never hands out 0 as the address of a real block. When
    /// that happens, this function returns [`AllocOutcome::Rejected`]
    /// instead of an error. A rejection is not a wasmtime failure and
    /// not a host bug; it is the module's own defence working as
    /// designed. The caller must match on the result and must not read
    /// or write through a rejected outcome.
    ///
    /// `alloc` can grow linear memory when it grants a block. A memory
    /// grow can move the memory buffer in the host process. Any
    /// `&mut [u8]` view taken before the grow is no longer valid to use.
    /// This function only takes the memory view AFTER the `alloc` call,
    /// so it never reads a stale view. A rejected `alloc` call never
    /// grows memory, so no view is needed on that path.
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<AllocOutcome> {
        let len = data.len() as u32;
        let ptr = self
            .alloc_fn
            .call(&mut self.store, len)
            .wasm_context("call to 'alloc' failed")?;
        if ptr == 0 {
            // The module never hands out address 0 for a real block.
            // A 0 return means the module's own length cap said no,
            // before it grew memory or read any byte. Report this as a
            // rejection, not as an error.
            return Ok(AllocOutcome::Rejected);
        }

        // Fetch the memory view now, after `alloc`. The grow above may
        // have moved the buffer, so any earlier view is stale.
        let mem_data = self.memory.data_mut(&mut self.store);
        let start = ptr as usize;
        let end = start + data.len();
        if end > mem_data.len() {
            bail!("'alloc' returned a block that does not fit in linear memory");
        }
        mem_data[start..end].copy_from_slice(data);

        Ok(AllocOutcome::Granted(ptr, len))
    }

    /// This frees a block that [`ScriptInstance::write_bytes`] made.
    pub fn free(&mut self, ptr: u32, len: u32) -> Result<()> {
        self.dealloc_fn
            .call(&mut self.store, (ptr, len))
            .wasm_context("call to 'dealloc' failed")?;
        Ok(())
    }

    /// This writes one `rank_answer` field into module linear memory.
    ///
    /// It follows the fixed host convention that the go-tester
    /// reference host and the `wazero-runner` tool both use: a zero
    /// length field is `ptr=0, len=0`, and `alloc` is never called for
    /// it. A non-empty field goes through the normal `alloc`-then-write
    /// cycle in [`ScriptInstance::write_bytes`].
    ///
    /// A zero length read never touches the pointer on the module
    /// side, so a host that always called `alloc(0)` would still get
    /// the same score. This host matches the convention exactly
    /// anyway, so a side-by-side call trace between this host and the
    /// wazero host also matches, not only the final score.
    fn write_field(&mut self, data: &[u8]) -> Result<AllocOutcome> {
        if data.is_empty() {
            return Ok(AllocOutcome::Granted(0, 0));
        }
        self.write_bytes(data)
    }

    /// This frees one field block that [`ScriptInstance::write_field`]
    /// made, on a best-effort basis.
    ///
    /// It does nothing for a zero length field, because
    /// [`ScriptInstance::write_field`] never calls `alloc` for one, so
    /// there is no block to free.
    fn free_field(&mut self, ptr: u32, len: u32) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        self.free(ptr, len)
    }

    /// This runs the full write/call/free cycle for `rank_answer`.
    ///
    /// It gives back only the score value. When the module's `alloc`
    /// cap rejects one of the three fields, this gives back `0.0`, the
    /// same value `rank_answer` itself would give for an input over
    /// the cap. Use [`ScriptInstance::rank_answer_outcome`] to also
    /// learn whether that rejection happened.
    pub fn rank_answer(&mut self, question: &[u8], gt: &[u8], ma: &[u8]) -> Result<f32> {
        self.rank_answer_outcome(question, gt, ma)
            .map(|outcome| outcome.value)
    }

    /// This runs the full write/call/free cycle for `rank_answer` and
    /// reports whether the module's `alloc` cap rejected the question,
    /// the ground truth, or the miner answer before `rank_answer` ran.
    ///
    /// It writes `question`, `gt`, and `ma` into linear memory, in that
    /// order, following the argument order the published ABI uses. It
    /// calls `rank_answer`, then frees every block it allocated. It
    /// frees the blocks even if the call itself returns an error, on a
    /// best-effort basis, so memory does not leak on a partial
    /// failure. A rejected allocation on any field skips the call to
    /// `rank_answer` and skips `dealloc` for the fields with no block
    /// to free.
    pub fn rank_answer_outcome(
        &mut self,
        question: &[u8],
        gt: &[u8],
        ma: &[u8],
    ) -> Result<RankAnswerOutcome> {
        let q_outcome = self
            .write_field(question)
            .context("cannot write question bytes")?;
        let (q_ptr, q_len) = match q_outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => {
                // The module's cap rejected the question before any
                // read. Do not call `rank_answer`. There is no block to
                // free.
                return Ok(RankAnswerOutcome {
                    value: 0.0,
                    alloc_rejected: true,
                });
            }
        };

        let gt_outcome = match self.write_field(gt) {
            Ok(outcome) => outcome,
            Err(e) => {
                // Best effort: free the question block before we give
                // up.
                let _ = self.free_field(q_ptr, q_len);
                return Err(e).context("cannot write ground_truth bytes");
            }
        };
        let (gt_ptr, gt_len) = match gt_outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => {
                // The ground truth was rejected. Free the question
                // block the host already holds, then stop. Do not call
                // `rank_answer`.
                let free_q = self.free_field(q_ptr, q_len);
                free_q.context(
                    "cannot free question block after a rejected ground_truth allocation",
                )?;
                return Ok(RankAnswerOutcome {
                    value: 0.0,
                    alloc_rejected: true,
                });
            }
        };

        let ma_outcome = match self.write_field(ma) {
            Ok(outcome) => outcome,
            Err(e) => {
                // Best effort: free the question and ground_truth
                // blocks before we give up.
                let _ = self.free_field(q_ptr, q_len);
                let _ = self.free_field(gt_ptr, gt_len);
                return Err(e).context("cannot write miner answer bytes");
            }
        };
        let (ma_ptr, ma_len) = match ma_outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => {
                // The miner answer was rejected. Free the question and
                // ground_truth blocks the host already holds, then
                // stop. Do not call `rank_answer`.
                let free_q = self.free_field(q_ptr, q_len);
                let free_gt = self.free_field(gt_ptr, gt_len);
                free_q.context(
                    "cannot free question block after a rejected miner answer allocation",
                )?;
                free_gt.context(
                    "cannot free ground_truth block after a rejected miner answer allocation",
                )?;
                return Ok(RankAnswerOutcome {
                    value: 0.0,
                    alloc_rejected: true,
                });
            }
        };

        let call_result = self.rank_answer_fn.call(
            &mut self.store,
            (q_ptr, q_len, gt_ptr, gt_len, ma_ptr, ma_len),
        );

        // Free every block on a best-effort basis. This runs whether
        // the call above worked or trapped, so memory does not leak.
        let free_q = self.free_field(q_ptr, q_len);
        let free_gt = self.free_field(gt_ptr, gt_len);
        let free_ma = self.free_field(ma_ptr, ma_len);

        let value = call_result.wasm_context("call to 'rank_answer' failed")?;
        free_q.context("cannot free question block after a successful call")?;
        free_gt.context("cannot free ground_truth block after a successful call")?;
        free_ma.context("cannot free miner answer block after a successful call")?;
        Ok(RankAnswerOutcome {
            value,
            alloc_rejected: false,
        })
    }
}
