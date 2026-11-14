import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import { byName } from '../../utils/helpers';
import { localizedDescription } from '../../utils/localizedText';
import { ExtensionConfigForm } from '../ExtensionConfigForm';
import { Icon } from '../Icon';
import { ExtensionAuditNotice } from '../servers/ModuleAuditNotice';
import { useContextMenu } from '../ui/ContextMenu';
import { SabaEmpty } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { TrustBadge } from '../ui/TrustBadge';
import { type AddonCard, useAddonVersions } from './AddonVersions';
import { QuestionModal } from './Modals';

// ─────────────────────────────────────────────────────────────
// 익스텐션 탭
// ─────────────────────────────────────────────────────────────
type Described = Parameters<typeof localizedDescription>[0];

export function ExtensionsTab() {
    const { t, i18n } = useTranslation('gui');
    const openMenu = useContextMenu();
    // 이 탭은 설정을 열 때 미리 띄워 두므로, 익스텐션 상태가 덜 채워져 있어도 빈 값으로 그린다
    const {
        extensions = [],
        refreshExtensions,
        manifestExtensions = [],
        availableUpdates = [],
        manifestLoading,
        installingIds = new Set<string>(),
        fetchManifest,
        installExtension,
        removeExtension,
        toggleExtension,
        loading,
    } = useExtensions();

    const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
    const [togglingIds, setTogglingIds] = useState<Set<string>>(new Set());
    const [removingIds, setRemovingIds] = useState<Set<string>>(new Set());
    const [rescanning, setRescanning] = useState(false);
    const [iconUrls, setIconUrls] = useState<Record<string, string>>({}); // { extId: dataUrl }
    const [showStore, setShowStore] = useState(false);
    const versions = useAddonVersions(refreshExtensions);

    // 아이콘 로드: has_icon이 true인 익스텐션의 icon.png를 data URL로 가져옴
    useEffect(() => {
        const toLoad = extensions.filter((e) => e.has_icon && !iconUrls[e.id]);
        if (toLoad.length === 0) return;
        let cancelled = false;
        (async () => {
            const results: Record<string, string> = {};
            await Promise.all(
                toLoad.map(async (ext) => {
                    try {
                        const url = await window.api?.extensionIcon?.(ext.id);
                        if (url) results[ext.id] = url;
                    } catch (_) {}
                })
            );
            if (!cancelled && Object.keys(results).length > 0) {
                setIconUrls((prev) => ({ ...prev, ...results }));
            }
        })();
        return () => { cancelled = true; };
    }, [extensions]); // eslint-disable-line react-hooks/exhaustive-deps

    // 설치된 익스텐션 설명은 데몬이 GUI 언어로 골라 준다 → 언어가 바뀌면 다시 받는다
    const firstLanguage = useRef(i18n.language);
    useEffect(() => {
        if (firstLanguage.current === i18n.language) return;
        firstLanguage.current = i18n.language;
        refreshExtensions?.();
    }, [i18n.language]); // eslint-disable-line react-hooks/exhaustive-deps

    // 익스텐션 디렉토리 재스캔 + 목록 갱신
    const handleRescan = useCallback(async () => {
        setRescanning(true);
        try {
            await window.api?.extensionRescan?.();
            await refreshExtensions();
        } catch (_) {}
        setRescanning(false);
    }, [refreshExtensions]);

    // 스토어 열기 (모듈 탭 "더보기" 패턴과 동일)
    const handleShowStore = useCallback(async () => {
        if (manifestExtensions.length > 0) {
            setShowStore(true);
            return;
        }
        setShowStore(true);
        await fetchManifest();
    }, [manifestExtensions, fetchManifest]);

    const handleConfirmRemove = useCallback(async () => {
        const id = confirmRemoveId;
        setConfirmRemoveId(null);
        if (!id) return;
        setRemovingIds((prev) => new Set(prev).add(id));
        await removeExtension(id);
        setRemovingIds((prev) => {
            const n = new Set(prev);
            n.delete(id);
            return n;
        });
    }, [confirmRemoveId, removeExtension]);

    const handleToggle = useCallback(
        async (id: string, enable: boolean) => {
            setTogglingIds((prev) => new Set(prev).add(id));
            await toggleExtension(id, enable);
            setTogglingIds((prev) => {
                const n = new Set(prev);
                n.delete(id);
                return n;
            });
        },
        [toggleExtension],
    );

    const installedIds = new Set(extensions.map((e) => e.id));
    const uninstalled = manifestExtensions.filter((r) => !installedIds.has(r.id));

    return (
        <div className="saba-storage-tab-content">
            {versions.banner}
            {/* 설치됨 */}
            <div data-spot="extensions.installed" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.installed_section', '설치됨')}
                </div>
                <button
                    className="ss-icon-btn"
                    disabled={rescanning}
                    onClick={handleRescan}
                    data-tooltip={t('extensions.rescan', '익스텐션 재스캔')}
                    aria-label={t('extensions.rescan', '익스텐션 재스캔')}
                >
                    <Icon name={rescanning ? 'loader' : 'refresh'} size="sm" />
                </button>
            </div>
            {extensions.length === 0 ? (
                // 목록을 받는 중에는 "없음"을 띄우지 않는다
                !loading && <SabaEmpty compact body={t('settings_modal.no_extensions', '설치된 익스텐션이 없습니다.')} />
            ) : (
                <div className="ss-cards">
                    {[...extensions].sort((a, b) => byName(a.name || a.id, b.name || b.id)).map((ext) => {
                        const updateInfo = availableUpdates.find((u) => u.id === ext.id);
                        const isRemoving = removingIds.has(ext.id);
                        const isBusy = isRemoving || installingIds.has(ext.id);
                        const card: AddonCard = {
                            key: `ext-${ext.id}`,
                            name: ext.name || ext.id,
                            kind: 'ext',
                            version: ext.version,
                            pinned: ext.pinned,
                            pending_version: ext.pending_version,
                        };
                        const progress = versions.progress(card);
                        return (
                            <div
                                className="ss-card"
                                key={ext.id}
                                onContextMenu={(e) =>
                                    openMenu(e, [
                                        updateInfo && {
                                            id: 'update',
                                            label: t('extensions.update_to', {
                                                version: updateInfo.latest_version,
                                                defaultValue: `v${updateInfo.latest_version}으로 업데이트`,
                                            }),
                                            icon: 'download',
                                            disabled: isBusy,
                                            onSelect: () =>
                                                installExtension(ext.id, { download_url: updateInfo.download_url }),
                                        },
                                        {
                                            id: 'toggle',
                                            label: ext.enabled ? t('context_menu.disable') : t('context_menu.enable'),
                                            icon: 'power',
                                            disabled: isBusy || togglingIds.has(ext.id),
                                            onSelect: () => handleToggle(ext.id, !ext.enabled),
                                        },
                                        ...versions.menuItems(card, isBusy),
                                        'separator',
                                        {
                                            id: 'remove',
                                            label: t('saba_storage.remove', '제거'),
                                            icon: 'trash',
                                            danger: true,
                                            disabled: isBusy,
                                            onSelect: () => setConfirmRemoveId(ext.id),
                                        },
                                    ])
                                }
                            >
                                <div className="ss-card-icon">
                                    {iconUrls[ext.id] ? (
                                        <img src={iconUrls[ext.id]} alt="" />
                                    ) : (
                                        <Icon name="extension" size="md" />
                                    )}
                                </div>
                                <div className="ss-card-body">
                                    <span className="ss-card-name">
                                        {ext.name}
                                        {ext.version && <span className="ss-card-version">v{ext.version}</span>}
                                        <TrustBadge trust={ext.trust} />
                                        {updateInfo && (
                                            <span className="ss-update-badge">v{updateInfo.latest_version}</span>
                                        )}
                                        {versions.chip(card)}
                                    </span>
                                    {progress ||
                                        ((localizedDescription(ext as Described, i18n.language) || ext.id) && (
                                            <span className="ss-card-desc">{localizedDescription(ext as Described, i18n.language) || ext.id}</span>
                                        ))}
                                    {versions.pinNote(card)}
                                    <ExtensionAuditNotice extId={ext.id} name={ext.name} />
                                    {ext.enabled && <ExtensionConfigForm ext={ext} />}
                                </div>
                                <div className="ss-card-actions">
                                    {updateInfo && (
                                        <button
                                            className="ss-icon-btn accent"
                                            disabled={isBusy}
                                            onClick={() =>
                                                installExtension(ext.id, { download_url: updateInfo.download_url })
                                            }
                                            data-tooltip={t('extensions.update_to', {
                                                version: updateInfo.latest_version,
                                                defaultValue: `v${updateInfo.latest_version}으로 업데이트`,
                                            })}
                                        >
                                            {installingIds.has(ext.id) ? (
                                                <SabaSpinner size="xs" />
                                            ) : (
                                                <Icon name="download" size="sm" />
                                            )}
                                        </button>
                                    )}
                                    <SabaToggle
                                        size="sm"
                                        checked={!!ext.enabled}
                                        disabled={isBusy || togglingIds.has(ext.id)}
                                        onChange={(checked) => handleToggle(ext.id, checked)}
                                        aria-label={ext.name || ext.id}
                                    />
                                    <button
                                        className="ss-icon-btn danger"
                                        disabled={isBusy}
                                        onClick={() => setConfirmRemoveId(ext.id)}
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

            {/* 스토어 */}
            <div data-spot="extensions.store" className="ss-store-header">
                <div className="ss-section-label" style={{ margin: 0 }}>
                    {t('extensions.store_section', '스토어')}
                </div>
                {showStore ? (
                    <button
                        className="ss-icon-btn"
                        disabled={manifestLoading}
                        onClick={fetchManifest}
                        data-tooltip={t('extensions.refresh_manifest', '매니페스트 새로고침')}
                        aria-label={t('extensions.refresh_manifest', '매니페스트 새로고침')}
                    >
                        <Icon name={manifestLoading ? 'loader' : 'refresh'} size="sm" />
                    </button>
                ) : (
                    <button
                        className="ss-icon-btn"
                        onClick={handleShowStore}
                        data-tooltip={t('saba_storage.show_more_extensions')}
                        aria-label={t('saba_storage.show_more_extensions')}
                    >
                        <Icon name="chevronDown" size="sm" />
                    </button>
                )}
            </div>

            {showStore && (
                <>
                    {manifestLoading ? (
                        <p className="ss-empty">{t('extensions.manifest_loading', '매니페스트를 가져오는 중...')}</p>
                    ) : uninstalled.length === 0 ? (
                        <p className="ss-empty">
                            {manifestExtensions.length === 0
                                ? t('extensions.store_empty', '새로고침 버튼으로 사용 가능한 익스텐션을 불러오세요.')
                                : t('extensions.all_installed', '모든 익스텐션이 설치되어 있습니다.')}
                        </p>
                    ) : (
                <div className="ss-cards">
                    {uninstalled.map((ext) => (
                        <div className="ss-card" key={ext.id}>
                            <div className="ss-card-icon">
                                <Icon name="extension" size="md" />
                            </div>
                            <div className="ss-card-body">
                                <span className="ss-card-name">
                                    {ext.name}
                                    {ext.version && <span className="ss-card-version">v{ext.version}</span>}
                                    {ext.author && <span className="ss-card-version">· {ext.author}</span>}
                                </span>
                                {(localizedDescription(ext as Described, i18n.language) || ext.id) && (
                                    <span className="ss-card-desc">{localizedDescription(ext as Described, i18n.language) || ext.id}</span>
                                )}
                            </div>
                            <div className="ss-card-actions">
                                <button
                                    className="ss-icon-btn accent"
                                    disabled={installingIds.has(ext.id)}
                                    onClick={() => installExtension(ext.id, { download_url: ext.download_url })}
                                    data-tooltip={t('extensions.install_btn', '설치')}
                                    aria-label={t('extensions.install_btn', '설치')}
                                >
                                    {installingIds.has(ext.id) ? (
                                        <SabaSpinner size="xs" />
                                    ) : (
                                        <Icon name="download" size="sm" />
                                    )}
                                </button>
                            </div>
                        </div>
                    ))}
                </div>
            )}
                </>
            )}

            {versions.dialogs}

            {confirmRemoveId && (
                <QuestionModal
                    title={t('saba_storage.remove_ext_confirm_title', '익스텐션 제거')}
                    message={t('saba_storage.remove_confirm_msg_ext', {
                        id: confirmRemoveId,
                        defaultValue: `'${confirmRemoveId}' 익스텐션을 제거하시겠습니까? 파일이 영구 삭제됩니다.`,
                    })}
                    onConfirm={handleConfirmRemove}
                    onCancel={() => setConfirmRemoveId(null)}
                />
            )}
        </div>
    );
}
