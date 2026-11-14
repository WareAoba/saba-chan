import clsx from 'clsx';
import { type ReactNode, useId } from 'react';
import { useTranslation } from 'react-i18next';
import type { SettingField } from '../types/daemon';
import { CustomDropdown, Icon, SabaCheckbox, SabaToggle } from './index';
import { SabaNumberInput } from './ui/SabaUI';
import { FindKeyTag } from './servers/PageFind';
import './ServerSettings.css';

/*
 * 서버 설정 화면의 공용 조각: 설정 한 줄(SettingRow), 모듈 설정 칸(SettingsField), 묶음 카드(Card).
 * 서버 상세 설정·사바쨩 탭(ServerSettingsModal, SabaBehaviorCards)과 서버 추가 창이 함께 쓴다.
 */


// 데몬이 받는 설정 type (module_loader SETTING_TYPES, 스키마 2의 list·multiline). 밖의 값은 글자 칸으로 그린다
const FIELD_TYPES = ['text', 'password', 'number', 'file', 'select', 'boolean', 'list', 'multiline'];
/** 줄 아래 전체 폭으로 그리는 칸 */
const STACKED_TYPES = ['multiline', 'list'];

/** 값 비교용 글자 (목록은 빈 항목을 빼고, 불리언·숫자는 글자로) */
export const valueKey = (v: unknown) => (Array.isArray(v) ? JSON.stringify(v.map(String).filter((x) => x.trim())) : v == null ? '' : String(v));

/** list 칸의 편집 중 값 (문자열 배열, 빈 줄 포함). 저장할 때 빈 항목을 뺀다 */
const listDraft = (value: unknown): string[] => (Array.isArray(value) ? value.map(String) : value ? String(value).split(',') : []);

/** 설정 이름 → 값 (모듈 설정은 모듈마다 모양이 달라 칸마다 좁혀 읽는다) */
// biome-ignore lint/suspicious/noExplicitAny: 모듈이 선언한 설정 값은 종류가 제각각이다
export type SettingValues = Record<string, any>;

/** checkPortConflicts·validateAllSettings가 돌려주는 칸별 오류 */
export interface SettingValidationError {
    errorType: string;
    min?: number;
    max?: number;
    value?: unknown;
}

/** 검사 오류를 사람이 읽는 글로 */
function validationText(t: ReturnType<typeof useTranslation>['t'], fieldLabel: string, e: SettingValidationError) {
    switch (e.errorType) {
        case 'required':
            return t('validation.required', { field: fieldLabel, defaultValue: `${fieldLabel} is required` });
        case 'type_mismatch':
            return t('validation.type_mismatch', { field: fieldLabel, defaultValue: `${fieldLabel} has an invalid type` });
        case 'out_of_range':
            return t('validation.out_of_range', {
                field: fieldLabel,
                min: e.min,
                max: e.max,
                value: e.value,
                defaultValue: `${fieldLabel} is out of range (${e.min ?? '−∞'} ~ ${e.max ?? '∞'})`,
            });
        case 'item_count':
            return t('validation.item_count', { field: fieldLabel, min: e.min ?? 0, max: e.max ?? '∞' });
        case 'too_long':
            return t('validation.too_long', { field: fieldLabel, max: e.max });
        case 'duplicate_item':
            return t('validation.duplicate_item', { field: fieldLabel, value: e.value });
        case 'invalid_option':
            return t('validation.invalid_option', {
                field: fieldLabel,
                value: e.value,
                defaultValue: `${fieldLabel}: '${e.value}' is not a valid option`,
            });
        case 'invalid':
            return t('validation.invalid', { field: fieldLabel });
        default:
            return null;
    }
}

