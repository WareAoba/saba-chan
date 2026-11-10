import clsx from 'clsx';
import React, { type ReactNode, useId, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { RestartPending } from '../hooks/useServerSettings';
import type { Server } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import type { ModuleInfo, SettingField } from '../types/daemon';
import { moduleSupports } from '../utils/helpers';
import { checkPortConflicts, hiddenFields, validateAllSettings } from '../utils/validation';
import { useExtensions } from '../contexts/ExtensionContext';
import { CliLine, changedForCli } from './CliLine';
import { CustomDropdown, ExtensionSlot, Icon, SabaCheckbox, SabaToggle } from './index';
import { LaunchPreview, launchPreviewAvailable } from './servers/LaunchPreview';
import { SabaNumberInput } from './ui/SabaUI';
import { LinkedText } from './ui/ScreenLink';
import { FindKeyTag } from './servers/PageFind';
import './ServerSettings.css';

/*
 * 서버 상세의 "설정"·"사바쨩" 탭 (옛 서버 설정 모달을 지금 화면 모양으로 다시 그린 것, 사용자 11-07).
 * 묶음마다 카드(dc-card) 하나, 설정 하나가 한 줄(ss-row): 이름·설명은 왼쪽, 짧은 입력은 오른쪽 끝,
 * 긴 입력(여러 줄·목록)은 줄 아래 전체 폭. 저장 막대는 아래에 붙는다.
 */

// 데몬이 받는 설정 type (module_loader SETTING_TYPES, 스키마 2의 list·multiline). 밖의 값은 글자 칸으로 그린다
const FIELD_TYPES = ['text', 'password', 'number', 'file', 'select', 'boolean', 'list', 'multiline'];
/** 줄 아래 전체 폭으로 그리는 칸 */
const STACKED_TYPES = ['multiline', 'list'];

/** 값 비교용 글자 (목록은 빈 항목을 빼고, 불리언·숫자는 글자로) */
const valueKey = (v: unknown) => (Array.isArray(v) ? JSON.stringify(v.map(String).filter((x) => x.trim())) : v == null ? '' : String(v));

/** list 칸의 편집 중 값 (문자열 배열, 빈 줄 포함). 저장할 때 빈 항목을 뺀다 */
const listDraft = (value: unknown): string[] => (Array.isArray(value) ? value.map(String) : value ? String(value).split(',') : []);

/** 설정 이름 → 값 (모듈 설정은 모듈마다 모양이 달라 칸마다 좁혀 읽는다) */
// biome-ignore lint/suspicious/noExplicitAny: 모듈이 선언한 설정 값은 종류가 제각각이다
export type SettingValues = Record<string, any>;

/** checkPortConflicts·validateAllSettings가 돌려주는 칸별 오류 */
interface SettingValidationError {
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
        default:
            return null;
    }
}

