import { ModalOverlay } from './Modals/Modals';
import clsx from 'clsx';
import React, { useId } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { useSettingsStore } from '../stores/useSettingsStore';
import { checkPortConflicts, validateAllSettings } from '../utils/validation';
import { CustomDropdown, ExtensionSlot, Icon, SabaToggle } from './index';

/**
 * SettingsField — Renders a single settings field based on field_type.
 * validationError가 있으면 필드 아래에 빨간 경고를 표시합니다.
 */
function SettingsField({ field, value, modNs, onChange, validationError }) {
    const inputId = useId();
    const { t } = useTranslation('gui');
    const fieldLabel = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const fieldDesc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const hasError = !!validationError;
    const errorClass = hasError ? 'setting-input-error' : '';

    return (
        <div className="settings-field">
            <label htmlFor={inputId}>
                {fieldLabel} {field.required ? '*' : ''}
            </label>
            {field.field_type === 'text' && (
                <input id={inputId}
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'password' && (
                <input id={inputId}
                    type="password"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'number' && (
                <input id={inputId}
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
            {field.field_type === 'file' && (
                <input id={inputId}
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'select' && (
                <CustomDropdown id={inputId}
                    value={String(value || '')}
                    onChange={(val) => onChange(field.name, val)}
                    placeholder={fieldLabel}
                    options={(field.options || []).map((opt) => ({ value: opt, label: opt }))}
                />
            )}
            {field.field_type === 'boolean' && (
                <div className="toggle-row">
                    <SabaToggle id={inputId}
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

/**
 * GeneralTab — The "General" settings tab content.
 */
function GeneralTab({
    settingsServer,
    settingsValues,
    modules,
    advancedExpanded,
    setAdvancedExpanded,
    communicationExpanded,
    setCommunicationExpanded,
    versionsLoading,
    availableVersions,
    versionInstalling,
    handleSettingChange,
    handleInstallVersion,
    handleResetServer,
    resettingServer,
    validationErrors,
    portConflicts,
}) {
    const { t } = useTranslation('gui');
    const module = modules.find((m) => m.name === settingsServer.module);
    const hasModuleSettings = module?.settings?.fields?.length > 0;
    const protocols = module?.protocols || {};
    const supportedProtocols = protocols.supported || [];
    const showProtocolToggle = supportedProtocols.includes('rest') && supportedProtocols.includes('rcon');
    const modNs = `mod_${settingsServer.module}`;

    return (
        <div className="settings-form">
            {/* Protocol mode toggle */}
            {showProtocolToggle && (
                <div className="protocol-mode-section">
                    <div className="protocol-mode-header">
                        <span className="protocol-mode-title">
                            <Icon name="plug" size="sm" /> {t('server_settings.protocol_title')}
                        </span>
                    </div>
                    <p className="protocol-mode-description">{t('server_settings.protocol_description')}</p>
                    <div className="protocol-toggle-container">
                        <span className={clsx('protocol-label', { active: settingsValues.protocol_mode === 'rest' })}>
                            REST
                        </span>
                        <SabaToggle
                            size="lg"
                            checked={settingsValues.protocol_mode === 'rcon'}
                            onChange={(checked) => handleSettingChange('protocol_mode', checked ? 'rcon' : 'rest')}
                        />
                        <span className={clsx('protocol-label', { active: settingsValues.protocol_mode === 'rcon' })}>
                            RCON
                        </span>
                    </div>
                    <p className="protocol-mode-hint">
                        <span className="hint-icon">
                            <Icon name="lightbulb" size="sm" />
                        </span>
                        {settingsValues.protocol_mode === 'rest'
                            ? t('server_settings.protocol_rest_hint')
                            : t('server_settings.protocol_rcon_hint')}
                    </p>
                </div>
            )}

            {/* Single protocol info */}
            {!showProtocolToggle && supportedProtocols.length > 0 && (
                <div className="protocol-mode-section protocol-mode-info">
                    <div className="protocol-mode-header">
                        <span className="protocol-mode-title">
                            <Icon name="plug" size="sm" /> {t('server_settings.protocol_title')}
                        </span>
                    </div>
                    <p className="protocol-mode-description">
                        <Trans t={t} i18nKey="server_settings.protocol_single_only"
                            values={{ protocol: supportedProtocols[0].toUpperCase() }} components={{ b: <strong /> }} />
                    </p>
                </div>
            )}

            {/* Module settings fields grouped */}
            {hasModuleSettings ? (
                (() => {
                    const fields = module.settings.fields;
                    const sabaFields = fields.filter((f) => f.group === 'saba-chan');
                    const basicFields = fields.filter((f) => !f.group || f.group === 'basic');
                    const communicationFields = fields.filter((f) => f.group === 'communication');
                    const advancedFields = fields.filter((f) => f.group === 'advanced');

                    return (
                        <>
                            {/* Port conflict warnings */}
                            {portConflicts && portConflicts.length > 0 && (
                                <div className="validation-warning-banner">
                                    <Icon name="alert-triangle" size="sm" />
                                    <div>
                                        <strong>{t('errors.port_conflict')}</strong>
                                        {portConflicts.map((c) => (
                                            <div key={`${c.port}:${c.conflict_name}:${c.field || ''}`} className="validation-warning-detail">
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
                                    {/* Server version selector — only for download-based modules (not SteamCMD) */}
                                    {module?.install?.method === 'download' && (
                                        <div className="settings-field">
                                            <label htmlFor="serversettingsmodal-234">
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
                                                    <CustomDropdown id="serversettingsmodal-234"
                                                        value={settingsValues.server_version || ''}
                                                        onChange={(val) => handleSettingChange('server_version', val)}
                                                        placeholder={t('server_settings.select_version', {
                                                            defaultValue: 'Select version',
                                                        })}
                                                        options={availableVersions.map((v) => ({
                                                            value: v.id || v.version || v,
                                                            label: `${v.id || v.version || v}${v.type ? ` (${v.type})` : ''}`,
                                                        }))}
                                                    />
                                                    <button type="button"
                                                        className="btn btn-sm btn-primary version-install-btn"
                                                        onClick={handleInstallVersion}
                                                        disabled={!settingsValues.server_version || versionInstalling}
                                                        data-tooltip={t('server_settings.install_version_tooltip', {
                                                            defaultValue: 'Download and install this version',
                                                        })}
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
                                                        'Select a version and click Install to download the server JAR.',
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

                            {/* Communication settings (collapsible) */}
                            {communicationFields.length > 0 && (
                                <div className="settings-group settings-group-advanced settings-group-communication">
                                    <button type="button" aria-expanded={communicationExpanded}
                                        className="settings-group-title settings-group-collapsible"
                                        onClick={() => setCommunicationExpanded(!communicationExpanded)}
                                    >
                                        <Icon name={communicationExpanded ? 'chevronDown' : 'chevronRight'} size="sm" />{' '}
                                        {t('server_settings.communication_group', {
                                            defaultValue: 'Communication Settings',
                                        })}
                                        <span className="settings-group-count">({communicationFields.length})</span>
                                    </button>
                                    {communicationExpanded &&
                                        communicationFields.map((f) => (
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
                                    <button type="button" aria-expanded={advancedExpanded}
                                        className="settings-group-title settings-group-collapsible"
                                        onClick={() => setAdvancedExpanded(!advancedExpanded)}
                                    >
                                        <Icon name={advancedExpanded ? 'chevronDown' : 'chevronRight'} size="sm" />{' '}
                                        {t('server_settings.advanced_group', { defaultValue: 'Advanced Settings' })}
                                        <span className="settings-group-count">({advancedFields.length})</span>
                                    </button>
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

                            {/* Danger zone */}
                            <div className="settings-group settings-group-danger">
                                <h4 className="settings-group-title settings-danger-title">
                                    <Icon name="alert-triangle" size="sm" />{' '}
                                    {t('server_settings.danger_zone', { defaultValue: 'Danger Zone' })}
                                </h4>
                                <div className="danger-zone-content">
                                    <div className="danger-zone-item">
                                        <div className="danger-zone-info">
                                            <span className="danger-zone-label">
                                                {settingsServer.module === 'palworld'
                                                    ? t('server_settings.reset_settings_label', {
                                                          defaultValue: 'Reset Settings',
                                                      })
                                                    : t('server_settings.reset_server_label', {
                                                          defaultValue: 'Reset Server',
                                                      })}
                                            </span>
                                            <span className="danger-zone-desc">
                                                {settingsServer.module === 'palworld'
                                                    ? t('server_settings.reset_settings_desc', {
                                                          defaultValue:
                                                              'Reset all server settings to factory defaults. World and save data will be preserved. This cannot be undone.',
                                                      })
                                                    : t('server_settings.reset_server_desc', {
                                                          defaultValue:
                                                              'Delete all worlds, settings, logs, and other data. Only the server JAR and eula.txt will be kept. This cannot be undone.',
                                                      })}
                                            </span>
                                        </div>
                                        <button type="button"
                                            className="danger-zone-btn"
                                            onClick={handleResetServer}
                                            disabled={resettingServer}
                                        >
                                            {resettingServer
                                                ? t('server_settings.resetting', { defaultValue: 'Resetting...' })
                                                : t('server_settings.reset_button', { defaultValue: 'Reset' })}
                                        </button>
                                    </div>
                                </div>
                            </div>
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
 * ServerSettingsModal — Full server settings modal.
 *
 * 디스코드 별명 탭은 제거됨 — 인스턴스별 별명/권한은 디스코드 모달의
 * "서버별 설정" 팝업(InstanceSettingsModal)에서 관리한다.
 */
export function ServerSettingsModal({
    settingsServer,
    settingsValues,
    settingsActiveTab,
    setSettingsActiveTab,
    modules,
    advancedExpanded,
    setAdvancedExpanded,
    communicationExpanded,
    setCommunicationExpanded,
    availableVersions,
    versionsLoading,
    versionInstalling,
    handleSettingChange,
    handleInstallVersion,
    handleSaveSettings,
    handleResetServer,
    resettingServer,
    isClosing,
    onClose,
    // 라이브 설정 읽기 상태 ('idle'|'loading'|'ok'|'empty'|'failed')
    // 'ok'가 아니면 화면값이 실제 설정 파일과 다를 수 있음을 경고한다.
    liveSettingsStatus,
    // validation props
    servers,
}) {
    const { t } = useTranslation('gui');

    const liveWarning = React.useMemo(() => {
        if (liveSettingsStatus === 'failed') {
            return t('server_settings.live_banner_failed', {
                defaultValue:
                    'Could not read the actual server configuration file. Displayed values may differ from the real settings — only fields you change here will be saved.',
            });
        }
        if (liveSettingsStatus === 'empty') {
            return t('server_settings.live_banner_empty', {
                defaultValue:
                    'The server configuration file is empty or missing, so displayed values are defaults. Only fields you change here will be saved.',
            });
        }
        return null;
    }, [liveSettingsStatus, t]);

    // ── Validation: 설정값 타입 검증 ──
    const module = modules.find((m) => m.name === settingsServer.module);
    const { valid: _settingsValid, errors: validationErrors } = React.useMemo(() => {
        if (!module?.settings?.fields) return { valid: true, errors: {} };
        return validateAllSettings(module.settings.fields, settingsValues);
    }, [module, settingsValues]);

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
            rcon_port: settingsValues.rcon_port ?? settingsServer.rcon_port,
            rest_port: settingsValues.rest_port ?? settingsServer.rest_port,
        };
        return checkPortConflicts(settingsServer.id, targetPorts, servers, moduleProtocols, settingsServer.module);
    }, [settingsServer, settingsValues, servers, moduleProtocols]);

    return (
        <ModalOverlay  className={clsx('modal-overlay', { closing: isClosing })} onClose={onClose}>
            <div className="modal-content modal-content-large" >
                <div className="modal-header">
                    <h3 style={{ fontSize: '1.3rem' }}>
                        {settingsServer.name} - {t('server_settings.title')}
                    </h3>
                </div>

                {liveWarning && (
                    <div
                        className="settings-live-warning"
                        data-testid="live-settings-warning"
                        role="alert"
                        style={{
                            margin: '10px 16px 0',
                            padding: '10px 12px',
                            borderRadius: '8px',
                            border: '1px solid var(--warning-border, #e0a800)',
                            background: 'var(--warning-bg, rgba(255, 193, 7, 0.12))',
                            color: 'var(--warning-text, inherit)',
                            fontSize: '0.85rem',
                            lineHeight: 1.45,
                        }}
                    >
                        ⚠️ {liveWarning}
                    </div>
                )}

                {/* 단일 페이지 구성 — 구(舊) 탭 헤더는 제거됨.
                    익스텐션(Docker 등) 설정은 ExtensionSlot이 일반 설정 아래에 섹션으로 렌더링한다. */}
                <div className="modal-body">
                    <GeneralTab
                        settingsServer={settingsServer}
                        settingsValues={settingsValues}
                        modules={modules}
                        advancedExpanded={advancedExpanded}
                        setAdvancedExpanded={setAdvancedExpanded}
                        communicationExpanded={communicationExpanded}
                        setCommunicationExpanded={setCommunicationExpanded}
                        versionsLoading={versionsLoading}
                        availableVersions={availableVersions}
                        versionInstalling={versionInstalling}
                        handleSettingChange={handleSettingChange}
                        handleInstallVersion={handleInstallVersion}
                        handleResetServer={handleResetServer}
                        resettingServer={resettingServer}
                        validationErrors={validationErrors}
                        portConflicts={portConflicts}
                    />
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
                    <button type="button" className="btn btn-confirm" onClick={handleSaveSettings}>
                        <Icon name="save" size="sm" /> {t('server_settings.save_settings')}
                    </button>
                    <button type="button" className="btn btn-cancel" onClick={onClose}>
                        <Icon name="close" size="sm" /> {t('server_settings.close')}
                    </button>
                </div>
            </div>
        </ModalOverlay>
    );
}
