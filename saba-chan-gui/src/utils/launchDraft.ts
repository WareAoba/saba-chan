import type { ApiBody, JsonObject, SettingField } from '../types/daemon';

/**
 * 받은 실행 인자 미리보기 (서버 id + 설정 → 응답). 사바쨩 탭에 다시 들어와도 같은 설정이면 다시 계산하지 않고 바로 보인다.
 * 설정을 새로 열 때(다른 서버·버전 설치 뒤 등) 비운다 (useServerSettingsStore.handleOpenSettings)
 */
export const launchPreviews = new Map<string, ApiBody>();
export const forgetLaunchPreviews = () => launchPreviews.clear();

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
