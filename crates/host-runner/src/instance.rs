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
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};
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

/// This is the result of one full call to a scoring function, through
/// the whole alloc/write/call/dealloc cycle.
///
/// A validator report needs more than the score value. It also needs
/// to know whether the module's own `alloc` cap rejected the ground
/// truth or the response before the scoring function ran at all. See
/// [`ScriptInstance::score_outcome`].
#[derive(Debug, Clone, Copy)]
pub struct ScoreOutcome {
    /// The score. This is `0.0` when `alloc_rejected` is `true`.
    pub value: f64,
    /// `true` when the module's `alloc` cap rejected the ground truth
    /// or the response before the host called the scoring function.
    /// `false` means the scoring function itself produced `value`. A
    /// `false` result can still carry a `value` of `0.0`, for the
    /// module's own reasons, such as invalid JSON. Read `value` and
    /// `alloc_rejected` together to tell which defence fired.
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
    score_fn: TypedFunc<(u32, u32, u32, u32), f64>,
    score_log_loss_fn: TypedFunc<(u32, u32, u32, u32), f64>,
    score_batch_fn: TypedFunc<(u32, u32), f64>,
    instantiation_path: InstantiationPath,
    export_names: Vec<String>,
    file_size_bytes: u64,
}

impl ScriptInstance {
    /// This loads a `.wasm` file and makes it ready to call.
    ///
    /// It reads the file, compiles it, checks its imports, builds a WASI
    /// preview1 linker with an empty context, and instantiates the module.
    /// It then finds the five ABI exports. It returns an error if any step
    /// fails.
    pub fn load(wasm_path: &Path) -> Result<Self> {
        let wasm_bytes = std::fs::read(wasm_path)
            .with_context(|| format!("cannot read wasm file at {}", wasm_path.display()))?;
        let file_size_bytes = wasm_bytes.len() as u64;

        let engine = Engine::default();
        let module = Module::new(&engine, &wasm_bytes).wasm_context(format!(
            "cannot compile wasm module {}",
            wasm_path.display()
        ))?;

        let export_names: Vec<String> = module.exports().map(|e| e.name().to_string()).collect();

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
        let score_fn = instance
            .get_typed_func::<(u32, u32, u32, u32), f64>(&mut store, "score")
            .wasm_context("module has no 'score' export with the right type")?;
        let score_log_loss_fn = instance
            .get_typed_func::<(u32, u32, u32, u32), f64>(&mut store, "score_log_loss")
            .wasm_context("module has no 'score_log_loss' export with the right type")?;
        let score_batch_fn = instance
            .get_typed_func::<(u32, u32), f64>(&mut store, "score_batch")
            .wasm_context("module has no 'score_batch' export with the right type")?;

        Ok(Self {
            store,
            memory,
            alloc_fn,
            dealloc_fn,
            score_fn,
            score_log_loss_fn,
            score_batch_fn,
            instantiation_path,
            export_names,
            file_size_bytes,
        })
    }

    /// This gives the linker path that this module needed.
    pub fn instantiation_path(&self) -> InstantiationPath {
        self.instantiation_path
    }

    /// This gives the list of export names found in the module.
    pub fn export_names(&self) -> &[String] {
        &self.export_names
    }

