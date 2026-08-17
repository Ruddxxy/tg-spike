package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"text/tabwriter"
	"time"
)

// TimingResult is one measured vector. Micros is the cost of the
// rank_answer call alone, in microseconds, over the fastest repeat.
//
// The measurement excludes module instantiation and excludes the host
// memory writes that put the three inputs in place. Those are host
// costs and they are the same for every module. What is left is the
// work the module itself does, which is the thing under test.
type TimingResult struct {
	Name   string  `json:"name"`
	Bytes  int     `json:"bytes"`
	Micros int64   `json:"micros"`
	Value  float64 `json:"value"`
}

// TimingOutput is the top level JSON document written by timing mode.
type TimingOutput struct {
	Runner     string         `json:"runner"`
	WasmPath   string         `json:"wasm_path"`
	WasmSHA256 string         `json:"wasm_sha256"`
	Repeats    int            `json:"repeats"`
	Vectors    []TimingResult `json:"vectors"`
}

// runTiming times rank_answer for every vector in the given file.
//
// Each repeat gets a FRESH module instance, because that is how a
// validator scores: load, score, drop. A reused instance would carry
// the memory the previous call allocated and would measure a state no
// validator is ever in.
//
// The result keeps the FASTEST repeat. A slow repeat holds the cost of
// something else on the machine. A fast repeat cannot hold less work
// than the call really does.
func runTiming(ctx context.Context, vectorsPath, wasmPath, outPath string, repeats int) error {
	vectorBytes, err := os.ReadFile(vectorsPath)
	if err != nil {
		return fmt.Errorf("cannot read timing vector file %q: %w", vectorsPath, err)
	}

	var file GoldenVectorFile
	if err := json.Unmarshal(vectorBytes, &file); err != nil {
		return fmt.Errorf("cannot parse timing vector file %q: %w", vectorsPath, err)
	}

	wasmBytes, err := os.ReadFile(wasmPath)
	if err != nil {
		return fmt.Errorf("cannot read wasm file %q: %w", wasmPath, err)
	}
	sum := sha256.Sum256(wasmBytes)

	results := make([]TimingResult, 0, len(file.Vectors))

	w := tabwriter.NewWriter(os.Stdout, 2, 4, 2, ' ', 0)
	fmt.Fprintln(w, "name\tbytes\tmicros\tvalue")

	for _, vector := range file.Vectors {
		best := time.Duration(0)
		var score float32
		for repeat := 0; repeat < repeats; repeat++ {
			host, err := NewHost(ctx, wasmBytes)
			if err != nil {
				return fmt.Errorf("vector %q: module load fails: %w", vector.Name, err)
			}

			q := host.WriteField(ctx, []byte(vector.Question))
			gt := host.WriteField(ctx, []byte(vector.GroundTruth))
			ma := host.WriteField(ctx, []byte(vector.MinerAnswer))
			if q.Err != nil || gt.Err != nil || ma.Err != nil {
				host.Close(ctx)
				return fmt.Errorf("vector %q: input write fails", vector.Name)
			}

			started := time.Now()
			score, err = host.RankAnswer(ctx, q, gt, ma)
			elapsed := time.Since(started)
			host.Close(ctx)
			if err != nil {
				return fmt.Errorf("vector %q: rank_answer fails: %w", vector.Name, err)
			}
			if best == 0 || elapsed < best {
				best = elapsed
			}
		}

		result := TimingResult{
			Name:   vector.Name,
			Bytes:  len(vector.MinerAnswer),
			Micros: best.Microseconds(),
			Value:  float64(score),
		}
		results = append(results, result)
		fmt.Fprintf(w, "%s\t%d\t%d\t%v\n", result.Name, result.Bytes, result.Micros, result.Value)
	}
	w.Flush()

	output := TimingOutput{
		Runner:     "wazero",
		WasmPath:   wasmPath,
		WasmSHA256: hex.EncodeToString(sum[:]),
		Repeats:    repeats,
		Vectors:    results,
	}

	outBytes, err := json.MarshalIndent(output, "", "  ")
	if err != nil {
		return fmt.Errorf("cannot encode timing output: %w", err)
	}
	outBytes = append(outBytes, '\n')

	if err := os.WriteFile(outPath, outBytes, 0o644); err != nil {
		return fmt.Errorf("cannot write timing output file %q: %w", outPath, err)
	}

	fmt.Printf("\ntimed %d vectors, %d repeats each, into %s\n", len(results), repeats, outPath)
	return nil
}
