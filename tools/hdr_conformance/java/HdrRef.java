import org.HdrHistogram.Histogram;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * WM-0007 conformance fixture generator (Java side, HdrHistogram 2.2.2).
 *
 * Emits reference geometry, index maps, statistics and V2 compressed blobs
 * for a fixed parameter matrix and fixed value sets. Private/package-private
 * internals are read via reflection so the numbers come from the real
 * library, never from a transcription.
 *
 * Build/run (jar fetched from Maven Central once):
 *   curl -o HdrHistogram-2.2.2.jar \
 *     https://repo1.maven.org/maven2/org/hdrhistogram/HdrHistogram/2.2.2/HdrHistogram-2.2.2.jar
 *   javac -cp HdrHistogram-2.2.2.jar HdrRef.java
 *   java  -cp HdrHistogram-2.2.2.jar:. HdrRef hdr-ref-java.json
 *
 * Verified 2026-11-09: openjdk 21.0.12, HdrHistogram 2.2.2.
 */
public class HdrRef {

    /** Deterministic LCG shared verbatim by the Go generator (must match). */
    static long lcgState = 0x853c49e6748fea9bL;
    static long lcgNext(long low, long span) {
        lcgState = lcgState * 6364136223846793005L + 1442695040888963407L;
        return low + (lcgState >>> 33) % span;
    }

    static long[][] PARAMS = {
            {1, 1, 3600000},
            {1, 2, 1000000},
            {1, 2, 3600000000000L},
            {1, 3, 1000000},
            {1, 3, 36000000000000L},
            {1000, 2, 10000000000L},
            {64, 3, 1000000000000L},
            {1, 4, 10000000},
            {1, 5, 1000000000},
            {2, 2, 4096},
    };
    static long[] FROZEN = {1, 2, 3600000000000L};

    public static void main(String[] args) throws Exception {
        Map<String, Object> doc = new LinkedHashMap<>();
        Map<String, Object> meta = new LinkedHashMap<>();
        meta.put("implementation", "HdrHistogram 2.2.2 (maven org.hdrhistogram:HdrHistogram:2.2.2)");
        meta.put("java", System.getProperty("java.version"));
        meta.put("date", "2026-11-09");
        doc.put("meta", meta);

        // ---- geometry parameter matrix ----
        List<Object> matrix = new ArrayList<>();
        for (long[] p : PARAMS) matrix.add(geometry((int) p[0], (int) p[1], p[2]));
        doc.put("param_matrix", matrix);

        // ---- frozen v1 geometry + full index map ----
        Histogram frozen = new Histogram((int) FROZEN[0], FROZEN[2], (int) FROZEN[1]);
        Map<String, Object> frozenDoc = new LinkedHashMap<>();
        frozenDoc.put("lowest", FROZEN[0]);
        frozenDoc.put("sigfigs", FROZEN[1]);
        frozenDoc.put("highest", FROZEN[2]);
        Map<String, Object> geo = geometry((int) FROZEN[0], (int) FROZEN[1], FROZEN[2]);
        frozenDoc.put("geometry", geo);
        int countsLen = intField(frozen, "countsArrayLength");
        long[] fromIndex = new long[countsLen];
        Method vfi = AbstractHistogramMethod("valueFromIndex", int.class);
        for (int i = 0; i < countsLen; i++) {
            fromIndex[i] = (Long) vfi.invoke(frozen, i);
        }
        frozenDoc.put("value_from_index", fromIndex);
        Method cai = AbstractHistogramMethod("countsArrayIndex", long.class);
        long[] probes = {0, 1, 2, 3, 63, 64, 127, 128, 129, 255, 256, 257,
                511, 512, 1023, 1024, 2047, 2048, 4095, 4096,
                100000, 123456789, 1000000000, 3599999999999L, 3600000000000L};
        List<Object> idxProbes = new ArrayList<>();
        for (long v : probes) {
            Map<String, Object> e = new LinkedHashMap<>();
            e.put("value", v);
            e.put("counts_index", ((Integer) cai.invoke(frozen, v)).longValue());
            idxProbes.add(e);
        }
        frozenDoc.put("counts_index_probes", idxProbes);
        doc.put("frozen_v1", frozenDoc);

        // ---- value cases ----
        List<Object> cases = new ArrayList<>();
        cases.add(valueCase("small", new long[]{1, 2, 1000000}, smallValues()));
        cases.add(valueCase("frozen-v1", FROZEN, frozenValues()));
        cases.add(valueCase("empty", new long[]{1, 2, 1000000}, new long[0]));
        doc.put("value_cases", cases);

        String json = toJson(doc);
        Path out = Paths.get(args[0]);
        Files.write(out, json.getBytes(StandardCharsets.UTF_8));
        System.out.println("wrote " + out);
    }

    static long[] smallValues() {
        return new long[]{0, 1, 1, 2, 3, 127, 128, 255, 256, 257, 511, 512,
                1000, 1500, 1500, 1500, 1999, 2000, 2001, 2047, 2048, 4095,
                4096, 8191, 8192, 16383, 16384, 65535, 65536, 999999, 999999};
    }

