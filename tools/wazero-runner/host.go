package main

import (
	"context"
	"fmt"

	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"
)

// wasmPageSize is the number of bytes in one WebAssembly memory page.
const wasmPageSize = 65536

// Host wraps one wazero runtime and one module instance. Each Host holds
// the exported functions for the scoring ABI: alloc, dealloc, and
// rank_answer. A Host is not safe for reuse after Close.
type Host struct {
	runtime   wazero.Runtime
	module    api.Module
	memory    api.Memory
	allocFn   api.Function
	deallocFn api.Function
	rankFn    api.Function
}

// AllocOutcome names the exact result of one alloc-then-write attempt.
// The host uses this to tell apart two different failure kinds that both
// look like "it did not work" at first glance, but come from different
// causes: the module's own alloc cap, or the host's out of bounds check.
type AllocOutcome int

const (
	// AllocOutcomeZeroLength means the input is empty. The host does not
	// call alloc for an empty input. This matches the reference host.
	AllocOutcomeZeroLength AllocOutcome = iota
	// AllocOutcomeOK means alloc returns a usable pointer and the host
	// write into module memory succeeds.
	AllocOutcomeOK
	// AllocOutcomeCallFails means the alloc call itself traps or errors.
	// The host has no pointer to use in this case.
	AllocOutcomeCallFails
	// AllocOutcomeRefused means alloc returns 0 for a non-empty input.
	// This is the module's own size cap. It rejects the request before
	// it grows memory or hands back a usable pointer.
	AllocOutcomeRefused
	// AllocOutcomeWriteRefused means alloc returns a non-zero pointer,
	// but that pointer names memory that cannot hold the input bytes.
	// The host write is refused as out of bounds. The module handed
	// back a pointer that looks valid but is not.
	AllocOutcomeWriteRefused
)

// String renders one AllocOutcome as short, plain report text.
func (o AllocOutcome) String() string {
	switch o {
	case AllocOutcomeZeroLength:
		return "zero length, alloc not called"
	case AllocOutcomeOK:
		return "ok"
	case AllocOutcomeCallFails:
		return "alloc call fails"
	case AllocOutcomeRefused:
		return "alloc refused (returned 0)"
	case AllocOutcomeWriteRefused:
		return "alloc granted but host write refused (out of bounds)"
	default:
		return "unknown outcome"
	}
}

// AllocDetail records what happens when the host writes one string into
// module memory. It holds the returned pointer, the write result, and the
// memory size before and after the alloc call. Callers use this detail to
// show exact host behaviour in reports.
type AllocDetail struct {
	// Ptr is the pointer that alloc returns. It is 0 for a zero length
	// write, because alloc is not called in that case. It is also 0
	// when alloc refuses the request.
	Ptr uint32
	// PagesBefore is the module memory size in pages before alloc runs.
	PagesBefore uint32
	// PagesAfter is the module memory size in pages after alloc runs.
	PagesAfter uint32
	// WriteOK is true when the host write into module memory succeeds.
	// It is false when the write is refused as out of bounds, and it is
	// also false for a refused alloc (AllocOutcomeRefused), because the
	// host does not attempt a write at address 0.
	WriteOK bool
	// AllocErr holds the error text from a failed alloc call, or the
	// empty string when alloc succeeds or alloc is not called.
	AllocErr string
	// Outcome names which of the four alloc-then-write results this
	// detail record holds.
	Outcome AllocOutcome
}

// Text renders this AllocDetail as one report line: the outcome name,
// with the pointer folded in when the pointer is the interesting part
// of the story (a refused write from a pointer that looked valid).
func (d AllocDetail) Text() string {
	if d.Outcome == AllocOutcomeWriteRefused {
		return fmt.Sprintf("alloc granted ptr=%d but host write refused (out of bounds)", d.Ptr)
	}
	if d.Outcome == AllocOutcomeCallFails {
		return fmt.Sprintf("alloc call fails: %s", d.AllocErr)
	}
	return d.Outcome.String()
}

