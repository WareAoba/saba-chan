import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import Icon from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

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
export function AddInstanceMigration({
    extensions,
    servers,
    onAddServer,
    onBack,
    onClose,
}) {
    const { t } = useTranslation('gui');
    const [step, setStep] = useState('pick-dir'); // 'pick-dir' | 'configure'

    // -- pick-dir state --
    const [dirPath, setDirPath] = useState('');
    const [scanning, setScanning] = useState(false);
    const [scanError, setScanError] = useState(null);
    const [detectedModule, setDetectedModule] = useState(null); // { name, displayName, icon, ... }
    const [noMatch, setNoMatch] = useState(false);

    // -- registry fallback state --
    const [registryModules, setRegistryModules] = useState(null);
    const [registryLoading, setRegistryLoading] = useState(false);
    const [registryError, setRegistryError] = useState(null);
    const [manualModule, setManualModule] = useState(null);

    // -- configure state --
    const [instanceName, setInstanceName] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const nameInputRef = useRef(null);

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
    const performScan = useCallback(async (path) => {
        setScanning(true);
        setScanError(null);
        setDetectedModule(null);
        setNoMatch(false);
        setManualModule(null);

        try {
            const result = await window.api?.migrationScanDir?.(path);
            if (!result?.ok) {
                setScanError(result?.error || t('migration_modal.scan_error'));
                setScanning(false);
                return;
            }

            const fileLower = result.files.map((f) => f.toLowerCase());

            // 각 로컬 모듈의 dir_signatures 와 매치
            let matched = null;
            for (const ext of extensions) {
                const sigs = ext.dir_signatures;
                if (!sigs || sigs.length === 0) continue;

                const allMatch = sigs.every((sig) =>
                    fileLower.includes(sig.toLowerCase()),
                );
                if (allMatch) {
                    const displayName = t(`mod_${ext.name}:module.display_name`, {
                        defaultValue: ext.game_name || ext.name,
                    });
                    matched = { ...ext, displayName };
                    break;
                }
            }

            if (matched) {
                setDetectedModule(matched);
            } else {
                setNoMatch(true);
            }
        } catch (err) {
            setScanError(err.message);
        }
        setScanning(false);
    }, [extensions, t]);

    // ─── 레지스트리 fetch (감지 실패 시) ───
    const fetchRegistry = useCallback(async () => {
        if (registryModules !== null) return; // 이미 로드됨
        setRegistryLoading(true);
        setRegistryError(null);
        try {
            const res = await window.api?.moduleRegistry?.();
            if (res?.ok && res.registry?.modules) {
                const mods = Object.entries(res.registry.modules).map(([id, info]) => ({
                    id,
                    ...info,
                }));
                setRegistryModules(mods);
            } else {
                setRegistryError(
                    res?.error || t('migration_modal.registry_fetch_failed'),
                );
                setRegistryModules([]);
            }
        } catch (e) {
            setRegistryError(e.message);
            setRegistryModules([]);
        }
        setRegistryLoading(false);
    }, [registryModules, t]);

    // noMatch가 되면 자동으로 레지스트리 페치
    useEffect(() => {
        if (noMatch) fetchRegistry();
    }, [noMatch, fetchRegistry]);

    // ─── 감지 결과로 다음 단계 ───
    const proceedWithModule = useCallback((mod) => {
        const name = mod.name || mod.id;
        const existingCount = servers?.filter((s) => s.module === name).length || 0;
        setInstanceName(`migrated-${name}-${existingCount + 1}`);
        setStep('configure');
    }, [servers]);

    // configure 단계 진입 시 포커스
    useEffect(() => {
        if (step === 'configure' && nameInputRef.current) {
            const timer = setTimeout(() => nameInputRef.current?.focus(), 80);
            return () => clearTimeout(timer);
        }
    }, [step]);

    // ─── 제출 ───
    const handleSubmit = useCallback(async () => {
        const mod = detectedModule || manualModule;
        if (!mod || !instanceName.trim()) return;

        setSubmitting(true);
        try {
            await onAddServer({
                name: instanceName.trim(),
                module_name: mod.name || mod.id,
                accept_eula: true,
                migration_source: dirPath,
            });
        } finally {
            setSubmitting(false);
        }
    }, [detectedModule, manualModule, instanceName, dirPath, onAddServer]);

    const handleBackToPickDir = useCallback(() => {
        setStep('pick-dir');
        setInstanceName('');
    }, []);

    const activeModule = detectedModule || manualModule;
    const activeModuleName = activeModule
        ? (activeModule.displayName || activeModule.display_name || activeModule.name || activeModule.id)
        : '';

    // 모듈별 네이티브 인스턴스 존재 여부
    const activeModuleId = activeModule?.name || activeModule?.id;
    const existingNative = activeModuleId
        ? servers.find((s) => s.module === activeModuleId && !s.extension_data?.docker_enabled)
        : null;

    const canSubmit = instanceName.trim() && activeModule && !submitting && !existingNative;

    // ══════════════════════════════════════════
    //  Step 1: 디렉토리 선택 + 스캔
    // ══════════════════════════════════════════
    if (step === 'pick-dir') {
        return (
            <>
                <div className="modal-header add-server-header">
                    <div>
                        <button className="ai-back-btn" type="button" onClick={onBack}>
                            <Icon name="chevronLeft" size="sm" />
                        </button>
                        <h3>{t('migration_modal.title')}</h3>
                        <p className="add-server-subtitle">{t('migration_modal.description')}</p>
                    </div>
                </div>

                <div className="modal-body add-server-body">
                    {/* 디렉토리 선택 */}
                    <div className="as-section">
                        <label className="as-label">
                            <Icon name="folder" size="sm" />
                            {t('migration_modal.source_directory')}
                        </label>
                        <div className="as-path-row">
                            <input
                                className="as-input as-input-mono"
                                type="text"
                                value={dirPath}
                                onChange={(e) => setDirPath(e.target.value)}
                                placeholder={t('migration_modal.source_placeholder')}
                                disabled={scanning}
                            />
                            <button
                                className="as-btn-icon"
                                onClick={handleBrowse}
                                disabled={scanning}
                                title={t('migration_modal.browse')}
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
                            <SabaSpinner size={20} />
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

                    {/* ── 감지 실패: 레지스트리 모듈 목록 ── */}
                    {noMatch && (
                        <div className="mg-no-match">
                            <p className="mg-no-match-hint">
                                <Icon name="alertCircle" size="sm" />
                                {t('migration_modal.no_auto_detect')}
                            </p>

                            <label className="as-label">
                                <Icon name="download" size="sm" />
                                {t('migration_modal.select_module_manually')}
                            </label>

                            {registryLoading && (
                                <div className="mg-status mg-status--scanning">
                                    <SabaSpinner size={20} />
                                    <span>{t('migration_modal.loading_registry')}</span>
                                </div>
                            )}

                            {registryError && (
                                <div className="mg-status mg-status--error">
                                    <Icon name="alertCircle" size="sm" />
                                    <span>{registryError}</span>
                                </div>
                            )}

                            {registryModules && registryModules.length > 0 && (
                                <div className="mg-module-list">
                                    {registryModules.map((m) => {
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
                                                        <span className="mg-module-item-desc">
                                                            {m.description}
                                                        </span>
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

                            {registryModules && registryModules.length === 0 && !registryLoading && (
                                <p className="mg-no-match-hint">{t('migration_modal.no_modules_available')}</p>
                            )}
                        </div>
                    )}
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
            </>
        );
    }

    // ══════════════════════════════════════════
    //  Step 2: 인스턴스 이름 설정 → 생성
    // ══════════════════════════════════════════
    return (
        <>
            <div className="modal-header add-server-header">
                <div>
                    <button className="ai-back-btn" type="button" onClick={handleBackToPickDir} disabled={submitting}>
                        <Icon name="chevronLeft" size="sm" />
                    </button>
                    <h3>{t('migration_modal.configure_title')}</h3>
                    <p className="add-server-subtitle">{t('migration_modal.configure_subtitle')}</p>
                </div>
            </div>

            <div className="modal-body add-server-body">
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
                <div className="as-section">
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

                {/* 네이티브 인스턴스 중복 경고 */}
                {existingNative && (
                    <p className="as-native-limit-warning">
                        <Icon name="alertTriangle" size="sm" />
                        {t('add_server_modal.native_limit_warning', {
                            existing: existingNative.name,
                            defaultValue: `A native instance '{{existing}}' already exists for this module. Enable container isolation to create another instance.`,
                        })}
                    </p>
                )}

                {/* 마이그레이션 안내 */}
                <p className="as-provision-hint">
                    <Icon name="info" size="sm" />
                    {t('migration_modal.migrate_hint')}
                </p>
            </div>

            <div className="modal-footer add-server-footer">
                <button className="btn btn-cancel" onClick={onClose} disabled={submitting}>
                    {t('modals.cancel')}
                </button>
                <button className="btn btn-confirm" onClick={handleSubmit} disabled={!canSubmit}>
                    {submitting ? (
                        <>
                            <SabaSpinner size={14} /> {t('migration_modal.executing')}
                        </>
                    ) : (
                        <>
                            <Icon name="download" size="sm" /> {t('migration_modal.execute_button')}
                        </>
                    )}
                </button>
            </div>
        </>
    );
}
