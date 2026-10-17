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
