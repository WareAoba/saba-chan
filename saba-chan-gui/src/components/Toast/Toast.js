import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import './Toast.css';

function Toast() {
    const [toasts, setToasts] = useState([]);
    const [removingToasts, setRemovingToasts] = useState(new Set());

    const removeToast = useCallback((id) => {
        setRemovingToasts((prev) => new Set([...prev, id]));
        setTimeout(() => {
            setToasts((prev) => prev.filter((t) => t.id !== id));
            setRemovingToasts((prev) => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
        }, 300);
    }, []);

    // 전역 토스트 이벤트 리스너 등록
    useEffect(() => {
        // 일반 토스트 표시 (자동 사라짐)
        // options.isNotice = true 이면 알림 모달에도 저장
        // options.source = 'saba-chan' | 서버 모듈 이름
        window.showToast = (message, type = 'info', duration = 3000, options = {}) => {
            const id = Date.now() + Math.random();
            const newToast = { id, message, type, isStatus: false };
            setToasts((prev) => [...prev, newToast]);

            // isNotice가 true이면 알림 저장소에도 추가
            if (options.isNotice && window.__sabaNotice) {
                const noticeType =
                    type === 'warning' ? 'info' : type === 'error' ? 'error' : type === 'success' ? 'success' : 'info';
                window.__sabaNotice.addNotice({
                    message,
                    type: noticeType,
                    source: options.source || 'saba-chan',
                });
            }

            // 모든 토스트는 자동으로 사라짐
            if (duration > 0) {
                setTimeout(() => {
                    removeToast(id);
                }, duration);
            }

            return id; // ID 반환하여 나중에 업데이트 가능
        };

        // 토스트 업데이트 기능
        window.updateToast = (id, message, type, duration = 3000) => {
            setToasts((prev) => prev.map((toast) => (toast.id === id ? { ...toast, message, type } : toast)));

            // 기존 타이머 취소하고 새 타이머 설정
            if (duration > 0) {
                setTimeout(() => {
                    removeToast(id);
                }, duration);
            }
        };

        // 상태 업데이트는 콘솔 로그로만 기록 (토스트 표시 안 함)
        window.showStatus = (step, message, _duration = 3000) => {
            // 초기화 상태 메시지는 로딩 화면에서 이미 표시되므로 토스트 불필요
            console.log('[Status]', step, ':', message);
        };

        // StatusBar의 상태 업데이트 신호를 받아 로그만 출력
        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Status Update]', data.step, ':', data.message);
            });
        }

        return () => {
            delete window.showToast;
            delete window.updateToast;
            delete window.showStatus;
        };
    }, [removeToast]);

    return (
        <div className="toast-container">
            {toasts.map((toast) => (
                <div
                    key={toast.id}
                    className={clsx('toast', `toast-${toast.type}`, { 'toast-removing': removingToasts.has(toast.id) })}
                    onClick={() => removeToast(toast.id)}
                >
                    {toast.icon && <span className="toast-icon">{toast.icon}</span>}
                    <span className="toast-message">{toast.message}</span>
                </div>
            ))}
        </div>
    );
}

export default Toast;
