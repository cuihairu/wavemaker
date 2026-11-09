#!/usr/bin/env python3
"""WM-0007 HDR V2 conformance fixture merge and independent verification.

Reads the Java and Go generator outputs, verifies them against each other AND
against an independent Python implementation of the HdrHistogram geometry and
the V2 blob wire format (zlib + zigzag LEB128 varints), then writes the merged
fixture consumed by the Rust conformance test.

The Python code here is a deliberate THIRD implementation, transcribed from the
published HdrHistogram algorithm (not copied from the Rust code), so agreement
of Java + Go + Python pins the geometry and blob format without trusting any
single implementation.

Notes on known implementation differences (recorded, not hidden):
  * Percentile rounding: Java uses ceil(nextafter(p)) as the target count;
    hdrhistogram-go truncates. Java semantics are canonical (upstream
    HdrHistogram); the Rust implementation follows Java. Go percentiles are
    kept in the fixture under ``go_expected`` for the record but are NOT
    asserted equal to Java.
  * Compressed blob bytes are NOT byte-identical across zlib implementations;
    only semantic equivalence (same decoded counts array) is asserted.

Run: python3 compare.py <java.json> <go.json> <out.json>
"""
import json
import math
import struct
import sys
import zlib


# ---------------------------------------------------------------------------
# Independent HdrHistogram geometry (transcribed from the upstream algorithm).
# ---------------------------------------------------------------------------
def geometry(lowest, sigfigs, highest):
    largest = 2 * (10 ** sigfigs)
    mag = int(math.ceil(math.log2(largest)))
    half_mag = mag - 1
    sub_count = 1 << mag
    half_count = sub_count // 2
    unit = int(math.floor(math.log2(lowest))) if lowest >= 1 else 0
    sub_mask = (sub_count - 1) << unit
    smallest_untrackable = sub_count << unit
    buckets = 1
    while smallest_untrackable <= highest:
        if smallest_untrackable > (2 ** 63 - 1) // 2:
            buckets += 1
            break
        smallest_untrackable <<= 1
        buckets += 1
    counts_len = (buckets + 1) * half_count
    return {
        "largest_value_single_unit_resolution": largest,
        "sub_bucket_count": sub_count,
        "sub_bucket_half_count": half_count,
        "sub_bucket_half_count_magnitude": half_mag,
        "unit_magnitude": unit,
        "bucket_count": buckets,
        "counts_len": counts_len,
    }


# ---------------------------------------------------------------------------
# Independent V2 blob parser (zlib + zigzag LEB128-64b9B).
# ---------------------------------------------------------------------------
V2_ENC = 0x1C849313
V2_COMP = 0x1C849314


def zigzag_decode_stream(data):
    """Decode zigzag LEB128-64b9B long stream -> list of signed counts."""
    out = []
    i = 0
    n = len(data)
    while i < n:
        b = data[i]
        i += 1
        # first byte: 7 payload bits, continuation
        payload = b & 0x7F
        shift = 7
        while b & 0x80:
            if shift >= 63:
                # final byte holds the remaining 8 bits (no continuation)
                b = data[i]
                i += 1
                payload |= (b & 0xFF) << shift
                break
            b = data[i]
            i += 1
            payload |= (b & 0x7F) << shift
            shift += 7
        # zigzag decode (unsigned payload -> signed)
        value = (payload >> 1) ^ -(payload & 1)
        out.append(value)
    return out


def parse_blob(blob):
    """Parse a compressed V2 blob -> (params_dict, counts, geometry)."""
    cookie, length = struct.unpack(">ii", blob[:8])
    assert cookie == V2_COMP, f"expected compressed V2 cookie 0x{V2_COMP:x}, got 0x{cookie & 0xFFFFFFFF:x}"
    inner = zlib.decompress(blob[8 : 8 + length])
    (icookie, payload_len, norm_offset, sigfigs, lowest, highest, ratio) = struct.unpack(
        ">iiiiqqd", inner[:40]
    )
    assert icookie == V2_ENC, f"expected V2 encoding cookie, got 0x{icookie & 0xFFFFFFFF:x}"
    assert payload_len == len(inner) - 40, (payload_len, len(inner) - 40)
    geo = geometry(lowest, sigfigs, highest)
    counts = [0] * geo["counts_len"]
    idx = 0
    for value in zigzag_decode_stream(inner[40:]):
        if value < 0:
            idx += -value
            assert idx <= geo["counts_len"], "zero-run overruns counts array"
        else:
            assert idx < geo["counts_len"], "count overruns counts array"
            counts[idx] = value
            idx += 1
    return {"lowest": lowest, "sigfigs": sigfigs, "highest": highest,
            "norm_offset": norm_offset, "ratio": ratio,
            "counts": counts, "geo": geo}


