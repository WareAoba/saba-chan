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
        if (!text) continue;
        // 격리된 환경(허용된 폴더만)에서 돈다면 한 줄 덧붙인다
        safeShowToast(w.isolated === true ? `${text}\n${t('trust.notice.isolated')}` : text, 'warning', 8000);
    }
}

/** 확인되지 않은 구성요소라서 데몬이 막은 이유 (action_required). 시작뿐 아니라 정지·설정 저장 등 모든 lifecycle 호출에서 올 수 있다 */
const START_BLOCKED = [
    'launch_outside_instance',
    'server_folder_label',
    'server_in_sync_folder',
    'sandbox_unavailable',
];

/** 응답이 위 이유로 막혔으면 알릴 문구, 아니면 null (모르는 코드는 원래 흐름대로) */
export function blockedText(result, t) {
    if (result?.error_code === 'isolation_unsupported') {
        return t('common:sandbox.isolation_unsupported.message');
    }
    // 활성 익스텐션 파일이 바뀌어 확인되지 않음: "설치하고 켜라"(extension_required)와 다르다
    if (result?.error_code === 'extension_not_verified' || result?.action_required === 'extension_not_verified') {
        const ids = Array.isArray(result.unverified_extensions) ? result.unverified_extensions : [];
        return t('trust.start_blocked.extension_not_verified', { extensions: ids.join(', ') || '?' });
    }
    const code = result?.action_required;
    if (!START_BLOCKED.includes(code)) return null;
    return (
        t(`trust.start_blocked.${code}`, {
            file: result.file || '?',
            component: result.component || '?',
            defaultValue: '',
        }) || null
    );
}

/** 격리를 준비하지 못해 막혔고, 격리 없이 돌리기 동의를 받을 수 있는지 (#43 preload가 있어야 한다) */
export const canAskUnisolated = (result) =>
    result?.action_required === 'sandbox_unavailable' &&
    typeof result.component === 'string' &&
    Boolean(window.api?.sandboxAllowUnisolated);
