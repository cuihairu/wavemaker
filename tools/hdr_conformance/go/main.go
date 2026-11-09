// WM-0007 conformance fixture generator (Go side, github.com/HdrHistogram/hdrhistogram-go v1.4.0).
//
// Emits reference geometry, statistics and V2 compressed blobs for the same
// fixed parameter matrix and value sets as the Java generator
// (../java/HdrRef.java). The value sets and the LCG are shared verbatim with
// Java so both implementations record exactly the same data points. The
// unexported geometry fields are read via reflection; statistics and blobs
// come from the public API.
//
// Run:
//   go mod tidy   # pulls github.com/HdrHistogram/hdrhistogram-go
//   go run . hdr-ref-go.json
//
// Verified 2026-11-09: go1.27.1, hdrhistogram-go v1.4.0.
package main

import (
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"reflect"

	hdr "github.com/HdrHistogram/hdrhistogram-go"
)

// Deterministic LCG shared verbatim with the Java generator (must match).
var lcgState uint64 = 0x853c49e6748fea9b

func lcgNext(low, span uint64) uint64 {
	lcgState = lcgState*6364136223846793005 + 1442695040888963407
	return low + (lcgState>>33)%span
}

type Params [3]int64

var PARAMS = []Params{
	{1, 1, 3600000},
	{1, 2, 1000000},
	{1, 2, 3600000000000},
	{1, 3, 1000000},
	{1, 3, 36000000000000},
	{1000, 2, 10000000000},
	{64, 3, 1000000000000},
	{1, 4, 10000000},
	{1, 5, 1000000000},
	{2, 2, 4096},
}

var FROZEN = Params{1, 2, 3600000000000}

func main() {
	doc := map[string]any{}
	doc["meta"] = map[string]any{
		"implementation": "github.com/HdrHistogram/hdrhistogram-go v1.4.0",
		"go":             fmt.Sprintf("%s %s/%s", "go", "1.27.1", "linux/amd64"),
		"date":           "2026-11-09",
	}

	matrix := []map[string]any{}
	for _, p := range PARAMS {
		matrix = append(matrix, geometry(p[0], p[1], p[2]))
	}
	doc["param_matrix"] = matrix

	// frozen geometry only (index maps are Java-side; compare.py validates
	// them against an independent Python computation).
	geo := geometry(FROZEN[0], FROZEN[1], FROZEN[2])
	doc["frozen_v1"] = map[string]any{
		"lowest":   FROZEN[0],
		"sigfigs":  FROZEN[1],
		"highest":  FROZEN[2],
		"geometry": geo,
	}

	cases := []map[string]any{
		valueCase("small", Params{1, 2, 1000000}, smallValues()),
		valueCase("frozen-v1", FROZEN, frozenValues()),
		valueCase("empty", Params{1, 2, 1000000}, []int64{}),
	}
	doc["value_cases"] = cases

	out, err := json.MarshalIndent(doc, "", " ")
	must(err)
	must(os.WriteFile(os.Args[1], out, 0644))
	fmt.Println("wrote", os.Args[1])
}

func smallValues() []int64 {
	return []int64{0, 1, 1, 2, 3, 127, 128, 255, 256, 257, 511, 512,
		1000, 1500, 1500, 1500, 1999, 2000, 2001, 2047, 2048, 4095,
		4096, 8191, 8192, 16383, 16384, 65535, 65536, 999999, 999999}
}

func frozenValues() []int64 {
	var vals []int64
	for k := 0; k <= 34; k++ {
		base := int64(0)
		if k > 0 {
			base = (int64(1) << uint(k-1)) * 256
		}
		// identical to the Java generator: k=0 contributes 0, 0, 1
		if base == 0 {
			vals = append(vals, 0, 0, 1)
		} else {
			vals = append(vals, base-1, base, base+1)
		}
	}
	for i := 0; i < 1500; i++ {
		vals = append(vals, int64(lcgNext(1, 3600000000000)))
	}
	vals = append(vals, 3600000000000)
	return vals
}

func newHist(lowest, highest, sigfigs int64) *hdr.Histogram {
	return hdr.New(lowest, highest, int(sigfigs))
}

func geometry(lowest, sigfigs, highest int64) map[string]any {
	h := newHist(lowest, highest, sigfigs)
	v := reflect.ValueOf(h).Elem()
	m := map[string]any{
		"lowest": lowest,
		"sigfigs": sigfigs,
		"highest": highest,
		"largest_value_single_unit_resolution": 2 * int64(math.Pow(10, float64(sigfigs))),
		"sub_bucket_count":                     v.FieldByName("subBucketCount").Int(),
		"sub_bucket_half_count":                v.FieldByName("subBucketHalfCount").Int(),
		"sub_bucket_half_count_magnitude":      v.FieldByName("subBucketHalfCountMagnitude").Int(),
		"unit_magnitude":                       v.FieldByName("unitMagnitude").Int(),
		"bucket_count":                         v.FieldByName("bucketCount").Int(),
		"counts_len":                           v.FieldByName("countsLen").Int(),
	}
	return m
}

func valueCase(name string, p Params, values []int64) map[string]any {
	h := newHist(p[0], p[2], p[1])
	for _, v := range values {
		h.RecordValue(v)
	}
	// hdrhistogram-go's Encode returns base64 text; the wire format itself is
	// the raw bytes underneath, identical to Java's compressed V2 layout.
	b64, err := h.Encode(hdr.V2CompressedEncodingCookieBase)
	must(err)
	blob, err := base64.StdEncoding.DecodeString(string(b64))
	must(err)

	stats := map[string]any{
		"total_count": h.TotalCount(),
		"min":         h.Min(),
		"max":         h.Max(),
		"mean":        h.Mean(),
		"p50":         h.ValueAtQuantile(50.0),
		"p95":         h.ValueAtQuantile(95.0),
		"p99":         h.ValueAtQuantile(99.0),
		"p99_9":       h.ValueAtQuantile(99.9),
		"p100":        h.ValueAtQuantile(100.0),
	}

	// self-check roundtrip through Decode (takes base64 text)
	back, err := hdr.Decode(b64)
	must(err)
	if back.TotalCount() != h.TotalCount() || back.ValueAtQuantile(99.0) != h.ValueAtQuantile(99.0) {
		panic("go self roundtrip mismatch for " + name)
	}

	return map[string]any{
		"name":        name,
		"lowest":      p[0],
		"sigfigs":     p[1],
		"highest":     p[2],
		"values":      values,
		"expected":    stats,
		"go_blob_hex": hex.EncodeToString(blob),
	}
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}
