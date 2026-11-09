"""Thread-safe inbound command queue (audit §8.3 #1).

The IPC receive thread may only enqueue here — it must never call into the
engine scripting API. The engine main loop (the bootstrap pump) is the sole
consumer, so every command executes on the single scripting thread (S1).

The queue is deliberately small and dependency-free: commands are opaque
objects (the WM-0206 command layer defines their shape); the bootstrap only
moves them in FIFO order.
"""

import threading
from collections import deque


class CommandQueue(object):
    """FIFO command queue with a bounded overflow guard."""

    def __init__(self, max_size=0):
        # max_size == 0 means unbounded; > 0 drops the newest command when
        # full and counts it as overflow (the consumer is never stalled by
        # IPC traffic beyond the bound).
        self._deque = deque()
        self._lock = threading.Lock()
        self._max_size = max_size
        self._overflow = 0

    def enqueue(self, command):
        """Push a command from any thread. Returns False on overflow."""
        with self._lock:
            if self._max_size and len(self._deque) >= self._max_size:
                self._overflow += 1
                return False
            self._deque.append(command)
            return True

    def drain(self):
        """Take and clear everything queued (main-loop side only)."""
        with self._lock:
            items = list(self._deque)
            self._deque.clear()
        return items

    def pending(self):
        with self._lock:
            return len(self._deque)

    def overflow_count(self):
        with self._lock:
            return self._overflow
