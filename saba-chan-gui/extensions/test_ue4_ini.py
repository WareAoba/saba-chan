"""Tests for extensions.ue4_ini module."""
import os
import sys
import tempfile

# Add project root to path for imports
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from extensions.ue4_ini import parse_option_settings, write_option_settings


def test_parse_simple_key_values():
    """Basic key=value parsing."""
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
    """Values with nested parentheses like CrossplayPlatforms=(Steam,Xbox)."""
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
    props = parse_option_settings("/nonexistent/file.ini")
    assert props == {}


def test_parse_no_option_settings():
    """File without OptionSettings line returns empty dict."""
    with tempfile.NamedTemporaryFile(mode='w', suffix='.ini', delete=False) as f:
        f.write("[SomeSection]\nKey=Value\n")
        path = f.name
    try:
        props = parse_option_settings(path)
        assert props == {}
    finally:
        os.unlink(path)


def test_write_and_read_roundtrip():
    """Write then read should preserve all key-value pairs."""
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
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "settings.ini")
        props = {"ServerName": "My Server, The Best", "Port": "8211"}
        
        write_option_settings(path, props)
        
        with open(path, 'r') as f:
            content = f.read()
        assert 'ServerName="My Server, The Best"' in content


def test_write_custom_section():
    """Custom section header is used."""
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "settings.ini")
        props = {"Key": "Value"}
        
        write_option_settings(path, props, section="[/Custom/Section]")
        
        with open(path, 'r') as f:
            content = f.read()
        assert "[/Custom/Section]" in content


def test_parse_boolean_values():
    """Boolean-like values are preserved as strings."""
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
