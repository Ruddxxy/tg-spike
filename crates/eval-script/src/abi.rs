//! This module holds the functions the WASM host calls.
//!
//! The host calls `alloc` first, to get a block of memory inside
//! the module. The host writes its input bytes into that block. The
//! host then calls `score`, `score_log_loss`, or `score_batch` with
//! the offset and the length of the block. The host calls `dealloc`
//! when it no longer needs the block.
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
//! paths, which return before any raw memory read happens.

use crate::error::ScoreError;
use crate::metrics;
use crate::MAX_INPUT_BYTES;
use std::alloc::{alloc as std_alloc, dealloc as std_dealloc, Layout};

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
#[no_mangle]
pub extern "C" fn alloc(len: u32) -> u32 {
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
    let ptr = unsafe { std_alloc(layout) };
    if ptr.is_null() {
        return 0;
    }
    ptr as u32
}

/// This function frees a block of memory that `alloc` made.
///
/// The parameter `ptr` is the offset that `alloc` returned. The
/// parameter `len` is the same `len` value the caller gave to
/// `alloc`. The function does nothing if `ptr` is 0. The function
/// does nothing if `len` does not fit a valid memory layout.
///
/// The caller must give back the same `ptr` and `len` pair that
/// `alloc` gave out. A wrong pair can corrupt the allocator. This is
/// the normal contract for a manual allocate-and-free pair of
/// functions.
#[no_mangle]
pub extern "C" fn dealloc(ptr: u32, len: u32) {
    if ptr == 0 {
        return;
    }
    let layout = match block_layout(len) {
        Some(layout) => layout,
        None => return,
    };
    // SECURITY: this call trusts the host to give back the exact
    // `ptr` and `len` pair that `alloc` gave out for this block. A
    // host that breaks that rule can corrupt the allocator. That
    // risk is inherent to a manual alloc and free ABI.
    unsafe { std_dealloc(ptr as *mut u8, layout) };
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

/// This function runs a two-input score function over two raw
/// memory blocks and turns any error into the worst score, 0.0.
fn run_pair_score(
    gt_ptr: u32,
    gt_len: u32,
    resp_ptr: u32,
    resp_len: u32,
    scorer: fn(&[u8], &[u8]) -> Result<f64, ScoreError>,
) -> f64 {
    let outcome = read_bytes(gt_ptr, gt_len).and_then(|gt_bytes| {
        let resp_bytes = read_bytes(resp_ptr, resp_len)?;
        scorer(gt_bytes, resp_bytes)
    });
    finish(outcome)
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
fn finish(outcome: Result<f64, ScoreError>) -> f64 {
    match outcome {
        Ok(value) if value.is_finite() && (0.0..=1.0).contains(&value) => value,
        _ => 0.0,
    }
}

/// This function calculates the Brier score for one ground truth
/// and response pair.
///
/// A high score is good. A low score is bad. The range is 0.0 to
/// 1.0. The function reads the ground truth JSON bytes from
/// `gt_ptr` and `gt_len`, and the response JSON bytes from
/// `resp_ptr` and `resp_len`. The function returns 0.0 if any input
/// is not correct, or if any input is bigger than `MAX_INPUT_BYTES`.
#[no_mangle]
pub extern "C" fn score(gt_ptr: u32, gt_len: u32, resp_ptr: u32, resp_len: u32) -> f64 {
    run_pair_score(
        gt_ptr,
        gt_len,
        resp_ptr,
        resp_len,
        metrics::brier_from_bytes,
    )
}

/// This function calculates the log loss score for one ground truth
/// and response pair.
///
/// A high score is good. A low score is bad. The range is 0.0 to
/// 1.0. See the `metrics` module for the normalization this function
/// runs on the raw log loss value. The function returns 0.0 if any
/// input is not correct, or if any input is bigger than
/// `MAX_INPUT_BYTES`.
#[no_mangle]
pub extern "C" fn score_log_loss(gt_ptr: u32, gt_len: u32, resp_ptr: u32, resp_len: u32) -> f64 {
    run_pair_score(
        gt_ptr,
        gt_len,
        resp_ptr,
        resp_len,
        metrics::log_loss_from_bytes,
    )
}

/// This function calculates the mean Brier score for a batch of
/// pairs.
///
/// The parameter `ptr` and `len` name a block that holds a JSON
/// array. See the `metrics` module for the array shape and for the
/// sort-then-Kahan-sum method this function uses to stay order
/// independent. The function returns 0.0 if the input is not
/// correct, or if the input is bigger than `MAX_INPUT_BYTES`.
#[no_mangle]
pub extern "C" fn score_batch(ptr: u32, len: u32) -> f64 {
    let outcome = read_bytes(ptr, len).and_then(metrics::batch_brier_from_bytes);
    finish(outcome)
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
        // This test does not call `dealloc`. Freeing a pointer that
        // `alloc` truncated to 32 bits would corrupt the native
        // heap. This ABI is only sound on a real 32-bit address
        // space, which is what the wasm32 target gives it. The leak
        // here is a few bytes for the life of the test process.
        let ptr = alloc(16);
        assert_ne!(ptr, 0);
    }

    #[test]
    fn alloc_of_zero_len_does_not_fail() {
        assert_ne!(alloc(0), 0);
    }

    #[test]
    fn dealloc_of_null_pointer_does_nothing() {
        dealloc(0, 16);
    }

    #[test]
    fn dealloc_of_null_pointer_does_nothing_for_several_lengths() {
        // A null pointer must stay a safe no-op for any length,
        // including a length of 0 and a length over the cap.
        for len in [0, 1, 16, MAX_INPUT_BYTES, MAX_INPUT_BYTES + 1, u32::MAX] {
            dealloc(0, len);
        }
    }

    #[test]
    fn alloc_at_the_cap_succeeds() {
        // This test does not call `dealloc`. See
        // `alloc_returns_nonzero_and_leaks_on_purpose` above for why
        // a native test never frees an `alloc` result.
        let ptr = alloc(MAX_INPUT_BYTES);
        assert_ne!(ptr, 0);
    }

    #[test]
    fn alloc_one_byte_over_the_cap_returns_zero() {
        assert_eq!(alloc(MAX_INPUT_BYTES + 1), 0);
    }

    #[test]
    fn alloc_of_u32_max_returns_zero_and_does_not_trap() {
        // `u32::MAX` is over the cap, so the cap check must reject
        // this value before `block_layout` or `std_alloc` ever runs.
        // A build that reached `std_alloc` with this size would ask
        // the allocator to grow the module memory past what a
        // 32-bit address space can hold.
        assert_eq!(alloc(u32::MAX), 0);
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
    fn score_returns_worst_score_on_pointer_overflow() {
        // gt_ptr + gt_len overflows u32. This never reaches a real
        // memory read.
        assert_eq!(score(u32::MAX, u32::MAX, 0, 0), 0.0);
    }

    #[test]
    fn score_returns_worst_score_on_null_pointer_with_length() {
        assert_eq!(score(0, 5, 0, 0), 0.0);
    }

    #[test]
    fn score_returns_worst_score_on_empty_input() {
        // Both sides are zero length. `read_bytes` returns an empty
        // slice without dereferencing anything. Parsing an empty
        // JSON document then fails.
        assert_eq!(score(0, 0, 0, 0), 0.0);
        assert_eq!(score_log_loss(0, 0, 0, 0), 0.0);
        assert_eq!(score_batch(0, 0), 0.0);
    }

    #[test]
    fn score_returns_worst_score_on_an_oversize_length_without_a_trap() {
        // gt_len is one byte over the cap. gt_ptr is a wild address
        // that is never a valid block. If the cap check did not run
        // before the bounds check and the memory read, this call
        // would need to touch that wild pointer. The cap check
        // catches it first, so this call stays safe to run on a
        // native target and returns the worst score instead of
        // trapping.
        assert_eq!(score(0xdead_beef, MAX_INPUT_BYTES + 1, 0, 0), 0.0);
        assert_eq!(score_batch(0xdead_beef, MAX_INPUT_BYTES + 1), 0.0);
    }

    #[test]
    fn alloc_rejection_flows_end_to_end_to_the_worst_score() {
        // This test proves the full failure path: an oversize `len`
        // makes `alloc` return the null pointer 0, and that same
        // null pointer with the same nonzero `len`, when passed on
        // to a score function, must produce the worst score, 0.0,
        // through `read_bytes` and then `finish`. This is the exact
        // sequence a host follows: call `alloc`, then call a score
        // function with the pointer `alloc` gave back.
        let len = MAX_INPUT_BYTES + 1;
        let ptr = alloc(len);
        assert_eq!(ptr, 0);
        assert_eq!(read_bytes(ptr, len), Err(ScoreError::InputTooLarge));
        assert_eq!(score(ptr, len, 0, 0), 0.0);
        assert_eq!(score_log_loss(ptr, len, 0, 0), 0.0);
        assert_eq!(score_batch(ptr, len), 0.0);
    }

    #[test]
    fn null_pointer_with_nonzero_length_flows_end_to_end_to_the_worst_score() {
        // This test covers the case where `len` sits inside the cap,
        // so `check_len_cap` passes and `read_bytes` must instead
        // catch the null pointer through `check_bounds` and the
        // explicit null check. This is the path `read_bytes_rejects_
        // null_pointer_with_nonzero_length` already proves for
        // `read_bytes` alone. This test proves the same failure
        // reaches `finish` and comes out as 0.0 from all three score
        // functions, not only `score`.
        assert_eq!(read_bytes(0, 5), Err(ScoreError::BadPointer));
        assert_eq!(score(0, 5, 0, 0), 0.0);
        assert_eq!(score_log_loss(0, 5, 0, 0), 0.0);
        assert_eq!(score_batch(0, 5), 0.0);
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
}
