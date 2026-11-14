import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import type { ModuleInfo } from '../../types/daemon';
import { byName, createTranslateError, safeShowToast } from '../../utils/helpers';
import { localizedDescription, localizedName } from '../../utils/localizedText';
import { type CatalogModule, unsupportedText } from '../../utils/moduleSupport';
import { formatBytes } from '../../utils/schedule';
import { Icon } from '../Icon';
import { ModuleMakerCard } from '../ModuleMakerCard';
import { clearModuleAuditCache } from '../servers/ModuleAuditNotice';
import { useContextMenu } from '../ui/ContextMenu';
import { Disclosure } from '../ui/Disclosure';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner } from '../ui/SabaUI';
import { TrustBadge } from '../ui/TrustBadge';
import { type AddonCard, useAddonVersions } from './AddonVersions';
import { QuestionModal } from './Modals';
import { useInstallingIds } from './useStoreInstall';

// ─────────────────────────────────────────────────────────────
// 모듈 카드 아이콘 (icon.png 로드 시도, 실패 시 package 아이콘)
// ─────────────────────────────────────────────────────────────
function ModuleCardIcon({ icon }: { icon?: string | null }) {
    if (!icon) {
        return (
            <div className="ss-card-icon">
                <Icon name="package" size="md" />
            </div>
        );
    }
    return (
        <div className="ss-card-icon">
            <img src={icon} alt="" />
        </div>
    );
}

