import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import type { ApiBody, ModuleInfo } from '../../types/daemon';
import { AgreementList, allChecked } from '../AgreementList';
import Icon from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { TrustCreateNotice } from '../ui/TrustBadge';
import type { AddServerPayload } from '../../hooks/useServerActions';

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
    onStepChange?: (step: string, progress?: number) => void;
    onBack: () => void;
    onClose: () => void;
}

export function AddInstanceMigration({
    extensions,
    servers,
    onAddServer,
    onStepChange,
    onBack,
    onClose,
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
            onStepChange?.('configure'); // 높이 캡처 후 부모 상태 변경 (배칭)
            const name = mod.name || mod.id;
            const existingCount = servers?.filter((s) => s.module === name).length || 0;
            setInstanceName(`migrated-${name}-${existingCount + 1}`);
            setAgreed({});
            setStep('configure');
        },
        [servers, onStepChange],
    );

    // configure 단계 진입 시 포커스
    useEffect(() => {
        if (step === 'configure' && nameInputRef.current) {
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    // onStepChange는 proceedWithModule/handleBackToPickDir에서 동기적으로 호출
    // (useEffect로 호출하면 paint 이후라 높이 애니메이션 캡처가 실패함)

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
        onStepChange?.('pick-dir'); // 높이 캡처 후 부모 상태 변경 (배칭)
        setStep('pick-dir');
        setInstanceName('');
    }, [onStepChange]);

    const activeModule = detectedModule || manualModule;
    const activeModuleName = activeModule
        ? activeModule.displayName || activeModule.display_name || activeModule.name || activeModule.id
        : '';

    const canSubmit = instanceName.trim() && activeModule && !submitting && allChecked(agreements, agreed);

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
    //  Step 1: 디렉토리 선택 + 스캔
    // ══════════════════════════════════════════
    if (step === 'pick-dir') {
        return (
            <div className="add-instance-stage" key="migration-pick-dir">
                <div className="modal-header add-server-header">
                    <div className="as-header-row">
                        <button className="ai-back-btn" type="button" onClick={onBack}>
                            <Icon name="chevronLeft" size="sm" />
                        </button>
                        <div className="as-header-copy">
                            <h3>{t('migration_modal.title')}</h3>
                            <p className="add-server-subtitle">{t('migration_modal.description')}</p>
                        </div>
                    </div>
                </div>

                <div className="modal-body add-server-body">
                    <div className="as-body-stack">
                        {/* 디렉토리 선택 */}
                        <div className="as-section as-section-card">
                            <label className="as-label">
                                <Icon name="folder" size="sm" />
                                {t('migration_modal.source_directory')}
                            </label>
                            <div className="as-path-row">
                                <input
                                    className="as-input as-input-mono"
                                    type="text"
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
                                    className="as-btn-icon"
                                    onClick={handleBrowse}
                                    disabled={scanning}
                                    data-tooltip={t('migration_modal.browse')}
                                    aria-label={t('migration_modal.browse')}
                                >
                                    <Icon name="folder" size="sm" />
                                </button>
                            </div>
                        </div>

                        {/* 수동 스캔 버튼 (경로를 직접 입력한 경우) */}
                        {dirPath && !scanning && !detectedModule && !noMatch && (
                            <button
                                className="btn btn-confirm mg-scan-btn"
                                type="button"
                                onClick={() => performScan(dirPath)}
                            >
                                <Icon name="search" size="sm" />
                                {t('migration_modal.scan_button')}
                            </button>
                        )}

                        {/* 스캔 중 */}
                        {scanning && (
                            <div className="mg-status mg-status--scanning">
                                <SabaSpinner size="md" />
                                <span>{t('migration_modal.scanning')}</span>
                            </div>
                        )}

                        {/* 스캔 에러 */}
                        {scanError && (
                            <div className="mg-status mg-status--error">
                                <Icon name="alertCircle" size="sm" />
                                <span>{scanError}</span>
                            </div>
                        )}

                        {/* ── 감지 성공 ── */}
                        {detectedModule && (
                            <div className="mg-detected">
                                <div className="mg-detected-icon">
                                    {detectedModule.icon ? (
                                        <img src={detectedModule.icon} alt={detectedModule.displayName} />
                                    ) : (
                                        <div className="as-game-card-icon-placeholder">
                                            <Icon name="gamepad" size="md" />
                                        </div>
                                    )}
                                </div>
                                <div className="mg-detected-info">
                                    <span className="mg-detected-name">{detectedModule.displayName}</span>
                                    <span className="mg-detected-hint">
                                        <Icon name="check" size="xs" />
                                        {t('migration_modal.auto_detected')}
                                    </span>
                                </div>
                                <button
                                    className="btn btn-confirm"
                                    type="button"
                                    onClick={() => proceedWithModule(detectedModule)}
                                >
                                    {t('migration_modal.next')}
                                    <Icon name="chevronRight" size="sm" />
                                </button>
                            </div>
                        )}

                        {(detectedModule || noMatch) && noticeBlock}

                        {/* ── 감지 실패: 레지스트리 모듈 목록 ── */}
                        {noMatch && (
                            <div className="mg-no-match as-section-card">
                                <p className="mg-no-match-hint">
                                    <Icon name="alertCircle" size="sm" />
                                    {t('migration_modal.no_auto_detect')}
                                </p>

                                <label className="as-label">
                                    <Icon name="download" size="sm" />
                                    {t('migration_modal.select_module_manually')}
                                </label>

                                {manifestLoading && (
                                    <div className="mg-status mg-status--scanning">
                                        <SabaSpinner size="md" />
                                        <span>{t('migration_modal.loading_manifest')}</span>
                                    </div>
                                )}

                                {manifestError && (
                                    <div className="mg-status mg-status--error">
                                        <Icon name="alertCircle" size="sm" />
                                        <span>{manifestError}</span>
                                    </div>
                                )}

                                {manifestModules && manifestModules.length > 0 && (
                                    <div className="mg-module-list">
                                        {manifestModules.map((m) => {
                                            const isSelected = manualModule?.id === m.id;
                                            return (
                                                <button
                                                    key={m.id}
                                                    className={`mg-module-item ${isSelected ? 'mg-module-item--selected' : ''}`}
                                                    type="button"
                                                    onClick={() => setManualModule(isSelected ? null : m)}
                                                >
                                                    <div className="mg-module-item-icon">
                                                        <Icon name="gamepad" size="sm" />
                                                    </div>
                                                    <div className="mg-module-item-text">
                                                        <span className="mg-module-item-name">
                                                            {m.display_name || m.id}
                                                        </span>
                                                        {m.description && (
                                                            <span className="mg-module-item-desc">{m.description}</span>
                                                        )}
                                                    </div>
                                                    {isSelected && (
                                                        <Icon name="check" size="sm" className="mg-module-item-check" />
                                                    )}
                                                </button>
                                            );
                                        })}
                                    </div>
                                )}

                                {manifestModules && manifestModules.length === 0 && !manifestLoading && (
                                    <p className="mg-no-match-hint">{t('migration_modal.no_modules_available')}</p>
                                )}
                            </div>
                        )}
                    </div>
                </div>

                {/* Footer */}
                <div className="modal-footer add-server-footer">
                    <button className="btn btn-cancel" onClick={onClose}>
                        {t('modals.cancel')}
                    </button>
                    {noMatch && manualModule && (
                        <button
                            className="btn btn-confirm"
                            type="button"
                            onClick={() => proceedWithModule(manualModule)}
                        >
                            {t('migration_modal.next')}
                            <Icon name="chevronRight" size="sm" />
                        </button>
                    )}
                </div>
            </div>
        );
    }

    // ══════════════════════════════════════════
    //  Step 2: 인스턴스 이름 설정 → 생성
    // ══════════════════════════════════════════
    return (
        <div className="add-instance-stage" key="migration-configure">
            <div className="modal-header add-server-header">
                <div className="as-header-row">
                    <button className="ai-back-btn" type="button" onClick={handleBackToPickDir} disabled={submitting}>
                        <Icon name="chevronLeft" size="sm" />
                    </button>
                    <div className="as-header-copy">
                        <h3>{t('migration_modal.configure_title')}</h3>
                        <p className="add-server-subtitle">{t('migration_modal.configure_subtitle')}</p>
                    </div>
                </div>
            </div>

            <div className="modal-body add-server-body">
                <div className="as-body-stack">
                    {/* 선택된 소스 + 모듈 요약 */}
                    <div className="as-selected-game">
                        <div className="as-selected-game-icon">
                            {activeModule?.icon ? (
                                <img src={activeModule.icon} alt={activeModuleName} />
                            ) : (
                                <div className="as-game-card-icon-placeholder">
                                    <Icon name="gamepad" size="md" />
                                </div>
                            )}
                        </div>
                        <div className="as-selected-game-info">
                            <span className="as-selected-game-name">{activeModuleName}</span>
                            <span className="as-selected-game-version mg-source-path">
                                <Icon name="folder" size="xs" /> {dirPath}
                            </span>
                        </div>
                        <button
                            className="as-change-game-btn"
                            type="button"
                            onClick={handleBackToPickDir}
                            disabled={submitting}
                        >
                            {t('add_server_modal.change_game')}
                        </button>
                    </div>

                    {/* 인스턴스 이름 */}
                    <div className="as-section as-section-card">
                        <label className="as-label">
                            <Icon name="server" size="sm" />
                            {t('migration_modal.instance_name')}
                        </label>
                        <input
                            ref={nameInputRef}
                            className="as-input"
                            type="text"
                            placeholder={t('migration_modal.instance_name_placeholder')}
                            value={instanceName}
                            onChange={(e) => setInstanceName(e.target.value)}
                            disabled={submitting}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter' && canSubmit) handleSubmit();
                            }}
                        />
                    </div>

                    {noticeBlock}

                    {/* 서버 폴더를 다루는 방식 */}
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="folder" size="sm" />
                            {t('migration_modal.mode_label')}
                        </span>
                        <div className="mg-modes" role="radiogroup" aria-label={t('migration_modal.mode_label')}>
                            {(['link', 'move'] as const).map((id) => (
                                <button
                                    key={id}
                                    type="button"
                                    role="radio"
                                    aria-checked={mode === id}
                                    className={`mg-mode ${mode === id ? 'mg-mode--selected' : ''}`}
                                    onClick={() => !submitting && setMode(id)}
                                >
                                    <span className="mg-mode-title">
                                        {t(`migration_modal.mode_${id}`)}
                                        {id === 'link' && <span className="mg-mode-tag">{t('migration_modal.recommended')}</span>}
                                    </span>
                                    <span className="mg-mode-desc">{t(`migration_modal.mode_${id}_desc`)}</span>
                                </button>
                            ))}
                        </div>
                    </div>

                    {/* 불러오면 바뀌는 것: 되돌릴 수 있는지까지 */}
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="info" size="sm" />
                            {t('migration_modal.changes_title')}
                        </span>
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
                    </div>

                    {/* 편해지는 점 */}
                    <div className="as-section as-section-card">
                        <span className="as-label">
                            <Icon name="star" size="sm" />
                            {t('migration_modal.benefits_title')}
                        </span>
                        <ul className="mg-benefits">
                            {['console', 'discord', 'settings', 'backup'].map((b) => (
                                <li key={b}>
                                    <Icon name="check" size="xs" /> {t(`migration_modal.benefit_${b}`)}
                                </li>
                            ))}
                        </ul>
                    </div>

                    {/* 확인 안 된·바뀐 모듈: 게임 서버는 격리되지 않는다 */}
                    <TrustCreateNotice trust={installedModule?.trust} />

                    {/* 모듈이 선언한 약관: 모두 동의해야 가져올 수 있다 */}
                    {agreements.length > 0 && (
                        <div className="as-section as-section-card">
                            <span className="as-label">
                                <Icon name="file" size="sm" />
                                {t('agreements.title')}
                            </span>
                            <AgreementList
                                module={agreementModuleName}
                                agreements={agreements}
                                checked={agreed}
                                onToggle={(id, on) => setAgreed((prev) => ({ ...prev, [id]: on }))}
                                disabled={submitting}
                            />
                        </div>
                    )}

                </div>
            </div>

            <div className="modal-footer add-server-footer">
                <button className="btn btn-cancel" onClick={onClose} disabled={submitting}>
                    {t('modals.cancel')}
                </button>
                <button className="btn btn-confirm" onClick={handleSubmit} disabled={!canSubmit}>
                    {submitting ? (
                        <>
                            <SabaSpinner size="md" /> {t('migration_modal.executing')}
                        </>
                    ) : (
                        <>
                            <Icon name="download" size="sm" /> {t('migration_modal.execute_button')}
                        </>
                    )}
                </button>
            </div>
        </div>
    );
}
