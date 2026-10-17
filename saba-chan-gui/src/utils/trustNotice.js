import { safeShowToast } from './helpers';

/**
 * 만들기·켜기 응답의 warnings[{code, module|extension}] (미검증·변경된 구성요소)를 알림으로 띄운다.
 * 알림일 뿐 막지 않는다. warnings가 없거나 모르는 code면 아무것도 하지 않는다.
 */
export function showTrustWarnings(result, t) {
    const warnings = Array.isArray(result?.warnings) ? result.warnings : [];
    for (const w of warnings) {
        if (typeof w?.code !== 'string') continue;
        const text = t(`trust.notice.${w.code}`, { id: w.module || w.extension || '?', defaultValue: '' });
        safeShowToast(text, 'warning', 8000);
    }
}

/** 확인되지 않은 구성요소라서 데몬이 시작을 막은 이유 (action_required) */
const START_BLOCKED = ['malware_detected', 'launch_outside_instance'];

/** 시작 응답이 위 이유로 막혔으면 알릴 문구, 아니면 null (모르는 코드는 원래 흐름대로) */
export function startBlockedText(result, t) {
    const code = result?.action_required;
    if (!START_BLOCKED.includes(code)) return null;
    return t(`trust.start_blocked.${code}`, { file: result.file || '?', defaultValue: '' }) || null;
}
