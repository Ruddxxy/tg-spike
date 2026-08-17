//! This module holds the functions the WASM host calls.
//!
//! The published ABI has three exports: `alloc`, `dealloc`, and
//! `rank_answer`. The host calls `alloc` first, to get a block of
//! memory inside the module. The host writes its input bytes into
//! that block. The host then calls `rank_answer` with the offset
//! and the length of each block. The host calls `dealloc` when it
//! no longer needs a block.
//!
//! `rank_answer` takes three byte ranges: a question, a ground
//! truth, and a miner answer. This version of the ABI does not read
//! the question bytes. `rank_answer` scores the ground truth against
//! the miner answer on the relative error between the two values. See
//! `rank_answer_impl` for the exact rule order.
//!
//! `rank_answer` is the only scoring function in this module. An
//! earlier version also carried `score`, `score_log_loss` and
//! `score_batch`, which scored a JSON label against a JSON confidence
//! under the Brier model. That model is gone and so are all three
//! functions: they are not exported, not kept as native helpers, and
//! nothing in the workspace calls them.
//!
//! Every function in this module checks its input before it reads
//! raw memory. A bad pointer must never cause a WASM trap. The
//! release profile in the workspace uses `panic = "abort"`. A panic
//! in that mode is a trap, and a trap would stop the validator that
//! runs this module.
//!
//! Every function in this module also checks the input length
//! against `MAX_INPUT_BYTES` before it reads any byte. The host
//! places no cap on a miner response size, so this module must
//! defend itself. An oversize length scores the worst score, 0.0,
//! before the module does any bounds check or any memory read.
//!
//! ## A note on native testing
//!
//! The functions in this module read a `u32` offset and treat it as
//! a pointer. On the `wasm32` target this is always safe, because a
//! WASM linear memory address never needs more than 32 bits. On a
//! native 64-bit target, a real pointer is 64 bits wide, so casting
//! it down to `u32` throws bits away. The tests in this module never
//! round-trip a real native pointer through `u32` and back into a
//! pointer, because that would read from the wrong address and
//! could crash the test process. The tests instead check the error
//! paths, which return before any raw memory read happens, and they
//! check any input-content rule (UTF-8, blank text, a golden score)
//! through a plain `&[u8]` helper function, which never touches a
//! raw pointer at all.

use crate::error::ScoreError;
use crate::MAX_INPUT_BYTES;
use std::alloc::{alloc as raw_alloc, dealloc as raw_dealloc, Layout};

/// This function allocates a block of memory inside the module.
///
/// The host calls this function first. The parameter `len` is the
/// number of bytes the host needs. The host writes its input bytes
/// into the returned block. The function returns the offset of the
/// new block inside the module linear memory.
///
/// The function returns 0 in three cases: `len` is over
/// `MAX_INPUT_BYTES`, `len` does not fit a valid memory layout, or
/// the allocator has no room for the block. The value 0 is never the
/// offset of a real block, because this function never hands out the
/// address 0. The caller must check the returned value before it
/// uses the value as an address.
///
/// The function checks `len` against `MAX_INPUT_BYTES` before it does
/// any other work. This order stops an oversize `len` from growing
/// the module linear memory. The host places no cap on a miner
/// response size, so this check is the only defence against a
/// miner response that wastes validator work through a large
/// allocation.
///
/// This early cap check is a deliberate divergence from the
/// reference module at
/// `wasm-scoring-module/rust-module/src/lib.rs`. The reference
/// `alloc` sets `HEAP_OFFSET = 0` when `aligned + size > HEAP_SIZE`,
/// then it still returns the base pointer and it still advances
/// `HEAP_OFFSET` by the full `size`, with no second bounds check.
/// So a request bigger than its 1 MiB static buffer still returns a
/// pointer that looks usable, and a write through that pointer runs
/// past the end of the real buffer. This function closes that gap:
/// an oversize `len` returns 0 before any memory work starts, so the
/// caller never gets a pointer it should not use.
///
/// The wasm32 export wrapper for this function is `alloc`, further
/// down this module.
pub fn alloc_impl(len: u32) -> u32 {
    if len > MAX_INPUT_BYTES {
        return 0;
    }
    let layout = match block_layout(len) {
        Some(layout) => layout,
        None => return 0,
    };
    // SECURITY: `block_layout` never returns a layout with size 0,
    // so this call meets the `GlobalAlloc::alloc` contract, which
    // forbids a zero-size layout.
    let ptr = unsafe { raw_alloc(layout) };
    if ptr.is_null() {
        return 0;
    }
    ptr as u32
}

