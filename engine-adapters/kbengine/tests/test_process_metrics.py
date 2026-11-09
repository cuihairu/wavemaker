"""Unit tests for the WM-0214 process metrics collection (CPU / RSS / FD).

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Self-reads of /proc plus a fake proc tree — no sockets, no engine.
"""

import os
import tempfile

from bridge.process_metrics import collect_process_metrics

# state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt
# utime stime ...
FIELDS_AFTER_COMM = "S 1 1 1 0 -1 4194560 100 0 0 0 {utime} {stime} 0 0"


class FakeProc(object):
    """A tiny fake /proc tree: fake_proc("1") -> the pid-1 directory."""

    def __init__(self):
        self.root = tempfile.mkdtemp(prefix="wm-proc-test-")

    def pid_dir(self, pid="1"):
        path = os.path.join(self.root, pid)
        os.makedirs(path, exist_ok=True)
        return path

    def write(self, pid, name, content):
        with open(os.path.join(self.pid_dir(pid), name), "w") as f:
            f.write(content)


def test_self_snapshot_reads_real_proc():
    metrics = collect_process_metrics()
    assert set(metrics) == {"cpu_seconds", "rss_bytes", "fd_count"}
    assert metrics["rss_bytes"] is not None and metrics["rss_bytes"] > 0
    assert metrics["fd_count"] is not None and metrics["fd_count"] >= 3
    assert metrics["cpu_seconds"] is not None and metrics["cpu_seconds"] >= 0


def test_cpu_seconds_from_stat_ticks():
    root = FakeProc()
    root.write("1", "stat", "1 (bots) " + FIELDS_AFTER_COMM.format(
        utime=250, stime=50))
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    clk_tck = float(os.sysconf("SC_CLK_TCK"))
    assert metrics["cpu_seconds"] == 300 / clk_tck


def test_comm_with_parentheses_does_not_confuse_the_offset():
    root = FakeProc()
    # A ')' inside comm would shift naive splits; the parser anchors on
    # the final ')' so utime/stime stay at their fixed offsets.
    root.write("1", "stat", "1 (a)b (c) " + FIELDS_AFTER_COMM.format(
        utime=10, stime=5))
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    clk_tck = float(os.sysconf("SC_CLK_TCK"))
    assert metrics["cpu_seconds"] == 15 / clk_tck


def test_malformed_stat_yields_none_not_zero():
    root = FakeProc()
    root.write("1", "stat", "1 (short) S 1 2 3")  # no utime/stime
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    assert metrics["cpu_seconds"] is None


def test_rss_from_status_vmrss_in_kib():
    root = FakeProc()
    root.write("1", "status", "Name:\tbots\nVmRSS:\t1234 kB\nThreads:\t3\n")
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    assert metrics["rss_bytes"] == 1234 * 1024


def test_missing_vmrss_yields_none_not_zero():
    root = FakeProc()
    root.write("1", "status", "Name:\tbots\nThreads:\t3\n")
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    assert metrics["rss_bytes"] is None


def test_fd_count_lists_the_open_descriptors():
    root = FakeProc()
    fd_dir = os.path.join(root.pid_dir("1"), "fd")
    os.mkdir(fd_dir)
    for name in ("0", "1", "2", "3"):
        with open(os.path.join(fd_dir, name), "w") as f:
            f.write("socket:[1]")
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    assert metrics["fd_count"] == 4


def test_unreadable_sources_yield_none_fields_never_fake_zeros():
    root = FakeProc()  # no pid subtree at all
    metrics = collect_process_metrics(pid=1, proc_root=root.root)
    assert metrics == {
        "cpu_seconds": None, "rss_bytes": None, "fd_count": None,
    }
