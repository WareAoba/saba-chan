import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import type { ApiBody, ModuleInfo } from '../../types/daemon';
import { AgreementList, allChecked } from '../AgreementList';
import clsx from 'clsx';
import Icon from '../Icon';
import { Card, SettingRow } from '../ServerSettingsParts';
import { SabaSpinner } from '../ui/SabaUI';
import { TrustCreateNotice } from '../ui/TrustBadge';
import type { AddServerPayload } from '../../stores/serverActions';
import { AddServerFrame } from './AddServerFrame';
import { isImeKey } from '../../utils/ime';

/**
 * AddInstanceMigration — "기존 서버 마이그레이션" 흐름.
 *
 * 2단계 구성:
 *   step 'pick-dir'   — 디렉토리 선택 → 스캔 → 모듈 자동 감지
 *   step 'configure'  — 감지된 (또는 수동 선택한) 모듈로 인스턴스 이름 설정 → 생성
 *
 * 자동 감지 실패 시:
 *   - 로컬에 설치된 모듈 중 dir_signatures가 매치되는 것이 없으면
 *   - 레지스트리에서 모듈 목록을 fetch 하여 수동 선택 가능하게 보여준다
 */
interface AddInstanceMigrationProps {
    extensions: ModuleInfo[];
    servers: Server[];
    onAddServer: (payload: AddServerPayload) => Promise<unknown>;
    onBack: () => void;
}

