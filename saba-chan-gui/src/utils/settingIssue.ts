import type { TFunction } from 'i18next';
import type { SettingField } from '../types/daemon';

// 설정 값 검사 오류를 사람이 읽는 글로 (서버 설정 칸 아래 오류와 서버 점검의 설정 문제가 같이 쓴다. 웹도 쓴다)

/** 칸 하나의 검사 오류 (validation.ts·데몬 validator의 error_type) */
export interface SettingError {
    errorType: string;
    min?: number;
    max?: number;
    value?: unknown;
}

/** 검사 오류 → 한 줄. 모르는 종류면 null */
export function validationText(t: TFunction, fieldLabel: string, e: SettingError) {
    switch (e.errorType) {
        case 'required':
            return t('gui:validation.required', { field: fieldLabel, defaultValue: `${fieldLabel} is required` });
        case 'type_mismatch':
        case 'invalid_format':
            return t('gui:validation.type_mismatch', { field: fieldLabel, defaultValue: `${fieldLabel} has an invalid type` });
        case 'out_of_range':
            return t('gui:validation.out_of_range', {
                field: fieldLabel,
                min: e.min ?? '−∞',
                max: e.max ?? '∞',
                value: e.value,
                defaultValue: `${fieldLabel} is out of range (${e.min ?? '−∞'} ~ ${e.max ?? '∞'})`,
            });
        case 'item_count':
            return t('gui:validation.item_count', { field: fieldLabel, min: e.min ?? 0, max: e.max ?? '∞' });
        case 'too_long':
            return t('gui:validation.too_long', { field: fieldLabel, max: e.max });
        case 'duplicate_item':
            return t('gui:validation.duplicate_item', { field: fieldLabel, value: e.value });
        case 'invalid_option':
            return t('gui:validation.invalid_option', {
                field: fieldLabel,
                value: e.value,
                defaultValue: `${fieldLabel}: '${e.value}' is not a valid option`,
            });
        default:
            return null;
    }
}

/** 서버 점검 결과 한 건 (모듈 validate·diagnose, 데몬이 더한 setting_invalid) */
export interface CheckIssue {
    code?: string;
    severity: string;
    message?: string;
    solution?: string;
    field?: string;
    error_type?: string;
    value?: unknown;
}

/**
 * 데몬이 더한 설정 값 문제(`setting_invalid`)를 칸 이름과 함께 번역한 글·해결 방법으로. 다른 문제는 모듈 문구 그대로
 */
export function checkIssueText(t: TFunction, issue: CheckIssue, moduleName: string | undefined, fields: SettingField[] = []) {
    if (issue.code !== 'setting_invalid' || !issue.field) {
        return { message: issue.message || issue.code || '', solution: issue.solution };
    }
    const field = fields.find((f) => f.name === issue.field);
    const label = t(`mod_${moduleName}:settings.${issue.field}.label`, { defaultValue: field?.label || issue.field });
    const message =
        validationText(t, label, { errorType: issue.error_type || '', min: field?.min ?? undefined, max: field?.max ?? undefined, value: issue.value }) ||
        issue.message ||
        issue.field;
    return { message, solution: t('gui:shell.check_setting_fix') };
}
