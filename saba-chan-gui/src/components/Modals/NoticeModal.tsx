import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import type { ApiBody } from '../../types/daemon';
import type { Notice } from '../../types/global';
import { clearAllNotices, loadNotices, markAllRead, NOTICE_UPDATE, refreshNotices, removeNotice } from '../../utils/noticeInbox';
import { groupNotices, noticeHintKey, noticeScreen, noticeTime, SERVER_UPDATE_ACTION } from '../../utils/noticeView';
import { openScreen } from '../../utils/openScreen';
import { initials } from '../../utils/serverView';
import { applyServerUpdateFromNotice } from '../../utils/serverUpdateNotice';
import './Modals.css';
import { Icon } from '../Icon';

/** 업데이트 결과 알림의 액션: 누르면 패치노트를 연다 */
export const PATCH_NOTES_ACTION = 'openPatchNotes';

/** 업데이트 결과 마커 → 알림 한 줄 */
export function updateResultMessage(res: ApiBody | null | undefined, t: TFunction) {
    const r = res?.result || {};
    const version = res?.version || '';
    if (r.success) return t('patch_notes.notice_success', { version: version || t('patch_notes.unknown_version') });
    return t('patch_notes.notice_failed', { count: (r.failed || []).length || 1 });
}

/** 업데이트 결과 알림을 눌렀다: 데몬의 마지막 결과(확인한 것 포함)를 패치노트로 연다 */
async function openPatchNotes(open: (data: unknown) => void) {
    const res = await window.api.updaterLastResult({ includeSeen: true });
    if (res?.result) open({ version: res.version || null, notes: res.notes ?? null, result: res.result });
}

// ─── 알림 카드 ─────────────────────────────────────────

interface NoticeCardProps {
    notice: Notice;
    onDismiss: (id: number) => void;
    onAction?: (action: string, notice: Notice) => void;
    t: TFunction;
}

function NoticeCard({ notice, onDismiss, onAction, t }: NoticeCardProps) {
    const typeConfig = {
        info: { icon: 'info', colorClass: 'notice-type-info' },
        success: { icon: 'checkCircle', colorClass: 'notice-type-success' },
        error: { icon: 'xCircle', colorClass: 'notice-type-error' },
    };
    const { icon, colorClass } = typeConfig[notice.type] || typeConfig.info;
    const clickable = !!notice.action;
    const isUpdateCard = notice.action === 'openUpdateModal';

    const timeStr = noticeTime(notice.timestamp, t);

    const handleClick = () => {
        if (clickable && onAction) onAction(notice.action!, notice);
    };

    return (
        <div
            className={clsx('notice-card', colorClass, {
                'notice-unread': !notice.read,
                'notice-clickable': clickable,
                'notice-update-card': isUpdateCard,
            })}
            onClick={handleClick}
            style={clickable ? { cursor: 'pointer' } : undefined}
        >
            <div className="notice-card-icon">
                <Icon name={icon} size="sm" />
            </div>
            <div className="notice-card-body">
                <p className="notice-card-message">{notice.message}</p>
                {notice.detail && <p className="notice-card-detail">{notice.detail}</p>}
                <span className="notice-card-time">
                    {timeStr}
                    {clickable && (
                        <span className="notice-action-hint">
                            {' '}
                            {t(noticeHintKey(notice.action!))}
                        </span>
                    )}
                </span>
            </div>
            <button
                className="notice-card-dismiss"
                onClick={(e) => {
                    e.stopPropagation();
                    onDismiss(notice.id);
                }}
                data-tooltip={t('notice_modal.dismiss')}
                aria-label={t('notice_modal.dismiss')}
            >
                <Icon name="close" size="xs" />
            </button>
        </div>
    );
}

// ── ─ 알림 모달 본체 ──

interface NoticeModalProps {
    isOpen: boolean;
    onClose: () => void;
    isClosing?: boolean;
    onOpenUpdateModal?: () => void;
    onOpenPatchNotes?: (data: unknown) => void;
}