// NewHost loads one wasm module and checks it exports the scoring ABI.
// It returns a clean error, not a panic, when the module fails to load
// or a required export is missing.
func NewHost(ctx context.Context, wasmBytes []byte) (*Host, error) {
	runtime := wazero.NewRuntime(ctx)

	module, err := runtime.Instantiate(ctx, wasmBytes)
	if err != nil {
		closeErr := runtime.Close(ctx)
		if closeErr != nil {
			return nil, fmt.Errorf("module fails to load: %w (runtime close also fails: %v)", err, closeErr)
		}
		return nil, fmt.Errorf("module fails to load: %w", err)
	}

	memory := module.Memory()
	if memory == nil {
		closeHostParts(ctx, module, runtime)
		return nil, fmt.Errorf("module exports no linear memory")
	}

	allocFn := module.ExportedFunction("alloc")
	deallocFn := module.ExportedFunction("dealloc")
	rankFn := module.ExportedFunction("rank_answer")

	if allocFn == nil {
		closeHostParts(ctx, module, runtime)
		return nil, fmt.Errorf("module has no alloc export")
	}
	if rankFn == nil {
		closeHostParts(ctx, module, runtime)
		return nil, fmt.Errorf("module has no rank_answer export")
	}
	// dealloc is part of the ABI, but the call path in this tool never
	// calls it, so a missing dealloc export does not block use.

	return &Host{
		runtime:   runtime,
		module:    module,
		memory:    memory,
		allocFn:   allocFn,
		deallocFn: deallocFn,
		rankFn:    rankFn,
	}, nil
}

// closeHostParts closes a module and a runtime. It is a helper for the
// early return paths in NewHost, where a Host value does not yet exist.
func closeHostParts(ctx context.Context, module api.Module, runtime wazero.Runtime) {
	_ = module.Close(ctx)
	_ = runtime.Close(ctx)
}

// Close releases the module and the runtime behind this Host. Call Close
// once, after all use of this Host ends.
func (h *Host) Close(ctx context.Context) {
	if h == nil {
		return
	}
	_ = h.module.Close(ctx)
	_ = h.runtime.Close(ctx)
}

// PagesNow returns the current module memory size in pages.
func (h *Host) PagesNow() uint32 {
	return h.memory.Size() / wasmPageSize
}

// WriteBytes copies data into module memory through alloc and a host
// side write. A zero length input returns ptr 0, length 0, and does not
// call alloc, matching the reference host convention.
//
// WriteBytes never panics. When alloc itself traps or errors, there is
// no pointer to use, and WriteBytes returns a non-nil error -- the only
// case where the caller cannot go on to call rank_answer.
//
// When alloc returns 0 for a non-empty input, WriteBytes does NOT try
// the host memory write, and does NOT return an error. Address 0 is not
// a free block; it sits inside the module's own data segment. A write
// there would scribble over the module's own data, for a payload small
// enough to fit, and that would corrupt the very module under test
// instead of testing it. So this function skips the write, records
// WriteOK false, and hands back ptr 0, length equal to the input
// length, and the AllocOutcomeRefused outcome. The caller still goes on
// to call rank_answer with this (0, length) pair, exactly as a naive
// host would that does not check the alloc return value before it
// uses it.
//
// When the host write is refused as out of bounds -- alloc granted a
// real, non-zero pointer, but the block does not fit in module memory
// -- WriteBytes also does NOT return an error. A naive host does not
// check the write result before it calls rank_answer, so this function
// hands back the pointer the module gave it and lets the caller proceed
// exactly as a naive host would. The detail record still tells the two
// failure kinds apart, so a report can show the true cause.
func (h *Host) WriteBytes(ctx context.Context, data []byte) (ptr uint32, length uint32, detail AllocDetail, err error) {
	if len(data) == 0 {
		return 0, 0, AllocDetail{Outcome: AllocOutcomeZeroLength}, nil
	}

	detail.PagesBefore = h.PagesNow()

	res, callErr := h.allocFn.Call(ctx, uint64(len(data)))
	if callErr != nil {
		detail.AllocErr = callErr.Error()
		detail.Outcome = AllocOutcomeCallFails
		detail.PagesAfter = h.PagesNow()
		return 0, uint32(len(data)), detail, fmt.Errorf("alloc call fails: %w", callErr)
	}

	p := uint32(res[0])
	detail.Ptr = p
	detail.PagesAfter = h.PagesNow()

	if p == 0 {
		// The module's own size cap refuses this request. This host does
		// NOT try the write. Address 0 sits inside the module's own data
		// segment. A payload small enough to fit there would land on
		// top of real module data, and that write would corrupt the
		// very module under test, not exercise a "naive host" edge
		// case. So the write is skipped, WriteOK stays false, and the
		// outcome is recorded as refused.
		detail.WriteOK = false
		detail.Outcome = AllocOutcomeRefused
		return p, uint32(len(data)), detail, nil
	}

	writeOK := h.memory.Write(p, data)
	detail.WriteOK = writeOK
	if !writeOK {
		detail.Outcome = AllocOutcomeWriteRefused
		return p, uint32(len(data)), detail, nil
	}

	detail.Outcome = AllocOutcomeOK
	return p, uint32(len(data)), detail, nil
}

