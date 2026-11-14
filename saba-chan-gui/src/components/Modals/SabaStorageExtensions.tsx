import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import { byName } from '../../utils/helpers';
import { localizedDescription } from '../../utils/localizedText';
import { ExtensionConfigForm } from '../ExtensionConfigForm';
import { Icon } from '../Icon';
import { ExtensionAuditNotice } from '../servers/ModuleAuditNotice';
import { AddonIcon } from '../ui/AddonIcon';
import { useContextMenu } from '../ui/ContextMenu';
import { Disclosure } from '../ui/Disclosure';
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
    const versions = useAddonVersions(refreshExtensions);

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

    // 받을 수 있는 목록은 늘 보인다: 데몬이 받아 둔 목록을 탭을 열 때 받는다 (새로고침 단추만 데몬도 새로 받는다)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 탭을 처음 열 때 한 번
    useEffect(() => {
        fetchManifest?.();
    }, []);

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
            <Disclosure
                className="ss-section"
                spot="extensions.installed"
                defaultOpen
                summary={
                    <>
                        {t('saba_storage.section_installed')}
                        <span className="ss-section-count">{extensions.length}</span>
                    </>
                }
                actions={
                    <button
                        type="button"
                        className="ss-icon-btn"
                        disabled={rescanning}
                        onClick={handleRescan}
                        data-tooltip={t('extensions.rescan')}
                        aria-label={t('extensions.rescan')}
                    >
                        <Icon name={rescanning ? 'loader' : 'refresh'} size="sm" />
                    </button>
                }
            >
            {extensions.length === 0 ? (
                // 목록을 받는 중에는 "없음"을 띄우지 않는다
                !loading && <p className="ss-empty">{t('settings_modal.no_extensions')}</p>
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
                                    <AddonIcon extId={ext.id} hasIcon={ext.has_icon} version={ext.version} fallback="extension" />
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

            </Disclosure>

            <Disclosure
                className="ss-section"
                spot="extensions.store"
                defaultOpen
                summary={
                    <>
                        {t('saba_storage.section_available')}
                        <span className="ss-section-count">{uninstalled.length}</span>
                    </>
                }
                actions={
                    <button
                        type="button"
                        className="ss-icon-btn"
                        disabled={manifestLoading}
                        onClick={() => fetchManifest(true)}
                        data-tooltip={t('saba_storage.refresh_manifest')}
                        aria-label={t('saba_storage.refresh_manifest')}
                    >
                        <Icon name={manifestLoading ? 'loader' : 'refresh'} size="sm" />
                    </button>
                }
            >
                {uninstalled.length === 0 ? (
                    manifestLoading ? (
                        <p className="ss-empty">{t('saba_storage.loading_manifest')}</p>
                    ) : (
                        manifestExtensions.length > 0 && <p className="ss-empty">{t('extensions.all_installed')}</p>
                    )
                ) : (
                    <div className="ss-catalog">
                        {uninstalled.map((ext) => (
                            <div className="ss-catalog-card" key={ext.id}>
                                <div className="ss-catalog-top">
                                    <div className="ss-card-icon">
                                        <AddonIcon storeKey={`ext-${ext.id}`} hasIcon={ext.has_icon} version={ext.version} fallback="extension" />
                                    </div>
                                    <div className="ss-catalog-title">
                                        <span className="ss-card-name">{ext.name || ext.id}</span>
                                        <span className="ss-card-version">
                                            {[ext.version && `v${ext.version}`, ext.author].filter(Boolean).join(' · ')}
                                        </span>
                                    </div>
                                </div>
                                <p className="ss-catalog-desc">{localizedDescription(ext as Described, i18n.language)}</p>
                                <div className="ss-catalog-foot">
                                    <button
                                        className="btn btn-primary btn-sm ss-install-btn"
                                        type="button"
                                        disabled={installingIds.has(ext.id)}
                                        onClick={() => installExtension(ext.id, { download_url: ext.download_url })}
                                    >
                                        {installingIds.has(ext.id) ? <SabaSpinner size="xs" /> : null}
                                        {t('saba_storage.install_btn')}
                                    </button>
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </Disclosure>

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
