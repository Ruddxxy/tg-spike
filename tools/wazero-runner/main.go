// Command wazero-runner is a host driver for the Telegraph Track 2
// scoring module ABI, built on the pure Go wazero runtime.
//
// The production Telegraph node host runs wasm scoring modules through
// wazero, not wasmtime. This tool matches that host exactly: it allocs
// each string field on its own, writes the bytes at the returned
// pointer, then calls rank_answer with three (ptr, len) pairs.
//
// This tool has three modes, chosen by flag:
//
//   - Compare mode (the default): score one input triple through two
//     wasm modules and print both scores side by side, with the raw f32
//     bit pattern of each.
//   - Matrix mode (-matrix): run a built in set of edge case inputs
//     through two wasm modules and print a differential table.
//   - Golden mode (-golden): score every vector in a golden vector file
//     through one wasm module and write a JSON file with the f32 bit
//     pattern of each result, for cross host bit equality checks.
//
// See README.md in this directory for exact command lines.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
)

// main parses the command line flags, picks a mode, and runs it. Every
// error path prints a clean message to stderr and exits with status 1.
// main does not panic in normal control flow.
func main() {
	pathA := flag.String("a", "", "path to wasm module A (or 'ours' in matrix mode)")
	pathB := flag.String("b", "", "path to wasm module B (or 'reference' in matrix mode)")
	question := flag.String("q", "", "question text (compare mode)")
	groundTruth := flag.String("gt", "", "ground truth text (compare mode)")
	minerAnswer := flag.String("ma", "", "miner answer text (compare mode)")
	matrixMode := flag.Bool("matrix", false, "run the built in differential matrix")
	goldenPath := flag.String("golden", "", "path to a golden vector JSON file, turns on golden mode")
	outPath := flag.String("out", "", "output path for golden mode or corpus mode result")
	corpusPath := flag.String("corpus", "", "path to prepared corpus rows, turns on corpus mode")
	flag.Parse()

	ctx := context.Background()

	switch {
	case *corpusPath != "":
		runCorpusMode(ctx, *pathA, *pathB, *corpusPath, *outPath)
	case *goldenPath != "":
		runGoldenMode(ctx, *goldenPath, *pathA, *outPath)
	case *matrixMode:
		runMatrixMode(ctx, *pathA, *pathB)
	default:
		runCompareMode(ctx, *pathA, *pathB, *question, *groundTruth, *minerAnswer)
	}
}

// runCorpusMode checks corpus mode flags, then scores the corpus with
// both modules. It exits with status 1 and a clean message on any flag
// or run error.
func runCorpusMode(ctx context.Context, oursPath, refPath, corpusPath, outPath string) {
	if oursPath == "" {
		fail("corpus mode needs -a <path to our wasm module>")
	}
	if refPath == "" {
		fail("corpus mode needs -b <path to the reference wasm module>")
	}
	if outPath == "" {
		fail("corpus mode needs -out <path to write jsonl>")
	}
	if err := scoreCorpus(ctx, oursPath, refPath, corpusPath, outPath); err != nil {
		fail(err.Error())
	}
}

// runGoldenMode checks golden mode flags, then runs golden mode. It
// exits with status 1 and a clean message on any flag or run error.
func runGoldenMode(ctx context.Context, goldenPath, wasmPath, outPath string) {
	if wasmPath == "" {
		fail("golden mode needs -a <path to wasm file>")
	}
	if outPath == "" {
		fail("golden mode needs -out <path to write json>")
	}
	if err := runGolden(ctx, goldenPath, wasmPath, outPath); err != nil {
		fail(err.Error())
	}
}

// runMatrixMode checks matrix mode flags, then runs matrix mode. Flag
// -a names our module, flag -b names the reference module, matching the
// matrix table column order.
func runMatrixMode(ctx context.Context, pathOurs, pathReference string) {
	if pathOurs == "" || pathReference == "" {
		fail("matrix mode needs -a <ours.wasm> and -b <reference.wasm>")
	}
	runMatrix(ctx, pathOurs, pathReference)
}

// runCompareMode checks compare mode flags, then runs compare mode.
func runCompareMode(ctx context.Context, pathA, pathB, question, groundTruth, minerAnswer string) {
	if pathA == "" || pathB == "" {
		fail("compare mode needs -a <path-to-wasm> and -b <path-to-wasm>")
	}
	runCompare(ctx, pathA, pathB, []byte(question), []byte(groundTruth), []byte(minerAnswer))
}

// fail prints one clean error message to stderr and exits with status
// 1. It is the only exit path for a usage or run error.
func fail(message string) {
	fmt.Fprintln(os.Stderr, "error: "+message)
	os.Exit(1)
}
