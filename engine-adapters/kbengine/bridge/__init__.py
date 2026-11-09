"""KBEngine Bridge (Phase 2, WM-0201+).

The public surface is intentionally small: the engine entry script imports
`create_default_bootstrap` and delegates `onInit`/`onFinish` to it.
"""

from .bootstrap import Bootstrap, BootstrapConfig

# One bootstrap per engine process (S6: onInit is once per process).
_default_bootstrap = None


def create_default_bootstrap(engine=None, config=None):
    """Build (or return) the process-wide bootstrap, wiring the live
    `KBEngine` module and Supervisor-injected configuration. Safe to call
    repeatedly: the first call wins."""
    global _default_bootstrap
    if _default_bootstrap is None:
        if engine is None:
            from .engine_port import EnginePort

            engine = EnginePort.default()
        if config is None:
            config = BootstrapConfig.from_env()
        _default_bootstrap = Bootstrap(engine, config)
    return _default_bootstrap


__all__ = ["Bootstrap", "BootstrapConfig", "create_default_bootstrap"]
