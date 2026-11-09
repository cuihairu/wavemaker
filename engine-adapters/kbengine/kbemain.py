"""KBEngine entry script that boots the Bridge bootstrap.

Point `<bots><entryScriptFile>` at this module (kbengine_defaults.xml, audit
§5), or `import kbemain` from your own entry script. The engine calls
`onInit(0)` exactly once before the main loop and `onFinish()` during
finalise; both are delegated to the process-wide bootstrap.
"""

from bridge import create_default_bootstrap


def onInit(arg0):
    return create_default_bootstrap().on_init(arg0)


def onFinish():
    return create_default_bootstrap().on_finish()
