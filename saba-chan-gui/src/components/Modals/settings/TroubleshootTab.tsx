import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import type { ModalConfig, ProgressBarConfig } from '../../../stores/useUIStore';
import { safeShowToast } from '../../../utils/helpers';
import { DiagnosticsPanel, diagnosticsAvailable } from '../../DiagnosticsPanel';
import { Icon } from '../../Icon';

interface TroubleshootTabProps {
    devMode: boolean;
    guiTestOpen: boolean;
    setGuiTestOpen: (open: boolean) => void;
    handleUninstall: () => void;
    onTestModal?: (modal: ModalConfig | null) => void;
    onTestProgressBar?: (config: ProgressBarConfig | null) => void;
    onTestWaitingImage?: () => void;
    onTestLoadingScreen?: () => void;
}

/** 설정 › 문제 해결: 진단·GUI 설정 초기화·사바쨩 삭제·GUI 시험(개발자 모드) */
export function TroubleshootTab({
    devMode,
    guiTestOpen,
    setGuiTestOpen,
    handleUninstall,
    onTestModal,
    onTestProgressBar,
    onTestWaitingImage,
    onTestLoadingScreen,
}: TroubleshootTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    return (
        <>
            {/* 데몬 진단 정보 (GET /api/diagnostics) */}
            {diagnosticsAvailable() && (
                <div className="setting-item setting-item-stack">
                    <label className="setting-label">
                        <span className="setting-title">
                            <Icon name="tool" size="sm" /> {t('gui:diagnostics.title')}
                        </span>
                        <span className="setting-description">{t('gui:diagnostics.description')}</span>
                    </label>
                    <DiagnosticsPanel />
                </div>
            )}

            {/* GUI 설정 기본값으로 (데몬 /api/config/gui/reset) */}
            {typeof window.api?.configReset === 'function' && <ResetGuiSettings />}

            {/* 사바쨩 삭제 */}
            <div className="setting-item setting-item-danger" onClick={handleUninstall}>
                <label className="setting-label" style={{ cursor: 'pointer' }}>
                    <span className="setting-title setting-title-danger">
                        <Icon name="trash" size="sm" /> {t('gui:settings_modal.uninstall_label')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.uninstall_description')}
                    </span>
                </label>
                <Icon name="chevronRight" size="sm" color="var(--stop)" />
            </div>

            {/* GUI 컴포넌트 테스트 섹션 (개발자 모드 전용) */}
            {devMode && (
                <div className={clsx('gui-test-section', { open: guiTestOpen })}>
                    <h4 className="gui-test-title" onClick={() => setGuiTestOpen(!guiTestOpen)}>
                        <Icon name="tool" size="sm" /> GUI Test
                        <Icon
                            name={guiTestOpen ? 'chevronUp' : 'chevronDown'}
                            size="sm"
                            className="gui-test-chevron"
                        />
                    </h4>
                    <div className="gui-test-grid">
                        <button
                            className="gui-test-btn gui-test-success"
                            onClick={() => {
                                onTestModal &&
                                    onTestModal({
                                        type: 'success',
                                        title: 'Success!',
                                        message: 'The task finished successfully.',
                                    });
                            }}
                        >
                            <Icon name="check" size="sm" /> Success
                        </button>
                        <button
                            className="gui-test-btn gui-test-failure"
                            onClick={() => {
                                onTestModal &&
                                    onTestModal({
                                        type: 'failure',
                                        title: 'Error!',
                                        message: 'An unexpected error occurred.',
                                    });
                            }}
                        >
                            <Icon name="x" size="sm" /> Failure
                        </button>
                        <button
                            className="gui-test-btn gui-test-notification"
                            onClick={() => {
                                onTestModal &&
                                    onTestModal({
                                        type: 'notification',
                                        title: 'Notice',
                                        message: 'A new update is available.',
                                    });
                            }}
                        >
                            <Icon name="info" size="sm" /> Notification
                        </button>
                        <button
                            className="gui-test-btn gui-test-question"
                            onClick={() => {
                                onTestModal &&
                                    onTestModal({
                                        type: 'question',
                                        title: 'Confirm?',
                                        message: 'Do you want to continue?',
                                        onConfirm: () => onTestModal(null),
                                        onCancel: () => onTestModal(null),
                                    });
                            }}
                        >
                            <Icon name="alertCircle" size="sm" /> Question
                        </button>
                        <button
                            className="gui-test-btn gui-test-toast-info"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('ℹ️ Info toast message.', 'info', 3000);
                            }}
                        >
                            <Icon name="info" size="sm" /> Toast (Info)
                        </button>
                        <button
                            className="gui-test-btn gui-test-toast-success"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('✅ Success toast message.', 'success', 3000);
                            }}
                        >
                            <Icon name="check" size="sm" /> Toast (Success)
                        </button>
                        <button
                            className="gui-test-btn gui-test-toast-warning"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('⚠️ Warning toast message.', 'warning', 3000);
                            }}
                        >
                            <Icon name="alertCircle" size="sm" /> Toast (Warning)
                        </button>
                        <button
                            className="gui-test-btn gui-test-toast-error"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('❌ Error toast message.', 'error', 4000);
                            }}
                        >
                            <Icon name="x" size="sm" /> Toast (Error)
                        </button>
                        <button
                            className="gui-test-btn gui-test-progress"
                            onClick={() => {
                                if (!onTestProgressBar) return;
                                onTestProgressBar({ message: 'Downloading...', percent: 0 });
                                let p = 0;
                                const iv = setInterval(() => {
                                    p += Math.random() * 15 + 5;
                                    if (p >= 100) {
                                        p = 100;
                                        onTestProgressBar({ message: 'Done!', percent: 100 });
                                        clearInterval(iv);
                                        setTimeout(() => onTestProgressBar(null), 1500);
                                    } else {
                                        onTestProgressBar({
                                            message: `Downloading... ${Math.round(p)}%`,
                                            percent: p,
                                        });
                                    }
                                }, 400);
                            }}
                        >
                            <Icon name="loader" size="sm" /> Progress Bar
                        </button>
                        <button
                            className="gui-test-btn gui-test-progress-ind"
                            onClick={() => {
                                if (!onTestProgressBar) return;
                                onTestProgressBar({ message: 'Processing...', indeterminate: true });
                                setTimeout(() => onTestProgressBar(null), 4000);
                            }}
                        >
                            <Icon name="loader" size="sm" /> Indeterminate
                        </button>
                        <button
                            className="gui-test-btn gui-test-waiting"
                            onClick={() => {
                                onTestWaitingImage && onTestWaitingImage();
                            }}
                        >
                            <Icon name="clock" size="sm" /> Waiting Image
                        </button>
                        <button
                            className="gui-test-btn gui-test-loading"
                            onClick={() => {
                                onTestLoadingScreen && onTestLoadingScreen();
                            }}
                        >
                            <Icon name="monitor" size="sm" /> Loading Screen
                        </button>
                        <button
                            className="gui-test-btn gui-test-notice-info"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('ℹ️ Info notification test', 'info', 3000, {
                                        isNotice: true,
                                        source: 'GUI Test',
                                    });
                            }}
                        >
                            <Icon name="bell" size="sm" /> Notice (Info)
                        </button>
                        <button
                            className="gui-test-btn gui-test-notice-success"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('✅ Success notification test', 'success', 3000, {
                                        isNotice: true,
                                        source: 'GUI Test',
                                    });
                            }}
                        >
                            <Icon name="bell" size="sm" /> Notice (Success)
                        </button>
                        <button
                            className="gui-test-btn gui-test-notice-error"
                            onClick={() => {
                                window.showToast &&
                                    window.showToast('❌ Error notification test', 'error', 4000, {
                                        isNotice: true,
                                        source: 'GUI Test',
                                    });
                            }}
                        >
                            <Icon name="bell" size="sm" /> Notice (Error)
                        </button>
                    </div>
                </div>
            )}
        </>
    );
}

