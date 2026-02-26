"""Tests for extensions.ue4_ini package structure (post-migration).

These tests verify:
1. The ue4_ini package can be imported from its new location
2. parse_option_settings / write_option_settings are accessible
3. The manifest.json is valid
4. All original test cases still pass via the new import path
"""
import json
import os
import sys
import tempfile

# Add project root to path for imports
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..'))


# ── Package structure tests ──────────────────────────────────

def test_package_import():
    """Can import from the package."""
    from extensions.ue4_ini import parse_option_settings, write_option_settings
    assert parse_option_settings is not None
    assert write_option_settings is not None


def test_direct_module_import():
    """Can import directly from the inner module."""
    from extensions.ue4_ini.ue4_ini import parse_option_settings, write_option_settings
    assert parse_option_settings is not None
    assert write_option_settings is not None


def test_manifest_exists_and_valid():
    """manifest.json exists and has required fields."""
    manifest_path = os.path.join(os.path.dirname(__file__), 'manifest.json')
    assert os.path.isfile(manifest_path), f"manifest.json not found at {manifest_path}"

    with open(manifest_path, 'r', encoding='utf-8') as f:
        manifest = json.load(f)

    assert manifest["id"] == "ue4-ini"
    assert "version" in manifest
    assert "description" in manifest
    assert manifest.get("dependencies") is not None


# ── Original functional tests (via new import path) ──────────

def test_parse_simple_key_values():
    """Basic key=value parsing."""
    from extensions.ue4_ini import parse_option_settings
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[/Script/Pal.PalGameWorldSettings]\n")
        f.write("OptionSettings=(Difficulty=2,DayTimeSpeedRate=1.000000,NightTimeSpeedRate=1.000000)\n")
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props["Difficulty"] == "2"
        assert props["DayTimeSpeedRate"] == "1.000000"
        assert props["NightTimeSpeedRate"] == "1.000000"
    finally:
        os.unlink(path)


def test_parse_quoted_values():
    """Values with commas inside quotes."""
    from extensions.ue4_ini import parse_option_settings
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[/Script/Pal.PalGameWorldSettings]\n")
        f.write('OptionSettings=(ServerName="My Server, The Best",Port=8211)\n')
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props["ServerName"] == "My Server, The Best"
        assert props["Port"] == "8211"
    finally:
        os.unlink(path)


def test_parse_parenthesized_values():
    """Values with nested parentheses."""
    from extensions.ue4_ini import parse_option_settings
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[/Script/Pal.PalGameWorldSettings]\n")
        f.write("OptionSettings=(Platforms=(Steam,Xbox),Port=8211)\n")
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props["Platforms"] == "(Steam,Xbox)"
        assert props["Port"] == "8211"
    finally:
        os.unlink(path)


def test_parse_missing_file():
    """Non-existent file returns empty dict."""
    from extensions.ue4_ini import parse_option_settings
    props = parse_option_settings("/nonexistent/file.ini")
    assert props == {}


def test_parse_no_option_settings():
    """File without OptionSettings."""
    from extensions.ue4_ini import parse_option_settings
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[SomeSection]\nKey=Value\n")
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props == {}
    finally:
        os.unlink(path)


def test_write_and_read_roundtrip():
    """Write then read preserves key-value pairs."""
    from extensions.ue4_ini import parse_option_settings, write_option_settings
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "sub", "PalWorldSettings.ini")
        original = {"Difficulty": "2", "ServerName": "Test", "Port": "8211"}

        ok = write_option_settings(path, original)
        assert ok is True
        assert os.path.isfile(path)

        result = parse_option_settings(path)
        assert result["Difficulty"] == "2"
        assert result["ServerName"] == "Test"
        assert result["Port"] == "8211"


def test_write_quoted_values():
    """Values with special chars get quoted on write."""
    from extensions.ue4_ini import write_option_settings
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "settings.ini")
        props = {"ServerName": "My Server, The Best", "Port": "8211"}

        write_option_settings(path, props)

        with open(path, 'r') as f:
            content = f.read()
        assert 'ServerName="My Server, The Best"' in content


def test_write_custom_section():
    """Custom section header is used."""
    from extensions.ue4_ini import write_option_settings
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "settings.ini")
        props = {"Key": "Value"}

        write_option_settings(path, props, section="[/Custom/Section]")

        with open(path, 'r') as f:
            content = f.read()
        assert "[/Custom/Section]" in content


def test_parse_boolean_values():
    """Boolean-like values preserved as strings."""
    from extensions.ue4_ini import parse_option_settings
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[/Script/Pal.PalGameWorldSettings]\n")
        f.write("OptionSettings=(RESTAPIEnabled=True,RCONEnabled=False,bIsMultiplay=True)\n")
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props["RESTAPIEnabled"] == "True"
        assert props["RCONEnabled"] == "False"
        assert props["bIsMultiplay"] == "True"
    finally:
        os.unlink(path)


if __name__ == "__main__":
    tests = [
        test_package_import,
        test_direct_module_import,
        test_manifest_exists_and_valid,
        test_parse_simple_key_values,
        test_parse_quoted_values,
        test_parse_parenthesized_values,
        test_parse_missing_file,
        test_parse_no_option_settings,
        test_write_and_read_roundtrip,
        test_write_quoted_values,
        test_write_custom_section,
        test_parse_boolean_values,
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