// ─────────────────────────────────────────────────────────────
// 모듈 탭
// ─────────────────────────────────────────────────────────────
export function ModulesTab() {
    const { t, i18n } = useTranslation('gui');
    const openMenu = useContextMenu();
    const { refreshExtensions } = useExtensions();
    const [installedModules, setInstalledModules] = useState<ModuleInfo[] | null>(null); // null = 받는 중
    const [manifestModules, setManifestModules] = useState<CatalogModule[] | null>(null); // null = 미로드, [] = 로드됨
    const [manifestLoading, setManifestLoading] = useState(false);
    const [manifestError, setManifestError] = useState<string | null>(null);
    const { installingIds, run: runInstall } = useInstallingIds();
    const [installResults, setInstallResults] = useState<Record<string, string | null>>({}); // { id: 'ok'|'error' }
    const [removingIds, setRemovingIds] = useState<Set<string>>(new Set());
    const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
    const [refreshingModules, setRefreshingModules] = useState(false);

    // 설치된 모듈 로드
    const loadInstalledModules = useCallback(async () => {
        try {
            const res = await window.api?.moduleList?.();
            if (res?.modules) {
                clearModuleAuditCache();
                setInstalledModules(res.modules);
                return;
            }
        } catch (_) {}
        // 받지 못하면 빈 목록으로 (계속 받는 중으로 두지 않는다)
        setInstalledModules((prev) => prev ?? []);
    }, []);

    // 모듈 캐시 새로고침 (디스크 재스캔)
    const handleRefreshModules = useCallback(async () => {
        setRefreshingModules(true);
        try {
            const res = await window.api?.moduleRefresh?.();
            if (res?.modules) {
                clearModuleAuditCache();
                setInstalledModules(res.modules);
            }
        } catch (_) {}
        setRefreshingModules(false);
    }, []);

    useEffect(() => {
        loadInstalledModules();
    }, [loadInstalledModules]);

    const versions = useAddonVersions(loadInstalledModules);

    // 받을 수 있는 목록은 탭을 열면 바로 받는다 (예전처럼 ▾ 뒤에 접어 두면 지나치기 쉽다)
    const loadManifest = useCallback(async () => {
        setManifestModules(null);
        setManifestLoading(true);
        setManifestError(null);
        try {
            const res = await window.api?.moduleManifest?.();
            if (res?.success && res.manifest?.modules) {
                // manifest.modules: { id: { version, display_name, display_names?, descriptions?, platform_supported, ... } }
                const mods = Object.entries(res.manifest.modules as Record<string, Omit<CatalogModule, 'id'>>).map(([id, info]) => ({ ...info, id }));
                setManifestModules(mods);
            } else {
                setManifestError(
                    res?.error || t('saba_storage.manifest_fetch_failed', '매니페스트를 가져오지 못했습니다.'),
                );
                setManifestModules([]);
            }
        } catch (e) {
            setManifestError((e as Error).message);
            setManifestModules([]);
        }
        setManifestLoading(false);
    }, [t]);

    useEffect(() => {
        loadManifest();
    }, [loadManifest]);

    const handleInstallModule = useCallback(async (moduleId: string) => runInstall(moduleId, async () => {
        setInstallResults((prev) => ({ ...prev, [moduleId]: null }));
        try {
            const res = await window.api?.moduleInstallFromManifest?.(moduleId);
            if (res?.success) {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'ok' }));
                // 데몬이 이 모듈이 쓰는 익스텐션을 받고 켰다: 알리고 익스텐션 목록도 새로
                const ext = res.extensions;
                if (ext?.enabled?.length) {
                    safeShowToast(t('saba_storage.extensions_enabled', { ids: ext.enabled.join(', ') }), 'info', 4000);
                }
                for (const f of ext?.failed || []) {
                    const error = createTranslateError(t)(f.error);
                    safeShowToast(t('saba_storage.extension_failed', { id: f.id, error }), 'warning', 6000);
                }
                if (ext?.installed?.length || ext?.enabled?.length) refreshExtensions?.();
                // 설치 후 로컬 모듈 목록 갱신
                clearModuleAuditCache();
                const listRes = await window.api?.moduleRefresh?.();
                if (listRes?.modules) setInstalledModules(listRes.modules);
            } else if (res?.error_code === 'module_busy') {
                // 같은 모듈의 다른 설치·제거가 끝나지 않았다: 실패로 굳히지 않고 다시 누를 수 있게 둔다
                safeShowToast(t('saba_storage.module_busy'), 'warning', 4000);
            } else {
                setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
            }
        } catch (_) {
            setInstallResults((prev) => ({ ...prev, [moduleId]: 'error' }));
        }
    }), [t, refreshExtensions, runInstall]);

    const handleConfirmRemove = useCallback(async () => {
        const id = confirmRemoveId;
        setConfirmRemoveId(null);
        if (!id) return;
        setRemovingIds((prev) => new Set(prev).add(id));
        try {
            const res = await window.api?.moduleRemove?.(id);
            if (res?.success) await loadInstalledModules();
            else if (res?.error_code === 'module_busy') safeShowToast(t('saba_storage.module_busy'), 'warning', 4000);
            else if (res?.error_code === 'module_in_use') safeShowToast(t('saba_storage.module_in_use'), 'warning', 5000);
            else safeShowToast(t('saba_storage.remove_failed', { error: createTranslateError(t)(res?.error) }), 'error', 5000);
        } catch (e) {
            safeShowToast(t('saba_storage.remove_failed', { error: createTranslateError(t)((e as Error)?.message) }), 'error', 5000);
        }
        setRemovingIds((prev) => {
            const n = new Set(prev);
            n.delete(id);
            return n;
        });
    }, [confirmRemoveId, loadInstalledModules, t]);

    const sortedModules = [...(installedModules || [])].sort((a, b) => byName(a.name, b.name));
    const installedIds = new Set(
        sortedModules.map((m) => {
            // m.name 은 "Minecraft" 같은 display name, path 에서 디렉토리 이름 추출
            const parts = (m.path || '').replace(/\\/g, '/').split('/');
            return parts[parts.length - 1]; // 마지막 경로 컴포넌트 = 모듈 디렉토리명
        }),
    );

    // 모듈 ID 추론 (디렉토리명 = path 마지막 컴포넌트)
    const getModuleId = (m: ModuleInfo) => {
        const parts = (m.path || '').replace(/\\/g, '/').split('/');
        return parts[parts.length - 1] || m.name?.toLowerCase();
    };

    // 매니페스트에서 미설치 모듈
    const uninstalledModules = manifestModules ? manifestModules.filter((m) => !installedIds.has(m.id)) : [];
    const runnableModules = uninstalledModules.filter((m) => m.platform_supported !== false);
    const unrunnableModules = uninstalledModules.filter((m) => m.platform_supported === false);

    return (
        <div className="saba-storage-tab-content">
            {versions.banner}
            {/* 설치된 모듈 */}
            <div data-spot="storage.installed" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.installed_modules', '설치된 모듈')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={refreshingModules}
                    onClick={handleRefreshModules}
                    data-tooltip={t('saba_storage.refresh_modules', '모듈 새로고침')}
                    aria-label={t('saba_storage.refresh_modules', '모듈 새로고침')}
                >
                    <Icon name={refreshingModules ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {installedModules === null ? null : installedModules.length === 0 ? (
                // 받을 수 있는 목록이 바로 아래 보이므로 그림 대신 한 줄로 둔다
                <p className="ss-empty ss-installed-empty">{t('saba_storage.no_installed_modules', '설치된 모듈이 없습니다.')}</p>
            ) : (
                <div className="ss-cards">
                    {sortedModules.map((m, idx) => {
                        const moduleId = getModuleId(m);
                        const isRemoving = removingIds.has(moduleId);
                        const card: AddonCard = {
                            key: `module-${moduleId}`,
                            name: m.name,
                            kind: 'module',
                            version: m.version,
                            pinned: m.pinned,
                            pending_version: m.pending_version,
                        };
                        const progress = versions.progress(card);
                        return (
                            <div
                                className="ss-card"
                                key={m.name || idx}
                                onContextMenu={(e) =>
                                    openMenu(e, [
                                        ...versions.menuItems(card, isRemoving),
                                        'separator',
                                        {
                                            id: 'remove',
                                            label: t('saba_storage.remove', '제거'),
                                            icon: 'trash',
                                            danger: true,
                                            disabled: isRemoving,
                                            onSelect: () => setConfirmRemoveId(moduleId),
                                        },
                                    ])
                                }
                            >
                                <ModuleCardIcon icon={m.icon} />
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {m.name}
                                        {m.version && <span className="ss-card-version">v{m.version}</span>}
                                        <TrustBadge trust={m.trust} />
                                        {versions.chip(card)}
                                    </span>
                                    {progress || (m.description && <span className="ss-card-desc">{m.description}</span>)}
                                    {versions.pinNote(card)}
                                </div>
                                <div className="ss-card-actions">
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isRemoving}
                                        onClick={() => setConfirmRemoveId(moduleId)}
                                        data-tooltip={t('saba_storage.remove', '제거')}
                                        aria-label={t('saba_storage.remove', '제거')}
                                    >
                                        {isRemoving ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />}
                                    </button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {/* 받을 수 있는 모듈: 이 데몬에서 도는 것은 카드로, 돌지 않는 것은 접힌 묶음에 이유와 함께 */}
            <div className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('saba_storage.available_modules', '받을 수 있는 게임')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={manifestLoading}
                    onClick={loadManifest}
                    data-tooltip={t('saba_storage.refresh_manifest', '목록 새로고침')}
                    aria-label={t('saba_storage.refresh_manifest', '목록 새로고침')}
                >
                    <Icon name={manifestLoading ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {manifestLoading && <p className="ss-empty">{t('saba_storage.loading_manifest', '목록 받는 중…')}</p>}
            {manifestError && <SabaFail message={manifestError} />}
            {!manifestLoading && manifestModules !== null && uninstalledModules.length === 0 && !manifestError && (
                <p className="ss-empty">
                    {t('saba_storage.all_modules_installed', '사용 가능한 모든 모듈이 설치되어 있습니다.')}
                </p>
            )}
            {runnableModules.length > 0 && (
                <div data-spot="storage.catalog" className="ss-catalog">
                    {runnableModules.map((m) => {
                        const isInstalling = installingIds.has(m.id);
                        const result = installResults[m.id];
                        const name = localizedName(m, i18n.language) || m.id;
                        return (
                            <div className="ss-catalog-card" key={m.id}>
                                <div className="ss-catalog-top">
                                    <ModuleCardIcon icon={m.icon} />
                                    <div className="ss-catalog-title">
                                        <span className="ss-card-name">{name}</span>
                                        <span className="ss-card-version">
                                            {[m.version && `v${m.version}`, typeof m.size_bytes === 'number' && formatBytes(m.size_bytes)].filter(Boolean).join(' · ')}
                                        </span>
                                    </div>
                                </div>
                                <p className="ss-catalog-desc">{localizedDescription(m, i18n.language)}</p>
                                <div className="ss-catalog-foot">
                                    {result === 'ok' ? (
                                        <span className="ss-status-ok">
                                            <Icon name="checkCircle" size="sm" />
                                        </span>
                                    ) : result === 'error' ? (
                                        <span className="ss-status-err">
                                            <Icon name="xCircle" size="sm" />
                                        </span>
                                    ) : (
                                        <button
                                            className="btn btn-primary btn-sm ss-install-btn"
                                            type="button"
                                            disabled={isInstalling}
                                            onClick={() => handleInstallModule(m.id)}
                                        >
                                            {isInstalling ? <SabaSpinner size="xs" /> : null}
                                            {t('saba_storage.install_btn', '설치')}
                                        </button>
                                    )}
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}
            {unrunnableModules.length > 0 && (
                <Disclosure
                    className="ss-unsupported"
                    summary={t('saba_storage.unsupported_here', {
                        count: unrunnableModules.length,
                        defaultValue: '이 기기에서는 실행할 수 없어요 · {{count}}',
                    })}
                >
                    {unrunnableModules.map((m) => {
                        const why = unsupportedText(t, m);
                        return (
                            <div className="ss-unsupported-row" key={m.id}>
                                <ModuleCardIcon icon={m.icon} />
                                <div className="ss-card-body">
                                    <span className="ss-card-name">{localizedName(m, i18n.language) || m.id}</span>
                                    <span className="ss-card-desc">{why?.detail}</span>
                                </div>
                                <span className="ss-unsupported-chip">{why?.label}</span>
                            </div>
                        );
                    })}
                </Disclosure>
            )}

            <ModuleMakerCard />

            {versions.dialogs}

            {confirmRemoveId && (
                <QuestionModal
                    title={t('saba_storage.remove_module_confirm_title', '모듈 제거')}
                    message={t('saba_storage.remove_confirm_msg', {
                        id: confirmRemoveId,
                        defaultValue: `'${confirmRemoveId}' 모듈을 제거하시겠습니까? 이 작업은 실행취소할 수 없으며, 해당 모듈을 사용하는 서버 인스턴스를 실행할 수 없게 됩니다.`,
                    })}
                    onConfirm={handleConfirmRemove}
                    onCancel={() => setConfirmRemoveId(null)}
                />
            )}
        </div>
    );
}
