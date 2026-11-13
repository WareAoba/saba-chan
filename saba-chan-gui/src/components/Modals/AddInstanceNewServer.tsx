import clsx from 'clsx';
import { type ReactNode, useEffect, useId, useRef, useState } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import type { ApiBody, Json, ModuleInfo, SettingField } from '../../types/daemon';
import {
    changedSettings,
    checkBlockers,
    choosesVersion,
    createBody,
    installChoices,
    installFields,
    nameVerdict,
    optionLabel,
    latestRelease,
    presetBaseline,
    presetFields,
    releaseVersions,
    versionSettings,
} from '../../utils/installFields';
import { initials } from '../../utils/serverView';
import { hiddenFields } from '../../utils/validation';
import { AgreementList, allChecked } from '../AgreementList';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { AddServerOptions, addServerExtensionData } from '../DataSlots';
import ExtensionSlot, { type SlotPage } from '../ExtensionSlot';
import Icon from '../Icon';
import { ModuleMakerCard } from '../ModuleMakerCard';
import { Card, SettingRow, SettingsField } from '../ServerSettingsModal';
import { PageFind, PageFindButton } from '../servers/PageFind';
import { LinkedText } from '../ui/ScreenLink';
import { SabaNumberInput, SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { unsupportedText } from '../../utils/moduleSupport';
import { Segmented } from '../ui/StatusChip';
import { TrustCreateNotice } from '../ui/TrustBadge';
import type { AddServerPayload } from '../../stores/serverActions';
import { AddServerFrame } from './AddServerFrame';
import { isImeKey } from '../../utils/ime';

/**
 * AddInstanceNewServer — 새 서버 만들기 (사용자 11-11 A안).
 *
 * 게임 목록 → 추천값이 채워진 요약 한 장(이름·서버 종류·버전·포트·위치·실행 방식) → 만들기.
 * "상세 설정"은 같은 창을 넓혀 서버 설정 화면과 같은 칸(사바쨩·기본·고급)으로 서버 설정을 미리 정한다 (Ctrl+F 찾기).
 * 이름·폴더·포트가 다른 서버와 겹치는지는 데몬이 본다 (`POST /api/instances/check`)
 */
export interface AddInstanceNewServerProps {
    extensions: ModuleInfo[];
    onAddServer: (payload: AddServerPayload) => Promise<unknown>;
    /** 게임 목록의 "쓰던 서버 가져오기" 칸 (없으면 그리지 않는다) */
    onMigrate?: () => void;
    /** 익스텐션이 더한 방법 (`AddServer.method`) */
    extMethods?: SlotPage[];
    onSelectExt?: (id: `ext:${string}`) => void;
    onClose: () => void;
}

type Step = 'select-game' | 'maker' | 'summary' | 'detail';

export function AddInstanceNewServer({
    extensions,
    onAddServer,
    onMigrate,
    extMethods = [],
    onSelectExt,
    onClose,
}: AddInstanceNewServerProps) {
    const { t } = useTranslation('gui');
    const [step, setStep] = useState<Step>('select-game');
    const [search, setSearch] = useState('');
    const [selectedExtension, setSelectedExtension] = useState('');
    const [newServerName, setNewServerName] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const [installValues, setInstallValues] = useState<Record<string, unknown>>({});
    const [preset, setPreset] = useState('');
    const [version, setVersion] = useState(''); // '' = 최신
    const [versions, setVersions] = useState<string[] | null>(null); // 고를 수 있는 릴리스 (null = 아직 모름)
    const [latest, setLatest] = useState(''); // 모듈이 알려 준 최신 릴리스 숫자 (모르면 '')
    const [folderMode, setFolderMode] = useState('default'); // 'default' | 'custom'
    const [folder, setFolder] = useState('');
    const [managed, setManaged] = useState(true);
    const [port, setPort] = useState<number | string | null>(null); // null = 데몬이 고른 포트 (고치지 않음)
    const [check, setCheck] = useState<ApiBody | null>(null); // 데몬 확인 결과
    const [checking, setChecking] = useState(false);
    const [agreed, setAgreed] = useState<Record<string, boolean>>({}); // 약관 id → 체크 여부
    const [extValues, setExtValues] = useState<Record<string, Json | undefined>>({}); // AddServer.options 답 (field → 값)
    const [edits, setEdits] = useState<Record<string, unknown>>({}); // 상세 설정에서 손댄 칸 (이름 → 값)
    const [advancedExpanded, setAdvancedExpanded] = useState(false);
    const { extensions: daemonExtensions } = useExtensions();
    const nameInputRef = useRef<HTMLInputElement>(null);
    const detailRef = useRef<HTMLDivElement>(null);
    const checkSeq = useRef(0);
    const advancedId = useId();
    const [autoName, setAutoName] = useState(false); // 이름 칸을 아직 고치지 않았다

    const selectedModule = extensions.find((m) => m.name === selectedExtension);
    const customFolder = folderMode === 'custom' ? folder.trim() : '';

    // 상세 설정: 처음 값은 모듈 기본값 위의 추천 설정, 보내는 것은 그와 다른 칸만
    const detailFields = presetFields(selectedModule);
    const baseline = presetBaseline(selectedModule, preset);
    const detailValues = { ...baseline, ...edits };
    const detailChanged = changedSettings(detailFields, edits, baseline);
    const changedCount = Object.keys(detailChanged).length;

    // 게임 칸을 눌렀을 때: 이름은 모듈 이름으로 두고, 겹치면 데몬 확인 답의 이름(-2부터)으로 바꾼다
    const handleGameSelect = (extName: string) => {
        setSelectedExtension(extName);
        setNewServerName(extName);
        setAutoName(true);
        setInstallValues({});
        setPreset('');
        setVersion('');
        setVersions(null);
        setLatest('');
        setFolderMode('default');
        setFolder('');
        setManaged(true);
        setPort(null);
        setCheck(null);
        setAgreed({});
        setExtValues({});
        setEdits({});
        setAdvancedExpanded(false);
        setStep('summary');
    };

    const body = () =>
        createBody({
            name: newServerName,
            module: selectedExtension,
            installValues,
            preset,
            version,
            folder: customFolder,
            managed,
            port,
            settings: detailChanged,
        });

    const onForm = step === 'summary' || step === 'detail';

    // 서버 버전: 고른 설정(서버 종류 등)으로 받을 수 있는 릴리스를 데몬에 묻는다 (못 받으면 최신만 고른다).
    // 설정을 바꾸면 다시 묻고, 앞 목록과 골라 둔 버전은 바로 비운다 (새 목록이 오기 전에 만들어도 앞 설정의 버전을 보내지 않게)
    const versionKey = choosesVersion(selectedModule)
        ? JSON.stringify(versionSettings(selectedModule, installValues, preset))
        : '';
    const versionPickedFor = useRef(versionKey);
    useEffect(() => {
        if (versionPickedFor.current === versionKey) return;
        versionPickedFor.current = versionKey;
        setVersion('');
        setVersions(null);
        setLatest('');
    }, [versionKey]);
    useEffect(() => {
        if (!onForm || !versionKey) return;
        let alive = true;
        const timer = setTimeout(() => {
            Promise.resolve(
                window.api?.moduleListVersions?.(selectedExtension, { per_page: 50, settings: JSON.parse(versionKey) }),
            )
                .then((res) => {
                    if (!alive) return;
                    const list = releaseVersions(res);
                    setVersions(list);
                    setLatest(latestRelease(res));
                    setVersion((chosen) => (list.includes(chosen) ? chosen : ''));
                })
                .catch(() => {
                    if (!alive) return;
                    setVersions([]);
                    setLatest('');
                    setVersion('');
                });
        }, 250);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [onForm, versionKey, selectedExtension]);

    // 겹치는지 데몬에 묻는다 (입력이 멈춘 뒤 한 번)
    const checkKey = JSON.stringify(onForm ? body() : null);
    useEffect(() => {
        if (!onForm) return;
        const seq = ++checkSeq.current;
        setChecking(true);
        const timer = setTimeout(async () => {
            let res = null;
            try {
                res = await window.api?.instanceCheck?.(JSON.parse(checkKey));
            } catch (_e) {
                res = null;
            }
            if (seq !== checkSeq.current) return;
            setCheck(res?.success ? res : { failed: true, error_code: res?.error_code, fields: res?.fields });
            setChecking(false);
        }, 250);
        return () => {
            clearTimeout(timer);
            setChecking(false);
        };
    }, [onForm, checkKey]);

    // 고치지 않은 기본 이름이 겹치면 데몬이 고른 이름으로
    const suggestedName = autoName && check?.name?.taken ? check.name.suggested : null;
    useEffect(() => {
        if (suggestedName) setNewServerName(suggestedName);
    }, [suggestedName]);

    // 요약에 들어오면 이름 칸에 포커스
    useEffect(() => {
        if (step === 'summary' && nameInputRef.current) {
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    const browseFolder = async () => {
        const picked = await window.api?.openFolderDialog?.();
        if (picked) setFolder(picked);
    };

    const agreements = selectedModule?.agreements || [];
    const verdict = nameVerdict(check, newServerName);
    const duplicateName = !!verdict?.taken;
    const nameComma = verdict?.invalid === 'comma';
    const blockers = check && !check.failed ? checkBlockers(check) : [];
    // 데몬이 폴더 경로를 받지 않았다 (처음부터 적지 않은 경로)
    const folderNotAbsolute =
        !!customFolder &&
        check?.failed &&
        check.error_code === 'invalid_format' &&
        (check.fields || []).some((f: ApiBody) => f.field === 'working_dir');
    // 데몬이 받지 않은 상세 설정 값 (폴더 경로 말고): 그 줄에 알리고 만들기를 막는다
    const invalidSettings: string[] =
        check?.failed && check.error_code === 'invalid_format'
            ? (check.fields || []).map((f: ApiBody) => f.field).filter((f: unknown) => typeof f === 'string' && f !== 'working_dir')
            : [];
    // 고친 포트는 1~65535의 정수만 (빈 칸은 데몬이 고른 포트)
    const portInvalid =
        port != null && port !== '' && !(Number.isInteger(Number(port)) && Number(port) >= 1 && Number(port) <= 65535);
    const formReady =
        newServerName.trim() &&
        !checking &&
        !duplicateName &&
        !nameComma &&
        !blockers.length &&
        !portInvalid &&
        !folderNotAbsolute &&
        !invalidSettings.length &&
        (folderMode === 'default' || !!customFolder);
    const canSubmit = formReady && selectedExtension && !submitting && allChecked(agreements, agreed);

    const handleSubmit = async () => {
        if (!canSubmit) return;

        setSubmitting(true);
        const extensionData = addServerExtensionData(daemonExtensions, extValues);
        let created: unknown;
        try {
            created = await onAddServer({
                ...body(),
                // 사용자가 체크한 약관만 (module.toml [[agreements]])
                ...(agreements.length ? { accept_agreements: agreements.map((a: ApiBody) => a.id) } : {}),
                // 익스텐션이 물은 칸 (AddServer.options)
                ...(extensionData ? { extension_data: extensionData } : {}),
            });
        } finally {
            setSubmitting(false);
        }
        // 만들지 못했으면 입력을 남겨 고쳐서 다시 보낼 수 있게 한다
        if (created === false) return;

        setNewServerName('');
        setSelectedExtension('');
        setStep('select-game');
    };

    // ──────────────────────────────────────────
    //  게임 목록 (가져오기·익스텐션 방법·직접 만들기 칸이 같은 줄에)
    // ──────────────────────────────────────────
    if (step === 'select-game') {
        const wanted = search.trim().toLowerCase();
        const shown = extensions.filter((m) => {
            if (!wanted) return true;
            const name = t(`mod_${m.name}:module.display_name`, { defaultValue: m.game_name || m.name });
            return `${name} ${m.name}`.toLowerCase().includes(wanted);
        });
        return (
            <AddServerFrame
                title={t('add_instance_modal.title')}
                description={t('add_server_modal.pick_subtitle')}
                footer={
                    <button type="button" className="btn btn-ghost" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                }
            >
                {extensions.length > 0 ? (
                    <input
                        type="search"
                        className="input as-search"
                        value={search}
                        placeholder={t('add_server_modal.search_games')}
                        aria-label={t('add_server_modal.search_games')}
                        onChange={(e) => setSearch(e.target.value)}
                    />
                ) : (
                    <p className="dc-hint as-empty">
                        <Icon name="alertCircle" size="sm" />
                        <span onClickCapture={onClose}>
                            <LinkedText i18nKey="add_server_modal.no_modules" to="settings/sabaStorage/modules" />
                        </span>
                    </p>
                )}
                <div className="as-games">
                    {shown.map((m) => {
                        const displayName = t(`mod_${m.name}:module.display_name`, {
                            defaultValue: m.game_name || m.name,
                        });
                        // 이 데몬의 OS·CPU에서 돌아가지 않는 모듈 (`platform_supported`, D3·M1)
                        const unsupportedHere = unsupportedText(t, m);
                        const unsupported = unsupportedHere !== null;
                        return (
                            <button
                                key={m.name}
                                className={clsx('as-game', { unsupported })}
                                type="button"
                                disabled={unsupported}
                                data-tooltip={unsupportedHere?.detail}
                                onClick={() => handleGameSelect(m.name)}
                            >
                                <GameIcon module={m} name={displayName} />
                                <span className="as-game-name">{displayName}</span>
                                <span className="as-game-sub">
                                    {unsupportedHere ? unsupportedHere.label : `v${m.version}`}
                                </span>
                            </button>
                        );
                    })}
                    {onMigrate && (
                        <button type="button" className="as-game as-tile" onClick={onMigrate}>
                            <Icon name="download" size="lg" />
                            <span className="as-game-name">{t('add_server_modal.import_tile')}</span>
                        </button>
                    )}
                    {extMethods.map((m) => (
                        <button
                            key={m.id}
                            type="button"
                            className="as-game as-tile"
                            onClick={() => onSelectExt?.(m.id as `ext:${string}`)}
                        >
                            <Icon name={m.icon} size="lg" />
                            <span className="as-game-name">{m.label}</span>
                        </button>
                    ))}
                    <button type="button" className="as-game as-tile" onClick={() => setStep('maker')}>
                        <Icon name="plus" size="lg" />
                        <span className="as-game-name">{t('add_server_modal.maker_tile')}</span>
                        <span className="as-game-sub">{t('add_server_modal.maker_tile_sub')}</span>
                    </button>
                </div>
            </AddServerFrame>
        );
    }

    // ──────────────────────────────────────────
    //  직접 만들기 (목록에 없는 게임)
    // ──────────────────────────────────────────
    if (step === 'maker') {
        return (
            <AddServerFrame
                title={t('add_server_modal.maker_tile')}
                description={t('add_server_modal.maker_tile_sub')}
                footer={
                    <button type="button" className="btn btn-ghost" onClick={() => setStep('select-game')}>
                        {t('add_server_modal.prev')}
                    </button>
                }
            >
                <ModuleMakerCard defaultOpen />
            </AddServerFrame>
        );
    }

    const selectedDisplayName = selectedModule
        ? t(`mod_${selectedModule.name}:module.display_name`, {
              defaultValue: selectedModule.game_name || selectedModule.name,
          })
        : selectedExtension;
    const media = selectedModule ? <GameIcon module={selectedModule} name={selectedDisplayName} /> : null;
    const createBtn = (
        <button type="button" className="btn btn-primary" onClick={handleSubmit} disabled={!canSubmit}>
            {submitting ? (
                <>
                    <SabaSpinner size="sm" /> {t('add_server_modal.provisioning')}
                </>
            ) : (
                t('add_server_modal.create')
            )}
        </button>
    );

    // ──────────────────────────────────────────
    //  상세 설정: 서버 설정 화면과 같은 칸
    // ──────────────────────────────────────────
    if (step === 'detail') {
        const modNs = `mod_${selectedExtension}`;
        const hidden = new Set(hiddenFields(detailFields, detailValues));
        const visible = detailFields.filter((f) => !hidden.has(f.name));
        const rows = (list: SettingField[]) =>
            list.map((f) => (
                <SettingsField
                    key={f.name}
                    field={f}
                    value={detailValues[f.name]}
                    modNs={modNs}
                    onChange={(name, value) => setEdits((prev) => ({ ...prev, [name]: value }))}
                    validationError={invalidSettings.includes(f.name) ? { errorType: 'invalid' } : undefined}
                    className={f.name in detailChanged ? 'as-changed' : undefined}
                />
            ));
        const saba = visible.filter((f) => f.group === 'saba-chan');
        const basic = visible.filter((f) => !f.group || f.group === 'basic');
        const advanced = visible.filter((f) => f.group === 'advanced');
        return (
            <AddServerFrame
                wide
                title={t('add_server_modal.detail_heading', { game: selectedDisplayName })}
                description={t('add_server_modal.detail_subtitle')}
                media={media}
                footer={
                    <>
                        {changedCount > 0 && (
                            <span className="as-foot-hint">
                                <span>{t('add_server_modal.detail_changed', { count: changedCount })}</span>
                                <button type="button" className="btn btn-ghost btn-sm" onClick={() => setEdits({})}>
                                    {t('add_server_modal.detail_revert')}
                                </button>
                            </span>
                        )}
                        <button type="button" className="btn btn-ghost" onClick={() => setStep('summary')}>
                            {t('add_server_modal.back_to_summary')}
                        </button>
                        {createBtn}
                    </>
                }
            >
                <div className="as-detail-bar">
                    <PageFindButton />
                </div>
                <PageFind modal rootRef={detailRef} enabled pageKey={`add-server-${selectedExtension}`} />
                <div className="ss-panel ss-form" ref={detailRef}>
                    {saba.length > 0 && (
                        <Card icon="sabaFace" title={t('server_settings.saba_chan_group', { defaultValue: 'saba-chan Settings' })}>
                            {rows(saba)}
                        </Card>
                    )}
                    {basic.length > 0 && (
                        <Card icon="gamepad" title={t('server_settings.basic_group', { defaultValue: 'Server Settings' })}>
                            {rows(basic)}
                        </Card>
                    )}
                    {advanced.length > 0 && (
                        <Card
                            icon="settings"
                            title=""
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
                                        <span className="ss-count">{advanced.length}</span>
                                    </h2>
                                    <Icon name={advancedExpanded ? 'chevronUp' : 'chevronDown'} size="sm" />
                                </button>
                            }
                        >
                            {/* 접혀 있어도 그려 둔다: Ctrl+F가 안을 세고, 그 결과로 가면 편다 */}
                            <div id={advancedId} className="ss-fold" data-find-fold hidden={!advancedExpanded}>
                                {rows(advanced)}
                            </div>
                        </Card>
                    )}
                </div>
            </AddServerFrame>
        );
    }

    // ──────────────────────────────────────────
    //  요약 한 장: 추천값이 채워져 있다
    // ──────────────────────────────────────────
    // 사용자 11-06: 비워 두면 받을 최신 버전을 숫자로 보인다 ("26.3 (최신)"). 모르면 "최신 버전"
    const latestLabel = latest
        ? t('add_server_modal.version_latest_of', { version: latest })
        : t('add_server_modal.version_latest');
    const mainPort = (check?.ports || []).find((p: ApiBody) => p.field === 'port');
    const alert = (text: string) => <span role="alert">{text}</span>;

    return (
        <AddServerFrame
            title={t('add_server_modal.summary_title', { game: selectedDisplayName })}
            description={t('add_server_modal.summary_subtitle')}
            media={media}
            footer={
                <>
                    <button
                        type="button"
                        className="btn btn-ghost as-foot-back"
                        onClick={() => setStep('select-game')}
                        disabled={submitting}
                    >
                        <Icon name="chevronLeft" size="sm" />
                        {t('add_server_modal.back_to_games')}
                    </button>
                    {createBtn}
                </>
            }
        >
            <div className="ss-panel ss-form">
                <section className="dc-card ss-card">
                    <div className="ss-rows">
                        <SettingRow
                            label={t('add_server_modal.server_name')}
                            htmlFor="as-server-name"
                            control={
                                <input
                                    id="as-server-name"
                                    ref={nameInputRef}
                                    className="input ss-text"
                                    type="text"
                                    placeholder={t('add_server_modal.server_name_placeholder')}
                                    value={newServerName}
                                    onChange={(e) => {
                                        setNewServerName(e.target.value);
                                        setAutoName(false);
                                    }}
                                    disabled={submitting}
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter' && !isImeKey(e)) handleSubmit();
                                    }}
                                    aria-invalid={duplicateName || nameComma || undefined}
                                />
                            }
                            error={
                                duplicateName
                                    ? alert(t('servers.rename_name_taken', { name: newServerName.trim() }))
                                    : nameComma
                                      ? alert(t('servers.name_comma'))
                                      : null
                            }
                        />

                        {/* 모듈이 만들 때 고르라고 한 설정 (설치할 서버 파일·새 월드가 달라진다) */}
                        {installFields(selectedModule).map((field) => (
                            <InstallField
                                key={field.name}
                                t={t}
                                module={selectedModule!}
                                field={field}
                                value={installValues[field.name]}
                                onChange={(val) => setInstallValues((prev) => ({ ...prev, [field.name]: val }))}
                                disabled={submitting}
                            />
                        ))}

                        {choosesVersion(selectedModule) && (
                            <SettingRow
                                label={t('add_server_modal.version')}
                                desc={t('add_server_modal.version_hint')}
                                control={
                                    <CustomDropdown
                                        className="ss-select"
                                        ariaLabel={t('add_server_modal.version')}
                                        value={version}
                                        onChange={setVersion}
                                        options={[
                                            { value: '', label: latestLabel },
                                            ...(versions || [])
                                                .filter((v) => v !== latest)
                                                .map((v) => ({ value: v, label: v })),
                                        ]}
                                        disabled={submitting}
                                    />
                                }
                            />
                        )}

                        {/* 모듈 추천 설정 (module.toml [[presets]]) */}
                        {(selectedModule?.presets?.length ?? 0) > 0 && (
                            <SettingRow
                                label={t('add_server_modal.preset_title')}
                                stack
                                desc={
                                    preset
                                        ? t(`mod_${selectedModule!.name}:presets.${preset}.description`, {
                                              defaultValue:
                                                  selectedModule!.presets!.find((p) => p.id === preset)?.description || '',
                                          })
                                        : t('add_server_modal.preset_none_hint')
                                }
                                control={
                                    <Segmented
                                        ariaLabel={t('add_server_modal.preset_title')}
                                        value={preset}
                                        onChange={setPreset}
                                        items={[
                                            { id: '', label: t('add_server_modal.preset_none') },
                                            ...selectedModule!.presets!.map((p) => ({
                                                id: p.id,
                                                label: t(`mod_${selectedModule!.name}:presets.${p.id}.label`, {
                                                    defaultValue: p.label || p.id,
                                                }),
                                            })),
                                        ]}
                                    />
                                }
                            />
                        )}

                        {(mainPort || port != null) && (
                            <SettingRow
                                label={t('add_server_modal.port')}
                                htmlFor="as-server-port"
                                desc={t('add_server_modal.port_hint')}
                                control={
                                    <SabaNumberInput
                                        id="as-server-port"
                                        className="input ss-num"
                                        min={1}
                                        max={65535}
                                        value={port ?? String(mainPort?.port ?? '')}
                                        aria-invalid={portInvalid || undefined}
                                        onChange={(e) => setPort(e.target.value)}
                                        disabled={submitting}
                                    />
                                }
                                error={portInvalid ? alert(t('add_server_modal.port_invalid')) : null}
                            />
                        )}
                    </div>
                </section>
                <CheckNotes t={t} check={check} checking={checking} />

                <section className="dc-card ss-card">
                    <div className="ss-rows">
                        <SettingRow
                            label={t('add_server_modal.location_title')}
                            stack
                            desc={t(
                                folderMode === 'custom'
                                    ? 'add_server_modal.location_custom_hint'
                                    : 'add_server_modal.location_default_hint',
                            )}
                            control={
                                <div className="as-stack">
                                    <Segmented
                                        ariaLabel={t('add_server_modal.location_title')}
                                        value={folderMode}
                                        onChange={setFolderMode}
                                        items={[
                                            { id: 'default', label: t('add_server_modal.location_default') },
                                            { id: 'custom', label: t('add_server_modal.location_custom') },
                                        ]}
                                    />
                                    {folderMode === 'custom' && (
                                        <div className="as-path-row">
                                            <input
                                                className="input ss-text mono"
                                                type="text"
                                                aria-label={t('add_server_modal.location_placeholder')}
                                                placeholder={t('add_server_modal.location_placeholder')}
                                                value={folder}
                                                onChange={(e) => setFolder(e.target.value)}
                                                aria-invalid={folderNotAbsolute || undefined}
                                            />
                                            <button
                                                className="icon-btn"
                                                type="button"
                                                onClick={browseFolder}
                                                data-tooltip={t('add_server_modal.location_browse')}
                                                aria-label={t('add_server_modal.location_browse')}
                                            >
                                                <Icon name="folder" size="sm" />
                                            </button>
                                        </div>
                                    )}
                                </div>
                            }
                            error={folderNotAbsolute ? alert(t('add_server_modal.folder_not_absolute')) : null}
                        />
                        <SettingRow
                            label={t('add_server_modal.run_title')}
                            stack
                            desc={t(managed ? 'add_server_modal.run_managed_hint' : 'add_server_modal.run_unmanaged_hint')}
                            control={
                                <Segmented
                                    ariaLabel={t('add_server_modal.run_title')}
                                    value={managed ? 'managed' : 'unmanaged'}
                                    onChange={(id) => setManaged(id === 'managed')}
                                    items={[
                                        { id: 'managed', label: t('add_server_modal.run_managed') },
                                        { id: 'unmanaged', label: t('add_server_modal.run_unmanaged') },
                                    ]}
                                />
                            }
                        />
                    </div>
                </section>

                {/* 상세 설정: 서버 설정을 미리 정한다 (사용자 11-11) */}
                {detailFields.length > 0 && (
                    <section className="dc-card ss-card">
                        <div className="ss-rows">
                            <SettingRow
                                label={t('add_server_modal.detail_title')}
                                desc={t('add_server_modal.detail_desc')}
                                control={
                                    <button type="button" className="btn btn-ghost" onClick={() => setStep('detail')}>
                                        <Icon name="settings" size="sm" />
                                        {t('add_server_modal.detail_open')}
                                        {changedCount > 0 && ` · ${t('add_server_modal.detail_changed', { count: changedCount })}`}
                                    </button>
                                }
                            />
                        </div>
                    </section>
                )}

                {/* 확인 안 된·바뀐 모듈: 게임 서버는 격리되지 않는다 */}
                <TrustCreateNotice trust={selectedModule?.trust} />

                {/* 모듈이 선언한 약관: 모두 동의해야 만들 수 있다 */}
                {agreements.length > 0 && (
                    <Card icon="file" title={t('agreements.title')}>
                        <AgreementList
                            module={selectedExtension}
                            agreements={agreements}
                            checked={agreed}
                            onToggle={(id, on) => setAgreed((prev) => ({ ...prev, [id]: on }))}
                            disabled={submitting}
                        />
                    </Card>
                )}

                {/* 익스텐션 슬롯 — 서버 생성 옵션 */}
                <ExtensionSlot slotId="AddServer.options" t={t} />
                <AddServerOptions
                    values={extValues}
                    onChange={(field, value) => setExtValues((prev) => ({ ...prev, [field]: value }))}
                    disabled={submitting}
                />

                <p className="dc-hint">
                    <Icon name="info" size="sm" /> {t('add_server_modal.provision_hint')}
                </p>
            </div>
        </AddServerFrame>
    );
}

/** 게임 아이콘: 모듈 icon.png, 없으면 이름 머리글자 */
function GameIcon({ module, name }: { module: ModuleInfo; name: string }) {
    return (
        <span className="as-game-icon" aria-hidden="true">
            {module.icon ? <img src={module.icon} alt="" /> : <span>{initials(name)}</span>}
        </span>
    );
}

/** 데몬 확인 결과 중 알릴 것: 막는 것(이름·폴더)은 경고로, 다른 서버와 같은 포트·예약된 포트는 안내로 */
function CheckNotes({ t, check, checking }: { t: TFunction; check: ApiBody | null; checking: boolean }) {
    if (checking && !check) return <p className="dc-hint">{t('add_server_modal.checking')}</p>;
    if (!check) return null;
    if (check.failed) {
        // 폴더 경로가 틀린 것은 그 줄에서, 상세 설정 값이 틀린 것은 상세 설정에서 알린다 (요약에는 한 줄로)
        if (check.error_code === 'invalid_format') {
            const settings = (check.fields || []).some((f: ApiBody) => f.field && f.field !== 'working_dir');
            return settings ? (
                <p className="dc-field-error as-detail-invalid" role="alert">
                    <Icon name="alertTriangle" size="sm" /> {t('add_server_modal.detail_invalid')}
                </p>
            ) : null;
        }
        return <p className="dc-hint">{t('add_server_modal.check_failed')}</p>;
    }
    const notes: string[][] = [];
    if (check.folder?.in_use_by)
        notes.push(['alert', t('add_server_modal.folder_in_use', { name: check.folder.in_use_by })]);
    if (check.folder?.not_empty) notes.push(['alert', t('add_server_modal.folder_not_empty')]);
    for (const p of (check.ports || []) as ApiBody[]) {
        if (p.in_use_by?.length) {
            notes.push(['note', t('add_server_modal.port_in_use', { port: p.port, servers: p.in_use_by.join(', ') })]);
        }
        if (p.reserved) notes.push(['alert', t('add_server_modal.port_reserved', { port: p.port })]);
    }
    return notes.map(([kind, text]) => (
        <p
            key={text}
            className={kind === 'alert' ? 'dc-field-error' : 'dc-hint'}
            role={kind === 'alert' ? 'alert' : undefined}
        >
            <Icon name={kind === 'alert' ? 'alertTriangle' : 'info'} size="sm" /> {text}
        </p>
    ));
}

/** 만들 때 고르는 설정 칸 하나. 고르지 않은 칸은 보내지 않는다 (데몬이 모듈 기본값을 채운다) */
interface InstallFieldProps {
    t: TFunction;
    module: ModuleInfo;
    field: SettingField;
    value: unknown;
    // biome-ignore lint/suspicious/noExplicitAny: 칸 종류(문자열·숫자·불리언)마다 값이 다르다
    onChange: (value: any) => void;
    disabled?: boolean;
}

function InstallField({ t, module, field, value, onChange, disabled }: InstallFieldProps) {
    const modNs = `mod_${module.name}`;
    const label = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const description = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const inputId = `as-install-${field.name}`;
    const shown = value ?? field.default;

    let control: ReactNode;
    if (field.field_type === 'select') {
        control = (
            <CustomDropdown
                className="ss-select"
                ariaLabel={label}
                value={String(shown ?? '')}
                onChange={onChange}
                options={installChoices(field).map((opt) => ({
                    value: opt,
                    label: optionLabel(t, module.name, field, opt),
                }))}
                disabled={disabled}
            />
        );
    } else if (field.field_type === 'boolean') {
        control = (
            <SabaToggle
                id={inputId}
                checked={shown === true || shown === 'true'}
                onChange={onChange}
                disabled={disabled}
                aria-label={label}
            />
        );
    } else {
        control = (
            <input
                id={inputId}
                className={clsx('input', field.field_type === 'number' ? 'ss-num' : 'ss-text')}
                type={field.field_type === 'number' ? 'number' : 'text'}
                value={String(shown ?? '')}
                min={field.min ?? undefined}
                max={field.max ?? undefined}
                onChange={(e) => {
                    const raw = e.target.value;
                    onChange(field.field_type === 'number' && raw !== '' ? Number(raw) : raw);
                }}
                disabled={disabled}
            />
        );
    }

    return (
        <SettingRow
            label={label}
            htmlFor={field.field_type === 'select' ? undefined : inputId}
            desc={description || undefined}
            control={control}
        />
    );
}