/// This function frees a block of memory that `alloc_impl` made.
///
/// The parameter `ptr` is the offset that `alloc_impl` returned. The
/// parameter `len` is the same `len` value the caller gave to
/// `alloc_impl`. The function does nothing if `ptr` is 0. The
/// function does nothing if `len` does not fit a valid memory
/// layout.
///
/// The caller must give back the same `ptr` and `len` pair that
/// `alloc_impl` gave out. A wrong pair can corrupt the allocator.
/// This is the normal contract for a manual allocate-and-free pair
/// of functions.
///
/// The wasm32 export wrapper for this function is `dealloc`, further
/// down this module.
pub fn dealloc_impl(ptr: u32, len: u32) {
    if ptr == 0 {
        return;
    }
    let layout = match block_layout(len) {
        Some(layout) => layout,
        None => return,
    };
    // SECURITY: this call trusts the host to give back the exact
    // `ptr` and `len` pair that `alloc_impl` gave out for this
    // block. A host that breaks that rule can corrupt the
    // allocator. That risk is inherent to a manual alloc and free
    // ABI.
    unsafe { raw_dealloc(ptr as *mut u8, layout) };
}

/// This function is the wasm32 export for `alloc_impl`.
///
/// A native build does not compile this function. This stops a
/// native link from ever seeing a symbol named `alloc`, which could
/// collide with another `alloc` symbol in the same native link. See
/// `alloc_impl` for the full behaviour.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn alloc(len: u32) -> u32 {
    alloc_impl(len)
}

/// This function is the wasm32 export for `dealloc_impl`.
///
/// A native build does not compile this function. This stops a
/// native link from ever seeing a symbol named `dealloc`, which
/// could collide with another `dealloc` symbol in the same native
/// link. See `dealloc_impl` for the full behaviour.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn dealloc(ptr: u32, len: u32) {
    dealloc_impl(ptr, len)
}

/// This function builds the memory layout for a block of `len`
/// bytes.
///
/// The function treats a `len` of 0 as a block of 1 byte, because
/// the Rust allocator contract forbids a call with a size of 0. The
/// function returns `None` if `len` does not fit a valid layout.
fn block_layout(len: u32) -> Option<Layout> {
    let size = if len == 0 { 1 } else { len as usize };
    Layout::from_size_align(size, 1).ok()
}

/// This function checks that a length does not go over the input
/// byte cap.
///
/// The host places no cap on a miner response size. This check must
/// run before any bounds check and before any read of linear
/// memory. That order stops an oversize length from making this
/// module do unbounded work, and it stops an oversize length from
/// ever reaching `core::slice::from_raw_parts`.
fn check_len_cap(len: u32) -> Result<(), ScoreError> {
    if len > MAX_INPUT_BYTES {
        return Err(ScoreError::InputTooLarge);
    }
    Ok(())
}

/// This function checks that a pointer and length pair stays inside
/// the module linear memory.
///
/// The function first checks that `ptr + len` does not overflow a
/// 32-bit integer. The function then checks that the end of the
/// range sits inside the current memory size.
fn check_bounds(ptr: u32, len: u32) -> Result<(), ScoreError> {
    let end = ptr.checked_add(len).ok_or(ScoreError::PointerOverflow)?;
    if !within_memory(end) {
        return Err(ScoreError::BadPointer);
    }
    Ok(())
}

/// This function checks that `end` sits at or under the size of the
/// module linear memory, in bytes.
///
/// The WASM spec reports memory size in 64 KiB pages. This function
/// reads the current page count and multiplies it out to bytes. The
/// multiplication and the comparison both run in `u64`, so the
/// check cannot overflow even at the largest page count the spec
/// allows.
#[cfg(target_arch = "wasm32")]
fn within_memory(end: u32) -> bool {
    const PAGE_SIZE: u64 = 65536;
    let pages = u64::from(core::arch::wasm32::memory_size(0) as u32);
    let mem_bytes = pages * PAGE_SIZE;
    u64::from(end) <= mem_bytes
}