    /** Boundary probes at every bucket edge + noise; deterministic via LCG. */
    static long[] frozenValues() {
        List<Long> vals = new ArrayList<>();
        // one value in every bucket, both edges of the boundary
        for (int k = 0; k <= 34; k++) {
            long base = (k == 0) ? 0L : (1L << (k - 1)) * 256;
            vals.add(base == 0 ? 0L : base - 1);
            vals.add(base);
            vals.add(base + 1);
        }
        for (int i = 0; i < 1500; i++) vals.add(lcgNext(1, 3600000000000L));
        vals.add(3600000000000L);
        long[] out = new long[vals.size()];
        for (int i = 0; i < out.length; i++) out[i] = vals.get(i);
        return out;
    }

    static Map<String, Object> geometry(int lowest, int sigfigs, long highest) throws Exception {
        Histogram h = new Histogram(lowest, highest, sigfigs);
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("lowest", lowest);
        m.put("sigfigs", sigfigs);
        m.put("highest", highest);
        m.put("largest_value_single_unit_resolution", 2L * (long) Math.pow(10, sigfigs));
        m.put("sub_bucket_count", (long) intField(h, "subBucketCount"));
        m.put("sub_bucket_half_count", (long) intField(h, "subBucketHalfCount"));
        m.put("sub_bucket_half_count_magnitude", (long) intField(h, "subBucketHalfCountMagnitude"));
        m.put("unit_magnitude", (long) intField(h, "unitMagnitude"));
        m.put("bucket_count", (long) intField(h, "bucketCount"));
        m.put("counts_len", (long) intField(h, "countsArrayLength"));
        return m;
    }

    static Map<String, Object> valueCase(String name, long[] p, long[] values) throws Exception {
        Histogram h = new Histogram((int) p[0], p[2], (int) p[1]);
        for (long v : values) h.recordValue(v);

        int cap = intField(h, "countsArrayLength") * 10 + 4096;
        ByteBuffer buf = ByteBuffer.allocate(cap).order(ByteOrder.BIG_ENDIAN);
        int n = h.encodeIntoCompressedByteBuffer(buf);
        byte[] blob = new byte[n];
        ((ByteBuffer) buf.flip()).get(blob);

        Map<String, Object> e = new LinkedHashMap<>();
        e.put("name", name);
        e.put("lowest", p[0]);
        e.put("sigfigs", p[1]);
        e.put("highest", p[2]);
        e.put("values", values);
        Map<String, Object> stats = new LinkedHashMap<>();
        stats.put("total_count", h.getTotalCount());
        stats.put("min", h.getMinValue());
        stats.put("max", h.getMaxValue());
        stats.put("mean", h.getMean());
        stats.put("p50", h.getValueAtPercentile(50.0));
        stats.put("p95", h.getValueAtPercentile(95.0));
        stats.put("p99", h.getValueAtPercentile(99.0));
        stats.put("p99_9", h.getValueAtPercentile(99.9));
        stats.put("p100", h.getValueAtPercentile(100.0));
        e.put("expected", stats);
        e.put("java_blob_hex", hex(blob));

        // self-check: decode our own blob and confirm the roundtrip stats
        Histogram back = Histogram.decodeFromCompressedByteBuffer(
                ByteBuffer.wrap(blob).order(ByteOrder.BIG_ENDIAN), 0);
        if (back.getTotalCount() != h.getTotalCount()
                || back.getValueAtPercentile(99.0) != h.getValueAtPercentile(99.0)) {
            throw new IllegalStateException("java self roundtrip mismatch for " + name);
        }
        return e;
    }

    static Method AbstractHistogramMethod(String name, Class<?> param) throws Exception {
        Class<?> c = Histogram.class;
        while (c != null && !c.getSimpleName().equals("AbstractHistogram")) c = c.getSuperclass();
        Method m = c.getDeclaredMethod(name, param);
        m.setAccessible(true);
        return m;
    }

    static int intField(Object o, String name) throws Exception {
        Class<?> c = o.getClass();
        while (c != null) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.getInt(o);
            } catch (NoSuchFieldException e) {
                c = c.getSuperclass();
            }
        }
        throw new NoSuchFieldException(name);
    }

    static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder(b.length * 2);
        for (byte x : b) sb.append(String.format("%02x", x));
        return sb.toString();
    }

    // ---- minimal stable JSON writer (deterministic ordering via LinkedHashMap) ----
    static String toJson(Object v) {
        StringBuilder sb = new StringBuilder();
        write(sb, v);
        return sb.toString();
    }

    static void write(StringBuilder sb, Object v) {
        if (v == null) sb.append("null");
        else if (v instanceof Map) {
            sb.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> e : ((Map<?, ?>) v).entrySet()) {
                if (!first) sb.append(',');
                first = false;
                sb.append('"').append(e.getKey()).append("\":");
                write(sb, e.getValue());
            }
            sb.append('}');
        } else if (v instanceof List) {
            sb.append('[');
            boolean first = true;
            for (Object x : (List<?>) v) {
                if (!first) sb.append(',');
                first = false;
                write(sb, x);
            }
            sb.append(']');
        } else if (v instanceof long[]) {
            sb.append('[');
            long[] a = (long[]) v;
            for (int i = 0; i < a.length; i++) {
                if (i > 0) sb.append(',');
                sb.append(a[i]);
            }
            sb.append(']');
        } else if (v instanceof Double) {
            sb.append(String.format(java.util.Locale.ROOT, "%.17g", v));
        } else if (v instanceof Integer || v instanceof Long) {
            sb.append(v);
        } else {
            sb.append('"').append(v).append('"');
        }
    }
}
