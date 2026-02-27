import clsx from 'clsx';
import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import Icon from '../Icon';
import { useModalClose } from '../../hooks/useModalClose';
import { AddInstanceNewServer } from './AddInstanceNewServer';
import { AddInstanceMigration } from './AddInstanceMigration';
import './Modals.css';

/**
 * AddInstanceModal — 인스턴스 추가 진입점.
 *
 * 두 가지 경로를 제공한다:
 *   1. "새 서버로 추가"     → AddInstanceNewServer
 *   2. "기존 서버 마이그레이션" → AddInstanceMigration
 *
 * mode 상태:
 *   null      — 선택 화면 (두 카드)
 *   'new'     — 새 서버 생성 흐름
 *   'migrate' — 마이그레이션 흐름
 */
export function AddInstanceModal({
    isOpen,
    onClose,
    // -- new-server props --
    extensions,
    servers,
    extensionsPath,
    settingsPath,
    onextensionsPathChange,
    onRefreshextensions,
    onAddServer,
    // -- migration props (향후 확장) --
}) {
    const { t } = useTranslation('gui');
    const [mode, setMode] = useState(null); // null | 'new' | 'migrate'

    const handleClose = useCallback(() => {
        setMode(null); // 모달이 닫힐 때 선택 화면으로 리셋
        onClose();
    }, [onClose]);

    const { isClosing, requestClose } = useModalClose(handleClose);

    const goBack = useCallback(() => setMode(null), []);

    if (!isOpen) return null;

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div
                className={clsx('modal-content', 'add-instance-modal', {
                    'add-instance-modal--chooser': mode === null,
                    'add-instance-modal--flow': mode !== null,
                })}
                onClick={(e) => e.stopPropagation()}
            >
                {/* ── 선택 화면 ── */}
                {mode === null && (
                    <InstanceChooser
                        t={t}
                        onSelectNew={() => setMode('new')}
                        onSelectMigrate={() => setMode('migrate')}
                        onClose={requestClose}
                    />
                )}

                {/* ── 새 서버 흐름 ── */}
                {mode === 'new' && (
                    <AddInstanceNewServer
                        extensions={extensions}
                        servers={servers}
                        extensionsPath={extensionsPath}
                        settingsPath={settingsPath}
                        onextensionsPathChange={onextensionsPathChange}
                        onRefreshextensions={onRefreshextensions}
                        onAddServer={onAddServer}
                        onBack={goBack}
                        onClose={requestClose}
                    />
                )}

                {/* ── 마이그레이션 흐름 ── */}
                {mode === 'migrate' && (
                    <AddInstanceMigration
                        extensions={extensions}
                        servers={servers}
                        onAddServer={onAddServer}
                        onBack={goBack}
                        onClose={requestClose}
                    />
                )}
            </div>
        </div>
    );
}

/* ─────────────────────────────────────────────
 *  InstanceChooser — 두 경로를 선택하는 카드 UI
 * ───────────────────────────────────────────── */
function InstanceChooser({ t, onSelectNew, onSelectMigrate, onClose }) {
    return (
        <>
            {/* Header */}
            <div className="modal-header add-instance-header">
                <div>
                    <h3>{t('add_instance_modal.title')}</h3>
                    <p className="add-instance-subtitle">
                        {t('add_instance_modal.subtitle')}
                    </p>
                </div>
            </div>

            {/* Body — 두 개의 선택 카드 */}
            <div className="modal-body add-instance-chooser-body">
                {/* 새 서버 */}
                <button
                    className="ai-choice-card"
                    type="button"
                    onClick={onSelectNew}
                >
                    <div className="ai-choice-icon ai-choice-icon--new">
                        <Icon name="plus" size="lg" />
                    </div>
                    <div className="ai-choice-text">
                        <span className="ai-choice-title">
                            {t('add_instance_modal.new_server_title')}
                        </span>
                        <span className="ai-choice-desc">
                            {t('add_instance_modal.new_server_desc')}
                        </span>
                    </div>
                    <Icon name="chevronRight" size="sm" className="ai-choice-arrow" />
                </button>

                {/* 기존 서버 마이그레이션 */}
                <button
                    className="ai-choice-card"
                    type="button"
                    onClick={onSelectMigrate}
                >
                    <div className="ai-choice-icon ai-choice-icon--migrate">
                        <Icon name="download" size="lg" />
                    </div>
                    <div className="ai-choice-text">
                        <span className="ai-choice-title">
                            {t('add_instance_modal.migrate_title')}
                        </span>
                        <span className="ai-choice-desc">
                            {t('add_instance_modal.migrate_desc')}
                        </span>
                    </div>
                    <Icon name="chevronRight" size="sm" className="ai-choice-arrow" />
                </button>
            </div>

            {/* Footer */}
            <div className="modal-footer add-instance-footer">
                <button className="btn btn-cancel" onClick={onClose}>
                    {t('modals.cancel')}
                </button>
            </div>
        </>
    );
}
