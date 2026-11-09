"""Process metrics collection (WM-0214, docs/bridge.md §Metrics Model).

Process-level observability for the bots process: CPU seconds, RSS bytes
and the open file-descriptor count, read from /proc (Linux best-effort).

Honesty rules (docs/kbengine.md no-unverified-claims): every field is None
when its source is unreadable — a missing metric is never reported as a
fake zero (a zero CPU reading reads like an idle process, not an unknown
one; a zero fd count reads like a closed process). Values are process-wide
(the whole bots interpreter, not per-bot); per-bot aggregates live in the
WM-0208 census / WM-0209 events layer.

Reads happen on the scripting thread (S1) in the heartbeat tick (WM-0212):
three small /proc reads, no locks, no engine calls — S2-safe by
construction. Non-Linux hosts have no /proc: every field is None.
"""

import os

# Kernel clock ticks per second when sysconf cannot answer (audit-style
# defensive default; Linux userland almost always reports 100).
CLK_TCK_FALLBACK = 100.0


def _read_stat_cpu(path):
    """utime+stime in seconds from /proc/<pid>/stat.

    The comm field may contain spaces and parentheses, so everything after
    the final ')' is fixed-offset: state, ppid, pgrp, session, tty, tpgid,
    flags, minflt, cminflt, majflt, cmajflt, utime, stime — utime is the
    12th and stime the 13th field after it.
    """
    try:
        with open(path, "r") as f:
            after = f.read().rsplit(")", 1)[-1]
        fields = after.split()
        ticks = int(fields[11]) + int(fields[12])
    except (OSError, ValueError, IndexError):
        return None
    try:
        clk_tck = float(os.sysconf("SC_CLK_TCK"))
    except (ValueError, OSError, AttributeError):
        clk_tck = CLK_TCK_FALLBACK
    if clk_tck <= 0:
        clk_tck = CLK_TCK_FALLBACK
    return ticks / clk_tck


def _read_status_kb(path, name):
    """A `name: <n> kB` line from /proc/<pid>/status (e.g. VmRSS:) in
    bytes; None when absent or unreadable."""
    try:
        with open(path, "r") as f:
            for line in f:
                if line.startswith(name):
                    return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def _read_fd_count(path):
    """Open file descriptors of the process (/proc/<pid>/fd listing);
    None when unreadable."""
    try:
        return len(os.listdir(path))
    except OSError:
        return None


def collect_process_metrics(pid=None, proc_root="/proc"):
    """One process-metrics snapshot:
    `{cpu_seconds, rss_bytes, fd_count}`.

    Fields are None when their /proc source is unavailable (non-Linux
    host, permissions, or a race with process exit) — never zeros.
    """
    prefix = os.path.join(proc_root, str(os.getpid() if pid is None else pid))
    return {
        "cpu_seconds": _read_stat_cpu(os.path.join(prefix, "stat")),
        "rss_bytes": _read_status_kb(
            os.path.join(prefix, "status"), "VmRSS:"),
        "fd_count": _read_fd_count(os.path.join(prefix, "fd")),
    }