export function AddInstanceMigration({
    extensions,
    servers,
    onAddServer,
    onBack,
}: AddInstanceMigrationProps) {
    const { t } = useTranslation('gui');
    const [step, setStep] = useState('pick-dir'); // 'pick-dir' | 'configure'

    // -- pick-dir state --
    const [dirPath, setDirPath] = useState('');
    const [scanning, setScanning] = useState(false);
    const [scanError, setScanError] = useState<string | null>(null);
    const [detectedModule, setDetectedModule] = useState<ApiBody | null>(null); // { name, displayName, icon, ... }
    const [noMatch, setNoMatch] = useState(false);
    // 그 폴더에서 미리 알릴 것 (데몬 detect `notices`: server_running·folder_not_writable)
    const [notices, setNotices] = useState<string[]>([]);

    // -- manifest fallback state --
    const [manifestModules, setManifestModules] = useState<ApiBody[] | null>(null);
    const [manifestLoading, setManifestLoading] = useState(false);
    const [manifestError, setManifestError] = useState<string | null>(null);
    const [manualModule, setManualModule] = useState<ApiBody | null>(null);

    // -- configure state --
    const [instanceName, setInstanceName] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const [agreed, setAgreed] = useState<Record<string, boolean>>({}); // 약관 id → 체크 여부
    // 'link': 서버 폴더를 그 자리에 연결 / 'move': 사바쨩 인스턴스 폴더로 옮겨 전속 (데몬 `migration_mode`)
    const [mode, setMode] = useState('link');
    const nameInputRef = useRef<HTMLInputElement>(null);

    // ─── 디렉토리 선택 ───
    const handleBrowse = useCallback(async () => {
        const selected = await window.api?.openFolderDialog?.();
        if (selected) {
            setDirPath(selected);
            // 선택하면 자동 스캔
            performScan(selected);
        }
    }, [extensions]); // eslint-disable-line react-hooks/exhaustive-deps

    // ─── 디렉토리 스캔 + 모듈 감지 ───
    const performScan = useCallback(
        async (path: string) => {
            setScanning(true);
            setScanError(null);
            setDetectedModule(null);
            setNoMatch(false);
            setManualModule(null);
            setNotices([]);

            try {
                const result = await window.api?.migrationScanDir?.(path);
                if (!result?.ok) {
                    setScanError(result?.error || t('migration_modal.scan_error'));
                    setScanning(false);
                    return;
                }

                const withName = (ext: ModuleInfo) => ({
                    ...ext,
                    displayName: t(`mod_${ext.name}:module.display_name`, {
                        defaultValue: ext.display_name || ext.game_name || ext.name,
                    }),
                });

                // scanDir가 곧 POST /api/modules/detect다: 데몬이 module.toml [detection].dir_signatures 로 판정한 결과
                setNotices(Array.isArray(result.notices) ? result.notices.filter((n: unknown) => typeof n === 'string') : []);
                const names = (result.matches || []).map((m: ApiBody) => m.name);
                const ext = names.map((n: string) => extensions.find((e) => e.name === n)).find(Boolean);
                const matched = ext ? withName(ext) : null;

                if (matched) {
                    setDetectedModule(matched);
                } else {
                    setNoMatch(true);
                }
            } catch (err) {
                setScanError((err as Error).message);
            }
            setScanning(false);
        },
        [extensions, t],
    );

    // ─── 매니페스트 fetch (감지 실패 시) ───
    const fetchManifest = useCallback(async () => {
        if (manifestModules !== null) return; // 이미 로드됨
        setManifestLoading(true);
        setManifestError(null);
        try {
            const res = await window.api?.moduleManifest?.();
            if (res?.ok && res.manifest?.modules) {
                const mods = Object.entries(res.manifest.modules as Record<string, ApiBody>).map(([id, info]) => ({
                    id,
                    ...info,
                }));
                setManifestModules(mods);
            } else {
                setManifestError(res?.error || t('migration_modal.manifest_fetch_failed'));
                setManifestModules([]);
            }
        } catch (e) {
            setManifestError((e as Error).message);
            setManifestModules([]);
        }
        setManifestLoading(false);
    }, [manifestModules, t]);

    // noMatch가 되면 자동으로 매니페스트 페치
    useEffect(() => {
        if (noMatch) fetchManifest();
    }, [noMatch, fetchManifest]);

    // ─── 감지 결과로 다음 단계 ───
    const proceedWithModule = useCallback(
        (mod: ApiBody) => {
            const name = mod.name || mod.id;
            const existingCount = servers?.filter((s) => s.module === name).length || 0;
            setInstanceName(`migrated-${name}-${existingCount + 1}`);
            setAgreed({});
            setStep('configure');
        },
        [servers],
    );

    // configure 단계 진입 시 포커스
    useEffect(() => {
        if (step === 'configure' && nameInputRef.current) {
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    // 설치된 모듈 목록(GET /api/modules)이 약관을 싣는다. 레지스트리에서 고른 모듈은 설치 뒤에 데몬이 묻는다
    const agreementModuleName = (detectedModule || manualModule)?.name || (detectedModule || manualModule)?.id;
    const installedModule = extensions.find((e) => e.name === agreementModuleName);
    const agreements = installedModule?.agreements || [];

    // ─── 제출 ───
    const handleSubmit = useCallback(async () => {
        const mod = detectedModule || manualModule;
        if (!mod || !instanceName.trim()) return;

        setSubmitting(true);
        try {
            await onAddServer({
                name: instanceName.trim(),
                module_name: mod.name || mod.id,
                migration_source: dirPath,
                migration_mode: mode,
                // 사용자가 체크한 약관 (module.toml [[agreements]]): 데몬이 만든 직후 모듈에 동의를 남긴다
                ...(agreements.length ? { accept_agreements: agreements.map((a) => a.id) } : {}),
            });
        } finally {
            setSubmitting(false);
        }
    }, [detectedModule, manualModule, instanceName, dirPath, mode, onAddServer, agreements]);

    const handleBackToPickDir = useCallback(() => {
        setStep('pick-dir');
        setInstanceName('');
    }, []);

    const activeModule = detectedModule || manualModule;
    const activeModuleName = activeModule
        ? activeModule.displayName || activeModule.display_name || activeModule.name || activeModule.id
        : '';

    const canSubmit = instanceName.trim() && activeModule && !submitting && allChecked(agreements, agreed);

    const gameIcon = (icon?: string) => (
        <span className="as-game-icon" aria-hidden="true">
            {icon ? <img src={icon} alt="" /> : <Icon name="gamepad" size="md" />}
        </span>
    );

    // 켜져 있음·쓸 수 없음: 불러오기를 누르기 전에 보인다
    const noticeBlock = notices.length > 0 && (
        <div className="mg-notices">
            {notices.includes('server_running') && (
                <div className="notice tone-warn mg-notice">
                    <Icon name="alertTriangle" size="sm" />
                    <div>
                        <strong>{t('migration_modal.running_title')}</strong>
                        <p>{t('migration_modal.running_body')}</p>
                    </div>
                </div>
            )}
            {notices.includes('folder_not_writable') && (
                <div className="notice tone-warn mg-notice">
                    <Icon name="alertTriangle" size="sm" />
                    <div>
                        <p>{t('server_actions.migrate_notice_folder_not_writable')}</p>
                    </div>
                </div>
            )}
        </div>
    );

    // ══════════════════════════════════════════
    //  Step 1: 폴더 고르기 + 스캔
    // ══════════════════════════════════════════
    if (step === 'pick-dir') {
        return (
            <AddServerFrame
                title={t('migration_modal.title')}
                description={t('migration_modal.description')}
                footer={
                    <>
                        <button type="button" className="btn btn-ghost as-foot-back" onClick={onBack}>
                            <Icon name="chevronLeft" size="sm" />
                            {t('add_server_modal.prev')}
                        </button>
                        {noMatch && manualModule && (
                            <button type="button" className="btn btn-primary" onClick={() => proceedWithModule(manualModule)}>
                                {t('migration_modal.next')}
                            </button>
                        )}
                    </>
                }
            >
                <div className="ss-panel ss-form">
                    <section className="dc-card ss-card">
                        <div className="ss-rows">
                            <SettingRow
                                label={t('migration_modal.source_directory')}
                                stack
                                control={
                                    <div className="as-path-row">
                                        <input
                                            className="input ss-text mono"
                                            type="text"
                                            aria-label={t('migration_modal.source_directory')}
                                            value={dirPath}
                                            onChange={(e) => {
                                                // 감지 결과는 스캔한 경로의 것이다: 경로를 고치면 다시 스캔해야 한다
                                                setDirPath(e.target.value);
                                                setDetectedModule(null);
                                                setNoMatch(false);
                                                setManualModule(null);
                                                setScanError(null);
                                            }}
                                            placeholder={t('migration_modal.source_placeholder')}
                                            disabled={scanning}
                                        />
                                        <button
                                            type="button"
                                            className="icon-btn"
                                            onClick={handleBrowse}
                                            disabled={scanning}
                                            data-tooltip={t('migration_modal.browse')}
                                            aria-label={t('migration_modal.browse')}
                                        >
                                            <Icon name="folder" size="sm" />
                                        </button>
                                        {/* 경로를 직접 적었으면 찾기 */}
                                        {dirPath && !scanning && !detectedModule && !noMatch && (
                                            <button type="button" className="btn btn-ghost" onClick={() => performScan(dirPath)}>
                                                <Icon name="search" size="sm" />
                                                {t('migration_modal.scan_button')}
                                            </button>
                                        )}
                                    </div>
                                }
                            />
                        </div>
                    </section>

                    {scanning && (
                        <p className="dc-hint as-status">
                            <SabaSpinner size="sm" /> {t('migration_modal.scanning')}
                        </p>
                    )}
                    {scanError && (
                        <p className="dc-field-error" role="alert">
                            <Icon name="alertTriangle" size="sm" /> {scanError}
                        </p>
                    )}

                    {/* ── 감지 성공 ── */}
                    {detectedModule && (
                        <section className="dc-card ss-card">
                            <div className="as-found">
                                {gameIcon(detectedModule.icon)}
                                <div className="as-found-text">
                                    <strong>{detectedModule.displayName}</strong>
                                    <span className="dc-hint">
                                        <Icon name="check" size="xs" /> {t('migration_modal.auto_detected')}
                                    </span>
                                </div>
                                <button type="button" className="btn btn-primary" onClick={() => proceedWithModule(detectedModule)}>
                                    {t('migration_modal.next')}
                                </button>
                            </div>
                        </section>
                    )}

                    {(detectedModule || noMatch) && noticeBlock}

                    {/* ── 감지 실패: 레지스트리 모듈 목록에서 고른다 ── */}
                    {noMatch && (
                        <Card
                            icon="download"
                            title={t('migration_modal.select_module_manually')}
                            desc={t('migration_modal.no_auto_detect')}
                        >
                            {manifestLoading && (
                                <p className="dc-hint as-status">
                                    <SabaSpinner size="sm" /> {t('migration_modal.loading_manifest')}
                                </p>
                            )}
                            {manifestError && (
                                <p className="dc-field-error" role="alert">
                                    <Icon name="alertTriangle" size="sm" /> {manifestError}
                                </p>
                            )}
                            {manifestModules && manifestModules.length > 0 && (
                                <div className="as-pick-list" role="listbox" aria-label={t('migration_modal.select_module_manually')}>
                                    {manifestModules.map((m) => {
                                        const isSelected = manualModule?.id === m.id;
                                        return (
                                            <button
                                                key={m.id}
                                                type="button"
                                                role="option"
                                                aria-selected={isSelected}
                                                className={clsx('as-pick-item', { selected: isSelected })}
                                                onClick={() => setManualModule(isSelected ? null : m)}
                                            >
                                                {gameIcon()}
                                                <span className="as-found-text">
                                                    <strong>{m.display_name || m.id}</strong>
                                                    {m.description && <span className="dc-hint">{m.description}</span>}
                                                </span>
                                                {isSelected && <Icon name="check" size="sm" />}
                                            </button>
                                        );
                                    })}
                                </div>
                            )}
                            {manifestModules && manifestModules.length === 0 && !manifestLoading && (
                                <p className="dc-hint">{t('migration_modal.no_modules_available')}</p>
                            )}
                        </Card>
                    )}
                </div>
            </AddServerFrame>
        );
    }

    // ══════════════════════════════════════════
    //  Step 2: 이름·폴더 다루는 방식 → 가져오기
    // ══════════════════════════════════════════
    return (
        <AddServerFrame
            title={t('migration_modal.configure_title')}
            description={t('migration_modal.configure_subtitle')}
            media={gameIcon(activeModule?.icon)}
            footer={
                <>
                    <button
                        type="button"
                        className="btn btn-ghost as-foot-back"
                        onClick={handleBackToPickDir}
                        disabled={submitting}
                    >
                        <Icon name="chevronLeft" size="sm" />
                        {t('add_server_modal.prev')}
                    </button>
                    <button type="button" className="btn btn-primary" onClick={handleSubmit} disabled={!canSubmit}>
                        {submitting ? (
                            <>
                                <SabaSpinner size="sm" /> {t('migration_modal.executing')}
                            </>
                        ) : (
                            t('migration_modal.execute_button')
                        )}
                    </button>
                </>
            }
        >
            <div className="ss-panel ss-form">
                <section className="dc-card ss-card">
                    <div className="ss-rows">
                        <SettingRow
                            label={activeModuleName}
                            desc={
                                <span className="mono">
                                    <Icon name="folder" size="xs" /> {dirPath}
                                </span>
                            }
                            control={
                                <button type="button" className="btn btn-ghost btn-sm" onClick={handleBackToPickDir} disabled={submitting}>
                                    {t('add_server_modal.change_game')}
                                </button>
                            }
                        />
                        <SettingRow
                            label={t('migration_modal.instance_name')}
                            htmlFor="mg-instance-name"
                            control={
                                <input
                                    id="mg-instance-name"
                                    ref={nameInputRef}
                                    className="input ss-text"
                                    type="text"
                                    placeholder={t('migration_modal.instance_name_placeholder')}
                                    value={instanceName}
                                    onChange={(e) => setInstanceName(e.target.value)}
                                    disabled={submitting}
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter' && !isImeKey(e) && canSubmit) handleSubmit();
                                    }}
                                />
                            }
                        />
                        <SettingRow
                            label={t('migration_modal.mode_label')}
                            stack
                            control={
                                <div className="mg-modes" role="radiogroup" aria-label={t('migration_modal.mode_label')}>
                                    {(['link', 'move'] as const).map((id) => (
                                        <button
                                            key={id}
                                            type="button"
                                            role="radio"
                                            aria-checked={mode === id}
                                            className="radio-card mg-mode"
                                            onClick={() => !submitting && setMode(id)}
                                        >
                                            <span className="radio-card-title mg-mode-title">
                                                {t(`migration_modal.mode_${id}`)}
                                                {id === 'link' && <span className="mg-mode-tag">{t('migration_modal.recommended')}</span>}
                                            </span>
                                            <span className="mg-mode-desc">{t(`migration_modal.mode_${id}_desc`)}</span>
                                        </button>
                                    ))}
                                </div>
                            }
                        />
                    </div>
                </section>

                {noticeBlock}

                {/* 불러오면 바뀌는 것: 되돌릴 수 있는지까지 */}
                <Card icon="info" title={t('migration_modal.changes_title')}>
                    <dl className="mg-changes">
                        {['start', 'settings', 'access', 'backup'].map((row) => (
                            <div key={row} className="mg-change">
                                <dt>{t(`migration_modal.change_${row}`)}</dt>
                                <dd>
                                    {t(`migration_modal.change_${row}_desc`)}
                                    <span className={`mg-change-undo${row === 'backup' ? ' mg-change-undo--warn' : ''}`}>{t(`migration_modal.change_${row}_undo`)}</span>
                                </dd>
                            </div>
                        ))}
                    </dl>
                </Card>

                {/* 편해지는 점 */}
                <Card icon="star" title={t('migration_modal.benefits_title')}>
                    <ul className="mg-benefits">
                        {['console', 'discord', 'settings', 'backup'].map((b) => (
                            <li key={b}>
                                <Icon name="check" size="xs" /> {t(`migration_modal.benefit_${b}`)}
                            </li>
                        ))}
                    </ul>
                </Card>

                {/* 확인 안 된·바뀐 모듈: 게임 서버는 격리되지 않는다 */}
                <TrustCreateNotice trust={installedModule?.trust} />

                {/* 모듈이 선언한 약관: 모두 동의해야 가져올 수 있다 */}
                {agreements.length > 0 && (
                    <Card icon="file" title={t('agreements.title')}>
                        <AgreementList
                            module={agreementModuleName}
                            agreements={agreements}
                            checked={agreed}
                            onToggle={(id, on) => setAgreed((prev) => ({ ...prev, [id]: on }))}
                            disabled={submitting}
                        />
                    </Card>
                )}
            </div>
        </AddServerFrame>
    );
}
