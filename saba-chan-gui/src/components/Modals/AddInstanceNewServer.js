import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import {
    checkBlockers,
    choosesVersion,
    createBody,
    installChoices,
    installFields,
    nameTaken,
    optionLabel,
    releaseVersions,
    suggestName,
    versionSettings,
    wizardSteps,
} from '../../utils/installFields';
import { AgreementList, allChecked } from '../AgreementList';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { AddServerOptions, addServerExtensionData } from '../DataSlots';
import ExtensionSlot from '../ExtensionSlot';
import Icon from '../Icon';
import { ModuleMakerCard } from '../ModuleMakerCard';
import { SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import { TrustCreateNotice } from '../ui/TrustBadge';

/**
 * AddInstanceNewServer — "새 서버로 추가" 마법사.
 *
 * 단계 (`wizardSteps`): 게임 → 서버 종류·버전(고를 것이 있을 때) → 파일 위치 → 실행 방식 → 이름·포트 → 확인.
 * 이름·폴더·포트가 다른 서버와 겹치는지는 데몬이 본다 (`POST /api/instances/check`)
 */
export function AddInstanceNewServer({
    extensions,
    servers,
    onRefreshextensions,
    onAddServer,
    onStepChange,
    onBack,
    onClose,
}) {
    const { t } = useTranslation('gui');
    const [step, setStep] = useState('select-game');
    const [selectedExtension, setSelectedExtension] = useState('');
    const [newServerName, setNewServerName] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const [installValues, setInstallValues] = useState({});
    const [preset, setPreset] = useState('');
    const [version, setVersion] = useState(''); // '' = 최신
    const [versions, setVersions] = useState(null); // 고를 수 있는 릴리스 (null = 아직 모름)
    const [folderMode, setFolderMode] = useState('default'); // 'default' | 'custom'
    const [folder, setFolder] = useState('');
    const [managed, setManaged] = useState(true);
    const [port, setPort] = useState(null); // null = 데몬이 고른 포트 (고치지 않음)
    const [check, setCheck] = useState(null); // 데몬 확인 결과
    const [checking, setChecking] = useState(false);
    const [leaving, setLeaving] = useState(false); // 파일 위치 단계의 폴더 확인
    const [agreed, setAgreed] = useState({}); // 약관 id → 체크 여부
    const [extValues, setExtValues] = useState({}); // AddServer.options 답 (field → 값)
    const { extensions: daemonExtensions } = useExtensions();
    const nameInputRef = useRef(null);
    const checkSeq = useRef(0);

    const selectedModule = extensions.find((m) => m.name === selectedExtension);
    const steps = wizardSteps(selectedModule);
    const stepIndex = steps.indexOf(step);
    const customFolder = folderMode === 'custom' ? folder.trim() : '';

    // 진행 막대는 부모가 그린다: 단계와 함께 지금이 몇 번째인지(0~1) 알린다
    const goTo = (next, list = steps) => {
        onStepChange?.(next, list.indexOf(next) / (list.length - 1)); // 높이 캡처 후 부모 상태 변경 (배칭)
        setStep(next);
    };

    // 게임 카드를 눌렀을 때
    const handleGameSelect = (extName) => {
        // 자동 이름: 모듈 이름 그대로, 겹치면 -2부터 (CLI와 같다)
        setSelectedExtension(extName);
        setNewServerName(suggestName(extName, servers));
        setInstallValues({});
        setPreset('');
        setVersion('');
        setVersions(null);
        setFolderMode('default');
        setFolder('');
        setManaged(true);
        setPort(null);
        setCheck(null);
        setAgreed({});
        setExtValues({});
        const nextSteps = wizardSteps(extensions.find((m) => m.name === extName));
        goTo(nextSteps[1], nextSteps);
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
        });

    // 서버 종류·버전 단계: 고른 설정(서버 종류 등)으로 받을 수 있는 릴리스를 데몬에 묻는다 (못 받으면 최신만 고른다).
    // 설정을 바꾸면 다시 묻고, 앞 목록과 골라 둔 버전은 바로 비운다 (새 목록이 오기 전에 넘어가도 앞 설정의 버전을 보내지 않게)
    const versionKey = choosesVersion(selectedModule)
        ? JSON.stringify(versionSettings(selectedModule, installValues, preset))
        : '';
    const versionPickedFor = useRef(versionKey);
    useEffect(() => {
        if (versionPickedFor.current === versionKey) return;
        versionPickedFor.current = versionKey;
        setVersion('');
        setVersions(null);
    }, [versionKey]);
    useEffect(() => {
        if (step !== 'server' || !versionKey) return;
        let alive = true;
        const timer = setTimeout(() => {
            Promise.resolve(
                window.api?.moduleListVersions?.(selectedExtension, { per_page: 50, settings: JSON.parse(versionKey) }),
            )
                .then((res) => {
                    if (!alive) return;
                    const list = releaseVersions(res);
                    setVersions(list);
                    setVersion((chosen) => (list.includes(chosen) ? chosen : ''));
                })
                .catch(() => {
                    if (!alive) return;
                    setVersions([]);
                    setVersion('');
                });
        }, 250);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [step, versionKey, selectedExtension]);

    // 이름·포트 단계와 확인 단계: 겹치는지 데몬에 묻는다 (입력이 멈춘 뒤 한 번)
    const checkKey = JSON.stringify(step === 'name' || step === 'confirm' ? body() : null);
    useEffect(() => {
        if (step !== 'name' && step !== 'confirm') return;
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
            setCheck(res?.success ? res : { failed: true, error_code: res?.error_code });
            setChecking(false);
        }, 250);
        return () => {
            clearTimeout(timer);
            setChecking(false);
        };
    }, [step, checkKey]);

    // 이름 단계 진입 시 이름 인풋에 포커스
    useEffect(() => {
        if (step === 'name' && nameInputRef.current) {
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    // 파일 위치 단계의 "다음": 고른 폴더가 다른 서버의 폴더이거나 파일이 들어 있으면 넘어가지 않는다
    const [folderProblem, setFolderProblem] = useState(null);
    // 확인을 기다리는 사이 단계나 폴더가 바뀌었으면 늦게 온 결과는 버린다
    const shown = useRef({});
    shown.current = { step, customFolder };
    const leaveLocation = async () => {
        setFolderProblem(null);
        if (!customFolder) return goTo(steps[stepIndex + 1]);
        setLeaving(true);
        let res = null;
        try {
            res = await window.api?.instanceCheck?.(body());
        } finally {
            setLeaving(false);
        }
        if (shown.current.step !== step || shown.current.customFolder !== customFolder) return;
        const badFolder =
            res?.error_code === 'invalid_format' && (res.fields || []).some((f) => f.field === 'working_dir');
        if (badFolder) return setFolderProblem({ code: 'folder_not_absolute' });
        if (res?.folder?.in_use_by) return setFolderProblem({ code: 'folder_in_use', name: res.folder.in_use_by });
        if (res?.folder?.not_empty) return setFolderProblem({ code: 'folder_not_empty' });
        goTo(steps[stepIndex + 1]);
    };

    const browseFolder = async () => {
        const picked = await window.api?.openFolderDialog?.();
        if (picked) {
            setFolder(picked);
            setFolderProblem(null);
        }
    };

    const handleSubmit = async () => {
        if (!canSubmit) return;

        setSubmitting(true);
        const extensionData = addServerExtensionData(daemonExtensions, extValues);
        let created;
        try {
            created = await onAddServer({
                ...body(),
                // 사용자가 체크한 약관만 (module.toml [[agreements]])
                ...(agreements.length ? { accept_agreements: agreements.map((a) => a.id) } : {}),
                // 익스텐션이 물은 칸 (AddServer.options)
                ...(extensionData ? { extension_data: extensionData } : {}),
            });
        } finally {
            setSubmitting(false);
        }
        // 만들지 못했으면 입력을 남겨 고쳐서 다시 보낼 수 있게 한다
        if (created === false) return;

        // 폼 초기화
        setNewServerName('');
        setSelectedExtension('');
        setStep('select-game');
    };

    const agreements = selectedModule?.agreements || [];
    const duplicateName =
        nameTaken(servers, newServerName) || (check?.name?.value === newServerName.trim() && check?.name?.taken);
    const blockers = check && !check.failed ? checkBlockers(check) : [];
    // 고친 포트는 1~65535의 정수만 (빈 칸은 데몬이 고른 포트)
    const portInvalid =
        port != null && port !== '' && !(Number.isInteger(Number(port)) && Number(port) >= 1 && Number(port) <= 65535);
    const nameReady = newServerName.trim() && !duplicateName && !blockers.length && !portInvalid;
    const canSubmit = nameReady && selectedExtension && !submitting && allChecked(agreements, agreed);
    const canLeave = {
        location: folderMode === 'default' || !!customFolder,
        name: nameReady && !checking,
    };

    // ──────────────────────────────────────────
    //  게임 선택 (아이콘 카드 그리드)
    // ──────────────────────────────────────────
    if (step === 'select-game') {
        return (
            <div className="add-instance-stage" key="new-select-game">
                <div className="modal-header add-server-header">
                    <div className="as-header-row">
                        <button className="ai-back-btn" type="button" onClick={onBack}>
                            <Icon name="chevronLeft" size="sm" />
                        </button>
                        <div className="as-header-copy">
                            <h3>{t('add_server_modal.title')}</h3>
                            <p className="add-server-subtitle">{t('add_server_modal.select_game_subtitle')}</p>
                        </div>
                    </div>
                </div>

                <div className="modal-body add-server-body">
                    <div className="as-body-stack">
                        {extensions.length === 0 ? (
                            <p className="as-empty-hint">
                                <Icon name="alertCircle" size="sm" />
                                {t('add_server_modal.no_modules')}
                            </p>
                        ) : (
                            <div className="as-grid-wrap">
                                <div className="as-game-grid">
                                    {extensions.map((m) => {
                                        const displayName = t(`mod_${m.name}:module.display_name`, {
                                            defaultValue: m.game_name || m.name,
                                        });
                                        // 이 데몬의 OS에서 돌아가지 않는 모듈 (`platform_supported`, D3·M1)
                                        const unsupported = m.platform_supported === false;
                                        return (
                                            <button
                                                key={m.name}
                                                className={clsx('as-game-card', { unsupported })}
                                                type="button"
                                                disabled={unsupported}
                                                data-tooltip={
                                                    unsupported
                                                        ? t('add_server_modal.platform_unsupported_detail', {
                                                              platforms: (m.platforms || []).join(', '),
                                                          })
                                                        : undefined
                                                }
                                                onClick={() => handleGameSelect(m.name)}
                                            >
                                                <div className="as-game-card-icon">
                                                    {m.icon ? (
                                                        <img src={m.icon} alt={displayName} />
                                                    ) : (
                                                        <div className="as-game-card-icon-placeholder">
                                                            <Icon name="gamepad" size="lg" />
                                                        </div>
                                                    )}
                                                </div>
                                                <span className="as-game-card-name">{displayName}</span>
                                                <span className="as-game-card-version">
                                                    {unsupported
                                                        ? t('add_server_modal.platform_unsupported')
                                                        : `v${m.version}`}
                                                </span>
                                            </button>
                                        );
                                    })}
                                </div>
                            </div>
                        )}
                        <ModuleMakerCard />
                    </div>
                </div>

                <div className="modal-footer add-server-footer">
                    <button className="btn btn-cancel" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                </div>
            </div>
        );
    }

    const selectedDisplayName = selectedModule
        ? t(`mod_${selectedModule.name}:module.display_name`, {
              defaultValue: selectedModule.game_name || selectedModule.name,
          })
        : selectedExtension;
    const goBack = () => {
        if (stepIndex <= 1) {
            onStepChange?.('select-game', 0);
            setStep('select-game');
            setSelectedExtension('');
            setNewServerName('');
            return;
        }
        goTo(steps[stepIndex - 1]);
    };
    const ports = check?.ports || [];
    const mainPort = ports.find((p) => p.field === 'port');

    let content;
    if (step === 'server') {
        content = (
            <>
                {/* 모듈이 만들 때 고르라고 한 설정 (설치할 서버 파일·새 월드가 달라진다) */}
                {installFields(selectedModule).map((field) => (
                    <InstallField
                        key={field.name}
                        t={t}
                        module={selectedModule}
                        field={field}
                        value={installValues[field.name]}
                        onChange={(val) => setInstallValues((prev) => ({ ...prev, [field.name]: val }))}
                        disabled={submitting}
                    />
                ))}

                {choosesVersion(selectedModule) && (
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="download" size="sm" />
                            {t('add_server_modal.version')}
                        </span>
                        <CustomDropdown
                            value={version}
                            onChange={setVersion}
                            options={[
                                { value: '', label: t('add_server_modal.version_latest') },
                                ...(versions || []).map((v) => ({ value: v, label: v })),
                            ]}
                            disabled={submitting}
                        />
                        <p className="as-provision-hint">{t('add_server_modal.version_hint')}</p>
                    </div>
                )}

                {/* 모듈 추천 프리셋 (module.toml [[presets]]) */}
                {selectedModule?.presets?.length > 0 && (
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="lightbulb" size="sm" />
                            {t('add_server_modal.preset_title')}
                        </span>
                        <Segmented
                            ariaLabel={t('add_server_modal.preset_title')}
                            value={preset}
                            onChange={setPreset}
                            items={[
                                { id: '', label: t('add_server_modal.preset_none') },
                                ...selectedModule.presets.map((p) => ({
                                    id: p.id,
                                    label: t(`mod_${selectedModule.name}:presets.${p.id}.label`, {
                                        defaultValue: p.label || p.id,
                                    }),
                                })),
                            ]}
                        />
                        <p className="as-provision-hint">
                            {preset
                                ? t(`mod_${selectedModule.name}:presets.${preset}.description`, {
                                      defaultValue:
                                          selectedModule.presets.find((p) => p.id === preset)?.description || '',
                                  })
                                : t('add_server_modal.preset_none_hint')}
                        </p>
                    </div>
                )}
            </>
        );
    } else if (step === 'location') {
        content = (
            <div className="as-section as-section-card">
                <span className="as-label">
                    <Icon name="folder" size="sm" />
                    {t('add_server_modal.location_title')}
                </span>
                <Segmented
                    ariaLabel={t('add_server_modal.location_title')}
                    value={folderMode}
                    onChange={(mode) => {
                        setFolderMode(mode);
                        setFolderProblem(null);
                    }}
                    items={[
                        { id: 'default', label: t('add_server_modal.location_default') },
                        { id: 'custom', label: t('add_server_modal.location_custom') },
                    ]}
                />
                <p className="as-provision-hint">
                    {t(
                        folderMode === 'custom'
                            ? 'add_server_modal.location_custom_hint'
                            : 'add_server_modal.location_default_hint',
                    )}
                </p>
                {folderMode === 'custom' && (
                    <div className="as-path-row">
                        <input
                            className="as-input as-input-mono"
                            type="text"
                            aria-label={t('add_server_modal.location_placeholder')}
                            placeholder={t('add_server_modal.location_placeholder')}
                            value={folder}
                            onChange={(e) => {
                                setFolder(e.target.value);
                                setFolderProblem(null);
                            }}
                            aria-invalid={folderProblem ? true : undefined}
                        />
                        <button
                            className="as-btn-icon"
                            type="button"
                            onClick={browseFolder}
                            data-tooltip={t('add_server_modal.location_browse')}
                            aria-label={t('add_server_modal.location_browse')}
                        >
                            <Icon name="folder" size="sm" />
                        </button>
                    </div>
                )}
                {folderProblem && (
                    <p className="as-name-taken" role="alert">
                        <Icon name="alertCircle" size="sm" />
                        {t(`add_server_modal.${folderProblem.code}`, { name: folderProblem.name })}
                    </p>
                )}
            </div>
        );
    } else if (step === 'run') {
        content = (
            <div className="as-section as-section-card">
                <span className="as-label">
                    <Icon name="play" size="sm" />
                    {t('add_server_modal.run_title')}
                </span>
                <Segmented
                    ariaLabel={t('add_server_modal.run_title')}
                    value={managed ? 'managed' : 'unmanaged'}
                    onChange={(id) => setManaged(id === 'managed')}
                    items={[
                        { id: 'managed', label: t('add_server_modal.run_managed') },
                        { id: 'unmanaged', label: t('add_server_modal.run_unmanaged') },
                    ]}
                />
                <p className="as-provision-hint">
                    {t(managed ? 'add_server_modal.run_managed_hint' : 'add_server_modal.run_unmanaged_hint')}
                </p>
            </div>
        );
    } else if (step === 'name') {
        content = (
            <>
                <div className="as-section as-section-card">
                    <label className="as-label" htmlFor="as-server-name">
                        <Icon name="server" size="sm" />
                        {t('add_server_modal.server_name')}
                    </label>
                    <input
                        id="as-server-name"
                        ref={nameInputRef}
                        className="as-input"
                        type="text"
                        placeholder={t('add_server_modal.server_name_placeholder')}
                        value={newServerName}
                        onChange={(e) => setNewServerName(e.target.value)}
                        disabled={submitting}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter' && canLeave.name) goTo(steps[stepIndex + 1]);
                        }}
                        aria-invalid={duplicateName || undefined}
                    />
                    {duplicateName && (
                        <p className="as-name-taken" role="alert">
                            <Icon name="alertCircle" size="sm" />
                            {t('servers.rename_name_taken', { name: newServerName.trim() })}
                        </p>
                    )}
                </div>
                {(mainPort || port != null) && (
                    <div className="as-section as-section-card">
                        <label className="as-label" htmlFor="as-server-port">
                            <Icon name="network" size="sm" />
                            {t('add_server_modal.port')}
                        </label>
                        <input
                            id="as-server-port"
                            className="as-input"
                            type="number"
                            min={1}
                            max={65535}
                            value={port ?? String(mainPort?.port ?? '')}
                            aria-invalid={portInvalid || undefined}
                            onChange={(e) => setPort(e.target.value)}
                            disabled={submitting}
                        />
                        {portInvalid ? (
                            <p className="as-name-taken" role="alert">
                                <Icon name="alertCircle" size="sm" />
                                {t('add_server_modal.port_invalid')}
                            </p>
                        ) : (
                            <p className="as-provision-hint">{t('add_server_modal.port_hint')}</p>
                        )}
                    </div>
                )}
                <CheckNotes t={t} check={check} checking={checking} />
            </>
        );
    } else {
        const settingRows = installFields(selectedModule)
            .filter((f) => installValues[f.name] != null && installValues[f.name] !== '')
            .map((f) => [
                t(`mod_${selectedModule.name}:settings.${f.name}.label`, { defaultValue: f.label }),
                f.field_type === 'select'
                    ? optionLabel(t, selectedModule.name, f, installValues[f.name])
                    : String(installValues[f.name]),
            ]);
        const rows = [
            [t('add_server_modal.summary_game'), selectedDisplayName],
            ...settingRows,
            ...(choosesVersion(selectedModule)
                ? [[t('add_server_modal.version'), version || t('add_server_modal.version_latest')]]
                : []),
            [t('add_server_modal.location_title'), customFolder || t('add_server_modal.location_default')],
            [
                t('add_server_modal.run_title'),
                t(managed ? 'add_server_modal.run_managed' : 'add_server_modal.run_unmanaged'),
            ],
            [t('add_server_modal.server_name'), newServerName.trim()],
            ...ports.map((p) => [`${t('add_server_modal.port')} (${p.field})`, String(p.port)]),
        ];
        content = (
            <>
                <dl className="as-summary as-section-card">
                    {rows.map(([label, value]) => (
                        <div className="as-summary-row" key={label}>
                            <dt>{label}</dt>
                            <dd>{value}</dd>
                        </div>
                    ))}
                </dl>
                <CheckNotes t={t} check={check} checking={checking} />

                {/* 확인 안 된·바뀐 모듈: 게임 서버는 격리되지 않는다 */}
                <TrustCreateNotice trust={selectedModule?.trust} />

                {/* 모듈이 선언한 약관: 모두 동의해야 만들 수 있다 */}
                {agreements.length > 0 && (
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="file" size="sm" />
                            {t('agreements.title')}
                        </span>
                        <AgreementList
                            module={selectedExtension}
                            agreements={agreements}
                            checked={agreed}
                            onToggle={(id, on) => setAgreed((prev) => ({ ...prev, [id]: on }))}
                            disabled={submitting}
                        />
                    </div>
                )}

                {/* 익스텐션 슬롯 — 서버 생성 옵션 */}
                <ExtensionSlot slotId="AddServer.options" t={t} />
                <AddServerOptions
                    values={extValues}
                    onChange={(field, value) => setExtValues((prev) => ({ ...prev, [field]: value }))}
                    disabled={submitting}
                />

                {/* 프로비저닝 안내 */}
                <p className="as-provision-hint">
                    <Icon name="info" size="sm" />
                    {t('add_server_modal.provision_hint')}
                </p>
            </>
        );
    }

    const isLast = step === 'confirm';
    const next = () => (step === 'location' ? leaveLocation() : goTo(steps[stepIndex + 1]));
    const nextDisabled = submitting || leaving || (step in canLeave && !canLeave[step]);

    return (
        <div className="add-instance-stage" key={`new-${step}`}>
            {/* ── Header ── */}
            <div className="modal-header add-server-header">
                <div className="as-header-row">
                    <button className="ai-back-btn" type="button" onClick={goBack} disabled={submitting}>
                        <Icon name="chevronLeft" size="sm" />
                    </button>
                    <div className="as-header-copy">
                        <h3>{t(`add_server_modal.${step}_step_title`)}</h3>
                        <p className="add-server-subtitle">
                            {t('add_server_modal.step_counter', { current: stepIndex, total: steps.length - 1 })}
                            {' · '}
                            {t(`add_server_modal.${step}_step_subtitle`)}
                        </p>
                    </div>
                </div>
            </div>

            {/* ── Body ── */}
            <div className="modal-body add-server-body">
                <div className="as-body-stack">
                    {/* 선택된 게임 요약 */}
                    <div className="as-selected-game">
                        <div className="as-selected-game-icon">
                            {selectedModule?.icon ? (
                                <img src={selectedModule.icon} alt={selectedDisplayName} />
                            ) : (
                                <div className="as-game-card-icon-placeholder">
                                    <Icon name="gamepad" size="md" />
                                </div>
                            )}
                        </div>
                        <div className="as-selected-game-info">
                            <span className="as-selected-game-name">{selectedDisplayName}</span>
                            <span className="as-selected-game-version">v{selectedModule?.version}</span>
                        </div>
                    </div>
                    {content}
                </div>
            </div>

            {/* ── Footer ── */}
            <div className="modal-footer add-server-footer">
                <button className="btn btn-cancel" onClick={onClose} disabled={submitting}>
                    {t('modals.cancel')}
                </button>
                <button className="btn btn-secondary" type="button" onClick={goBack} disabled={submitting}>
                    {t('add_server_modal.prev')}
                </button>
                {isLast ? (
                    <button className="btn btn-confirm" onClick={handleSubmit} disabled={!canSubmit}>
                        {submitting ? (
                            <>
                                <Icon name="refresh" size="sm" className="spin" /> {t('add_server_modal.provisioning')}
                            </>
                        ) : (
                            <>
                                <Icon name="plus" size="sm" /> {t('add_server_modal.add_server')}
                            </>
                        )}
                    </button>
                ) : (
                    <button className="btn btn-confirm" type="button" onClick={next} disabled={nextDisabled}>
                        {t('add_server_modal.next')}
                    </button>
                )}
            </div>
        </div>
    );
}

