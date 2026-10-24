"""saba-chan import guard (M6): only enabled extensions can be imported.

The daemon puts this file's folder on PYTHONPATH, so Python loads it at
startup. `import extensions.<name>` works only when <name> is in
SABA_ENABLED_EXTENSIONS (ids; `-` and `_` are the same). Anything else raises
ModuleNotFoundError, which modules already treat as "extension missing".
"""
import os
import sys

_trusted_roots = {os.path.normcase(os.path.realpath(sys.base_prefix)), os.path.normcase(os.path.realpath(sys.exec_prefix))}
_trusted = []
_other = []
for _path in sys.path:
    _real = os.path.normcase(os.path.realpath(_path or os.curdir))
    if any(_real == _root or _real.startswith(_root + os.sep) for _root in _trusted_roots):
        _trusted.append(_path)
    else:
        _other.append(_path)
sys.path[:] = _trusted + _other

_ENABLED = os.environ.get("SABA_ENABLED_EXTENSIONS")
# Enabled but unverified extensions a run outside the sandbox may not import
# (a verified module would otherwise run their code unisolated).
_UNVERIFIED = os.environ.get("SABA_UNVERIFIED_EXTENSIONS", "")


def _ids(text):
    return {e.strip().replace("-", "_") for e in text.split(",") if e.strip()}


class _ExtensionGuard:
    def __init__(self, enabled, unverified):
        self.enabled = _ids(enabled)
        self.unverified = _ids(unverified)

    def find_spec(self, fullname, path=None, target=None):
        parts = fullname.split(".")
        if len(parts) >= 2 and parts[0] == "extensions" and parts[1].replace("-", "_") not in self.enabled:
            if parts[1].replace("-", "_") in self.unverified:
                raise ModuleNotFoundError(
                    "saba-chan extension '%s' is not verified, so a verified component cannot use it" % parts[1],
                    name=fullname,
                )
            raise ModuleNotFoundError(
                "saba-chan extension '%s' is not enabled" % parts[1], name=fullname
            )
        return None


if _ENABLED is not None:
    sys.meta_path.insert(0, _ExtensionGuard(_ENABLED, _UNVERIFIED))
