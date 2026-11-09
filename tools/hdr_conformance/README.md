# HDR V2 conformance fixture generators (WM-0007)

`crates/wm-core/tests/fixtures/hdr_conformance.json` is generated once by the
canonical HdrHistogram implementations and verified three-way before being
committed. The Rust conformance tests (`crates/wm-core/tests/hdr_conformance.rs`)
consume the committed fixture; CI does not need Java, Go, or Python.

## Layout

- `java/HdrRef.java` — generator on HdrHistogram 2.2.2 (reflection on the real
  library internals: geometry fields, value/index maps, V2 compressed blobs).
- `go/main.go` — mirror generator on hdrhistogram-go v1.4.0 (same parameter
  matrix, same LCG, same value sets; same V2 blobs via its public API).
- `compare.py` — independent third implementation: decodes both blob sets with
  its own zlib + zigzag LEB128 parser, re-derives geometry and statistics, and
  asserts Java == Go == Python before merging into the committed fixture.

## Known divergence (recorded, not hidden)

Percentile rounding: Java ceils `(nextafter(p) * total) / 100`; hdrhistogram-go
truncates. Java semantics are canonical and the Rust implementation follows
Java. Go percentiles are kept in the fixture under `go_expected` for the record
and are deliberately not asserted equal (`go_percentile_divergence_is_documented`
test pins this). Blobs are semantically equal (same counts array); compressed
bytes differ across zlib implementations and are not compared byte-for-byte.

## Regeneration

Fixture regeneration requires JDK 21+, Go 1.22+, Python 3. The jar comes from
Maven Central; the Go module from the module proxy.

```sh
mkdir -p /tmp/hdr-ref && cd /tmp/hdr-ref
curl -sO https://repo1.maven.org/maven2/org/hdrhistogram/HdrHistogram/2.2.2/HdrHistogram-2.2.2.jar
javac -cp HdrHistogram-2.2.2.jar $REPO/tools/hdr_conformance/java/HdrRef.java
java  -cp HdrHistogram-2.2.2.jar:. HdrRef hdr-ref-java.json
cd $REPO/tools/hdr_conformance/go && go run . /tmp/hdr-ref/hdr-ref-go.json
cd $REPO/tools/hdr_conformance
python3 compare.py /tmp/hdr-ref/hdr-ref-java.json /tmp/hdr-ref/hdr-ref-go.json \
  $REPO/crates/wm-core/tests/fixtures/hdr_conformance.json
```

Generated and verified 2026-11-09: OpenJDK 21.0.12.1, HdrHistogram 2.2.2,
go1.27.1 linux/amd64, hdrhistogram-go v1.4.0.
