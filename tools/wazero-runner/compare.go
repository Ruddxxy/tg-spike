package main

import (
	"context"
	"fmt"
	"math"
	"os"
	"text/tabwriter"
)

// ScoreOutcome holds the result of one scoring attempt through one wasm
// module. Valid is false when the module fails to load, or the score
// call fails, or the module traps. In that case ErrText holds the
// failure text and Score is not meaningful.
type ScoreOutcome struct {
	// Label names this side, for example the wasm file path.
	Label string
	// Valid is true when Score holds a real result.
	Valid bool
	// Score is the f32 score. It is meaningful only when Valid is true.
	Score float32
	// ErrText is the failure text. It is empty when Valid is true.
	ErrText string
}

// scoreOneModule loads one wasm module fresh, scores one triple, and
// closes the module. It never panics: every failure path returns a
// ScoreOutcome with Valid false and a clean error message.
func scoreOneModule(ctx context.Context, label, wasmPath string, question, groundTruth, minerAnswer []byte) ScoreOutcome {
	wasmBytes, err := os.ReadFile(wasmPath)
	if err != nil {
		return ScoreOutcome{Label: label, ErrText: fmt.Sprintf("cannot read wasm file: %v", err)}
	}

	host, err := NewHost(ctx, wasmBytes)
	if err != nil {
		return ScoreOutcome{Label: label, ErrText: err.Error()}
	}
	defer host.Close(ctx)

	score, _, _, _, err := host.Score(ctx, question, groundTruth, minerAnswer)
	if err != nil {
		return ScoreOutcome{Label: label, ErrText: err.Error()}
	}

	return ScoreOutcome{Label: label, Valid: true, Score: score}
}

// runCompare loads module A and module B, scores the same input triple
// through both, and prints a side by side table with the raw f32 bit
// pattern of each score. When one module fails, its row shows the
// failure text and the other module still shows its score.
func runCompare(ctx context.Context, pathA, pathB string, question, groundTruth, minerAnswer []byte) {
	outA := scoreOneModule(ctx, "a", pathA, question, groundTruth, minerAnswer)
	outB := scoreOneModule(ctx, "b", pathB, question, groundTruth, minerAnswer)

	w := tabwriter.NewWriter(os.Stdout, 2, 4, 2, ' ', 0)
	fmt.Fprintln(w, "module\tpath\tscore\tbits (hex)")
	printCompareRow(w, "a", pathA, outA)
	printCompareRow(w, "b", pathB, outB)
	w.Flush()

	if outA.Valid && outB.Valid {
		delta := outA.Score - outB.Score
		fmt.Printf("\ndelta (a - b): %.6f\n", delta)
	} else {
		fmt.Println("\ndelta: not available, one or both modules fail to score")
	}
}

// printCompareRow writes one row of the compare table for one module.
func printCompareRow(w *tabwriter.Writer, side, path string, outcome ScoreOutcome) {
	if outcome.Valid {
		bits := math.Float32bits(outcome.Score)
		fmt.Fprintf(w, "%s\t%s\t%.4f\t0x%08x\n", side, path, outcome.Score, bits)
		return
	}
	fmt.Fprintf(w, "%s\t%s\tFAIL: %s\t-\n", side, path, outcome.ErrText)
}
