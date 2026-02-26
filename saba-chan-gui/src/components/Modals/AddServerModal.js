import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import ExtensionSlot from '../ExtensionSlot';
import Icon from '../Icon';
import './Modals.css';
import { useModalClose } from '../../hooks/useModalClose';

export function AddServerModal({
    isOpen,
    onClose,
    extensions,
    servers,
    extensionsPath,
    settingsPath,
    onextensionsPathChange,
    onRefreshextensions,
    onAddServer,
    onOpenMigration,
}) {
    const { t } = useTranslation('gui');
    const [newServerName, setNewServerName] = useState('');
    const [selectedExtension, setselectedExtension] = useState('');
    const [submitting, setSubmitting] = useState(false);
    const [useContainerIsolation, setUseContainerIsolation] = useState(false);
    const [showAdvanced, setShowAdvanced] = useState(false);
    const { isClosing, requestClose } = useModalClose(onClose);

    // 모듈 선택 시 자동으로 서버 이름 생성
    const handleExtensionSelect = (extName) => {
        setselectedExtension(extName);

        // 이름이 비어있거나 자동 생성된 이름인 경우에만 자동완성
        if (!newServerName || newServerName.startsWith('my-')) {
            const existingCount = servers.filter((s) => s.module === extName).length;
            const suggestedName = `my-${extName}-${existingCount + 1}`;
            setNewServerName(suggestedName);
        }
    };

    const handleSubmit = async () => {
        if (!newServerName.trim()) return;
        if (!selectedExtension) return;

        setSubmitting(true);
        try {
            await onAddServer({
                name: newServerName.trim(),
                module_name: selectedExtension,
                accept_eula: true,
                use_container: useContainerIsolation,
            });
        } finally {
            setSubmitting(false);
        }

        // 폼 초기화
        setNewServerName('');
        setselectedExtension('');
        setUseContainerIsolation(false);
    };

    const handleOpenMigration = () => {
        requestClose();
        setTimeout(() => onOpenMigration?.(), 350);
    };

    const canSubmit = newServerName.trim() && selectedExtension && !submitting;

    if (!isOpen) return null;

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal-content add-server-modal" onClick={(e) => e.stopPropagation()}>
                {/* ── Header ── */}
                <div className="modal-header add-server-header">
                    <div>
                        <h3>{t('add_server_modal.title')}</h3>
                        <p className="add-server-subtitle">{t('add_server_modal.subtitle')}</p>
                    </div>
                </div>

                {/* ── Body ── */}
                <div className="modal-body add-server-body">
                    {/* 1. 게임 모듈 선택 */}
                    <div className="as-section">
                        <label className="as-label">
                            <Icon name="gamepad" size="sm" />
                            {t('add_server_modal.game_extension')}
                        </label>
                        <CustomDropdown
                            value={selectedExtension}
                            onChange={(val) => handleExtensionSelect(val)}
                            placeholder={t('add_server_modal.select_extension')}
                            options={extensions.map((m) => ({
                                value: m.name,
                                label: `${t(`mod_${m.name}:module.display_name`, { defaultValue: m.name })} v${m.version}`,
                            }))}
                            disabled={submitting}
                        />
                        {extensions.length === 0 && (
                            <p className="as-empty-hint">
                                <Icon name="alertCircle" size="sm" />
                                {t('add_server_modal.no_extensions')}
                            </p>
                        )}
                    </div>

                    {/* 2. 서버 이름 */}
                    <div className="as-section">
                        <label className="as-label">
                            <Icon name="server" size="sm" />
                            {t('add_server_modal.server_name')}
                        </label>
                        <input
                            className="as-input"
                            type="text"
                            placeholder={t('add_server_modal.server_name_placeholder')}
                            value={newServerName}
                            onChange={(e) => setNewServerName(e.target.value)}
                            disabled={submitting}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter' && canSubmit) handleSubmit();
                            }}
                        />
                    </div>

                    {/* 3. 익스텐션 슬롯 — 컨테이너 격리 토글 등 */}
                    <ExtensionSlot
                        slotId="AddServer.options"
                        options={{ use_container: useContainerIsolation }}
                        onOptionsChange={(opts) => {
                            if (opts.use_container !== undefined) setUseContainerIsolation(opts.use_container);
                        }}
                        t={t}
                    />

                    {/* 4. 프로비저닝 안내 */}
                    <p className="as-provision-hint">
                        <Icon name="info" size="sm" />
                        {t('add_server_modal.provision_hint')}
                    </p>

                    {/* ── 구분선 ── */}
                    <div className="as-divider" />

                    {/* 5. 고급 설정 (접이식) */}
                    <button
                        className="as-advanced-toggle"
                        type="button"
                        onClick={() => setShowAdvanced((prev) => !prev)}
                    >
                        <Icon name={showAdvanced ? 'chevronDown' : 'chevronRight'} size="sm" />
                        {t('add_server_modal.advanced_settings')}
                    </button>

                    {showAdvanced && (
                        <div className="as-advanced-panel">
                            <label className="as-label">
                                <Icon name="folder" size="sm" />
                                {t('add_server_modal.modules_directory')}
                            </label>
                            <div className="as-path-row">
                                <input
                                    className="as-input as-input-mono"
                                    type="text"
                                    value={extensionsPath}
                                    onChange={(e) => onextensionsPathChange(e.target.value)}
                                    placeholder="extensions/"
                                />
                                <button
                                    className="as-btn-icon"
                                    onClick={onRefreshextensions}
                                    disabled={submitting}
                                    title={t('add_server_modal.reload_extensions')}
                                >
                                    <Icon name="refresh" size="sm" />
                                </button>
                            </div>
                            <small className="as-hint">{t('add_server_modal.place_modules_hint')}</small>
                            {settingsPath && (
                                <small className="as-hint as-settings-path">
                                    <Icon name="database" size="xs" /> {t('add_server_modal.settings_path')}{' '}
                                    {settingsPath}
                                </small>
                            )}
                        </div>
                    )}

                    {/* 6. 마이그레이션 링크 */}
                    <button
                        className="as-migrate-link"
                        type="button"
                        onClick={handleOpenMigration}
                        disabled={submitting}
                    >
                        <Icon name="download" size="sm" />
                        {t('add_server_modal.migrate_existing')}
                    </button>
                </div>

                {/* ── Footer ── */}
                <div className="modal-footer add-server-footer">
                    <button className="btn btn-cancel" onClick={requestClose} disabled={submitting}>
                        {t('modals.cancel')}
                    </button>
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
                </div>
            </div>
        </div>
    );
}
