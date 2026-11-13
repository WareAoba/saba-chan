import type { JsonObject, SettingField } from '../types/daemon';

/** 아직 저장하지 않은 설정값 (저장할 때와 같은 변환) */
export function launchDraft(fields: Pick<SettingField, 'name' | 'field_type'>[] | null | undefined, values: JsonObject | null | undefined) {
    const out: JsonObject = {};
    for (const f of fields || []) {
        const v = values?.[f.name];
        if (f.field_type === 'boolean') out[f.name] = v === true || v === 'true';
        else if (v === '' || v === null || v === undefined) continue;
        else out[f.name] = f.field_type === 'number' ? Number(v) : v;
    }
    return out;
}