    /// This gives the size of the `.wasm` file in bytes.
    pub fn file_size_bytes(&self) -> u64 {
        self.file_size_bytes
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

    /// This runs the full alloc/write/call/dealloc cycle for `score`.
    ///
    /// It gives back only the score value. When the module's `alloc`
    /// cap rejects the input, this gives back `0.0`, the same value the
    /// scoring function itself would give for an input over the cap.
    /// Use [`ScriptInstance::score_outcome`] to also learn whether that
    /// rejection happened.
    pub fn score(&mut self, gt: &[u8], resp: &[u8]) -> Result<f64> {
        self.score_outcome(gt, resp).map(|outcome| outcome.value)
    }

    /// This runs the full alloc/write/call/dealloc cycle for `score` and
    /// reports whether the module's `alloc` cap rejected the ground
    /// truth or the response before the scoring function ran.
    ///
    /// It writes `gt` and `resp` into linear memory, calls `score`, then
    /// frees both blocks. It frees the blocks even if the call itself
    /// returns an error, on a best-effort basis, so memory does not leak
    /// on a partial failure. A rejected allocation skips the call to
    /// `score` and skips `dealloc` for the rejected block, because there
    /// is no block to free.
    pub fn score_outcome(&mut self, gt: &[u8], resp: &[u8]) -> Result<ScoreOutcome> {
        self.run_score_call(gt, resp, |me, gt_ptr, gt_len, resp_ptr, resp_len| {
            me.score_fn
                .call(&mut me.store, (gt_ptr, gt_len, resp_ptr, resp_len))
        })
    }

    /// This runs the full alloc/write/call/dealloc cycle for
    /// `score_log_loss`. See [`ScriptInstance::score`] for the rejected
    /// allocation behavior.
    pub fn score_log_loss(&mut self, gt: &[u8], resp: &[u8]) -> Result<f64> {
        self.score_log_loss_outcome(gt, resp)
            .map(|outcome| outcome.value)
    }

    /// This runs the full alloc/write/call/dealloc cycle for
    /// `score_log_loss` and reports whether the module's `alloc` cap
    /// rejected the input first. See
    /// [`ScriptInstance::score_outcome`] for the full rule.
    pub fn score_log_loss_outcome(&mut self, gt: &[u8], resp: &[u8]) -> Result<ScoreOutcome> {
        self.run_score_call(gt, resp, |me, gt_ptr, gt_len, resp_ptr, resp_len| {
            me.score_log_loss_fn
                .call(&mut me.store, (gt_ptr, gt_len, resp_ptr, resp_len))
        })
    }

    /// This is the shared alloc/write/call/dealloc cycle for the two
    /// two-input score functions. `call` is the wasmtime call itself.
    ///
    /// A rejected allocation on either side never reaches `call`. The
    /// worst score, `0.0`, is exactly what the scoring function would
    /// have returned anyway for an input over the cap, so this shortcut
    /// changes no visible score, only the work the host does to get
    /// there.
    fn run_score_call(
        &mut self,
        gt: &[u8],
        resp: &[u8],
        call: impl FnOnce(&mut Self, u32, u32, u32, u32) -> wasmtime::Result<f64>,
    ) -> Result<ScoreOutcome> {
        let gt_outcome = self
            .write_bytes(gt)
            .context("cannot write ground_truth bytes")?;
        let (gt_ptr, gt_len) = match gt_outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => {
                // The module's cap rejected ground_truth before any
                // read. Do not call the scoring function. There is no
                // block to free.
                return Ok(ScoreOutcome {
                    value: 0.0,
                    alloc_rejected: true,
                });
            }
        };

        let resp_outcome = match self.write_bytes(resp) {
            Ok(outcome) => outcome,
            Err(e) => {
                // Best effort: free the ground_truth block before we
                // give up.
                let _ = self.free(gt_ptr, gt_len);
                return Err(e).context("cannot write response bytes");
            }
        };
        let (resp_ptr, resp_len) = match resp_outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => {
                // The response was rejected. Free the ground_truth
                // block the host already holds, then stop. Do not call
                // the scoring function.
                let free_gt = self.free(gt_ptr, gt_len);
                free_gt.context(
                    "cannot free ground_truth block after a rejected response allocation",
                )?;
                return Ok(ScoreOutcome {
                    value: 0.0,
                    alloc_rejected: true,
                });
            }
        };

        let call_result = call(self, gt_ptr, gt_len, resp_ptr, resp_len);

        // Free both blocks on a best-effort basis. This runs whether the
        // call above worked or trapped, so memory does not leak.
        let free_gt = self.free(gt_ptr, gt_len);
        let free_resp = self.free(resp_ptr, resp_len);

        let value = call_result.wasm_context("call to a score function failed")?;
        free_gt.context("cannot free ground_truth block after a successful call")?;
        free_resp.context("cannot free response block after a successful call")?;
        Ok(ScoreOutcome {
            value,
            alloc_rejected: false,
        })
    }

    /// This runs the full alloc/write/call/dealloc cycle for
    /// `score_batch`.
    ///
    /// When the module's `alloc` cap rejects the batch input, this
    /// gives back `0.0` at once. It skips the call to `score_batch` and
    /// skips `dealloc`, because there is no block to free.
    pub fn score_batch(&mut self, batch_json: &[u8]) -> Result<f64> {
        let outcome = self
            .write_bytes(batch_json)
            .context("cannot write batch bytes")?;
        let (ptr, len) = match outcome {
            AllocOutcome::Granted(ptr, len) => (ptr, len),
            AllocOutcome::Rejected => return Ok(0.0),
        };
        let call_result = self.score_batch_fn.call(&mut self.store, (ptr, len));
        let free_result = self.free(ptr, len);
        let value = call_result.wasm_context("call to 'score_batch' failed")?;
        free_result.context("cannot free batch block after a successful call")?;
        Ok(value)
    }
}