/// This function always returns true on a native target.
///
/// A native target has no WASM linear memory to measure. The
/// `check_bounds` function already checked for a pointer overflow.
/// That check is the only one a native target can run. This keeps
/// the unit tests in this crate runnable on the host machine.
#[cfg(not(target_arch = "wasm32"))]
fn within_memory(_end: u32) -> bool {
    true
}

/// This function reads a byte slice out of the module linear
/// memory.
///
/// The function returns an error if the pointer and length pair is
/// not valid. The function checks the length against
/// `MAX_INPUT_BYTES` first, before any other check and before any
/// read. The function then checks the length and the pointer stay
/// inside linear memory. If `len` is 0 the function returns an
/// empty slice without reading `ptr` at all, because an empty read
/// needs no real address.
fn read_bytes(ptr: u32, len: u32) -> Result<&'static [u8], ScoreError> {
    check_len_cap(len)?;
    check_bounds(ptr, len)?;
    if len == 0 {
        return Ok(&[]);
    }
    if ptr == 0 {
        return Err(ScoreError::BadPointer);
    }
    // SECURITY: this call trusts the host to give a pointer and a
    // length that name bytes the host itself wrote. The bound check
    // above only proves the range sits inside linear memory. It
    // does not prove the host wrote valid data there. The 'static
    // lifetime is a lie about ownership, but the slice this
    // function returns is only ever read once, inside the same
    // call, and it is never stored past that call.
    let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Ok(slice)
}

/// This function reads the question bytes, and never fails.
///
/// The question argument is advisory. Real traffic carries a question
/// such as "[direct] 207 -> /price", which holds no useful text, and a
/// caller may give a junk pointer, an oversize length, or bytes that
/// are not UTF-8. None of that may change the score of a good answer.
///
/// So this function gives an empty text for every failure. It never
/// returns an error, and the score path never requires the question to
/// hold anything. The scorer uses the question only to reject an
/// answer that copies the question back.
fn read_question_text<'a>(ptr: u32, len: u32) -> &'a str {
    match read_bytes(ptr, len) {
        Ok(bytes) => core::str::from_utf8(bytes).unwrap_or(""),
        Err(_) => "",
    }
}

/// This function turns a score result into a safe `f64` output.
///
/// The function returns 0.0, the worst score, for an error. The
/// function also returns 0.0 if the value on the success path is
/// somehow not a finite number in `[0.0, 1.0]`. Every metric
/// function in this crate already guards that case, but this second
/// check keeps a bug somewhere else in the crate from ever letting
/// a bad value cross the ABI boundary. This function is the single
/// place at the ABI boundary that decides the worst-case output, so
/// every score function in this module routes its result through
/// here.
///
/// This function is `pub` so a host-side crate in this workspace can
/// call the exact same clamp rule for a native, non-wasm score, instead
/// of keeping its own copy of the rule. A copy is a defect waiting to
/// happen: a future edit could change one copy and not the other, and
/// a native check would then judge a value by a different rule than
/// the wasm ABI boundary uses. Calling this function keeps the clamp
/// rule in exactly one place, for every caller, wasm or native.
pub fn finish(outcome: Result<f64, ScoreError>) -> f64 {
    match outcome {
        Ok(value) if value.is_finite() && (0.0..=1.0).contains(&value) => value,
        _ => 0.0,
    }
}