/** 설정 한 줄: 이름·설명 왼쪽, 입력은 오른쪽 끝(또는 stack이면 아래 전체 폭). 서버 추가 창도 쓴다 */
export function SettingRow({
    label,
    labelId,
    htmlFor,
    desc,
    control,
    stack,
    disabled,
    error,
    className,
    findKey,
    findLabel,
}: {
    label: ReactNode;
    labelId?: string;
    htmlFor?: string;
    desc?: ReactNode;
    control: ReactNode;
    stack?: boolean;
    disabled?: boolean;
    error?: ReactNode;
    className?: string;
    /** 원문 키 (Ctrl+F 찾기가 이 키로도 찾는다: 설정 이름, 데몬 설정 키) */
    findKey?: string;
    /** 이름표 글자 (label이 글자가 아닐 때). 이름에 이미 맞으면 키 꼬리표를 띄우지 않는다 */
    findLabel?: string;
}) {
    return (
        <div className={clsx('ss-row', className, { 'ss-stack': stack, 'ss-disabled': disabled })}>
            <label className="ss-name" id={labelId} htmlFor={htmlFor} data-find="name">
                {label}
                <FindKeyTag findKey={findKey} label={findLabel ?? (typeof label === 'string' ? label : '')} />
            </label>
            {desc && <p className="ss-desc">{desc}</p>}
            <div className="ss-control">{control}</div>
            {error && (
                <p className="dc-field-error ss-error">
                    <Icon name="alertTriangle" size="xs" /> {error}
                </p>
            )}
        </div>
    );
}

interface SettingsFieldProps {
    field: SettingField;
    // biome-ignore lint/suspicious/noExplicitAny: 칸 종류에 따라 문자열·불리언·배열이 온다
    value: any;
    modNs: string;
    // biome-ignore lint/suspicious/noExplicitAny: 칸 종류에 따라 문자열·불리언·배열을 올린다
    onChange: (name: string, value: any) => void;
    validationError?: SettingValidationError;
    /** 화면을 연 뒤 값이 바뀌었는가 (다시 시작해야 적용되는 칸이면 그렇다고 표시한다) */
    changed?: boolean;
    /** 줄에 더할 class (서버 추가 창은 바꾼 줄을 표시한다) */
    className?: string;
}

