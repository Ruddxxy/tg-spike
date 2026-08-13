package main

import (
	"bytes"
	"context"
	"fmt"
	"math"
	"os"
	"strings"
	"text/tabwriter"
)

// MatrixRow is one differential test case. Fields are raw byte slices,
// not Go strings, so a row can carry input that is not valid UTF-8.
type MatrixRow struct {
	// Name is a short label for the row, shown in the table.
	Name string
	// Question is the question field for rank_answer.
	Question []byte
	// GroundTruth is the ground truth field for rank_answer.
	GroundTruth []byte
	// MinerAnswer is the miner answer field for rank_answer.
	MinerAnswer []byte
}

// buildMatrixRows returns the built in differential input matrix, in a
// fixed order. Row 1 is the doc example. Rows 7 and 8 (index 6 and 7)
// carry the two edge cases that need extra detail in the report:
// invalid UTF-8 input, and a 2 MiB miner answer.
func buildMatrixRows() []MatrixRow {
	question := []byte("What is the capital of France?")
	groundTruth := []byte("Paris is the capital of France.")

	twoMiB := bytes.Repeat([]byte("Paris "), (2*1024*1024)/6+1)[:2*1024*1024]

	return []MatrixRow{
		{
			Name:        "doc_example",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte("The capital of France is Paris"),
		},
		{
			Name:        "empty_miner_answer",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte(""),
		},
		{
			Name:        "whitespace_only_miner_answer",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte("   \t\n  "),
		},
		{
			Name:        "miner_answer_is",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte("is"),
		},
		{
			Name:        "miner_answer_subset_of_ground_truth",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte("capital of France"),
		},
		{
			Name:        "compact_json_vs_prose_ground_truth",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte(`{"answer":"Paris"}`),
		},
		{
			Name:        "invalid_utf8_miner_answer",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: []byte{0xff, 0xfe, 0x80, 0x41},
		},
		{
			Name:        "miner_answer_2mib",
			Question:    question,
			GroundTruth: groundTruth,
			MinerAnswer: twoMiB,
		},
	}
}

// MatrixCell is the result of running one row through one module, with a
// fresh runtime and a fresh module instance. A cell that hits a trap or
// a panic still fills PanicText or ErrText, so the caller can print a
// clean row and move to the next case.
type MatrixCell struct {
	// Valid is true when Score and Bits hold a real rank_answer result.
	Valid bool
	// Score is the f32 score from rank_answer.
	Score float32
	// Bits is the IEEE-754 bit pattern of Score, valid only when Valid
	// is true.
	Bits uint32
	// ErrText is the failure text from module load or from the
	// rank_answer call itself (for example, a wasm trap).
	ErrText string
	// PanicText holds text from a recovered Go panic, if one happens.
	PanicText string
	// QDetail, GTDetail, and MADetail record the alloc-then-write
	// outcome for the question, ground truth, and miner answer fields.
	// rank_answer still runs on a refused alloc or a refused write, so
	// these details can show an anomaly even on a row where Valid is
	// true.
	QDetail  AllocDetail
	GTDetail AllocDetail
	MADetail AllocDetail
}

// FailText returns the best available failure text for this cell, or
// the empty string when the cell holds a valid score.
func (c MatrixCell) FailText() string {
	if c.Valid {
		return ""
	}
	if c.PanicText != "" {
		return c.PanicText
	}
	return c.ErrText
}

// runMatrixCell runs one row through one wasm module, using a fresh
// wazero runtime and a fresh module instance. A recover() guards this
// call: a trap, a rejected alloc, or a refused memory write in this row
// is recorded in the returned MatrixCell, and the process keeps running.
//
// This function calls rank_answer even when alloc is refused or the
// host write is refused, because that is the real question rows 7 and 8
// ask: what does the module do when the host proceeds after alloc hands
// back a pointer it should not trust? A trap from that call is caught
// here and recorded as ErrText, not raised.
func runMatrixCell(ctx context.Context, wasmBytes []byte, row MatrixRow) (cell MatrixCell) {
	defer func() {
		if r := recover(); r != nil {
			cell.Valid = false
			cell.PanicText = fmt.Sprintf("recovered panic: %v", r)
		}
	}()

	host, err := NewHost(ctx, wasmBytes)
	if err != nil {
		cell.ErrText = err.Error()
		return cell
	}
	defer host.Close(ctx)

	score, q, gt, ma, err := host.Score(ctx, row.Question, row.GroundTruth, row.MinerAnswer)
	cell.QDetail = q.Detail
	cell.GTDetail = gt.Detail
	cell.MADetail = ma.Detail
	if err != nil {
		cell.ErrText = err.Error()
		return cell
	}

	cell.Valid = true
	cell.Score = score
	cell.Bits = math.Float32bits(score)
	return cell
}

