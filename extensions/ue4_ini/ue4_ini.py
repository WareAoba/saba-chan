"""
saba-chan UE4 INI Extension
============================
Parse and write Unreal Engine 4 INI files with OptionSettings format.

Used by game modules that run on UE4 (Palworld, ARK, etc.).

Usage:
    from extensions.ue4_ini import parse_option_settings, write_option_settings

    props = parse_option_settings("/path/to/PalWorldSettings.ini")
    props["Difficulty"] = "2"
    write_option_settings("/path/to/PalWorldSettings.ini", props,
                          section="/Script/Pal.PalGameWorldSettings")
"""

import os
import re


def parse_option_settings(ini_path):
    """Parse UE4 INI OptionSettings into a dict.

    Handles the UE4 format:
      [/Script/Some.Settings]
      OptionSettings=(Key1=Val1,Key2=Val2,...)

    Supports:
    - Quoted string values with commas: Key="Value with, commas"
    - Parenthesized values: CrossplayPlatforms=(Steam,Xbox)

    Args:
        ini_path: Path to the .ini file.

    Returns:
        dict of key-value pairs, or empty dict if file missing/unparseable.
    """
    if not os.path.isfile(ini_path):
        return {}
    try:
        with open(ini_path, "r", encoding="utf-8") as f:
            content = f.read()
        m = re.search(r'OptionSettings=\((.+)\)', content)
        if not m:
            return {}
        raw = m.group(1)

        props = {}
        i = 0
        while i < len(raw):
            eq = raw.find("=", i)
            if eq == -1:
                break
            key = raw[i:eq].strip()
            i = eq + 1
            if i < len(raw) and raw[i] == '"':
                # Quoted value
                end_quote = raw.find('"', i + 1)
                if end_quote == -1:
                    value = raw[i + 1:]
                    i = len(raw)
                else:
                    value = raw[i + 1:end_quote]
                    i = end_quote + 1
                    if i < len(raw) and raw[i] == ',':
                        i += 1
            elif i < len(raw) and raw[i] == '(':
                # Parenthesized value like CrossplayPlatforms=(Steam,Xbox)
                depth = 1
                start = i
                i += 1
                while i < len(raw) and depth > 0:
                    if raw[i] == '(':
                        depth += 1
                    elif raw[i] == ')':
                        depth -= 1
                    i += 1
                value = raw[start:i]
                if i < len(raw) and raw[i] == ',':
                    i += 1
            else:
                comma = raw.find(",", i)
                if comma == -1:
                    value = raw[i:].strip()
                    i = len(raw)
                else:
                    value = raw[i:comma].strip()
                    i = comma + 1
            props[key] = value
        return props
    except OSError:
        return {}


def _should_quote(value):
    """Determine if a value needs quoting in UE4 INI OptionSettings.

    Unquoted: True/False, numeric literals, parenthesized groups.
    Quoted:   everything else (strings, passwords, URLs, etc.).
    """
    if not isinstance(value, str):
        return False
    if value.startswith("("):
        return False
    if value in ("True", "False"):
        return False
    try:
        float(value)
        return False
    except ValueError:
        pass
    return True


def write_option_settings(ini_path, props, section="[/Script/Pal.PalGameWorldSettings]"):
    """Write dict back to UE4 INI OptionSettings format.

    Args:
        ini_path: Path to the .ini file.
        props: dict of key-value pairs.
        section: INI section header (default: Palworld settings).

    Returns:
        True on success, False on OSError.
    """
    os.makedirs(os.path.dirname(ini_path), exist_ok=True)

    parts = []
    for key, value in props.items():
        # UE4 INI: string values must be quoted; booleans, numbers,
        # and parenthesized groups (e.g. CrossplayPlatforms=(Steam,Xbox))
        # stay unquoted.
        if isinstance(value, str) and _should_quote(value):
            parts.append(f'{key}="{value}"')
        else:
            parts.append(f"{key}={value}")

    content = (
        f"{section}\n"
        f"OptionSettings=({','.join(parts)})\n"
    )
    try:
        with open(ini_path, "w", encoding="utf-8") as f:
            f.write(content)
        return True
    except OSError:
        return False
