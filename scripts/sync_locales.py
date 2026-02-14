#!/usr/bin/env python3
import json
import os
import shutil
from pathlib import Path

ROOT = Path('locales')
REF_LANGS = ['en', 'ko', 'ja']
BACKUP_SUFFIX = '.bak'

def load_json(p):
    try:
        with open(p, 'r', encoding='utf-8') as f:
            return json.load(f)
    except FileNotFoundError:
        return None
    except Exception as e:
        print(f'Error loading {p}: {e}')
        return None

def save_json(p, data):
    tmp = str(p) + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False, indent=2, sort_keys=False)
        f.write('\n')
    shutil.move(tmp, p)

def merge_refs(refs):
    # Return a merged reference object choosing priority en > ko > ja
    for r in refs:
        if r is not None:
            return r
    return None

def ensure_keys(target, reference):
    """Recursively add missing keys from reference into target."""
    if reference is None:
        return False
    changed = False
    if isinstance(reference, dict):
        if target is None or not isinstance(target, dict):
            # replace completely
            return reference, True
        for k, v in reference.items():
            if k not in target:
                target[k] = v
                changed = True
            else:
                # recurse for nested dicts
                if isinstance(v, dict) and isinstance(target.get(k), dict):
                    sub_changed = ensure_keys(target[k], v)
                    if isinstance(sub_changed, tuple):
                        # replacement case
                        target[k] = sub_changed[0]
                        if sub_changed[1]:
                            changed = True
                    elif sub_changed:
                        changed = True
                # if types differ, do not overwrite existing translation
        return changed
    else:
        # reference is not a dict; if target missing, should have been handled by caller
        return False


def collect_files():
    langs = []
    if not ROOT.exists():
        print('locales folder not found')
        return langs
    for p in ROOT.iterdir():
        if p.is_dir():
            langs.append(p.name)
    return langs


def gather_filenames(langs):
    names = set()
    for lang in langs:
        d = ROOT / lang
        for f in d.glob('**/*.json'):
            rel = f.relative_to(d)
            names.add(str(rel).replace('\\\\','/'))
    return sorted(names)


def main():
    langs = collect_files()
    if not langs:
        return
    filenames = gather_filenames(langs)
    if not filenames:
        print('No json files found under locales')
        return

    updated = []
    created = []

    for name in filenames:
        # load reference versions in priority
        ref_objs = []
        for r in REF_LANGS:
            ref_path = ROOT / r / name
            ref_objs.append(load_json(ref_path))
        # merged reference is first non-None
        merged_ref = merge_refs(ref_objs)
        # if no reference exists for this filename at all, skip
        if merged_ref is None:
            continue

        for lang in langs:
            target_path = ROOT / lang / name
            target_exists = target_path.exists()
            target_obj = load_json(target_path) if target_exists else None

            if not target_exists:
                # create file using merged_ref
                target_path.parent.mkdir(parents=True, exist_ok=True)
                backup_path = target_path.with_suffix(target_path.suffix + BACKUP_SUFFIX)
                if target_path.exists():
                    shutil.copy2(target_path, backup_path)
                save_json(target_path, merged_ref)
                created.append(str(target_path))
                continue

            # both exist -> ensure keys
            changed_flag = False
            if isinstance(merged_ref, dict):
                res = ensure_keys(target_obj, merged_ref)
                if isinstance(res, tuple):
                    # replaced entirely
                    target_obj = res[0]
                    changed_flag = res[1]
                else:
                    changed_flag = bool(res)
            else:
                # merged_ref not a dict; if target missing or empty, set it
                if target_obj is None:
                    target_obj = merged_ref
                    changed_flag = True

            if changed_flag:
                # backup
                backup_path = target_path.with_suffix(target_path.suffix + BACKUP_SUFFIX)
                shutil.copy2(target_path, backup_path)
                save_json(target_path, target_obj)
                updated.append(str(target_path))

    print(f'Created files: {len(created)}')
    for c in created:
        print('  +', c)
    print(f'Updated files: {len(updated)}')
    for u in updated:
        print('  *', u)

if __name__ == '__main__':
    main()
