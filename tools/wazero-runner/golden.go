package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"text/tabwriter"
)

// GoldenVectorFile is the shape of the golden_vectors.json input file.
type GoldenVectorFile struct {
	Vectors []GoldenVector `json:"vectors"`
}

// GoldenVector is one golden vector case. GroundTruth and Response feed
// rank_answer as the ground truth field and the miner answer field.
type GoldenVector struct {
	Name        string `json:"name"`
	GroundTruth string `json:"ground_truth"`
	Response    string `json:"response"`
}

// GoldenResult is one output entry. bits_hex holds the IEEE-754 f32 bit
// pattern of the score, as an 8 digit lowercase hex string with a 0x
// prefix. This shape is exact and a Rust host runner reads it to check
// bit equality against its own wasmtime results.
type GoldenResult struct {
	Name    string  `json:"name"`
	BitsHex string  `json:"bits_hex"`
	Value   float64 `json:"value"`
}

// GoldenOutput is the top level JSON document written by golden mode.
type GoldenOutput struct {
	Runner     string         `json:"runner"`
	WasmPath   string         `json:"wasm_path"`
	WasmSHA256 string         `json:"wasm_sha256"`
	Vectors    []GoldenResult `json:"vectors"`
}

// runGolden reads the golden vector file, scores every vector through
// the module at wasmPath, and writes the result JSON to outPath. Golden
// mode stops on the first error, because bit equality evidence with a
// gap in it is not evidence. The error message names the failing
// vector, so the cause is clear.
func runGolden(ctx context.Context, vectorsPath, wasmPath, outPath string) error {
	vectorBytes, err := os.ReadFile(vectorsPath)
	if err != nil {
		return fmt.Errorf("cannot read golden vector file %q: %w", vectorsPath, err)
	}

	var file GoldenVectorFile
	if err := json.Unmarshal(vectorBytes, &file); err != nil {
		return fmt.Errorf("cannot parse golden vector file %q: %w", vectorsPath, err)
	}

	wasmBytes, err := os.ReadFile(wasmPath)
	if err != nil {
		return fmt.Errorf("cannot read wasm file %q: %w", wasmPath, err)
	}
	sum := sha256.Sum256(wasmBytes)
	wasmSHA256 := hex.EncodeToString(sum[:])

	results := make([]GoldenResult, 0, len(file.Vectors))

	w := tabwriter.NewWriter(os.Stdout, 2, 4, 2, ' ', 0)
	fmt.Fprintln(w, "name\tbits_hex\tvalue")

	for _, vector := range file.Vectors {
		host, err := NewHost(ctx, wasmBytes)
		if err != nil {
			return fmt.Errorf("vector %q: module load fails: %w", vector.Name, err)
		}

		score, _, _, _, err := host.Score(ctx, []byte(""), []byte(vector.GroundTruth), []byte(vector.Response))
		host.Close(ctx)
		if err != nil {
			return fmt.Errorf("vector %q: rank_answer fails: %w", vector.Name, err)
		}

		bits := math.Float32bits(score)
		result := GoldenResult{
			Name:    vector.Name,
			BitsHex: fmt.Sprintf("0x%08x", bits),
			Value:   float64(score),
		}
		results = append(results, result)
		fmt.Fprintf(w, "%s\t%s\t%v\n", result.Name, result.BitsHex, result.Value)
	}
	w.Flush()

	output := GoldenOutput{
		Runner:     "wazero",
		WasmPath:   wasmPath,
		WasmSHA256: wasmSHA256,
		Vectors:    results,
	}

	outBytes, err := json.MarshalIndent(output, "", "  ")
	if err != nil {
		return fmt.Errorf("cannot encode golden output: %w", err)
	}
	outBytes = append(outBytes, '\n')

	if err := os.WriteFile(outPath, outBytes, 0o644); err != nil {
		return fmt.Errorf("cannot write golden output file %q: %w", outPath, err)
	}

	fmt.Printf("\nwrote %d vectors to %s\n", len(results), outPath)
	return nil
}