/** 두 번 눌러야 실행되는 "GUI 설정 기본값으로" 줄 */
function ResetGuiSettings() {
    const { t } = useTranslation(['gui']);
    const [armed, setArmed] = useState(false);
    const [busy, setBusy] = useState(false);
    return (
        <div className="setting-item">
            <label className="setting-label">
                <span className="setting-title">
                    <Icon name="refresh" size="sm" /> {t('gui:settings_modal.reset_gui_label')}
                </span>
                <span className="setting-description">{t('gui:settings_modal.reset_gui_description')}</span>
            </label>
            <button
                type="button"
                className={clsx('btn', armed ? 'btn-danger' : 'btn-secondary')}
                disabled={busy}
                onBlur={() => setArmed(false)}
                onClick={async () => {
                    if (!armed) {
                        setArmed(true);
                        return;
                    }
                    setBusy(true);
                    const ok = await useSettingsStore.getState().resetToDefaults();
                    setBusy(false);
                    setArmed(false);
                    safeShowToast(
                        t(ok ? 'gui:settings_modal.reset_gui_done' : 'gui:settings_modal.reset_gui_failed'),
                        ok ? 'success' : 'error',
                        3000,
                    );
                }}
            >
                {armed ? t('gui:settings_modal.reset_gui_confirm') : t('gui:settings_modal.reset_gui_button')}
            </button>
        </div>
    );
}