def stats(counts, geo):
    total = sum(counts)
    if total == 0:
        return {"total_count": 0, "min": 0, "max": 0, "mean": 0.0}
    # min = value_from_index of first nonzero index > 0 (or 0 if count[0]>0)
    idxs = [i for i, c in enumerate(counts) if c > 0]
    first_nonzero = idxs[0]
    min_val = 0 if first_nonzero == 0 else value_from_index(geo, first_nonzero)
    max_idx = idxs[-1]
    max_val = highest_equiv(geo, value_from_index(geo, max_idx))
    mean = sum(counts[i] * median_equiv(geo, value_from_index(geo, i)) for i in idxs) / total
    return {"total_count": total, "min": min_val, "max": max_val, "mean": mean}


def value_from_index(geo, index):
    half_mag = geo["sub_bucket_half_count_magnitude"]
    half = geo["sub_bucket_half_count"]
    unit = geo["unit_magnitude"]
    bucket = (index >> half_mag) - 1
    sub = (index & (half - 1)) + half
    if bucket < 0:
        sub -= half
        bucket = 0
    return sub << (bucket + unit)


def bucket_of(geo, value):
    sub_mask = (geo["sub_bucket_count"] - 1) << geo["unit_magnitude"]
    leading_zero_base = 64 - geo["unit_magnitude"] - int(
        math.log2(geo["sub_bucket_count"])
    )
    return leading_zero_base - (64 - (value | sub_mask).bit_length())


def sub_bucket_of(geo, value, bucket):
    return value >> (bucket + geo["unit_magnitude"])


def counts_index(geo, value):
    half = geo["sub_bucket_half_count"]
    half_mag = geo["sub_bucket_half_count_magnitude"]
    b = bucket_of(geo, value)
    s = sub_bucket_of(geo, value, b)
    return ((b + 1) << half_mag) + (s - half)


def unit_size(geo, bucket):
    return 1 << (bucket + geo["unit_magnitude"])


def lowest_equiv(geo, value):
    return value_from_index(geo, counts_index(geo, value)) if value != 0 else 0


def next_non_equiv(geo, value):
    return lowest_equiv(geo, value) + unit_size(geo, bucket_of(geo, value))


def highest_equiv(geo, value):
    return next_non_equiv(geo, value) - 1


def median_equiv(geo, value):
    return lowest_equiv(geo, value) + (unit_size(geo, bucket_of(geo, value)) >> 1)


def percentile(geo, counts, p):
    total = sum(counts)
    if total == 0:
        return 0
    # Java canonical: ceil(nextafter(p) * total / 100), clamped to [1, total]
    import math as m
    target = m.ceil((p * total) / 100.0)
    target = max(1, min(target, total))
    acc = 0
    for i, c in enumerate(counts):
        acc += c
        if acc >= target:
            v = value_from_index(geo, i)
            return lowest_equiv(geo, v) if p == 0.0 else highest_equiv(geo, v)
    return 0


