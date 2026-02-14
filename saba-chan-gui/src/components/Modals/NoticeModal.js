import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';

// ─── localStorage 키 ───────────────────────────────────
const STORAGE_KEY = 'saba-chan-notices';

// ─── 알림 유틸 ───────────────────────────────────────────

/** localStorage에서 알림 목록 불러오기 */
function loadNotices() {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        return raw ? JSON.parse(raw) : [];
    } catch {
        return [];
    }
}

/** localStorage에 알림 목록 저장 */
function saveNotices(notices) {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(notices));
    } catch { /* quota exceeded 등 무시 */ }
}

/** 새 알림 추가 (전역 함수로 노출) */
function addNotice({ message, type = 'info', source = 'saba-chan', action = null, dedup = false }) {
    const notices = loadNotices();

    // 중복 방지: 같은 source + action이 이미 있으면 기존 알림을 업데이트
    if (dedup && action) {
        const existIdx = notices.findIndex(n => n.source === source && n.action === action);
        if (existIdx >= 0) {
            // 메시지만 업데이트하고 상단으로 이동
            const existing = notices.splice(existIdx, 1)[0];
            existing.message = message;
            existing.type = type;
            existing.timestamp = new Date().toISOString();
            existing.read = false;
            notices.unshift(existing);
            saveNotices(notices);
            window.dispatchEvent(new CustomEvent('saba-notice-update'));
            return existing;
        }
    }

    const notice = {
        id: Date.now() + Math.random(),
        message,
        type,       // 'info' | 'success' | 'error'
        source,     // 'saba-chan' 또는 서버 모듈 이름
        action,     // 'openUpdateModal' 등 클릭 시 실행할 액션
        timestamp: new Date().toISOString(),
        read: false,
    };
    notices.unshift(notice);
    saveNotices(notices);
    // 커스텀 이벤트로 리렌더 트리거
    window.dispatchEvent(new CustomEvent('saba-notice-update'));
    return notice;
}

/** 단일 알림 삭제 */
function removeNotice(id) {
    const notices = loadNotices().filter(n => n.id !== id);
    saveNotices(notices);
    window.dispatchEvent(new CustomEvent('saba-notice-update'));
}

/** 전체 알림 삭제 */
function clearAllNotices() {
    saveNotices([]);
    window.dispatchEvent(new CustomEvent('saba-notice-update'));
}

/** 읽지 않은 알림 개수 */
function getUnreadCount() {
    return loadNotices().filter(n => !n.read).length;
}

/** 모든 알림을 읽음 처리 */
function markAllRead() {
    const notices = loadNotices().map(n => ({ ...n, read: true }));
    saveNotices(notices);
    window.dispatchEvent(new CustomEvent('saba-notice-update'));
}

// 전역으로 내보내기 (Toast 등에서 사용)
window.__sabaNotice = { addNotice, removeNotice, clearAllNotices, getUnreadCount, loadNotices, markAllRead };


// ─── 알림 카드 ─────────────────────────────────────────

function NoticeCard({ notice, onDismiss, onAction, t }) {
    const typeConfig = {
        info:    { icon: 'info',        colorClass: 'notice-type-info' },
        success: { icon: 'checkCircle', colorClass: 'notice-type-success' },
        error:   { icon: 'xCircle',     colorClass: 'notice-type-error' },
    };
    const { icon, colorClass } = typeConfig[notice.type] || typeConfig.info;
    const clickable = !!notice.action;
    const isUpdateCard = notice.action === 'openUpdateModal';

    const timeStr = formatTime(notice.timestamp);

    const handleClick = () => {
        if (clickable && onAction) onAction(notice.action);
    };

    return (
        <div
            className={`notice-card ${colorClass} ${notice.read ? '' : 'notice-unread'} ${clickable ? 'notice-clickable' : ''} ${isUpdateCard ? 'notice-update-card' : ''}`}
            onClick={handleClick}
            style={clickable ? { cursor: 'pointer' } : undefined}
        >
            <div className="notice-card-icon">
                <Icon name={icon} size="sm" />
            </div>
            <div className="notice-card-body">
                <p className="notice-card-message">{notice.message}</p>
                <span className="notice-card-time">
                    {timeStr}
                    {clickable && <span className="notice-action-hint"> — 클릭하여 열기</span>}
                </span>
            </div>
            <button
                className="notice-card-dismiss"
                onClick={(e) => { e.stopPropagation(); onDismiss(notice.id); }}
                title={t('notice_modal.dismiss')}
            >
                <Icon name="close" size="xs" />
            </button>
        </div>
    );
}

function formatTime(isoString) {
    const d = new Date(isoString);
    const now = new Date();
    const diff = now - d;

    // 1분 이내
    if (diff < 60_000) return '방금 전';
    // 1시간 이내
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}분 전`;
    // 오늘
    if (d.toDateString() === now.toDateString()) {
        return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
    }
    // 어제
    const yesterday = new Date(now);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) {
        return `어제 ${d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`;
    }
    // 그 외
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) +
        ' ' + d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}


// ─── 알림 모달 본체 ─────────────────────────────────────

function NoticeModal({ isOpen, onClose, isClosing, onOpenUpdateModal }) {
    const { t } = useTranslation('gui');
    const [notices, setNotices] = useState(loadNotices);

    // 알림 변경 이벤트 구독
    useEffect(() => {
        const handler = () => setNotices(loadNotices());
        window.addEventListener('saba-notice-update', handler);
        return () => window.removeEventListener('saba-notice-update', handler);
    }, []);

    // 모달 열릴 때 모두 읽음 처리
    useEffect(() => {
        if (isOpen) markAllRead();
    }, [isOpen]);

    const handleDismiss = useCallback((id) => {
        removeNotice(id);
    }, []);

    const handleClearAll = useCallback(() => {
        clearAllNotices();
    }, []);

    if (!isOpen) return null;

    // source별 그룹핑
    const groups = {};
    for (const n of notices) {
        const key = n.source || 'saba-chan';
        if (!groups[key]) groups[key] = [];
        groups[key].push(n);
    }

    // saba-chan 그룹을 먼저 표시
    const sortedKeys = Object.keys(groups).sort((a, b) => {
        if (a === 'saba-chan') return -1;
        if (b === 'saba-chan') return 1;
        return a.localeCompare(b);
    });

    const sourceLabel = (key) => {
        if (key === 'saba-chan') return t('notice_modal.source_main');
        return key;
    };

    return (
        <div className={`notice-modal-container ${isClosing ? 'closing' : ''}`} onClick={(e) => e.stopPropagation()}>
            <div className="notice-modal-header">
                <div className="notice-modal-title">
                    <h2>{t('notice_modal.title')}</h2>
                    {notices.length > 0 && (
                        <span className="notice-count-badge">{notices.length}</span>
                    )}
                </div>
                <div className="notice-modal-actions">
                    {notices.length > 0 && (
                        <button className="notice-clear-all-btn" onClick={handleClearAll} title={t('notice_modal.clear_all')}>
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
                    sortedKeys.map(key => (
                        <div key={key} className="notice-group">
                            <div className="notice-group-header">
                                <span className="notice-group-label">{sourceLabel(key)}</span>
                                <span className="notice-group-count">{groups[key].length}</span>
                            </div>
                            {groups[key].map(notice => (
                                <NoticeCard
                                    key={notice.id}
                                    notice={notice}
                                    onDismiss={handleDismiss}
                                    onAction={(action) => {
                                        if (action === 'openUpdateModal' && onOpenUpdateModal) {
                                            onClose();
                                            onOpenUpdateModal();
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