function NoticeModal({ isOpen, onClose, isClosing, onOpenUpdateModal, onOpenPatchNotes }: NoticeModalProps) {
    const { t } = useTranslation('gui');
    const [notices, setNotices] = useState<Notice[]>(loadNotices);
    const servers = useServerStore((s) => s.servers);
    const modules = useServerStore((s) => s.modules);

    // 알림 변경 이벤트 구독
    useEffect(() => {
        const handler = () => {
            setNotices(loadNotices());
            // 열려 있는 동안 새로 온 것도 본 것이다
            if (isOpen) markAllRead();
        };
        window.addEventListener(NOTICE_UPDATE, handler);
        return () => window.removeEventListener(NOTICE_UPDATE, handler);
    }, [isOpen]);

    // 모달 열릴 때 데몬에서 다시 읽고 모두 읽음 처리
    useEffect(() => {
        if (!isOpen) return;
        markAllRead();
        void refreshNotices().then(markAllRead);
    }, [isOpen]);

    const handleDismiss = useCallback((id: number) => {
        removeNotice(id);
    }, []);

    const handleClearAll = useCallback(() => {
        clearAllNotices();
    }, []);

    if (!isOpen) return null;

    const groups = groupNotices(notices, (id) => servers.find((s) => s.id === id)?.name);
    const groupIcon = (instanceId: string | undefined, label: string) => {
        if (!instanceId) return null;
        const server = servers.find((s) => s.id === instanceId);
        const module = server && modules.find((m) => m.name === server.module);
        return (
            <span className="notice-group-icon" aria-hidden="true">
                {module?.icon ? <img src={module.icon} alt="" /> : initials(label)}
            </span>
        );
    };

    return (
        <div className={clsx('notice-modal-container', { closing: isClosing })} onClick={(e) => e.stopPropagation()}>
            <div className="notice-modal-header">
                <div className="notice-modal-title">
                    <h2>{t('notice_modal.title')}</h2>
                    {notices.length > 0 && <span className="notice-count-badge">{notices.length}</span>}
                </div>
                <div className="notice-modal-actions">
                    {notices.length > 0 && (
                        <button
                            className="notice-clear-all-btn"
                            onClick={handleClearAll}
                            data-tooltip={t('notice_modal.clear_all')}
                        >
                            <Icon name="trash" size="xs" />
                            <span>{t('notice_modal.clear_all')}</span>
                        </button>
                    )}
                    <button className="notice-modal-close" onClick={onClose}>
                        <Icon name="close" size="sm" />
                    </button>
                </div>
            </div>

            <div className="notice-modal-content">
                {notices.length === 0 ? (
                    <div className="notice-empty">
                        <Icon name="bell" size="xl" />
                        <p>{t('notice_modal.empty')}</p>
                    </div>
                ) : (
                    groups.map(({ key, instanceId, label, notices: list }) => (
                        <div key={key} className="notice-group">
                            <div className="notice-group-header">
                                {groupIcon(instanceId, label)}
                                <span className="notice-group-label">{label}</span>
                                <span className="notice-group-count">{list.length}</span>
                            </div>
                            {list.map((notice) => (
                                <NoticeCard
                                    key={notice.id}
                                    notice={notice}
                                    onDismiss={handleDismiss}
                                    onAction={(action, n) => {
                                        const screenAddress = noticeScreen(n);
                                        if (screenAddress) {
                                            onClose();
                                            openScreen(screenAddress);
                                        } else if (action === 'openUpdateModal' && onOpenUpdateModal) {
                                            onClose();
                                            onOpenUpdateModal();
                                        } else if (action === PATCH_NOTES_ACTION && onOpenPatchNotes) {
                                            onClose();
                                            void openPatchNotes(onOpenPatchNotes);
                                        } else if (action === SERVER_UPDATE_ACTION) {
                                            void applyServerUpdateFromNotice(n, t);
                                        }
                                    }}
                                    t={t}
                                />
                            ))}
                        </div>
                    ))
                )}
            </div>
        </div>
    );
}

export default NoticeModal;