def main():
    java = json.load(open(sys.argv[1]))
    go = json.load(open(sys.argv[2]))
    out_path = sys.argv[3]

    # 1) geometry matrices must agree exactly
    assert len(java["param_matrix"]) == len(go["param_matrix"])
    for jrow, grow in zip(java["param_matrix"], go["param_matrix"]):
        assert jrow == grow, (jrow, grow)
    jfrozen = java["frozen_v1"]
    gfrozen = go["frozen_v1"]
    assert jfrozen["geometry"] == gfrozen["geometry"]
    geo = jfrozen["geometry"]

    # 2) independent Python geometry must agree with both implementations
    for row in java["param_matrix"]:
        pgeo = geometry(row["lowest"], row["sigfigs"], row["highest"])
        for k, v in pgeo.items():
            assert v == row[k], (row, k, v, row[k])

    # 3) independent index map must agree with Java reflection values
    for i, v in enumerate(jfrozen["value_from_index"]):
        assert v == value_from_index(geo, i), ("value_from_index", i, v)
    for probe in jfrozen["counts_index_probes"]:
        assert probe["counts_index"] == counts_index(geo, probe["value"]), probe

    # 4) value cases: multisets, blob decodes, stats
    cases = []
    for jc in java["value_cases"]:
        name = jc["name"]
        gc = next(c for c in go["value_cases"] if c["name"] == name)
        assert sorted(jc["values"]) == sorted(gc["values"]), (name, "value sets differ")

        jblob = bytes.fromhex(jc["java_blob_hex"])
        gblob = bytes.fromhex(gc["go_blob_hex"])
        jd = parse_blob(jblob)
        gd = parse_blob(gblob)
        assert jd["counts"] == gd["counts"], (name, "blobs decode to different counts")
        assert jd["lowest"] == gc["lowest"] and jd["sigfigs"] == gc["sigfigs"]
        assert jd["highest"] == gc["highest"]

        # independent stats must match the Java oracle (counts arrays agree)
        s = stats(jd["counts"], geo_for(jd))
        exp = jc["expected"]
        assert s["total_count"] == exp["total_count"], (name, s, exp)
        assert s["min"] == exp["min"], (name, s, exp)
        assert s["max"] == exp["max"], (name, s, exp)
        assert abs(s["mean"] - exp["mean"]) < 1e-6, (name, s, exp)
        for pkey, p in (("p50", 50.0), ("p95", 95.0), ("p99", 99.0),
                        ("p99_9", 99.9), ("p100", 100.0)):
            got = percentile(geo_for(jd), jd["counts"], p)
            assert got == exp[pkey], (name, pkey, got, exp[pkey])
        assert exp["total_count"] == gc["expected"]["total_count"], name
        assert exp["min"] == gc["expected"]["min"] and exp["max"] == gc["expected"]["max"], name
        jm, gm = exp["mean"], gc["expected"]["mean"]
        rel = abs(jm - gm) / max(abs(jm), abs(gm)) if jm or gm else 0.0
        assert rel < 1e-12, (name, "mean divergence", rel)

        cases.append({
            "name": name,
            "lowest": jc["lowest"],
            "sigfigs": jc["sigfigs"],
            "highest": jc["highest"],
            "values": jc["values"],
            "expected": exp,
            "go_expected": gc["expected"],
            "java_blob_hex": jc["java_blob_hex"],
            "go_blob_hex": gc["go_blob_hex"],
        })

    merged = {
        "meta": {
            "java": java["meta"],
            "go": go["meta"],
            "python": "independent third implementation (tools/hdr_conformance/compare.py)",
            "percentile_rounding": "Java canonical: ceil(nextafter(p)*total/100); hdrhistogram-go truncates; Rust follows Java",
        },
        "param_matrix": java["param_matrix"],
        "frozen_v1": {
            "lowest": jfrozen["lowest"],
            "sigfigs": jfrozen["sigfigs"],
            "highest": jfrozen["highest"],
            "geometry": jfrozen["geometry"],
            "value_from_index": jfrozen["value_from_index"],
            "counts_index_probes": jfrozen["counts_index_probes"],
        },
        "value_cases": cases,
    }
    with open(out_path, "w") as f:
        json.dump(merged, f)
    print(f"verified java==go==python; wrote {out_path}")


def geo_for(d):
    # geometry dict keyed like the fixture rows
    g = d["geo"]
    return {
        "largest_value_single_unit_resolution": g["largest_value_single_unit_resolution"],
        "sub_bucket_count": g["sub_bucket_count"],
        "sub_bucket_half_count": g["sub_bucket_half_count"],
        "sub_bucket_half_count_magnitude": g["sub_bucket_half_count_magnitude"],
        "unit_magnitude": g["unit_magnitude"],
        "bucket_count": g["bucket_count"],
        "counts_len": g["counts_len"],
    }


if __name__ == "__main__":
    main()