/** 데몬 확인 결과 중 알릴 것: 막는 것(이름·폴더)은 경고로, 다른 서버와 같은 포트·예약된 포트는 안내로 */
function CheckNotes({ t, check, checking }) {
    if (checking && !check) return <p className="as-provision-hint">{t('add_server_modal.checking')}</p>;
    if (!check) return null;
    if (check.failed) return <p className="as-provision-hint">{t('add_server_modal.check_failed')}</p>;
    const notes = [];
    if (check.folder?.in_use_by)
        notes.push(['alert', t('add_server_modal.folder_in_use', { name: check.folder.in_use_by })]);
    if (check.folder?.not_empty) notes.push(['alert', t('add_server_modal.folder_not_empty')]);
    for (const p of check.ports || []) {
        if (p.in_use_by?.length) {
            notes.push(['note', t('add_server_modal.port_in_use', { port: p.port, servers: p.in_use_by.join(', ') })]);
        }
        if (p.reserved) notes.push(['alert', t('add_server_modal.port_reserved', { port: p.port })]);
    }
    return notes.map(([kind, text]) => (
        <p
            key={text}
            className={kind === 'alert' ? 'as-name-taken' : 'as-provision-hint'}
            role={kind === 'alert' ? 'alert' : undefined}
        >
            <Icon name={kind === 'alert' ? 'alertCircle' : 'info'} size="sm" />
            {text}
        </p>
    ));
}

/** 만들 때 고르는 설정 칸 하나. 고르지 않은 칸은 보내지 않는다 (데몬이 모듈 기본값을 채운다) */
function InstallField({ t, module, field, value, onChange, disabled }) {
    const modNs = `mod_${module.name}`;
    const label = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
    const description = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    const inputId = `as-install-${field.name}`;
    const shown = value ?? field.default;

    let control;
    if (field.field_type === 'select') {
        control = (
            <CustomDropdown
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
                className="as-input"
                type={field.field_type === 'number' ? 'number' : 'text'}
                value={String(shown ?? '')}
                min={field.min}
                max={field.max}
                onChange={(e) => {
                    const raw = e.target.value;
                    onChange(field.field_type === 'number' && raw !== '' ? Number(raw) : raw);
                }}
                disabled={disabled}
            />
        );
    }

    return (
        <div className={clsx('as-section', 'as-section-card', { 'as-section-row': field.field_type === 'boolean' })}>
            <label
                className="as-label"
                htmlFor={field.field_type === 'select' || field.field_type === 'boolean' ? undefined : inputId}
            >
                <Icon name="settings" size="sm" />
                {label}
            </label>
            {control}
            {description && <p className="as-provision-hint">{description}</p>}
        </div>
    );
}