/** 설정 한 줄: 이름·설명 왼쪽, 입력은 오른쪽 끝(또는 stack이면 아래 전체 폭) */
function SettingRow({
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
}

/** 모듈이 선언한 설정 칸 하나 (field_type에 맞는 입력) */
export function SettingsField({ field, value, modNs, onChange, validationError, changed }: SettingsFieldProps) {
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
    const textClass = clsx('input', { 'setting-input-error': invalid });

    let control: ReactNode = null;
    if (fieldType === 'text' || fieldType === 'file' || fieldType === 'password') {
        control = (
            <input
                id={inputId}
                type={fieldType === 'password' ? 'password' : 'text'}
                className={clsx(textClass, 'ss-text')}
                value={String(value || '')}
                onChange={(e) => onChange(field.name, e.target.value)}
            />
        );
    } else if (fieldType === 'number') {
        control = (
            <SabaNumberInput
                id={inputId}
                className={clsx(textClass, 'ss-num')}
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
        />
    );
}

/** 프로토콜 이름은 모듈(미검증일 수 있음)이 준 값이라 HTML로 넣지 않고 글자로만 그린다 */
export function ProtocolNotice({ protocol }: { protocol: string }) {
    const { t } = useTranslation('gui');
    const MARK = '\u0001';
    const [before = '', after = ''] = t('server_settings.protocol_single_only', { protocol: MARK })
        .replace(/<\/?strong>/g, '')
        .split(MARK);
    return (
        <p className="ss-desc">
            {before}
            <strong>{String(protocol).toUpperCase()}</strong>
            {after}
        </p>
    );
}

/** 설정 묶음 카드 */
function Card({
    icon,
    title,
    desc,
    className,
    children,
    head,
}: {
    icon: string;
    title: ReactNode;
    desc?: ReactNode;
    className?: string;
    children?: ReactNode;
    /** 머리를 직접 그릴 때 (접는 묶음) */
    head?: ReactNode;
}) {
    return (
        <section className={clsx('dc-card ss-card', className)}>
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

/**
 * GeneralTab — 설정 묶음 카드들
 */
interface GeneralTabProps {
    settingsServer: Server;
    settingsValues: SettingValues;
    unsetSettings?: Set<string>;
    modules: ModuleInfo[];
    advancedExpanded: boolean;
    setAdvancedExpanded: (expanded: boolean) => void;
    versionsLoading?: boolean;
    availableVersions: { id?: string; version?: string; type?: string }[];
    versionInstalling?: boolean;
    handleSettingChange: (name: string, value: unknown) => void;
    handleInstallVersion: () => void;
    handleResetServer: () => void;
    resettingServer?: boolean;
    validationErrors?: Record<string, SettingValidationError>;
    portConflicts?: { port: number | string; conflictName: string }[];
    serverRunning?: boolean;
    section?: 'server' | 'saba';
    /** 화면을 열었을 때의 값 (바뀐 칸 표시용) */
    openedValues?: SettingValues;
}

function GeneralTab({
    settingsServer,
    settingsValues,
    unsetSettings,
    modules,
    advancedExpanded,
    setAdvancedExpanded,
    versionsLoading,
    availableVersions,
    versionInstalling,
    handleSettingChange,
    handleInstallVersion,
    handleResetServer,
    resettingServer,
    validationErrors,
    portConflicts,
    serverRunning,
    section,
    openedValues,
}: GeneralTabProps) {
    const { t } = useTranslation('gui');
    const versionId = useId();
    const rowId = useId();
    const advancedId = useId();
    // section: 'server' = 게임 서버 설정만, 'saba' = 사바쨩이 서버를 다루는 설정만, 없으면 모두
    const showSaba = section !== 'server';
    const showServer = section !== 'saba';
    const module = modules.find((m) => m.name === settingsServer.module);
    const hasModuleSettings = (module?.settings?.fields?.length ?? 0) > 0;
    const modNs = `mod_${settingsServer.module}`;
    // visible_when으로 숨긴 설정은 그리지 않는다 (값은 지우지 않는다)
    const hidden = new Set(hasModuleSettings ? hiddenFields(module!.settings!.fields, settingsValues, unsetSettings) : []);
    const fields = hasModuleSettings ? module!.settings!.fields.filter((f) => !hidden.has(f.name)) : [];
    const sabaFields = showSaba ? fields.filter((f) => f.group === 'saba-chan') : [];
    const basicFields = showServer ? fields.filter((f) => !f.group || f.group === 'basic') : [];
    const advancedFields = showServer ? fields.filter((f) => f.group === 'advanced') : [];
    const versionPicker =
        showSaba &&
        module?.install?.method === 'download' &&
        moduleSupports(module, 'list_versions') &&
        moduleSupports(module, 'install_server');
    const crashOff = String(settingsValues.auto_restart) === 'false';
    const fieldRows = (list: SettingField[]) =>
        list.map((f) => (
            <SettingsField
                key={f.name}
                field={f}
                value={settingsValues[f.name]}
                modNs={modNs}
                onChange={handleSettingChange}
                validationError={validationErrors?.[f.name]}
                changed={!!openedValues && valueKey(settingsValues[f.name]) !== valueKey(openedValues[f.name])}
            />
        ));

    return (
        <div className="ss-form">
            {/* 사바쨩 설정(managed 시작·정상 정지 같은 모듈 saba-chan 그룹, 버전 고르기): 실행 인자 바로 아래 맨 위 */}
            {(sabaFields.length > 0 || versionPicker) && (
                <Card icon="sabaFace" title={t('server_settings.saba_chan_group', { defaultValue: 'saba-chan Settings' })} className="ss-saba">
                    {versionPicker && (
                        <SettingRow
                            label={t('server_settings.server_version', { defaultValue: 'Server Version' })}
                            htmlFor={versionId}
                            desc={t('server_settings.version_description_install', {
                                defaultValue: 'Select a version and click Install to download the server files.',
                            })}
                            control={
                                versionsLoading ? (
                                    <span className="muted ss-loading">
                                        <Icon name="loader" size="sm" />{' '}
                                        {t('server_settings.loading_versions', { defaultValue: 'Loading versions...' })}
                                    </span>
                                ) : (
                                    <span className="ss-inline">
                                        <CustomDropdown
                                            id={versionId}
                                            className="ss-select"
                                            value={settingsValues.server_version || ''}
                                            onChange={(val) => handleSettingChange('server_version', val)}
                                            placeholder={t('server_settings.select_version', { defaultValue: 'Select version' })}
                                            options={availableVersions.map((v) => ({
                                                value: (v.id || v.version || v) as string,
                                                label: `${v.id || v.version || v}${v.type ? ` (${v.type})` : ''}`,
                                            }))}
                                        />
                                        <button
                                            type="button"
                                            className="btn btn-primary btn-sm ss-version-install"
                                            onClick={handleInstallVersion}
                                            disabled={!settingsValues.server_version || versionInstalling || serverRunning}
                                            data-tooltip={
                                                serverRunning
                                                    ? t('server_settings.install_server_running')
                                                    : t('server_settings.install_version_tooltip', {
                                                          defaultValue: 'Download and install this version',
                                                      })
                                            }
                                        >
                                            <Icon name={versionInstalling ? 'loader' : 'download'} size="sm" />{' '}
                                            {versionInstalling
                                                ? t('server_settings.installing', { defaultValue: 'Installing...' })
                                                : t('server_settings.install_version_button', { defaultValue: 'Install' })}
                                        </button>
                                    </span>
                                )
                            }
                        />
                    )}
                    {fieldRows(sabaFields)}
                </Card>
            )}

            {/* 크래시 자동 재시작 (데몬 A1: instance module_settings 의 auto_restart / restart_on_unknown_exit) */}
            {showSaba && (
                <Card icon="refresh" title={t('shell.restart_title')} desc={t('shell.restart_desc')} className="ss-restart">
                    <SettingRow
                        label={t('shell.restart_on_crash')}
                        findKey="auto_restart"
                        htmlFor={`${rowId}-crash`}
                        desc={t('shell.restart_on_crash_hint')}
                        control={
                            <SabaToggle
                                id={`${rowId}-crash`}
                                checked={!crashOff}
                                onChange={(checked) => handleSettingChange('auto_restart', checked)}
                            />
                        }
                    />
                    <SettingRow
                        label={t('shell.restart_on_unknown')}
                        findKey="restart_on_unknown_exit"
                        htmlFor={`${rowId}-unknown`}
                        desc={t('shell.restart_on_unknown_hint')}
                        disabled={crashOff}
                        control={
                            <SabaToggle
                                id={`${rowId}-unknown`}
                                checked={String(settingsValues.restart_on_unknown_exit) === 'true'}
                                disabled={crashOff}
                                onChange={(checked) => handleSettingChange('restart_on_unknown_exit', checked)}
                            />
                        }
                    />
                </Card>
            )}

            {/* 램 상한 (데몬 인스턴스 memory_limit_mb, MiB). 비우면 제한 없음, 다음 시작부터 적용 */}
            {showSaba && (
                <Card icon="monitor" title={t('memory_limit.title')} desc={t('memory_limit.desc')} className="ss-memory">
                    <SettingRow
                        label={t('memory_limit.label')}
                        findKey="memory_limit_mb"
                        htmlFor={`${rowId}-mem`}
                        desc={t('memory_limit.hint')}
                        control={
                            <input
                                id={`${rowId}-mem`}
                                type="text"
                                inputMode="numeric"
                                className="input ss-short"
                                value={settingsValues.memory_limit_mb ?? ''}
                                placeholder={t('memory_limit.placeholder')}
                                aria-label={t('memory_limit.label')}
                                onChange={(e) => handleSettingChange('memory_limit_mb', e.target.value)}
                            />
                        }
                    />
                    {settingsServer.status === 'running' &&
                        String(settingsValues.memory_limit_mb ?? '').trim() !==
                            (settingsServer.memory_limit_mb == null ? '' : String(settingsServer.memory_limit_mb)) && (
                            <p className="dc-hint">{t('memory_limit.restart_note')}</p>
                        )}
                </Card>
            )}

            {/* 게임 채팅 전달 (데몬 인스턴스 game_chat·game_chat_keyword). 모듈이 채팅 줄을 알려 주고 데몬이 콘솔을 읽을 때만 */}
            {showSaba && settingsServer.game_chat_supported && (
                <Card icon="messageSquare" title={t('game_chat.title')} desc={t('game_chat.desc')} className="ss-game-chat">
                    <SettingRow
                        label={t('game_chat.mode_label')}
                        findKey="game_chat"
                        control={
                            <CustomDropdown
                                className="ss-select"
                                ariaLabel={t('game_chat.mode_label')}
                                value={settingsValues.game_chat || 'off'}
                                onChange={(val) => handleSettingChange('game_chat', val)}
                                options={['off', 'all', 'keyword'].map((mode) => ({ value: mode, label: t(`game_chat.mode_${mode}`) }))}
                            />
                        }
                    />
                    {settingsValues.game_chat === 'keyword' && (
                        <SettingRow
                            label={t('game_chat.keyword_label')}
                            findKey="game_chat_keyword"
                            htmlFor={`${rowId}-keyword`}
                            desc={t('game_chat.keyword_hint')}
                            control={
                                <input
                                    id={`${rowId}-keyword`}
                                    type="text"
                                    className="input ss-short"
                                    maxLength={32}
                                    value={settingsValues.game_chat_keyword ?? ''}
                                    placeholder={t('game_chat.keyword_placeholder')}
                                    aria-label={t('game_chat.keyword_label')}
                                    onChange={(e) => handleSettingChange('game_chat_keyword', e.target.value)}
                                />
                            }
                        />
                    )}
                    <p className="dc-hint">
                        <LinkedText i18nKey="game_chat.channel_note" to="discord/*/moderation/channels" />
                    </p>
                </Card>
            )}

            {/* 정지 대기 시간 (데몬 MK14 module_settings.stop_timeout, 초). 비우면 모듈 값, 지나면 강제로 끈다.
                데몬이 띄우지 않는 서버(managed:false)는 모듈 stop()이 정지를 맡아 이 값을 쓰지 않는다 */}
            {showSaba && settingsServer.managed !== false && (
                <Card icon="power" title={t('stop_timeout.title')} desc={t('stop_timeout.desc')} className="ss-stop-timeout">
                    <SettingRow
                        label={t('stop_timeout.label')}
                        findKey="stop_timeout"
                        htmlFor={`${rowId}-stop`}
                        desc={t('stop_timeout.hint')}
                        control={
                            <input
                                id={`${rowId}-stop`}
                                type="text"
                                inputMode="numeric"
                                className="input ss-short"
                                value={settingsValues.stop_timeout ?? ''}
                                placeholder={t('stop_timeout.placeholder')}
                                aria-label={t('stop_timeout.label')}
                                onChange={(e) => handleSettingChange('stop_timeout', e.target.value)}
                            />
                        }
                    />
                </Card>
            )}

            {/* 포트 충돌 경고 */}
            {showServer && portConflicts && portConflicts.length > 0 && (
                <div className="notice tone-warn">
                    <Icon name="alertTriangle" size="sm" />
                    <div>
                        <strong>{t('errors.port_conflict')}</strong>
                        {portConflicts.map((c, i) => (
                            // biome-ignore lint/suspicious/noArrayIndexKey: 검사할 때마다 새로 만드는 읽기 전용 목록
                            <div key={i}>{t('errors.port_conflict_detail', { port: c.port, name: c.conflictName })}</div>
                        ))}
                    </div>
                </div>
            )}

            {basicFields.length > 0 && (
                <Card icon="gamepad" title={t('server_settings.basic_group', { defaultValue: 'Server Settings' })} className="ss-basic">
                    {fieldRows(basicFields)}
                </Card>
            )}

            {/* 고급 설정: 머리를 눌러 접고 편다 */}
            {advancedFields.length > 0 && (
                <Card
                    icon="settings"
                    title=""
                    className="ss-advanced"
                    head={
                        <button
                            type="button"
                            className="ss-card-toggle"
                            aria-expanded={advancedExpanded}
                            aria-controls={advancedId}
                            onClick={() => setAdvancedExpanded(!advancedExpanded)}
                        >
                            <h2 className="ss-card-title" data-find="name">
                                <Icon name="tool" size="sm" />{' '}
                                {t('server_settings.advanced_group', { defaultValue: 'Advanced Settings' })}
                                <span className="ss-count">{advancedFields.length}</span>
                            </h2>
                            <Icon name={advancedExpanded ? 'chevronUp' : 'chevronDown'} size="sm" />
                        </button>
                    }
                >
                    {/* 접혀 있어도 그려 둔다: Ctrl+F가 안을 세고, 그 결과로 가면 편다 */}
                    <div id={advancedId} className="ss-fold" data-find-fold hidden={!advancedExpanded}>
                        {fieldRows(advancedFields)}
                    </div>
                </Card>
            )}

            {/* 위험 구역: 초기화를 구현한 모듈만 (MK1). 무엇을 지우는지는 모듈 locales의 reset.label·description이 먼저 */}
            {showServer && moduleSupports(module, 'reset_server') && (
                <Card icon="alertTriangle" title={t('server_settings.danger_zone', { defaultValue: 'Danger Zone' })} className="ss-danger">
                    <SettingRow
                        label={t(`mod_${settingsServer.module}:reset.label`, { defaultValue: t('server_settings.reset_server_label') })}
                        desc={t(`mod_${settingsServer.module}:reset.description`, { defaultValue: t('server_settings.reset_server_desc') })}
                        control={
                            <button
                                type="button"
                                className="btn btn-ghost btn-danger-text ss-reset"
                                onClick={handleResetServer}
                                disabled={resettingServer || serverRunning}
                                data-tooltip={serverRunning ? t('server_settings.reset_server_running') : undefined}
                            >
                                <Icon name="trash" size="sm" />{' '}
                                {resettingServer
                                    ? t('server_settings.resetting', { defaultValue: 'Resetting...' })
                                    : t('server_settings.reset_button', { defaultValue: 'Reset' })}
                            </button>
                        }
                    />
                </Card>
            )}

            {showServer && !basicFields.length && !advancedFields.length && (section === 'server' || !hasModuleSettings) && (
                <div className="empty-inline">
                    <Icon name="settings" size="lg" />
                    <p>{t('server_settings.no_settings')}</p>
                </div>
            )}
        </div>
    );
}

/** 저장 막대 위의 다시 시작 안내 (사용자 11-06 #5): [지금 다시 시작] [나중에] */
export function RestartNotice({
    pending,
    server,
    onRestart,
    onDismiss,
}: {
    pending: RestartPending | null | undefined;
    server: Pick<Server, 'id' | 'status' | 'start_time'>;
    onRestart: () => void;
    onDismiss: () => void;
}) {
    const { t } = useTranslation('gui');
    const live = pending && pending.serverId === server.id && server.status === 'running' && (server.start_time ?? null) === pending.startTime;
    if (!live) return null;
    return (
        <div className="notice tone-warn settings-restart-notice">
            <Icon name="refresh" size="sm" />
            <span role="status">
                {t('server_settings.restart_pending', { count: pending.fields.length, fields: pending.fields.join(', ') })}
            </span>
            <button type="button" className="btn btn-primary btn-sm" onClick={onRestart}>
                {t('server_settings.restart_now')}
            </button>
            <button type="button" className="btn btn-ghost btn-sm" onClick={onDismiss}>
                {t('server_settings.restart_later')}
            </button>
        </div>
    );
}

/**
 * ServerSettingsModal — 서버 상세의 설정·사바쨩 탭 내용 (이름은 옛 모달에서 이어 온 것. 이제 겹쳐 뜨는 창은 없다)
 */
interface ServerSettingsModalProps extends Omit<GeneralTabProps, 'portConflicts' | 'validationErrors' | 'serverRunning'> {
    settingsSource?: string;
    settingsActiveTab: string;
    setSettingsActiveTab: (tab: string) => void;
    handleSaveSettings: () => void;
    savingSettings?: boolean;
    servers?: Server[];
    restartPending?: RestartPending | null;
    restartNow?: () => void;
    dismissRestart?: () => void;
}

export function ServerSettingsModal({
    settingsServer,
    settingsValues,
    unsetSettings,
    settingsSource,
    settingsActiveTab,
    setSettingsActiveTab,
    modules,
    advancedExpanded,
    setAdvancedExpanded,
    availableVersions,
    versionsLoading,
    versionInstalling,
    handleSettingChange,
    handleInstallVersion,
    handleSaveSettings,
    handleResetServer,
    resettingServer,
    savingSettings,
    restartPending,
    restartNow,
    dismissRestart,
    // 서버 상세에 나눠 보이기: 'server' | 'saba' (GeneralTab 참고)
    section,
    // validation props
    servers,
}: ServerSettingsModalProps) {
    const { t } = useTranslation('gui');
    // 익스텐션이 더한 사바쨩 쪽 탭 수 (없으면 탭 머리를 그리지 않는다)
    const { slots } = useExtensions();
    const extensionTabs = ((slots as Record<string, unknown[]>)['ServerSettings.tab'] || []).length;

    // "CLI로 보기"에 덧붙일 값의 기준: 이 화면을 열었을 때의 설정
    const openedWith = useRef<{ id: string | null; values: SettingValues }>({ id: null, values: {} });
    if (openedWith.current.id !== settingsServer.id && Object.keys(settingsValues).length) {
        openedWith.current = { id: settingsServer.id, values: settingsValues };
    }

    // 화면을 연 뒤에 켜질 수 있으니 지금 상태는 servers 목록에서 본다 (데몬도 켜진 서버의 설치·초기화를 거절한다)
    const liveServer = (servers || []).find((s) => s.id === settingsServer.id) ?? settingsServer;
    const serverRunning = ['running', 'starting', 'stopping'].includes(liveServer.status);

    // ── Validation: 설정값 타입 검증 ──
    const module = modules.find((m) => m.name === settingsServer.module);
    const changedSettings = changedForCli(settingsValues, openedWith.current.values, module?.settings?.fields);
    const { errors: validationErrors } = React.useMemo(() => {
        if (!module?.settings?.fields) return { valid: true, errors: {} };
        return validateAllSettings(module.settings.fields, settingsValues, unsetSettings) as {
            valid: boolean;
            errors: Record<string, SettingValidationError>;
        };
    }, [module, settingsValues, unsetSettings]);

    // ── Validation: 포트 충돌 검사 (모듈 프로토콜 인지) ──
    const moduleProtocols = React.useMemo(() => {
        const map: Record<string, string[]> = {};
        for (const m of modules) {
            if (m.protocols?.supported) map[m.name] = m.protocols.supported;
        }
        return map;
    }, [modules]);

    const portConflicts = React.useMemo(() => {
        if (!useSettingsStore.getState().portConflictCheck) return [];
        if (!settingsServer || !servers) return [];
        const targetPorts = {
            port: settingsValues.port ?? settingsServer.port,
            rest_port: settingsValues.rest_port ?? settingsServer.rest_port,
        };
        return checkPortConflicts(settingsServer.id, targetPorts, servers, moduleProtocols, settingsServer.module);
    }, [settingsServer, settingsValues, servers, moduleProtocols]);

    return (
        <div className="ss-panel">
            {/* 탭 머리: 익스텐션이 사바쨩 쪽에 탭을 더할 때만 (탭이 하나면 "일반 설정" 단추는 쓸모가 없다) */}
            {section !== 'server' && extensionTabs > 0 && (
                <div className="ss-tabs" role="tablist">
                    <button
                        type="button"
                        role="tab"
                        aria-selected={settingsActiveTab === 'general'}
                        className={clsx('ss-tab', { active: settingsActiveTab === 'general' })}
                        onClick={() => setSettingsActiveTab('general')}
                    >
                        <Icon name="gamepad" size="sm" /> {t('server_settings.general_tab')}
                    </button>
                </div>
            )}

            {section !== 'server' && settingsActiveTab === 'general' && launchPreviewAvailable() && (
                <LaunchPreview
                    key={settingsServer.id}
                    serverId={settingsServer.id}
                    modNs={`mod_${settingsServer.module}`}
                    fields={module?.settings?.fields}
                    values={settingsValues}
                    extraArgs={settingsValues._launch_extra_args}
                    onExtraArgsChange={(v: unknown) => handleSettingChange('_launch_extra_args', v)}
                    onExtraArgsLoaded={(v: unknown) => handleSettingChange('_launch_extra_args', v)}
                    onSettingsFromArgs={(found: Record<string, unknown>) => {
                        for (const [k, v] of Object.entries(found)) handleSettingChange(k, v);
                    }}
                />
            )}
            {section !== 'saba' && settingsSource === 'stored' && (
                <div className="notice tone-warn">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('server_settings.stored_values_notice')}</span>
                </div>
            )}
            {(section === 'server' || settingsActiveTab === 'general') && (
                <GeneralTab
                    section={section}
                    settingsServer={settingsServer}
                    settingsValues={settingsValues}
                    unsetSettings={unsetSettings}
                    modules={modules}
                    advancedExpanded={advancedExpanded}
                    setAdvancedExpanded={setAdvancedExpanded}
                    versionsLoading={versionsLoading}
                    availableVersions={availableVersions}
                    versionInstalling={versionInstalling}
                    handleSettingChange={handleSettingChange}
                    handleInstallVersion={handleInstallVersion}
                    handleResetServer={handleResetServer}
                    resettingServer={resettingServer}
                    validationErrors={validationErrors}
                    openedValues={openedWith.current.values}
                    portConflicts={portConflicts}
                    serverRunning={serverRunning}
                />
            )}
            {section !== 'server' && (
                <ExtensionSlot
                    slotId="ServerSettings.tab"
                    server={settingsServer}
                    activeTab={settingsActiveTab}
                    setActiveTab={setSettingsActiveTab}
                    settings={settingsValues}
                    onSettingsChange={handleSettingChange}
                    t={t}
                />
            )}

            {/* 저장 막대: 칸 아래에 붙어 따라온다. 다시 시작 안내가 그 위에 */}
            <div className="ss-footer">
                {restartNow && dismissRestart && (
                    <RestartNotice pending={restartPending} server={liveServer} onRestart={restartNow} onDismiss={dismissRestart} />
                )}
                <div className="dc-savebar ss-savebar">
                    <CliLine actionId="instance.settings.set" server={settingsServer.name} extra={changedSettings} />
                    <button type="button" className="btn btn-primary" onClick={handleSaveSettings} disabled={savingSettings}>
                        <Icon name="save" size="sm" /> {t('server_settings.save_settings')}
                    </button>
                </div>
            </div>
        </div>
    );
}
