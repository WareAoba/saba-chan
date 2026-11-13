import clsx from 'clsx';
import React, { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type RestartPending, unsavedSettingKeys } from '../stores/useServerSettingsStore';
import type { Server } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';
import type { ModuleInfo, SettingField } from '../types/daemon';
import { moduleSupports } from '../utils/helpers';
import { extensionInstallMethod, versionBranchMethod } from '../utils/installMethod';
import { checkPortConflicts, hiddenFields, validateAllSettings } from '../utils/validation';
import { useExtensions } from '../contexts/ExtensionContext';
import { changedForCli, SECRET_KEY } from '../utils/cliValues';
import { CliLine } from './CliLine';
import { CustomDropdown, ExtensionSlot, Icon } from './index';
import { LaunchPreview, launchPreviewAvailable } from './servers/LaunchPreview';
import { type ResetChoice, type ResetScope, ResetServerDialog } from './servers/ResetServerDialog';
import { Popover } from './ui/Popover';
import { SaveBar } from './ui/SaveBar';
import { SabaBehaviorCards } from './SabaBehaviorCards';
import { Card, type SettingValidationError, type SettingValues, SettingRow, SettingsField, valueKey } from './ServerSettingsParts';
import './ServerSettings.css';

/*
 * 서버 상세의 "설정"·"사바쨩" 탭 (옛 서버 설정 모달을 지금 화면 모양으로 다시 그린 것, 사용자 11-07).
 * 묶음마다 카드(dc-card) 하나, 설정 하나가 한 줄(ss-row): 이름·설명은 왼쪽, 짧은 입력은 오른쪽 끝,
 * 긴 입력(여러 줄·목록)은 줄 아래 전체 폭. 저장 막대는 아래에 붙는다.
 */
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
    availableVersions: { id?: string; version?: string; type?: string; name?: string }[];
    versionInstalling?: boolean;
    handleSettingChange: (name: string, value: unknown) => void;
    handleInstallVersion: () => void;
    /** 지금 값을 저장한다 (걸리지 않는 메모리 상한 "비우기") */
    handleSaveSettings?: () => void;
    /** 고른 범위·백업 여부가 오면 묻지 않고 초기화한다 (확인 창이 이미 물었다) */
    handleResetServer: (choice?: ResetChoice) => void;
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
    handleSaveSettings,
    handleResetServer,
    resettingServer,
    validationErrors,
    portConflicts,
    serverRunning,
    section,
    openedValues,
}: GeneralTabProps) {
    const { t } = useTranslation('gui');
    const { extensions } = useExtensions();
    const versionId = useId();
    const advancedId = useId();
    // 초기화 확인 창에 보일 범위 (데몬이 알려 준 지울 항목). 모듈이 알려 주지 않으면 예전 확인 문구로 묻는다
    const [resetScopes, setResetScopes] = useState<ResetScope[] | null>(null);
    // 버전 때문에 숨긴 설정의 이름을 펼쳤는가 ("목록 보기")
    const [versionHiddenListed, setVersionHiddenListed] = useState(false);
    const askReset = async () => {
        const seen =
            typeof window.api?.instanceResetPreview === 'function'
                ? await window.api.instanceResetPreview(settingsServer.id).catch(() => null)
                : null;
        if (seen?.success && Array.isArray(seen.scopes) && seen.scopes.length) setResetScopes(seen.scopes);
        else handleResetServer();
    };
    // section: 'server' = 게임 서버 설정만, 'saba' = 사바쨩이 서버를 다루는 설정만, 없으면 모두
    const showSaba = section !== 'server';
    const showServer = section !== 'saba';
    const module = modules.find((m) => m.name === settingsServer.module);
    const hasModuleSettings = (module?.settings?.fields?.length ?? 0) > 0;
    const modNs = `mod_${settingsServer.module}`;
    // 설치된 버전에 없는 설정 (데몬이 계산한 version_hidden): 그리지 않고 설정 탭 아래 안내 줄로 알린다
    const versionHidden = new Set(settingsServer.version_hidden || []);
    const versionHiddenFields =
        showServer && hasModuleSettings
            ? module!.settings!.fields.filter((f) => versionHidden.has(f.name) && f.group !== 'saba-chan')
            : [];
    // visible_when으로 숨긴 설정은 그리지 않는다 (값은 지우지 않는다)
    const hidden = new Set([
        ...(hasModuleSettings ? hiddenFields(module!.settings!.fields, settingsValues, unsetSettings) : []),
        ...versionHidden,
    ]);
    const fields = hasModuleSettings ? module!.settings!.fields.filter((f) => !hidden.has(f.name)) : [];
    const sabaFields = showSaba ? fields.filter((f) => f.group === 'saba-chan') : [];
    const basicFields = showServer ? fields.filter((f) => !f.group || f.group === 'basic') : [];
    const advancedFields = showServer ? fields.filter((f) => f.group === 'advanced') : [];
    // 설치 방식 익스텐션으로 받는 서버는 같은 칸에서 브랜치를 고른다 (모듈이 브랜치 목록을 줄 때만)
    const branchMethod = versionBranchMethod(module);
    const branchPicker = !!branchMethod && availableVersions.length > 0;
    const branchSource = extensions.find((e) => e.id === branchMethod)?.name || branchMethod;
    const versionPicker =
        showSaba &&
        (module?.install?.method === 'download' || branchPicker) &&
        moduleSupports(module, 'list_versions') &&
        moduleSupports(module, 'install_server');
    const autoUpdatable = extensionInstallMethod(settingsServer, module) !== null;
    // 명령 팔레트가 고급 묶음의 칸을 찾아 달라고 하면 펼친다 (칸을 찾아 반짝이는 일은 팔레트가 한다)
    const findField = useUIStore((s) => s.findSettingField);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 찾기 요청이 올 때만
    useEffect(() => {
        if (findField && advancedFields.some((f) => f.name === findField.name)) {
            setAdvancedExpanded(true);
            useUIStore.setState({ findSettingField: null }); // 한 번만 (다음에 이 탭을 열 때 다시 펼치지 않게)
        }
    }, [findField?.seq]);
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
                            label={
                                branchPicker
                                    ? t('server_settings.branch', { source: branchSource })
                                    : t('server_settings.server_version', { defaultValue: 'Server Version' })
                            }
                            htmlFor={versionId}
                            desc={
                                branchPicker
                                    ? t('server_settings.branch_desc')
                                    : t('server_settings.version_description_install', {
                                          defaultValue: 'Select a version and click Install to download the server files.',
                                      })
                            }
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
                                                label: v.name || `${v.id || v.version || v}${v.type ? ` (${v.type})` : ''}`,
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
                                                : branchPicker
                                                  ? t('server_settings.branch_install')
                                                  : t('server_settings.install_version_button', { defaultValue: 'Install' })}
                                        </button>
                                    </span>
                                )
                            }
                        />
                    )}
                    {versionPicker && branchPicker && (
                        <div className="notice tone-info idle-stopped-notice ss-branch-notice" role="note">
                            <Icon name="info" size="sm" />
                            <span>{t('server_settings.branch_notice')}</span>
                        </div>
                    )}
                    {fieldRows(sabaFields)}
                </Card>
            )}

            {showSaba && (
                <SabaBehaviorCards
                    settingsServer={settingsServer}
                    settingsValues={settingsValues}
                    handleSettingChange={handleSettingChange}
                    handleSaveSettings={handleSaveSettings}
                    autoUpdatable={autoUpdatable}
                />
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

            {/* 설치된 버전에 없어 숨긴 설정: 몇 개인지와 모듈이 적어 둔 까닭(locales settings_note.version_hidden) */}
            {versionHiddenFields.length > 0 && (
                <div className="notice tone-info idle-stopped-notice ss-version-hidden" role="status">
                    <Icon name="info" size="sm" />
                    <span>
                        {t('server_settings.version_hidden', {
                            version: settingsServer.server_version,
                            count: versionHiddenFields.length,
                        })}{' '}
                        {t(`${modNs}:settings_note.version_hidden`, { defaultValue: '' })}
                        {versionHiddenListed && (
                            <span className="notice-detail">
                                {versionHiddenFields
                                    .map((f) => t(`${modNs}:settings.${f.name}.label`, { defaultValue: f.label }))
                                    .join(', ')}
                            </span>
                        )}
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        aria-expanded={versionHiddenListed}
                        onClick={() => setVersionHiddenListed(!versionHiddenListed)}
                    >
                        {t('server_settings.version_hidden_list')}
                    </button>
                </div>
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
                                onClick={askReset}
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
                    {resetScopes && (
                        <ResetServerDialog
                            name={settingsServer.name}
                            scopes={resetScopes}
                            onClose={() => setResetScopes(null)}
                            onConfirm={(choice) => {
                                setResetScopes(null);
                                handleResetServer(choice);
                            }}
                        />
                    )}
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

/** 모듈 설정이 아닌, 데몬이 인스턴스에 두는 칸의 화면 이름 (GeneralTab의 줄 이름과 같은 키) */
const OWN_LABELS: Record<string, string> = {
    server_version: 'server_settings.server_version',
    auto_restart: 'shell.restart_on_crash',
    restart_on_unknown_exit: 'shell.restart_on_unknown',
    auto_update: 'server_settings.auto_update_label',
    memory_limit_mb: 'memory_limit.label',
    game_chat: 'game_chat.mode_label',
    game_chat_keyword: 'game_chat.keyword_label',
    stop_timeout: 'stop_timeout.label',
    _launch_extra_args: 'launch_preview.extra_label',
};

/**
 * 저장 막대의 "N개 바뀜" 칩 (사용자 11-12): 누르면 칸마다 `이전 → 새 값`과 칸 하나 되돌리기를 보인다.
 * 다시 시작해야 적용되는 칸에는 설정 줄과 같은 표시를 붙인다.
 */
function ChangedFields({
    keys,
    values,
    opened,
    fields = [],
    modNs,
    onRevert,
}: {
    keys: string[];
    values: SettingValues;
    opened: SettingValues;
    fields?: SettingField[];
    modNs: string;
    onRevert?: (key: string) => void;
}) {
    const { t } = useTranslation('gui');
    const [open, setOpen] = useState(false);
    if (!keys.length) return null;
    const count = keys.length;
    const text = (field: SettingField | undefined, key: string, v: unknown): string => {
        if (v == null || v === '' || (Array.isArray(v) && !v.length)) return t('server_settings.value_empty');
        if (field?.field_type === 'password' || SECRET_KEY.test(key)) return '••••';
        if (field ? field.field_type === 'boolean' : ['true', 'false'].includes(String(v)))
            return t(String(v) === 'true' ? 'server_settings.toggle_on' : 'server_settings.toggle_off');
        const option = (opt: unknown) =>
            t(`settings.${key}.options.${opt}`, { ns: modNs, nsSeparator: false, defaultValue: String(opt) });
        if (Array.isArray(v)) return v.map(field?.options?.length ? option : String).join(', ');
        if (field?.field_type === 'select') return option(v);
        if (key === 'game_chat') return t(`game_chat.mode_${v}`, { defaultValue: String(v) });
        return typeof v === 'object' ? JSON.stringify(v) : String(v);
    };
    return (
        <Popover
            open={open}
            onOpenChange={setOpen}
            side="top"
            align="center"
            label={t('server_settings.changed_title', { count })}
            className="ss-changed"
            trigger={
                <button type="button" className="ss-changed-chip">
                    {t('server_settings.changed_count', { count })}
                </button>
            }
        >
            <h5 className="ss-changed-title">{t('server_settings.changed_title', { count })}</h5>
            <ul className="ss-changed-list">
                {keys.map((key) => {
                    const field = fields.find((f) => f.name === key);
                    const name = field
                        ? t(`${modNs}:settings.${key}.label`, { defaultValue: field.label || key })
                        : OWN_LABELS[key]
                          ? t(OWN_LABELS[key])
                          : key;
                    return (
                        <li key={key} className="ss-changed-row">
                            <div className="ss-changed-name">
                                {name}
                                {field?.restart_required && <span className="ss-chip">{t('server_settings.restart_badge')}</span>}
                            </div>
                            <div className="ss-changed-values">
                                <s>{text(field, key, opened[key])}</s> → <b>{text(field, key, values[key])}</b>
                            </div>
                            {onRevert && (
                                <button
                                    type="button"
                                    className="icon-btn"
                                    data-tooltip={t('server_settings.revert_field', { name })}
                                    aria-label={t('server_settings.revert_field', { name })}
                                    onClick={() => onRevert(key)}
                                >
                                    <Icon name="undo" size="sm" />
                                </button>
                            )}
                        </li>
                    );
                })}
            </ul>
        </Popover>
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
    /** 저장 막대의 기준: 이 서버를 연 때의 값 (useServerSettingsStore) */
    openedSettings?: SettingValues;
    /** 기준 값으로 되돌린다 */
    revertSettings?: () => void;
    /** 칸 하나만 기준 값으로 되돌린다 (바뀐 칸 목록) */
    revertSetting?: (key: string) => void;
    /** 화면이 읽어 온 값을 넣는다 (바꾼 것으로 치지 않는다) */
    handleSettingLoaded?: (key: string, value: unknown) => void;
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
    // 기준을 받지 못했으면(이 화면만 따로 그릴 때) 지금 값이 기준이다
    openedSettings = settingsValues,
    revertSettings,
    revertSetting,
    handleSettingLoaded,
    // 서버 상세에 나눠 보이기: 'server' | 'saba' (GeneralTab 참고)
    section,
    // validation props
    servers,
}: ServerSettingsModalProps) {
    const { t } = useTranslation('gui');
    // 익스텐션이 더한 사바쨩 쪽 탭 수 (없으면 탭 머리를 그리지 않는다)
    const { slots } = useExtensions();
    const extensionTabs = ((slots as Record<string, unknown[]>)['ServerSettings.tab'] || []).length;

    // 화면을 연 뒤에 켜질 수 있으니 지금 상태는 servers 목록에서 본다 (데몬도 켜진 서버의 설치·초기화를 거절한다)
    const liveServer = (servers || []).find((s) => s.id === settingsServer.id) ?? settingsServer;
    const serverRunning = ['running', 'starting', 'stopping'].includes(liveServer.status);

    // ── Validation: 설정값 타입 검증 ──
    const module = modules.find((m) => m.name === settingsServer.module);
    const changedSettings = changedForCli(settingsValues, openedSettings, module?.settings?.fields);
    // 저장 막대·"CLI로 보기"의 기준은 이 서버를 연 때의 설정. 저장 막대: 연 때와 다른 키 (`_`로 시작하는 GUI 값도 저장되므로 함께 본다)
    const revertKeys = unsavedSettingKeys(settingsValues, openedSettings, module?.settings?.fields);
    const { errors: validationErrors } = React.useMemo(() => {
        if (!module?.settings?.fields) return { valid: true, errors: {} };
        // 설치된 버전에 없어 숨긴 설정은 검사하지 않는다 (데몬도 required를 보지 않는다)
        const shown = module.settings.fields.filter((f) => !settingsServer?.version_hidden?.includes(f.name));
        return validateAllSettings(shown, settingsValues, unsetSettings) as {
            valid: boolean;
            errors: Record<string, SettingValidationError>;
        };
    }, [module, settingsValues, unsetSettings, settingsServer?.version_hidden]);

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
                    onExtraArgsLoaded={(v: unknown) => handleSettingLoaded?.('_launch_extra_args', v)}
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
                    handleSaveSettings={handleSaveSettings}
                    handleResetServer={handleResetServer}
                    resettingServer={resettingServer}
                    validationErrors={validationErrors}
                    openedValues={openedSettings}
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
                <SaveBar
                    className="ss-savebar"
                    dirty={revertKeys.length > 0}
                    saving={!!savingSettings}
                    onSave={handleSaveSettings}
                    onRevert={() => revertSettings?.()}
                    saveLabel={t('server_settings.save_settings')}
                    extra={
                        <>
                            <ChangedFields
                                keys={revertKeys}
                                values={settingsValues}
                                opened={openedSettings}
                                fields={module?.settings?.fields}
                                modNs={`mod_${settingsServer.module}`}
                                onRevert={revertSetting}
                            />
                            <CliLine actionId="instance.settings.set" server={settingsServer.name} extra={changedSettings} />
                        </>
                    }
                />
            </div>
        </div>
    );
}
