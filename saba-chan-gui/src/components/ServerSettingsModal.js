import clsx from 'clsx';
import React, { useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { checkAliasConflicts, checkPortConflicts, validateAllSettings } from '../utils/validation';
import { useSettingsStore } from '../stores/useSettingsStore';
import { CustomDropdown, ExtensionSlot, Icon, SabaToggle } from './index';

/**
 * SettingsField — Renders a single settings field based on field_type.
 * validationError가 있으면 필드 아래에 빨간 경고를 표시합니다.
 */
function SettingsField({ field, value, modNs, onChange, validationError }) {
    const { t } = useTranslation('gui');
    const fieldLabel = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const fieldDesc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const hasError = !!validationError;
    const errorClass = hasError ? 'setting-input-error' : '';

    return (
        <div className="settings-field">
            <label>
                {fieldLabel} {field.required ? '*' : ''}
            </label>
            {field.field_type === 'text' && (
                <input
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'password' && (
                <input
                    type="password"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'number' && (
                <input
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
                <input
                    type="text"
                    className={clsx(errorClass)}
                    value={String(value || '')}
                    onChange={(e) => onChange(field.name, e.target.value)}
                    placeholder={fieldDesc}
                />
            )}
            {field.field_type === 'select' && (
                <CustomDropdown
                    value={String(value || '')}
                    onChange={(val) => onChange(field.name, val)}
                    placeholder={fieldLabel}
                    options={(field.options || []).map((opt) => ({ value: opt, label: opt }))}
                />
            )}
            {field.field_type === 'boolean' && (
                <div className="toggle-row">
                    <SabaToggle
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
                    <p
                        className="protocol-mode-description"
                        dangerouslySetInnerHTML={{
                            __html: t('server_settings.protocol_single_only', {
                                protocol: supportedProtocols[0].toUpperCase(),
                            }),
                        }}
                    />
                </div>
            )}

            {/* Module settings fields grouped */}
            {hasModuleSettings ? (
                (() => {
                    const fields = module.settings.fields;
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
                                    {/* Server version selector — only for download-based modules (not SteamCMD) */}
                                    {module?.install?.method === 'download' && (
                                    <div className="settings-field">
                                        <label>
                                            {t('server_settings.server_version', { defaultValue: 'Server Version' })}
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
                                                <button
                                                    className="btn btn-sm btn-primary version-install-btn"
                                                    onClick={handleInstallVersion}
                                                    disabled={!settingsValues.server_version || versionInstalling}
                                                    title={t('server_settings.install_version_tooltip', {
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
                                        <button
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
 * AliasesTab — The "Discord Aliases" tab content.
 */
function AliasesTab({
    settingsServer,
    editingModuleAliases,
    setEditingModuleAliases,
    editingCommandAliases,
    setEditingCommandAliases,
    handleSaveAliasesForModule,
    handleResetAliasesForModule,
    aliasConflicts,
    servers,
}) {
    const { t } = useTranslation('gui');
    const modNs = `mod_${settingsServer.module}`;

    // 동일 모듈을 사용하는 다른 인스턴스 검색
    const sameModuleInstances = React.useMemo(() => {
        if (!servers || !settingsServer) return [];
        return servers.filter((s) => s.module === settingsServer.module && s.id !== settingsServer.id);
    }, [servers, settingsServer]);

    return (
        <div className="aliases-tab-content">
            {/* 동일 모듈 다중 인스턴스 경고 */}
            {sameModuleInstances.length > 0 && (
                <div className="validation-warning-banner">
                    <Icon name="alertCircle" size="sm" />
                    <div>
                        <strong>{t('settings.alias_conflict_title')}</strong>
                        <div className="validation-warning-detail">
                            {t('settings.multiple_instances_warning', {
                                module: settingsServer.module,
                                names: sameModuleInstances.map((s) => s.name).join(', '),
                            })}
                        </div>
                    </div>
                </div>
            )}
            {/* Alias conflict warnings */}
            {aliasConflicts && aliasConflicts.length > 0 && (
                <div className="validation-warning-banner">
                    <Icon name="alertCircle" size="sm" />
                    <div>
                        <strong>{t('settings.alias_conflict_title')}</strong>
                        {aliasConflicts.map((c, i) => (
                            <div key={i} className="validation-warning-detail">
                                {t('settings.alias_conflict_detail', { alias: c.alias, module: c.conflictModule })}
                            </div>
                        ))}
                    </div>
                </div>
            )}
            <div className="module-aliases-detail">
                <h4>
                    <Icon name="edit" size="sm" /> {t('server_settings.module_aliases_title')}
                </h4>
                <small>{t('server_settings.module_aliases_hint', { module: settingsServer.module })}</small>
                <div className="module-aliases-input">
                    <input
                        type="text"
                        placeholder={t('server_settings.module_aliases_placeholder', { module: settingsServer.module })}
                        value={editingModuleAliases.join(' ')}
                        onChange={(e) => {
                            const aliases = e.target.value.split(/\s+/).filter((a) => a.length > 0);
                            setEditingModuleAliases(aliases);
                        }}
                    />
                    {editingModuleAliases.length === 0 && (
                        <div className="placeholder-hint">
                            <small>
                                <Icon name="lightbulb" size="xs" /> {t('server_settings.module_aliases_empty_hint')}{' '}
                                <code>{settingsServer.module}</code>
                            </small>
                        </div>
                    )}
                </div>
                <div className="aliases-display">
                    {editingModuleAliases.map((alias, idx) => (
                        <span key={idx} className="alias-badge">
                            {alias}
                        </span>
                    ))}
                </div>

                <h4>
                    <Icon name="zap" size="sm" /> {t('server_settings.command_aliases_title')}
                </h4>
                <small>{t('server_settings.command_aliases_hint')}</small>
                <div className="command-aliases-input">
                    {Object.entries(editingCommandAliases).map(([cmd, cmdData]) => {
                        const aliases = cmdData.aliases || [];
                        const description = t(`${modNs}:commands.${cmd}.description`, {
                            defaultValue: cmdData.description || '',
                        });
                        const label = t(`${modNs}:commands.${cmd}.label`, { defaultValue: cmdData.label || cmd });
                        return (
                            <div key={cmd} className="command-alias-editor">
                                <div className="cmd-header">
                                    <span className="cmd-name">{cmd}</span>
                                    {label !== cmd && <span className="cmd-label">({label})</span>}
                                    {description && (
                                        <span className="cmd-help" title={description}>
                                            ?
                                        </span>
                                    )}
                                </div>
                                <input
                                    type="text"
                                    placeholder={t('server_settings.command_aliases_placeholder', { cmd })}
                                    value={aliases.join(', ')}
                                    onChange={(e) => {
                                        const newAliases = e.target.value
                                            .split(',')
                                            .map((a) => a.trim())
                                            .filter((a) => a.length > 0);
                                        setEditingCommandAliases({
                                            ...editingCommandAliases,
                                            [cmd]: { ...cmdData, aliases: newAliases },
                                        });
                                    }}
                                />
                                <div className="aliases-display">
                                    {aliases.length === 0 ? (
                                        <span className="alias-badge-default">{cmd}</span>
                                    ) : (
                                        aliases.map((alias, idx) => (
                                            <span key={idx} className="alias-badge-sm">
                                                {alias}
                                            </span>
                                        ))
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>

                <div className="module-aliases-actions">
                    <button className="btn btn-save" onClick={() => handleSaveAliasesForModule(settingsServer.module)}>
                        <Icon name="save" size="sm" /> {t('server_settings.save_aliases')}
                    </button>
                    <button
                        className="btn btn-reset"
                        onClick={() => handleResetAliasesForModule(settingsServer.module)}
                    >
                        <Icon name="refresh" size="sm" /> {t('server_settings.reset_aliases')}
                    </button>
                </div>
            </div>
        </div>
    );
}

/**
 * ServerSettingsModal — Full server settings modal with general + aliases tabs.
 */
export function ServerSettingsModal({
    settingsServer,
    settingsValues,
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
    editingModuleAliases,
    setEditingModuleAliases,
    editingCommandAliases,
    setEditingCommandAliases,
    handleSaveAliasesForModule,
    handleResetAliasesForModule,
    isClosing,
    onClose,
    // validation props
    servers,
    moduleAliasesPerModule,
    discordModuleAliases,
}) {
    const { t } = useTranslation('gui');

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

    // ── Validation: 별명 충돌 검사 ──
    const aliasConflicts = React.useMemo(() => {
        if (!settingsServer || !editingModuleAliases) return [];
        return checkAliasConflicts(
            settingsServer.module,
            editingModuleAliases,
            moduleAliasesPerModule || {},
            discordModuleAliases || {},
        );
    }, [settingsServer, editingModuleAliases, moduleAliasesPerModule, discordModuleAliases]);

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

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={onClose}>
            <div className="modal-content modal-content-large" onClick={(e) => e.stopPropagation()}>
                <div className="modal-header">
                    <h3 style={{ fontSize: '1.3rem' }}>
                        {settingsServer.name} - {t('server_settings.title')}
                    </h3>
                </div>

                {/* Tab header */}
                <div className="settings-tabs" ref={tabsRef}>
                    <div className="settings-tab-indicator" ref={indicatorRef} />
                    <button
                        className={clsx('settings-tab', { active: settingsActiveTab === 'general' })}
                        onClick={() => setSettingsActiveTab('general')}
                    >
                        <Icon name="gamepad" size="sm" /> {t('server_settings.general_tab')}
                    </button>
                    <button
                        className={clsx('settings-tab', { active: settingsActiveTab === 'aliases' })}
                        onClick={() => setSettingsActiveTab('aliases')}
                    >
                        <Icon name="discord" size="sm" /> {t('server_settings.aliases_tab')}
                    </button>
                </div>

                <div className="modal-body">
                    {settingsActiveTab === 'general' && (
                        <GeneralTab
                            settingsServer={settingsServer}
                            settingsValues={settingsValues}
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
                        />
                    )}
                    {settingsActiveTab === 'aliases' && (
                        <AliasesTab
                            settingsServer={settingsServer}
                            editingModuleAliases={editingModuleAliases}
                            setEditingModuleAliases={setEditingModuleAliases}
                            editingCommandAliases={editingCommandAliases}
                            setEditingCommandAliases={setEditingCommandAliases}
                            handleSaveAliasesForModule={handleSaveAliasesForModule}
                            handleResetAliasesForModule={handleResetAliasesForModule}
                            aliasConflicts={aliasConflicts}
                            servers={servers}
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
                    {settingsActiveTab !== 'aliases' && (
                        <button className="btn btn-confirm" onClick={handleSaveSettings}>
                            <Icon name="save" size="sm" /> {t('server_settings.save_settings')}
                        </button>
                    )}
                    <button className="btn btn-cancel" onClick={onClose}>
                        <Icon name="close" size="sm" /> {t('server_settings.close')}
                    </button>
                </div>
            </div>
        </div>
    );
}