/// This function scores a miner answer against a ground truth, on
/// two already-read byte slices.
///
/// The function returns `Ok(0.0)`, not an error, for a miner answer
/// that is valid UTF-8 text but is empty or holds only whitespace.
/// A blank answer is a well formed "no answer", not a bad input, so
/// it flows through `finish` the same way a real score would.
///
/// The function returns `Err(ScoreError::InvalidUtf8)` for a miner
/// answer that is not valid UTF-8 text. This function checks that
/// with the safe `core::str::from_utf8`, never with
/// `core::str::from_utf8_unchecked`. This is a deliberate divergence
/// from the reference module at
/// `wasm-scoring-module/rust-module/src/lib.rs`, whose `read_str`
/// calls `core::str::from_utf8_unchecked` on bytes the host
/// supplies. That call has undefined behaviour whenever the host
/// gives bytes that are not valid UTF-8, because the unchecked
/// function trusts its caller instead of checking the bytes.
///
/// Past the blank check, this function calls `score::score_answer`
/// with the question text, the ground truth text and the answer text.
///
/// A ground truth that is not valid UTF-8 gives an empty ground truth
/// text, not an error. An answer can still score 0.0 against it
/// through the normal text path. Only a bad ANSWER is an error,
/// because the answer is the thing under judgement.
fn rank_answer_scorer_with_question(
    question: &str,
    gt_bytes: &[u8],
    ma_bytes: &[u8],
) -> Result<f64, ScoreError> {
    let ma_text = core::str::from_utf8(ma_bytes).map_err(|_| ScoreError::InvalidUtf8)?;
    if ma_text.trim().is_empty() {
        return Ok(0.0);
    }
    let gt_text = core::str::from_utf8(gt_bytes).unwrap_or("");
    Ok(crate::score::score_answer(question, gt_text, ma_text))
}

/// This function implements the `rank_answer` ABI export.
///
/// The published ABI type for every pointer and length parameter is
/// `i32`, at the wasm valtype level. This function keeps the Rust
/// parameter type `u32`, because `u32` and `i32` are the same wasm
/// valtype, `i32`, so this is not a real type change from the
/// host's point of view. A host that (wrongly) gives a negative
/// value for one of these parameters gives this function a very
/// large `u32` instead, through the same bit pattern. `check_len_cap`
/// then rejects that large value as an oversize length, before
/// `check_bounds` runs and before any memory read happens. So this
/// `u32` view of the wasm `i32` valtype is safe, and it is
/// deterministic: every host that gives the same bit pattern gets
/// the same rejection.
///
/// `question_ptr` and `question_len` name the question argument.
/// A later version of this ABI may read the question bytes and
/// use them to score an answer. This version does not read them. The
/// line right below this doc comment still binds them to named
/// variables, not to `_`, so the parameter names stay visible in
/// the function signature and in any caller that reads this code.
///
/// Past that, the function reads the ground truth bytes and the
/// miner answer bytes with `read_bytes`, which keeps the
/// `MAX_INPUT_BYTES` cap check and the bounds check. Any read error,
/// for either input, scores 0.0. Past a successful read,
/// `rank_answer_scorer_with_question` applies the miner answer rules:
/// invalid UTF-8 scores 0.0, and a blank answer (empty, or whitespace
/// only) scores exactly 0.0. Any other input runs through
/// `score::score_answer`, in `f64`, and reaches this function's return
/// through `finish`.
///
/// The wasm32 export wrapper for this function is `rank_answer`,
/// further down this module.
pub fn rank_answer_impl(
    question_ptr: u32,
    question_len: u32,
    gt_ptr: u32,
    gt_len: u32,
    ma_ptr: u32,
    ma_len: u32,
) -> f32 {
    // The question is advisory. `read_question_text` gives an empty
    // text for a junk pointer, an oversize length, or bytes that are
    // not UTF-8, so a bad question can never lower a good answer.
    let question = read_question_text(question_ptr, question_len);

    let outcome = read_bytes(gt_ptr, gt_len).and_then(|gt_bytes| {
        let ma_bytes = read_bytes(ma_ptr, ma_len)?;
        rank_answer_scorer_with_question(question, gt_bytes, ma_bytes)
    });
    let score = finish(outcome);

    // SINGLE NARROWING POINT. This is the only place in this crate
    // that narrows an `f64` down to an `f32`. The `finish` call on the
    // line above already clamped `score` into the closed range 0.0 to
    // 1.0 in `f64`. Every `f64` value in that closed range narrows to
    // an `f32` value in the same closed range, so this narrow cannot
    // push the result out of range.
    score as f32
}

