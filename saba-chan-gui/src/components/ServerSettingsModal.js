import clsx from 'clsx';
import React, { useCallback, useEffect, useId, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { moduleSupports } from '../utils/helpers';
import { checkPortConflicts, hiddenFields, validateAllSettings } from '../utils/validation';
import { useSettingsStore } from '../stores/useSettingsStore';
import { CliLine, changedForCli } from './CliLine';
import { CustomDropdown, ExtensionSlot, Icon, SabaCheckbox, SabaToggle } from './index';
import { LaunchPreview, launchPreviewAvailable } from './servers/LaunchPreview';

// 데몬이 받는 설정 type (module_loader SETTING_TYPES, 스키마 2의 list·multiline). 밖의 값은 글자 칸으로 그린다
const FIELD_TYPES = ['text', 'password', 'number', 'file', 'select', 'boolean', 'list', 'multiline'];

/** list 칸의 편집 중 값 (문자열 배열, 빈 줄 포함). 저장할 때 빈 항목을 뺀다 */
const listDraft = (value) => (Array.isArray(value) ? value.map(String) : value ? String(value).split(',') : []);

/**
 * SettingsField — Renders a single settings field based on field_type.
 * validationError가 있으면 필드 아래에 빨간 경고를 표시합니다.
 */
export function SettingsField({ field, value, modNs, onChange, validationError }) {
    const { t } = useTranslation('gui');
    // 이름표를 입력 칸에 잇는다 (화면 낭독기, 이름표 눌러 칸으로 가기)
    const inputId = useId();
    const fieldLabel = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const fieldDesc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const hasError = !!validationError;
    const errorClass = hasError ? 'setting-input-error' : '';
    const fieldType = FIELD_TYPES.includes(field.field_type) ? field.field_type : 'text';
    const labelId = `${inputId}-label`;
    const picked = listDraft(value);

    return (
        <div className="settings-field">
            <label id={labelId} htmlFor={inputId}>
                {fieldLabel} {field.required ? '*' : ''}
            </label>
            {fieldType === 'text' && (
                <input
                    id={inputId}
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {fieldType === 'password' && (
                <input
                    id={inputId}
                    type="password"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {fieldType === 'number' && (
                <input
                    id={inputId}
                    type="number"
                    className={clsx('setting-input-number', errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    min={field.min}
                    max={field.max}
                    step={field.step || (field.min != null && !Number.isInteger(field.min) ? 'any' : undefined)}
                    placeholder={fieldDesc}
                />
            )}
            {fieldType === 'file' && (
                <input
                    id={inputId}
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {fieldType === 'select' && (
                <CustomDropdown
                    id={inputId}
                    value={String(value || '')}
                    onChange={(val) => onChange(field.name, val)}
                    placeholder={fieldLabel}
                    options={(field.options || []).map((opt) => ({ value: opt, label: opt }))}
                />
            )}
            {fieldType === 'multiline' && (
                <textarea
                    id={inputId}
                    className={clsx('setting-textarea', errorClass)}
                    value={String(value ?? '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    rows={4}
                    placeholder={fieldDesc}
                />
            )}
            {/* 선택지가 있는 목록: 고른 것을 선택지 순서로 */}
            {fieldType === 'list' && field.options?.length > 0 && (
                <fieldset id={inputId} aria-labelledby={labelId} className="setting-list-options">
                    {field.options.map((opt, i) => (
                        <span key={opt} className="setting-list-option">
                            <SabaCheckbox
                                size="sm"
                                id={`${inputId}-${i}`}
                                checked={picked.includes(opt)}
                                onChange={(on) =>
                                    onChange(
                                        field.name,
                                        field.options.filter((o) => (o === opt ? on : picked.includes(o))),
                                    )
                                }
                            />
                            <label htmlFor={`${inputId}-${i}`}>{opt}</label>
                        </span>
                    ))}
                </fieldset>
            )}
            {/* 선택지가 없는 목록: 한 줄에 하나씩 */}
            {fieldType === 'list' && !(field.options?.length > 0) && (
                <textarea
                    id={inputId}
                    className={clsx('setting-textarea', errorClass)}
                    value={picked.join('\n')}
                    onChange={(e) => onChange(field.name, e.target.value.split('\n'))}
                    rows={4}
                    placeholder={t('validation.list_hint')}
                />
            )}
            {fieldType === 'boolean' && (
                <div className="toggle-row">
                    <SabaToggle
                        id={inputId}
                        checked={value === true || value === 'true'}
                        onChange={(checked) => onChange(field.name, checked)}
                    />
                    <span className="toggle-label-text">{value === true || value === 'true' ? 'ON' : 'OFF'}</span>
                </div>
            )}
            {hasError && (
                <div className="setting-validation-error">
                    <Icon name="alert-triangle" size="xs" />
                    {validationError.errorType === 'required' &&
                        t('validation.required', { field: fieldLabel, defaultValue: `${fieldLabel} is required` })}
                    {validationError.errorType === 'type_mismatch' &&
                        t('validation.type_mismatch', {
                            field: fieldLabel,
                            defaultValue: `${fieldLabel} has an invalid type`,
                        })}
                    {validationError.errorType === 'out_of_range' &&
                        t('validation.out_of_range', {
                            field: fieldLabel,
                            min: validationError.min,
                            max: validationError.max,
                            value: validationError.value,
                            defaultValue: `${fieldLabel} is out of range (${validationError.min ?? '−∞'} ~ ${validationError.max ?? '∞'})`,
                        })}
                    {validationError.errorType === 'item_count' &&
                        t('validation.item_count', {
                            field: fieldLabel,
                            min: validationError.min ?? 0,
                            max: validationError.max ?? '∞',
                        })}
                    {validationError.errorType === 'too_long' &&
                        t('validation.too_long', { field: fieldLabel, max: validationError.max })}
                    {validationError.errorType === 'duplicate_item' &&
                        t('validation.duplicate_item', { field: fieldLabel, value: validationError.value })}
                    {validationError.errorType === 'invalid_option' &&
                        t('validation.invalid_option', {
                            field: fieldLabel,
                            value: validationError.value,
                            defaultValue: `${fieldLabel}: '${validationError.value}' is not a valid option`,
                        })}
                </div>
            )}
            {fieldDesc && <small className="field-description">{fieldDesc}</small>}
        </div>
    );
}

/** 프로토콜 이름은 모듈(미검증일 수 있음)이 준 값이라 HTML로 넣지 않고 글자로만 그린다 */
export function ProtocolNotice({ protocol }) {
    const { t } = useTranslation('gui');
    const MARK = '\u0001';
    const [before = '', after = ''] = t('server_settings.protocol_single_only', { protocol: MARK })
        .replace(/<\/?strong>/g, '')
        .split(MARK);
    return (
        <p className="protocol-mode-description">
            {before}
            <strong>{String(protocol).toUpperCase()}</strong>
            {after}
        </p>
    );
}

/**
 * GeneralTab — The "General" settings tab content.
 */
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
}) {
    const { t } = useTranslation('gui');
    const versionId = useId();
    const module = modules.find((m) => m.name === settingsServer.module);
    const hasModuleSettings = module?.settings?.fields?.length > 0;
    const protocols = module?.protocols || {};
    const supportedProtocols = protocols.supported || [];
    const modNs = `mod_${settingsServer.module}`;

    return (
        <div className="settings-form">
            {/* Single protocol info */}
            {supportedProtocols.length > 0 && (
                <div className="protocol-mode-section protocol-mode-info">
                    <div className="protocol-mode-header">
                        <span className="protocol-mode-title">
                            <Icon name="plug" size="sm" /> {t('server_settings.protocol_title')}
                        </span>
                    </div>
                    <ProtocolNotice protocol={supportedProtocols[0]} />
                </div>
            )}

            {/* 크래시 자동 재시작 (데몬 A1: instance module_settings 의 auto_restart / restart_on_unknown_exit) */}
            <div className="protocol-mode-section restart-section">
                <div className="protocol-mode-header">
                    <span className="protocol-mode-title">
                        <Icon name="refresh" size="sm" /> {t('shell.restart_title')}
                    </span>
                </div>
                <p className="protocol-mode-description">{t('shell.restart_desc')}</p>
                <div className="restart-row">
                    <span>
                        <strong>{t('shell.restart_on_crash')}</strong>
                        <small>{t('shell.restart_on_crash_hint')}</small>
                    </span>
                    <SabaToggle
                        checked={String(settingsValues.auto_restart) !== 'false'}
                        onChange={(checked) => handleSettingChange('auto_restart', checked)}
                    />
                </div>
                <div className={clsx('restart-row', { disabled: String(settingsValues.auto_restart) === 'false' })}>
                    <span>
                        <strong>{t('shell.restart_on_unknown')}</strong>
                        <small>{t('shell.restart_on_unknown_hint')}</small>
                    </span>
                    <SabaToggle
                        checked={String(settingsValues.restart_on_unknown_exit) === 'true'}
                        disabled={String(settingsValues.auto_restart) === 'false'}
                        onChange={(checked) => handleSettingChange('restart_on_unknown_exit', checked)}
                    />
                </div>
            </div>

            {/* 램 상한 (데몬 인스턴스 memory_limit_mb, MiB). 비우면 제한 없음, 다음 시작부터 적용 */}
            <div className="protocol-mode-section memory-limit-section">
                <div className="protocol-mode-header">
                    <span className="protocol-mode-title">
                        <Icon name="monitor" size="sm" /> {t('memory_limit.title')}
                    </span>
                </div>
                <p className="protocol-mode-description">{t('memory_limit.desc')}</p>
                <div className="restart-row">
                    <span>
                        <strong>{t('memory_limit.label')}</strong>
                        <small>{t('memory_limit.hint')}</small>
                    </span>
                    <input
                        type="text"
                        inputMode="numeric"
                        className="input memory-limit-input"
                        value={settingsValues.memory_limit_mb ?? ''}
                        placeholder={t('memory_limit.placeholder')}
                        aria-label={t('memory_limit.label')}
                        onChange={(e) => handleSettingChange('memory_limit_mb', e.target.value)}
                    />
                </div>
                {settingsServer.status === 'running' &&
                    String(settingsValues.memory_limit_mb ?? '').trim() !==
                        (settingsServer.memory_limit_mb == null ? '' : String(settingsServer.memory_limit_mb)) && (
                        <p className="protocol-mode-description">{t('memory_limit.restart_note')}</p>
                    )}
            </div>

            {/* 정지 대기 시간 (데몬 MK14 module_settings.stop_timeout, 초). 비우면 모듈 값, 지나면 강제로 끈다.
                데몬이 띄우지 않는 서버(managed:false)는 모듈 stop()이 정지를 맡아 이 값을 쓰지 않는다 */}
            {settingsServer.managed !== false && (
                <div className="protocol-mode-section stop-timeout-section">
                    <div className="protocol-mode-header">
                        <span className="protocol-mode-title">
                            <Icon name="clock" size="sm" /> {t('stop_timeout.title')}
                        </span>
                    </div>
                    <p className="protocol-mode-description">{t('stop_timeout.desc')}</p>
                    <div className="restart-row">
                        <span>
                            <strong>{t('stop_timeout.label')}</strong>
                            <small>{t('stop_timeout.hint')}</small>
                        </span>
                        <input
                            type="text"
                            inputMode="numeric"
                            className="input memory-limit-input"
                            value={settingsValues.stop_timeout ?? ''}
                            placeholder={t('stop_timeout.placeholder')}
                            aria-label={t('stop_timeout.label')}
                            onChange={(e) => handleSettingChange('stop_timeout', e.target.value)}
                        />
                    </div>
                </div>
            )}

            {/* Module settings fields grouped */}
            {hasModuleSettings ? (
                (() => {
                    // visible_when으로 숨긴 설정은 그리지 않는다 (값은 지우지 않는다)
                    const hidden = new Set(hiddenFields(module.settings.fields, settingsValues, unsetSettings));
                    const fields = module.settings.fields.filter((f) => !hidden.has(f.name));
                    const sabaFields = fields.filter((f) => f.group === 'saba-chan');
                    const basicFields = fields.filter((f) => !f.group || f.group === 'basic');
                    const advancedFields = fields.filter((f) => f.group === 'advanced');

                    return (
                        <>
                            {/* Port conflict warnings */}
                            {portConflicts && portConflicts.length > 0 && (
                                <div className="validation-warning-banner">
                                    <Icon name="alert-triangle" size="sm" />
                                    <div>
                                        <strong>{t('errors.port_conflict')}</strong>
                                        {portConflicts.map((c, i) => (
                                            <div key={i} className="validation-warning-detail">
                                                {t('errors.port_conflict_detail', {
                                                    port: c.port,
                                                    name: c.conflictName,
                                                })}
                                            </div>
                                        ))}
                                    </div>
                                </div>
                            )}
                            {/* saba-chan settings */}
                            {sabaFields.length > 0 && (
                                <div className="settings-group">
                                    <h4 className="settings-group-title">
                                        <Icon name="settings" size="sm" />{' '}
                                        {t('server_settings.saba_chan_group', { defaultValue: 'saba-chan Settings' })}
                                    </h4>
                                    {/* Server version selector — only for download-based modules that list and install versions */}
                                    {module?.install?.method === 'download' &&
                                        moduleSupports(module, 'list_versions') &&
                                        moduleSupports(module, 'install_server') && (
                                            <div className="settings-field">
                                                <label htmlFor={versionId}>
                                                    {t('server_settings.server_version', {
                                                        defaultValue: 'Server Version',
                                                    })}
                                                </label>
                                                {versionsLoading ? (
                                                    <div className="version-loading">
                                                        <Icon name="loader" size="sm" />{' '}
                                                        {t('server_settings.loading_versions', {
                                                            defaultValue: 'Loading versions...',
                                                        })}
                                                    </div>
                                                ) : (
                                                    <div className="version-select-row">
                                                        <CustomDropdown
                                                            id={versionId}
                                                            value={settingsValues.server_version || ''}
                                                            onChange={(val) =>
                                                                handleSettingChange('server_version', val)
                                                            }
                                                            placeholder={t('server_settings.select_version', {
                                                                defaultValue: 'Select version',
                                                            })}
                                                            options={availableVersions.map((v) => ({
                                                                value: v.id || v.version || v,
                                                                label: `${v.id || v.version || v}${v.type ? ` (${v.type})` : ''}`,
                                                            }))}
                                                        />
                                                        <button
                                                            className="btn btn-sm btn-primary version-install-btn"
                                                            onClick={handleInstallVersion}
                                                            disabled={
                                                                !settingsValues.server_version ||
                                                                versionInstalling ||
                                                                serverRunning
                                                            }
                                                            title={
                                                                serverRunning
                                                                    ? t('server_settings.install_server_running')
                                                                    : t('server_settings.install_version_tooltip', {
                                                                          defaultValue:
                                                                              'Download and install this version',
                                                                      })
                                                            }
                                                        >
                                                            {versionInstalling ? (
                                                                <>
                                                                    <Icon name="loader" size="sm" />{' '}
                                                                    {t('server_settings.installing', {
                                                                        defaultValue: 'Installing...',
                                                                    })}
                                                                </>
                                                            ) : (
                                                                <>
                                                                    <Icon name="download" size="sm" />{' '}
                                                                    {t('server_settings.install_version_button', {
                                                                        defaultValue: 'Install',
                                                                    })}
                                                                </>
                                                            )}
                                                        </button>
                                                    </div>
                                                )}
                                                <small className="field-description">
                                                    {t('server_settings.version_description_install', {
                                                        defaultValue:
                                                            'Select a version and click Install to download the server files.',
                                                    })}
                                                </small>
                                            </div>
                                        )}
                                    {sabaFields.map((f) => (
                                        <SettingsField
                                            key={f.name}
                                            field={f}
                                            value={settingsValues[f.name]}
                                            modNs={modNs}
                                            onChange={handleSettingChange}
                                            validationError={validationErrors?.[f.name]}
                                        />
                                    ))}
                                </div>
                            )}

                            {/* Basic settings */}
                            {basicFields.length > 0 && (
                                <div className="settings-group">
                                    <h4 className="settings-group-title">
                                        <Icon name="gamepad" size="sm" />{' '}
                                        {t('server_settings.basic_group', { defaultValue: 'Server Settings' })}
                                    </h4>
                                    {basicFields.map((f) => (
                                        <SettingsField
                                            key={f.name}
                                            field={f}
                                            value={settingsValues[f.name]}
                                            modNs={modNs}
                                            onChange={handleSettingChange}
                                            validationError={validationErrors?.[f.name]}
                                        />
                                    ))}
                                </div>
                            )}

                            {/* Advanced settings (collapsible) */}
                            {advancedFields.length > 0 && (
                                <div className="settings-group settings-group-advanced">
                                    <h4
                                        className="settings-group-title settings-group-collapsible"
                                        onClick={() => setAdvancedExpanded(!advancedExpanded)}
                                    >
                                        <Icon name={advancedExpanded ? 'chevronDown' : 'chevronRight'} size="sm" />{' '}
                                        {t('server_settings.advanced_group', { defaultValue: 'Advanced Settings' })}
                                        <span className="settings-group-count">({advancedFields.length})</span>
                                    </h4>
                                    {advancedExpanded &&
                                        advancedFields.map((f) => (
                                            <SettingsField
                                                key={f.name}
                                                field={f}
                                                value={settingsValues[f.name]}
                                                modNs={modNs}
                                                onChange={handleSettingChange}
                                                validationError={validationErrors?.[f.name]}
                                            />
                                        ))}
                                </div>
                            )}

                            {/* Danger zone: 초기화를 구현한 모듈만 (MK1) */}
                            {moduleSupports(module, 'reset_server') && (
                                <div className="settings-group settings-group-danger">
                                    <h4 className="settings-group-title settings-danger-title">
                                        <Icon name="alert-triangle" size="sm" />{' '}
                                        {t('server_settings.danger_zone', { defaultValue: 'Danger Zone' })}
                                    </h4>
                                    <div className="danger-zone-content">
                                        <div className="danger-zone-item">
                                            <div className="danger-zone-info">
                                                {/* 무엇을 지우는지는 모듈마다 다르다: 모듈 locales의 reset.label·description이 먼저 */}
                                                <span className="danger-zone-label">
                                                    {t(`mod_${settingsServer.module}:reset.label`, {
                                                        defaultValue: t('server_settings.reset_server_label'),
                                                    })}
                                                </span>
                                                <span className="danger-zone-desc">
                                                    {t(`mod_${settingsServer.module}:reset.description`, {
                                                        defaultValue: t('server_settings.reset_server_desc'),
                                                    })}
                                                </span>
                                            </div>
                                            <button
                                                className="danger-zone-btn"
                                                onClick={handleResetServer}
                                                disabled={resettingServer || serverRunning}
                                                title={
                                                    serverRunning
                                                        ? t('server_settings.reset_server_running')
                                                        : undefined
                                                }
                                            >
                                                {resettingServer
                                                    ? t('server_settings.resetting', { defaultValue: 'Resetting...' })
                                                    : t('server_settings.reset_button', { defaultValue: 'Reset' })}
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            )}
                        </>
                    );
                })()
            ) : (
                <p className="no-settings" style={{ marginTop: '16px' }}>
                    {t('server_settings.no_settings')}
                </p>
            )}
        </div>
    );
}

/**
 * ServerSettingsModal — Full server settings modal with general tab (+ a link to the Discord words page).
 */
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
    isClosing,
    onClose,
    embedded = false,
    // validation props
    servers,
}) {
    const { t } = useTranslation('gui');

    // "CLI로 보기"에 덧붙일 값의 기준: 이 화면을 열었을 때의 설정
    const openedWith = useRef({ id: null, values: {} });
    if (openedWith.current.id !== settingsServer.id && Object.keys(settingsValues).length) {
        openedWith.current = { id: settingsServer.id, values: settingsValues };
    }

    // 창을 연 뒤에 켜질 수 있으니 지금 상태는 servers 목록에서 본다 (데몬도 켜진 서버의 설치·초기화를 거절한다)
    const liveStatus = (servers || []).find((s) => s.id === settingsServer.id)?.status ?? settingsServer.status;
    const serverRunning = ['running', 'starting', 'stopping'].includes(liveStatus);

    // ── Validation: 설정값 타입 검증 ──
    const module = modules.find((m) => m.name === settingsServer.module);
    const changedSettings = changedForCli(settingsValues, openedWith.current.values, module?.settings?.fields);
    const { valid: _settingsValid, errors: validationErrors } = React.useMemo(() => {
        if (!module?.settings?.fields) return { valid: true, errors: {} };
        return validateAllSettings(module.settings.fields, settingsValues, unsetSettings);
    }, [module, settingsValues, unsetSettings]);

    // ── Validation: 포트 충돌 검사 (모듈 프로토콜 인지) ──
    const moduleProtocols = React.useMemo(() => {
        const map = {};
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

    // ── Dynamic tab indicator ──
    const tabsRef = useRef(null);
    const indicatorRef = useRef(null);

    const syncIndicator = useCallback(() => {
        const container = tabsRef.current;
        const indicator = indicatorRef.current;
        if (!container || !indicator) return;
        const activeBtn = container.querySelector('.settings-tab.active');
        if (!activeBtn) return;
        const containerRect = container.getBoundingClientRect();
        const btnRect = activeBtn.getBoundingClientRect();
        indicator.style.left = `${btnRect.left - containerRect.left}px`;
        indicator.style.width = `${btnRect.width}px`;
    }, []);

    // biome-ignore lint/correctness/useExhaustiveDependencies: settingsActiveTab triggers DOM re-render that syncIndicator reads via querySelector
    useEffect(() => {
        syncIndicator();
    }, [settingsActiveTab, syncIndicator]);

    // Recalc on resize
    useEffect(() => {
        window.addEventListener('resize', syncIndicator);
        return () => window.removeEventListener('resize', syncIndicator);
    }, [syncIndicator]);

    const content = (
        <>
            {!embedded && (
                <div className="modal-header">
                    <h3 style={{ fontSize: '1.3rem' }}>
                        {settingsServer.name} - {t('server_settings.title')}
                    </h3>
                </div>
            )}

            {/* Tab header */}
            <div className="settings-tabs" ref={tabsRef}>
                <div className="settings-tab-indicator" ref={indicatorRef} />
                <button
                    className={clsx('settings-tab', { active: settingsActiveTab === 'general' })}
                    onClick={() => setSettingsActiveTab('general')}
                >
                    <Icon name="gamepad" size="sm" /> {t('server_settings.general_tab')}
                </button>
            </div>

            <div className="modal-body">
                {settingsActiveTab === 'general' && launchPreviewAvailable() && (
                    <LaunchPreview
                        key={settingsServer.id}
                        serverId={settingsServer.id}
                        modNs={`mod_${settingsServer.module}`}
                        fields={module?.settings?.fields}
                        values={settingsValues}
                        extraArgs={settingsValues._launch_extra_args}
                        onExtraArgsChange={(v) => handleSettingChange('_launch_extra_args', v)}
                        onExtraArgsLoaded={(v) => handleSettingChange('_launch_extra_args', v)}
                        onSettingsFromArgs={(found) => {
                            for (const [k, v] of Object.entries(found)) handleSettingChange(k, v);
                        }}
                    />
                )}
                {settingsSource === 'stored' && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('server_settings.stored_values_notice')}</span>
                    </div>
                )}
                {settingsActiveTab === 'general' && (
                    <GeneralTab
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
                        portConflicts={portConflicts}
                        serverRunning={serverRunning}
                    />
                )}
                <ExtensionSlot
                    slotId="ServerSettings.tab"
                    server={settingsServer}
                    activeTab={settingsActiveTab}
                    setActiveTab={setSettingsActiveTab}
                    settings={settingsValues}
                    onSettingsChange={handleSettingChange}
                    t={t}
                />
            </div>

            <div className="modal-footer">
                <CliLine actionId="instance.settings.set" server={settingsServer.name} extra={changedSettings} />
                <button className="btn btn-confirm" onClick={handleSaveSettings} disabled={savingSettings}>
                    <Icon name="save" size="sm" /> {t('server_settings.save_settings')}
                </button>
                {!embedded && (
                    <button className="btn btn-cancel" onClick={onClose}>
                        <Icon name="close" size="sm" /> {t('server_settings.close')}
                    </button>
                )}
            </div>
        </>
    );

    // 서버 상세의 "설정" 탭에 박아 넣을 때: 겹쳐 뜨는 창 없이 같은 내용
    if (embedded) return <div className="server-settings-embedded">{content}</div>;

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={onClose}>
            <div className="modal-content modal-content-large" onClick={(e) => e.stopPropagation()}>
                {content}
            </div>
        </div>
    );
}
