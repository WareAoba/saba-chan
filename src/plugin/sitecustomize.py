"""saba-chan import guard (M6): only enabled extensions can be imported.

The daemon puts this file's folder on PYTHONPATH, so Python loads it at
startup. `import extensions.<name>` works only when <name> is in
SABA_ENABLED_EXTENSIONS (ids; `-` and `_` are the same). Anything else raises
ModuleNotFoundError, which modules already treat as "extension missing".
"""
import os
import sys

_ENABLED = os.environ.get("SABA_ENABLED_EXTENSIONS")


class _ExtensionGuard:
    def __init__(self, enabled):
        self.enabled = {e.strip().replace("-", "_") for e in enabled.split(",") if e.strip()}

    def find_spec(self, fullname, path=None, target=None):
        parts = fullname.split(".")
        if len(parts) >= 2 and parts[0] == "extensions" and parts[1].replace("-", "_") not in self.enabled:
            raise ModuleNotFoundError(
                "saba-chan extension '%s' is not enabled" % parts[1], name=fullname
            )
        return None


if _ENABLED is not None:
    sys.meta_path.insert(0, _ExtensionGuard(_ENABLED))