// WriteResult bundles the pointer, length, and diagnostic detail for one
// string write. Callers that need to report per-field detail (question,
// ground truth, miner answer) use this to keep the three writes apart.
type WriteResult struct {
	Ptr    uint32
	Len    uint32
	Detail AllocDetail
	Err    error
}

// WriteField writes one input field (question, ground truth, or miner
// answer) into module memory and returns a WriteResult. A write error is
// carried in the result, not raised as a panic.
func (h *Host) WriteField(ctx context.Context, data []byte) WriteResult {
	ptr, length, detail, err := h.WriteBytes(ctx, data)
	return WriteResult{Ptr: ptr, Len: length, Detail: detail, Err: err}
}

// RankAnswer calls rank_answer with three already written fields and
// returns the raw f32 score. It never panics; a trap or a call error
// comes back as a Go error.
func (h *Host) RankAnswer(ctx context.Context, q, gt, ma WriteResult) (float32, error) {
	res, err := h.rankFn.Call(ctx,
		uint64(q.Ptr), uint64(q.Len),
		uint64(gt.Ptr), uint64(gt.Len),
		uint64(ma.Ptr), uint64(ma.Len),
	)
	if err != nil {
		return 0, fmt.Errorf("rank_answer call fails: %w", err)
	}
	return api.DecodeF32(res[0]), nil
}

// Score writes question, ground truth, and miner answer into module
// memory, then calls rank_answer. It returns the score and the three
// WriteResult values, so a caller can inspect alloc detail even when the
// final call fails.
//
// Score calls rank_answer even after a refused alloc or a refused host
// write, exactly as a naive host would, because that is the real
// question a differential test asks: what does rank_answer do with a
// (ptr, len) pair that does not name real written bytes -- the (0, len)
// pair from a refused alloc, or a granted pointer whose write never
// landed? A write failure only stops Score early when alloc itself
// traps, because then there is no pointer at all to pass on.
func (h *Host) Score(ctx context.Context, question, groundTruth, minerAnswer []byte) (score float32, q, gt, ma WriteResult, err error) {
	q = h.WriteField(ctx, question)
	if q.Err != nil {
		return 0, q, gt, ma, fmt.Errorf("question write fails: %w", q.Err)
	}
	gt = h.WriteField(ctx, groundTruth)
	if gt.Err != nil {
		return 0, q, gt, ma, fmt.Errorf("ground truth write fails: %w", gt.Err)
	}
	ma = h.WriteField(ctx, minerAnswer)
	if ma.Err != nil {
		return 0, q, gt, ma, fmt.Errorf("miner answer write fails: %w", ma.Err)
	}

	score, err = h.RankAnswer(ctx, q, gt, ma)
	if err != nil {
		return 0, q, gt, ma, err
	}
	return score, q, gt, ma, nil
}
