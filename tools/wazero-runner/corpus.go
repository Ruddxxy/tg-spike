package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
)

// EvalRow is one prepared corpus row. `corpus-eval prepare` writes
// these, after it extracts the single miner value from the full
// upstream response.
type EvalRow struct {
	RowID      int     `json:"row_id"`
	Question   string  `json:"question"`
	GtBare     string  `json:"gt_bare"`
	GtProse    string  `json:"gt_prose"`
	GtJSON     string  `json:"gt_json"`
	MinerValue string  `json:"miner_value"`
	MinerSlug  string  `json:"miner_slug"`
	Intent     string  `json:"intent"`
	ValidTime  string  `json:"valid_time"`
	ActualC    float64 `json:"actual_c"`
	MinerC     float64 `json:"miner_c"`
	ClusterID  string  `json:"cluster_id"`
}

// ScoredRow carries one row's six scores: two modules, three
// ground-truth renderings each.
type ScoredRow struct {
	RowID     int     `json:"row_id"`
	MinerSlug string  `json:"miner_slug"`
	Intent    string  `json:"intent"`
	ClusterID string  `json:"cluster_id"`
	ActualC   float64 `json:"actual_c"`
	MinerC    float64 `json:"miner_c"`

	OursBare  float64 `json:"ours_bare"`
	OursProse float64 `json:"ours_prose"`
	OursJSON  float64 `json:"ours_json"`

	RefBare  float64 `json:"ref_bare"`
	RefProse float64 `json:"ref_prose"`
	RefJSON  float64 `json:"ref_json"`
}

// scoreCorpus scores every prepared row with two modules against three
// ground-truth renderings, and writes one result line per row.
//
// Both modules load one time and stay loaded. A fresh runtime per row
// would be correct too, but 6000 rows times 6 calls is 36000 calls, and
// a fresh runtime for each one takes far longer than the run needs. The
// repeat-stability check in the host runner already proves that a reused
// instance gives the same bits as a fresh one.
func scoreCorpus(ctx context.Context, oursPath, refPath, inPath, outPath string) error {
	oursBytes, err := os.ReadFile(oursPath)
	if err != nil {
		return fmt.Errorf("cannot read our module: %w", err)
	}
	refBytes, err := os.ReadFile(refPath)
	if err != nil {
		return fmt.Errorf("cannot read the reference module: %w", err)
	}

	ours, err := NewHost(ctx, oursBytes)
	if err != nil {
		return fmt.Errorf("cannot load our module: %w", err)
	}
	defer ours.Close(ctx)

	reference, err := NewHost(ctx, refBytes)
	if err != nil {
		return fmt.Errorf("cannot load the reference module: %w", err)
	}
	defer reference.Close(ctx)

	input, err := os.Open(inPath)
	if err != nil {
		return fmt.Errorf("cannot open the prepared rows: %w", err)
	}
	defer input.Close()

	output, err := os.Create(outPath)
	if err != nil {
		return fmt.Errorf("cannot create the score file: %w", err)
	}
	defer output.Close()

	writer := bufio.NewWriter(output)
	defer writer.Flush()

	scanner := bufio.NewScanner(input)
	// A prepared row is small, but a JSON ground truth can be long, so
	// give the scanner room.
	scanner.Buffer(make([]byte, 0, 1024*1024), 8*1024*1024)

	count := 0
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var row EvalRow
		if err := json.Unmarshal(line, &row); err != nil {
			return fmt.Errorf("cannot read a prepared row: %w", err)
		}

		scored := ScoredRow{
			RowID:     row.RowID,
			MinerSlug: row.MinerSlug,
			Intent:    row.Intent,
			ClusterID: row.ClusterID,
			ActualC:   row.ActualC,
			MinerC:    row.MinerC,
		}

		question := []byte(row.Question)
		answer := []byte(row.MinerValue)

		renderings := []struct {
			text string
			ours *float64
			ref  *float64
		}{
			{row.GtBare, &scored.OursBare, &scored.RefBare},
			{row.GtProse, &scored.OursProse, &scored.RefProse},
			{row.GtJSON, &scored.OursJSON, &scored.RefJSON},
		}

		for _, rendering := range renderings {
			truth := []byte(rendering.text)

			oursScore, _, _, _, err := ours.Score(ctx, question, truth, answer)
			if err != nil {
				return fmt.Errorf("our module failed on row %d: %w", row.RowID, err)
			}
			*rendering.ours = float64(oursScore)

			refScore, _, _, _, err := reference.Score(ctx, question, truth, answer)
			if err != nil {
				return fmt.Errorf("the reference module failed on row %d: %w", row.RowID, err)
			}
			*rendering.ref = float64(refScore)
		}

		encoded, err := json.Marshal(scored)
		if err != nil {
			return fmt.Errorf("cannot write a score row: %w", err)
		}
		if _, err := writer.Write(encoded); err != nil {
			return err
		}
		if err := writer.WriteByte('\n'); err != nil {
			return err
		}
		count++
	}
	if err := scanner.Err(); err != nil {
		return fmt.Errorf("cannot read the prepared rows: %w", err)
	}

	fmt.Printf("scored %d rows, 3 renderings, 2 modules -> %s\n", count, outPath)
	return nil
}