/** 모듈이 선언한 설정 칸 하나 (field_type에 맞는 입력) */
export function SettingsField({ field, value, modNs, onChange, validationError, changed, className }: SettingsFieldProps) {
    const { t } = useTranslation('gui');
    // 이름표를 입력 칸에 잇는다 (화면 낭독기, 이름표 눌러 칸으로 가기)
    const inputId = useId();
    const fieldLabel = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const fieldDesc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const invalid = !!validationError;
    const fieldType = FIELD_TYPES.includes(field.field_type) ? field.field_type : 'text';
    const labelId = `${inputId}-label`;
    const picked = listDraft(value);
    // 선택지 표시 이름 (모듈 locales `settings.<칸>.options.<값>`, 없으면 값 그대로)
    // 값에 `:`가 있어도(`minecraft:normal`) 네임스페이스로 읽지 않게 네임스페이스를 따로 넘긴다
    const optionText = (opt: string) =>
        t(`settings.${field.name}.options.${opt}`, { ns: modNs, nsSeparator: false, defaultValue: String(opt) });
    const textClass = 'input';

    let control: ReactNode = null;
    if (fieldType === 'text' || fieldType === 'file' || fieldType === 'password') {
        control = (
            <input
                id={inputId}
                type={fieldType === 'password' ? 'password' : 'text'}
                className={clsx(textClass, 'ss-text')}
                aria-invalid={invalid || undefined}
                value={String(value || '')}
                onChange={(e) => onChange(field.name, e.target.value)}
            />
        );
    } else if (fieldType === 'number') {
        control = (
            <SabaNumberInput
                id={inputId}
                className={clsx(textClass, 'ss-num')}
                aria-invalid={invalid || undefined}
                value={String(value || '')}
                onChange={(e) => onChange(field.name, e.target.value)}
                min={field.min ?? undefined}
                max={field.max ?? undefined}
                step={field.step || (field.min != null && !Number.isInteger(field.min) ? 'any' : undefined)}
            />
        );
    } else if (fieldType === 'select') {
        control = (
            <CustomDropdown
                id={inputId}
                className="ss-select"
                value={String(value || '')}
                onChange={(val) => onChange(field.name, val)}
                placeholder={fieldLabel}
                options={(field.options || []).map((opt) => ({ value: opt, label: optionText(opt) }))}
            />
        );
    } else if (fieldType === 'boolean') {
        // 스위치 옆에 화면 언어의 켜짐·꺼짐 (사용자 11-06 #5)
        const on = value === true || value === 'true';
        control = (
            <span className="ss-inline">
                <span className="ss-toggle-state">{t(on ? 'server_settings.toggle_on' : 'server_settings.toggle_off')}</span>
                <SabaToggle id={inputId} checked={on} onChange={(checked) => onChange(field.name, checked)} />
            </span>
        );
    } else if (fieldType === 'multiline') {
        control = (
            <textarea
                id={inputId}
                className={clsx(textClass, 'ss-textarea')}
                aria-invalid={invalid || undefined}
                value={String(value ?? '')}
                onChange={(e) => onChange(field.name, e.target.value)}
                rows={4}
            />
        );
    } else if ((field.options?.length ?? 0) > 0) {
        // 선택지가 있는 목록: 고른 것을 선택지 순서로
        control = (
            <fieldset id={inputId} aria-labelledby={labelId} className="ss-options">
                {field.options!.map((opt, i) => (
                    <SabaCheckbox
                        key={opt}
                        id={`${inputId}-${i}`}
                        checked={picked.includes(opt)}
                        onChange={(on) => onChange(field.name, field.options!.filter((o) => (o === opt ? on : picked.includes(o))))}
                    >
                        {optionText(opt)}
                    </SabaCheckbox>
                ))}
            </fieldset>
        );
    } else {
        // 선택지가 없는 목록: 한 줄에 하나씩
        control = (
            <textarea
                id={inputId}
                className={clsx(textClass, 'ss-textarea')}
                aria-invalid={invalid || undefined}
                value={picked.join('\n')}
                onChange={(e) => onChange(field.name, e.target.value.split('\n'))}
                rows={4}
                placeholder={t('validation.list_hint')}
            />
        );
    }

    return (
        <SettingRow
            label={
                <>
                    {fieldLabel}
                    {field.required ? ' *' : ''}
                    {/* 서버 설정은 다시 시작해야 적용되는 게 보통이라, 바꾼 칸에만 표시한다 (사용자 11-07) */}
                    {changed && field.restart_required && <span className="ss-chip">{t('server_settings.restart_badge')}</span>}
                </>
            }
            labelId={labelId}
            findKey={field.name}
            findLabel={fieldLabel}
            htmlFor={fieldType === 'list' && (field.options?.length ?? 0) > 0 ? undefined : inputId}
            desc={fieldDesc}
            control={control}
            stack={STACKED_TYPES.includes(fieldType)}
            error={validationError ? validationText(t, fieldLabel, validationError) : null}
            className={className}
        />
    );
}

/** 설정 묶음 카드. 서버 추가 창도 쓴다 */
export function Card({
    icon,
    title,
    desc,
    className,
    children,
    head,
    spot,
}: {
    icon: string;
    title: ReactNode;
    desc?: ReactNode;
    className?: string;
    children?: ReactNode;
    /** 머리를 직접 그릴 때 (접는 묶음) */
    head?: ReactNode;
    /** 가이드가 짚는 자리 이름 (화면 강조) */
    spot?: string;
}) {
    return (
        <section className={clsx('dc-card ss-card', className)} data-spot={spot}>
            {head ?? (
                <div className="dc-card-head">
                    <h2 className="ss-card-title" data-find="name">
                        <Icon name={icon} size="sm" /> {title}
                    </h2>
                </div>
            )}
            {desc && <p className="dc-card-desc">{desc}</p>}
            {children && <div className="ss-rows">{children}</div>}
        </section>
    );
}
