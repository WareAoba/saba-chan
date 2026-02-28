import clsx from 'clsx';
import { useState, useCallback, useRef, useLayoutEffect, useEffect } from 'react';
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
    onRefreshextensions,
    onAddServer,
    // -- migration props (향후 확장) --
}) {
    const { t } = useTranslation('gui');
    const [mode, setMode] = useState(null); // null | 'new' | 'migrate'
    const [newFlowStep, setNewFlowStep] = useState('select-game');
    const [migrationFlowStep, setMigrationFlowStep] = useState('pick-dir');

    const handleClose = useCallback(() => {
        setMode(null); // 모달이 닫힐 때 선택 화면으로 리셋
        setNewFlowStep('select-game');
        setMigrationFlowStep('pick-dir');
        onClose();
    }, [onClose]);

    const { isClosing, requestClose } = useModalClose(handleClose);

    // ── 높이 애니메이션 ──
    const contentRef = useRef(null);
    const snapshotHeightRef = useRef(null);
    const animatingRef = useRef(false);

    // 상태 변경 직전에 현재 높이를 캡처하는 래퍼
    const captureAndSetMode = useCallback((newMode) => {
        const el = contentRef.current;
        if (el) snapshotHeightRef.current = el.getBoundingClientRect().height;
        setMode(newMode);
    }, []);

    const goBack = useCallback(() => captureAndSetMode(null), [captureAndSetMode]);

    // step 변경도 캡처
    const handleNewFlowStep = useCallback((step) => {
        const el = contentRef.current;
        if (el) snapshotHeightRef.current = el.getBoundingClientRect().height;
        setNewFlowStep(step);
    }, []);
    const handleMigrationFlowStep = useCallback((step) => {
        const el = contentRef.current;
        if (el) snapshotHeightRef.current = el.getBoundingClientRect().height;
        setMigrationFlowStep(step);
    }, []);

    // DOM이 업데이트된 직후, 페인트 전에 실행
    useLayoutEffect(() => {
        const el = contentRef.current;
        if (!el) return;

        const prevHeight = snapshotHeightRef.current;
        snapshotHeightRef.current = null;

        if (prevHeight == null) return;

        // 새 높이 측정 (인라인 height가 없으므로 CSS 자연 높이)
        const newHeight = el.getBoundingClientRect().height;

        if (Math.abs(prevHeight - newHeight) < 2) return;

        // 트랜지션 끄고, 이전 높이로 즉시 설정
        animatingRef.current = true;
        el.style.transition = 'none';
        el.style.height = prevHeight + 'px';
        // 강제 레이아웃 — 브라우저가 이전 높이를 인식
        void el.offsetWidth;

        // 트랜지션 켜고, 새 높이로 전환
        el.style.transition = 'height 0.28s cubic-bezier(0.4, 0, 0.2, 1)';
        el.style.height = newHeight + 'px';

        const cleanup = () => {
            el.style.transition = '';
            el.style.height = '';
            animatingRef.current = false;
        };

        const onEnd = (e) => {
            if (e.propertyName !== 'height') return;
            el.removeEventListener('transitionend', onEnd);
            cleanup();
        };
        el.addEventListener('transitionend', onEnd);

        // 안전장치
        const timer = setTimeout(cleanup, 350);
        return () => {
            clearTimeout(timer);
            el.removeEventListener('transitionend', onEnd);
        };
    }, [mode, newFlowStep, migrationFlowStep]);

    const progressPercent = (() => {
        if (mode === null) return 20;
        if (mode === 'new') return newFlowStep === 'configure' ? 78 : 50;
        if (mode === 'migrate') return migrationFlowStep === 'configure' ? 78 : 50;
        return 20;
    })();

    if (!isOpen) return null;

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div
                ref={contentRef}
                className={clsx('modal-content', 'add-instance-modal', {
                    'add-instance-modal--chooser': mode === null,
                    'add-instance-modal--flow': mode !== null,
                })}
                onClick={(e) => e.stopPropagation()}
            >
                <div className="add-instance-progress-wrap" aria-hidden="true">
                    <div className="add-instance-progress-track">
                        <div
                            className="add-instance-progress-fill"
                            style={{ width: `${progressPercent}%` }}
                        />
                    </div>
                </div>

                {/* ── 선택 화면 ── */}
                {mode === null && (
                    <InstanceChooser
                        t={t}
                        onSelectNew={() => captureAndSetMode('new')}
                        onSelectMigrate={() => captureAndSetMode('migrate')}
                        onClose={requestClose}
                    />
                )}

                {/* ── 새 서버 흐름 ── */}
                {mode === 'new' && (
                    <AddInstanceNewServer
                        extensions={extensions}
                        servers={servers}
                        onRefreshextensions={onRefreshextensions}
                        onAddServer={onAddServer}
                        onStepChange={handleNewFlowStep}
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
                        onStepChange={handleMigrationFlowStep}
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
        <div className="add-instance-stage" key="chooser">
            {/* Header */}
            <div className="modal-header add-instance-header">
                <div className="add-instance-title-wrap">
                    <div className="add-instance-header-icon">
                        <Icon name="server" size="lg" />
                    </div>
                    <h3>{t('add_instance_modal.title')}</h3>
                    <p className="add-instance-subtitle">
                        {t('add_instance_modal.subtitle')}
                    </p>
                </div>
            </div>

            {/* Body — 두 개의 선택 카드 */}
            <div className="modal-body add-instance-chooser-body">
                <div className="add-instance-chooser-grid" role="group" aria-label={t('add_instance_modal.title')}>
                {/* 새 서버 */}
                <button
                    className="ai-choice-card"
                    type="button"
                    onClick={onSelectNew}
                >
                    <div className="ai-choice-main">
                        <div className="ai-choice-icon ai-choice-icon--new">
                            <Icon name="plus" size="xl" />
                        </div>
                        <div className="ai-choice-text">
                            <span className="ai-choice-title">
                                {t('add_instance_modal.new_server_title')}
                            </span>
                            <span className="ai-choice-desc">
                                {t('add_instance_modal.new_server_desc')}
                            </span>
                        </div>
                    </div>
                </button>

                {/* 기존 서버 마이그레이션 */}
                <button
                    className="ai-choice-card"
                    type="button"
                    onClick={onSelectMigrate}
                >
                    <div className="ai-choice-main">
                        <div className="ai-choice-icon ai-choice-icon--migrate">
                            <Icon name="download" size="xl" />
                        </div>
                        <div className="ai-choice-text">
                            <span className="ai-choice-title">
                                {t('add_instance_modal.migrate_title')}
                            </span>
                            <span className="ai-choice-desc">
                                {t('add_instance_modal.migrate_desc')}
                            </span>
                        </div>
                    </div>
                </button>
                </div>
            </div>

            {/* Footer */}
            <div className="modal-footer add-instance-footer">
                <button className="btn btn-cancel ai-cancel-btn" onClick={onClose}>
                    {t('modals.cancel')}
                </button>
            </div>
        </div>
    );
}