// runMatrix reads both wasm files once, then runs every row in
// buildMatrixRows against both modules. Each row and module pair gets a
// fresh runtime, so a bad row cannot corrupt state for the next row.
// pathOurs is printed as "our score" and pathReference as
// "reference score", matching the -a/-b convention for matrix mode.
func runMatrix(ctx context.Context, pathOurs, pathReference string) {
	oursBytes, err := os.ReadFile(pathOurs)
	if err != nil {
		fmt.Fprintf(os.Stderr, "cannot read -a wasm file %q: %v\n", pathOurs, err)
		os.Exit(1)
	}
	referenceBytes, err := os.ReadFile(pathReference)
	if err != nil {
		fmt.Fprintf(os.Stderr, "cannot read -b wasm file %q: %v\n", pathReference, err)
		os.Exit(1)
	}

	rows := buildMatrixRows()

	w := tabwriter.NewWriter(os.Stdout, 2, 4, 2, ' ', 0)
	fmt.Fprintln(w, "case\treference score (bits)\tour score (bits)\tdelta / notes")

	type savedDetail struct {
		index   int
		row     MatrixRow
		refCell MatrixCell
		ourCell MatrixCell
	}
	var detailRows []savedDetail

	for i, row := range rows {
		refCell := runMatrixCell(ctx, referenceBytes, row)
		ourCell := runMatrixCell(ctx, oursBytes, row)

		refText := cellScoreText(refCell)
		ourText := cellScoreText(ourCell)
		note := deltaOrNote(refCell, ourCell)

		fmt.Fprintf(w, "%d. %s\t%s\t%s\t%s\n", i+1, row.Name, refText, ourText, note)

		if row.Name == "invalid_utf8_miner_answer" || row.Name == "miner_answer_2mib" {
			detailRows = append(detailRows, savedDetail{index: i, row: row, refCell: refCell, ourCell: ourCell})
		}
	}
	w.Flush()

	for _, d := range detailRows {
		fmt.Printf("\n--- detail for row %d (%s) ---\n", d.index+1, d.row.Name)
		printCellDetail("reference", d.refCell)
		printCellDetail("ours", d.ourCell)
	}
}

// cellScoreText renders one cell as a short table cell string: a score
// with four decimal places and its f32 bit pattern in hex, or a failure
// marker with the failure text. A wasm trap message can span several
// lines (it carries a stack trace); oneLine folds it onto one line so
// the table stays aligned. The detail block below the table prints the
// same failure text in full, unfolded.
func cellScoreText(cell MatrixCell) string {
	if cell.Valid {
		return fmt.Sprintf("%.4f (0x%08x)", cell.Score, cell.Bits)
	}
	return "FAIL: " + oneLine(cell.FailText())
}

// oneLine collapses a multi-line string onto one line, joining trimmed,
// non-empty lines with " | ". A wazero trap message carries a stack
// trace with several lines; this keeps a table row on one line.
func oneLine(s string) string {
	rawLines := strings.Split(strings.ReplaceAll(s, "\r\n", "\n"), "\n")
	lines := make([]string, 0, len(rawLines))
	for _, line := range rawLines {
		trimmed := strings.TrimSpace(line)
		if trimmed != "" {
			lines = append(lines, trimmed)
		}
	}
	return strings.Join(lines, " | ")
}

// deltaOrNote renders the delta between two valid cells, or a short note
// when one or both cells fail. It also appends a note on the miner
// answer alloc outcome whenever that outcome is not a plain success, so
// a refused alloc and a refused write show up as distinct facts in the
// table, not as one merged failure.
func deltaOrNote(refCell, ourCell MatrixCell) string {
	base := ""
	switch {
	case refCell.Valid && ourCell.Valid:
		base = fmt.Sprintf("delta %.6f", ourCell.Score-refCell.Score)
	case !refCell.Valid && !ourCell.Valid:
		base = "both modules fail on this row"
	case !refCell.Valid:
		base = "reference module fails on this row"
	default:
		base = "our module fails on this row"
	}

	anomaly := maAllocAnomalyNote(refCell, ourCell)
	if anomaly == "" {
		return base
	}
	return base + "; " + anomaly
}

// maAllocAnomalyNote reports the miner answer alloc outcome for both
// modules, but only when at least one side is not a plain success. This
// is the field that carries the invalid UTF-8 and 2 MiB edge cases, so
// it is the field most likely to show a refused alloc or a refused
// write.
func maAllocAnomalyNote(refCell, ourCell MatrixCell) string {
	refOK := refCell.MADetail.Outcome == AllocOutcomeOK || refCell.MADetail.Outcome == AllocOutcomeZeroLength
	ourOK := ourCell.MADetail.Outcome == AllocOutcomeOK || ourCell.MADetail.Outcome == AllocOutcomeZeroLength
	if refOK && ourOK {
		return ""
	}
	return fmt.Sprintf("ma alloc: reference=%s, ours=%s", refCell.MADetail.Text(), ourCell.MADetail.Text())
}

// printCellDetail prints the alloc detail for the question, ground
// truth, and miner answer writes of one cell. This detail matters most
// for the invalid UTF-8 row and the 2 MiB row, where alloc or the host
// memory write can behave in an interesting way.
func printCellDetail(label string, cell MatrixCell) {
	fmt.Printf("%s:\n", label)
	if cell.PanicText != "" {
		fmt.Printf("  recovered panic: %s\n", cell.PanicText)
	}
	if cell.ErrText != "" {
		fmt.Printf("  error: %s\n", cell.ErrText)
	}
	if cell.Valid {
		fmt.Printf("  score: %.4f (0x%08x)\n", cell.Score, cell.Bits)
	}
	printFieldDetail("  question", cell.QDetail)
	printFieldDetail("  ground_truth", cell.GTDetail)
	printFieldDetail("  miner_answer", cell.MADetail)
}

// printFieldDetail prints one AllocDetail record: the alloc pointer, the
// memory size in pages before and after alloc, whether the host write
// succeeds, and a plain text label for the exact outcome (a plain
// success, a refused alloc, a refused write, or a failed alloc call).
func printFieldDetail(label string, d AllocDetail) {
	fmt.Printf("%s: ptr=%d pages_before=%d pages_after=%d write_ok=%t outcome=%q\n",
		label, d.Ptr, d.PagesBefore, d.PagesAfter, d.WriteOK, d.Text())
}
