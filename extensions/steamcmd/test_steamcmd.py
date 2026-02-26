"""Tests for extensions.steamcmd package structure.

These tests verify:
1. The steamcmd package can be imported from its new location
2. The SteamCMD class is accessible via the package
3. The manifest.json is valid
4. The plugin runner protocol still works
5. Status API returns expected structure
"""
import json
import os
import sys

# Add project root to path for imports
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..'))


def test_package_import():
    """Can import SteamCMD from the package."""
    from extensions.steamcmd import SteamCMD
    assert SteamCMD is not None


def test_direct_module_import():
    """Can import directly from the inner module."""
    from extensions.steamcmd.steamcmd import SteamCMD
    assert SteamCMD is not None


def test_class_instantiation():
    """SteamCMD object can be created with no arguments."""
    from extensions.steamcmd import SteamCMD
    steam = SteamCMD()
    assert hasattr(steam, 'available')
    assert hasattr(steam, 'path')


def test_status_structure():
    """status() returns the expected dict shape."""
    from extensions.steamcmd import SteamCMD
    steam = SteamCMD()
    status = steam.status()
    assert isinstance(status, dict)
    assert "available" in status
    assert "path" in status
    assert "portable_dir" in status
    assert "auto_bootstrap" in status
    assert isinstance(status["available"], bool)
    assert status["auto_bootstrap"] is True


def test_explicit_path_nonexistent():
    """Passing a nonexistent explicit path makes available=False."""
    from extensions.steamcmd import SteamCMD
    steam = SteamCMD(explicit_path="/nonexistent/steamcmd")
    assert steam.available is False


def test_manifest_exists_and_valid():
    """manifest.json exists and has required fields."""
    manifest_path = os.path.join(os.path.dirname(__file__), 'manifest.json')
    assert os.path.isfile(manifest_path), f"manifest.json not found at {manifest_path}"

    with open(manifest_path, 'r', encoding='utf-8') as f:
        manifest = json.load(f)

    assert manifest["id"] == "steamcmd"
    assert "version" in manifest
    assert "description" in manifest
    assert "hooks" in manifest or "python_modules" in manifest
    assert manifest.get("dependencies") is not None


def test_plugin_functions_mapping():
    """Plugin runner FUNCTIONS dict is intact."""
    from extensions.steamcmd.steamcmd import FUNCTIONS
    assert "ensure" in FUNCTIONS
    assert "install" in FUNCTIONS
    assert "update" in FUNCTIONS
    assert "status" in FUNCTIONS
    assert callable(FUNCTIONS["status"])


def test_plugin_status_via_functions():
    """Calling the status function via plugin protocol returns valid JSON."""
    from extensions.steamcmd.steamcmd import FUNCTIONS
    result = FUNCTIONS["status"]({})
    assert isinstance(result, dict)
    assert "available" in result


if __name__ == "__main__":
    tests = [
        test_package_import,
        test_direct_module_import,
        test_class_instantiation,
        test_status_structure,
        test_explicit_path_nonexistent,
        test_manifest_exists_and_valid,
        test_plugin_functions_mapping,
        test_plugin_status_via_functions,
    ]
    passed = 0
    failed = 0
    for test in tests:
        try:
            test()
            print(f"  PASS: {test.__name__}")
            passed += 1
        except Exception as e:
            print(f"  FAIL: {test.__name__}: {e}")
            failed += 1
    print(f"\n{passed} passed, {failed} failed")
    sys.exit(1 if failed > 0 else 0)