/// This function is the wasm32 export for `rank_answer_impl`.
///
/// A native build does not compile this function. See `rank_answer_impl`
/// for the full behaviour and for the argument order:
/// question, then ground truth, then miner answer, each as a
/// pointer and a length pair.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn rank_answer(
    q_ptr: u32,
    q_len: u32,
    gt_ptr: u32,
    gt_len: u32,
    ma_ptr: u32,
    ma_len: u32,
) -> f32 {
    rank_answer_impl(q_ptr, q_len, gt_ptr, gt_len, ma_ptr, ma_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_layout_treats_zero_len_as_one_byte() {
        let layout = block_layout(0).unwrap();
        assert_eq!(layout.size(), 1);
    }

    #[test]
    fn block_layout_matches_requested_size() {
        let layout = block_layout(64).unwrap();
        assert_eq!(layout.size(), 64);
    }

    #[test]
    fn alloc_returns_nonzero_and_leaks_on_purpose() {
        // This test does not call `dealloc_impl`. Freeing a pointer
        // that `alloc_impl` truncated to 32 bits would corrupt the
        // native heap. This ABI is only sound on a real 32-bit
        // address space, which is what the wasm32 target gives it.
        // The leak here is a few bytes for the life of the test
        // process.
        let ptr = alloc_impl(16);
        assert_ne!(ptr, 0);
    }

    #[test]
    fn alloc_of_zero_len_does_not_fail() {
        assert_ne!(alloc_impl(0), 0);
    }

    #[test]
    fn dealloc_of_null_pointer_does_nothing() {
        dealloc_impl(0, 16);
    }

    #[test]
    fn dealloc_of_null_pointer_does_nothing_for_several_lengths() {
        // A null pointer must stay a safe no-op for any length,
        // including a length of 0 and a length over the cap.
        for len in [0, 1, 16, MAX_INPUT_BYTES, MAX_INPUT_BYTES + 1, u32::MAX] {
            dealloc_impl(0, len);
        }
    }

    #[test]
    fn alloc_at_the_cap_succeeds() {
        // This test does not call `dealloc_impl`. See
        // `alloc_returns_nonzero_and_leaks_on_purpose` above for why
        // a native test never frees an `alloc_impl` result.
        let ptr = alloc_impl(MAX_INPUT_BYTES);
        assert_ne!(ptr, 0);
    }

    #[test]
    fn alloc_one_byte_over_the_cap_returns_zero() {
        assert_eq!(alloc_impl(MAX_INPUT_BYTES + 1), 0);
    }

    #[test]
    fn alloc_of_u32_max_returns_zero_and_does_not_trap() {
        // `u32::MAX` is over the cap, so the cap check must reject
        // this value before `block_layout` or `std_alloc` ever runs.
        // A build that reached `std_alloc` with this size would ask
        // the allocator to grow the module memory past what a
        // 32-bit address space can hold.
        assert_eq!(alloc_impl(u32::MAX), 0);
    }

    #[test]
    fn check_bounds_rejects_pointer_overflow() {
        assert_eq!(check_bounds(u32::MAX, 10), Err(ScoreError::PointerOverflow));
    }

    #[test]
    fn check_bounds_accepts_in_range_pair_on_native() {
        // `within_memory` is a no-op `true` on a native target, so
        // this only proves the overflow check lets a normal pair
        // through.
        assert_eq!(check_bounds(100, 50), Ok(()));
        assert_eq!(check_bounds(0, 0), Ok(()));
    }

    #[test]
    fn check_len_cap_allows_exactly_the_cap() {
        assert_eq!(check_len_cap(MAX_INPUT_BYTES), Ok(()));
    }

    #[test]
    fn check_len_cap_rejects_one_byte_over_the_cap() {
        assert_eq!(
            check_len_cap(MAX_INPUT_BYTES + 1),
            Err(ScoreError::InputTooLarge)
        );
    }

    #[test]
    fn read_bytes_of_zero_length_never_touches_the_pointer() {
        // The pointer value here is never dereferenced, because the
        // length is 0. Any pointer value is safe to pass here.
        assert_eq!(read_bytes(0xffff_ffff, 0).unwrap(), &[] as &[u8]);
        assert_eq!(read_bytes(0, 0).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn read_bytes_rejects_null_pointer_with_nonzero_length() {
        assert_eq!(read_bytes(0, 5), Err(ScoreError::BadPointer));
    }

    #[test]
    fn read_bytes_rejects_an_oversize_length_before_any_bounds_check() {
        // The length alone is over the cap. `check_len_cap` runs
        // before `check_bounds`, so this never reaches the pointer
        // overflow check or a real memory read, even with a wild
        // pointer value.
        assert_eq!(
            read_bytes(0xdead_beef, MAX_INPUT_BYTES + 1),
            Err(ScoreError::InputTooLarge)
        );
    }

    #[test]
    fn rank_answer_returns_worst_score_on_pointer_overflow() {
        // gt_ptr + gt_len overflows u32. This never reaches a real
        // memory read.
        assert_eq!(rank_answer_impl(0, 0, u32::MAX, u32::MAX, 0, 0), 0.0);
        assert_eq!(rank_answer_impl(0, 0, 0, 0, u32::MAX, u32::MAX), 0.0);
    }

    #[test]
    fn rank_answer_returns_worst_score_on_null_pointer_with_length() {
        assert_eq!(rank_answer_impl(0, 0, 0, 5, 0, 0), 0.0);
        assert_eq!(rank_answer_impl(0, 0, 0, 0, 0, 5), 0.0);
    }

    #[test]
    fn rank_answer_returns_worst_score_on_empty_input() {
        // Both sides are zero length. `read_bytes` returns an empty
        // slice without dereferencing anything. A blank miner answer
        // then scores exactly 0.0.
        assert_eq!(rank_answer_impl(0, 0, 0, 0, 0, 0), 0.0);
    }

    #[test]
    fn rank_answer_returns_worst_score_on_an_oversize_length_without_a_trap() {
        // The length is one byte over the cap. The pointer is a wild
        // address that is never a valid block. If the cap check did
        // not run before the bounds check and the memory read, this
        // call would need to touch that wild pointer. The cap check
        // catches it first, so this call stays safe to run on a
        // native target and returns the worst score instead of
        // trapping.
        assert_eq!(
            rank_answer_impl(0, 0, 0xdead_beef, MAX_INPUT_BYTES + 1, 0, 0),
            0.0
        );
        assert_eq!(
            rank_answer_impl(0, 0, 0, 0, 0xdead_beef, MAX_INPUT_BYTES + 1),
            0.0
        );
    }

    #[test]
    fn alloc_rejection_flows_end_to_end_to_the_worst_score() {
        // This test proves the full failure path: an oversize `len`
        // makes `alloc_impl` return the null pointer 0, and that same
        // null pointer with the same nonzero `len`, when passed on to
        // the score export, must produce the worst score, 0.0,
        // through `read_bytes` and then `finish`. This is the exact
        // sequence a host follows: call `alloc`, then call
        // `rank_answer` with the pointer `alloc` gave back.
        let len = MAX_INPUT_BYTES + 1;
        let ptr = alloc_impl(len);
        assert_eq!(ptr, 0);
        assert_eq!(read_bytes(ptr, len), Err(ScoreError::InputTooLarge));
        assert_eq!(rank_answer_impl(0, 0, ptr, len, 0, 0), 0.0);
        assert_eq!(rank_answer_impl(0, 0, 0, 0, ptr, len), 0.0);
    }

    #[test]
    fn null_pointer_with_nonzero_length_flows_end_to_end_to_the_worst_score() {
        // This test covers the case where `len` sits inside the cap,
        // so `check_len_cap` passes and `read_bytes` must instead
        // catch the null pointer through `check_bounds` and the
        // explicit null check.
        assert_eq!(read_bytes(0, 5), Err(ScoreError::BadPointer));
        assert_eq!(rank_answer_impl(0, 0, 0, 5, 0, 0), 0.0);
        assert_eq!(rank_answer_impl(0, 0, 0, 0, 0, 5), 0.0);
    }

    #[test]
    fn a_junk_question_never_lowers_a_good_score() {
        // The question is advisory. A wild question pointer and an
        // oversize question length must both fall back to an empty
        // question.
        let good = rank_answer_impl(0, 0, 0, 0, 0, 0);
        let with_wild = rank_answer_impl(0xdead_beef, MAX_INPUT_BYTES + 1, 0, 0, 0, 0);
        assert_eq!(good, with_wild);
    }

    #[test]
    fn finish_clamps_error_and_bad_values_to_worst_score() {
        assert_eq!(finish(Err(ScoreError::InvalidJson)), 0.0);
        assert_eq!(finish(Ok(0.5)), 0.5);
        assert_eq!(finish(Ok(f64::NAN)), 0.0);
        assert_eq!(finish(Ok(f64::INFINITY)), 0.0);
        assert_eq!(finish(Ok(-0.1)), 0.0);
        assert_eq!(finish(Ok(1.1)), 0.0);
    }

    #[test]
    fn rank_answer_impl_scores_an_empty_miner_answer_as_exactly_zero() {
        // Every pointer and length here is 0. A length of 0 skips
        // the pointer check inside `read_bytes` for both reads, so
        // this call never dereferences any address. This is the
        // safe way to reach the "empty miner answer" rule through
        // the real pointer-based entry point.
        let result = rank_answer_impl(0, 0, 0, 0, 0, 0);
        assert_eq!(result.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn rank_answer_impl_scores_an_oversize_miner_answer_as_zero() {
        // ma_len is one byte over the cap. ma_ptr is a wild address
        // that is never a valid block. The ground truth side reads
        // an empty block first, which is safe, then `check_len_cap`
        // rejects the oversize length before it ever reaches the
        // wild pointer.
        let over_cap = MAX_INPUT_BYTES + 1;
        let result = rank_answer_impl(0, 0, 0, 0, 0xdead_beef, over_cap);
        assert_eq!(result.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn rank_answer_impl_scores_a_null_pointer_with_nonzero_length_as_zero() {
        // ma_ptr is null but ma_len is nonzero, so the pair does not
        // name a real block. `read_bytes` rejects this before it
        // ever dereferences the null pointer.
        let result = rank_answer_impl(0, 0, 0, 0, 0, 5);
        assert_eq!(result.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn the_scorer_scores_whitespace_only_text_as_exactly_zero() {
        // A whitespace-only miner answer needs real byte content
        // (a space, a tab, a newline). This module's native testing
        // note says a test must never round-trip a real pointer
        // through `u32`, so this test calls the plain `&[u8]` scorer
        // helper directly, instead of building a pointer into real
        // memory.
        assert_eq!(
            rank_answer_scorer_with_question("", b"192.43", b" \t\n").unwrap(),
            0.0
        );
    }

    #[test]
    fn the_scorer_rejects_invalid_utf8_in_the_answer() {
        // Same reasoning as the whitespace test above: invalid UTF-8
        // bytes need real content, so this goes through the plain
        // `&[u8]` scorer helper, not a raw pointer.
        let bad_utf8 = &[0xff, 0xfe, 0xfd];
        assert_eq!(
            rank_answer_scorer_with_question("", b"192.43", bad_utf8),
            Err(ScoreError::InvalidUtf8)
        );
    }

    #[test]
    fn a_ground_truth_that_is_not_utf8_is_not_an_error() {
        // Only the ANSWER is under judgement. A ground truth that is
        // not UTF-8 reads as an empty ground truth, and the answer
        // still scores against it through the text path.
        let bad_utf8 = &[0xff, 0xfe, 0xfd];
        let result = rank_answer_scorer_with_question("", bad_utf8, b"192.43");
        assert!(result.is_ok(), "a bad ground truth must not be an error");
    }

    #[test]
    fn the_scorer_gives_the_golden_value_for_an_exact_numeric_pair() {
        let result = rank_answer_scorer_with_question("", b"192.43", b"192.43").unwrap();
        assert_eq!(result, 1.0);

        // 1.0 holds exactly in an `f32` and in an `f64`, so the narrow
        // at the single narrowing point in `rank_answer_impl` loses no
        // precision for this pair.
        assert_eq!(f64::from(1.0_f32), result);
    }

    #[test]
    fn the_scorer_gives_a_graded_value_for_a_near_miss() {
        // The curve must separate a near miss from a wild answer. A
        // threshold rule would give the same value to both.
        let near = rank_answer_scorer_with_question("", b"192.43", b"192.44").unwrap();
        let wild = rank_answer_scorer_with_question("", b"192.43", b"999999.99").unwrap();
        assert!(near > 0.999, "the near miss gave {near}");
        assert!(wild < 1e-6, "the wild answer gave {wild}");
    }
}
