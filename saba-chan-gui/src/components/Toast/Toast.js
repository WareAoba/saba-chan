import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useSettingsStore } from '../../stores/useSettingsStore';
import './Toast.css';

// 알림 표시 시간 설정 (toastDuration)에 따른 배율. 사라지지 않는 토스트(0)는 그대로
const DURATION_SCALE = { short: 0.6, normal: 1, long: 2 };
const scaled = (duration) =>
    duration > 0 ? Math.round(duration * (DURATION_SCALE[useSettingsStore.getState().toastDuration] ?? 1)) : duration;

function Toast() {
    const [toasts, setToasts] = useState([]);
    const [removingToasts, setRemovingToasts] = useState(new Set());
    // 사라지기 타이머: 언마운트 뒤에 setState가 돌지 않도록 모아 두었다가 지운다
    const timers = useRef(new Set());
    const later = useCallback((fn, ms) => {
        const id = setTimeout(() => {
            timers.current.delete(id);
            fn();
        }, ms);
        timers.current.add(id);
        return id;
    }, []);
    // 토스트마다 걸린 자동 사라짐 타이머 (갱신할 때 이전 것을 지운다)
    const durationTimers = useRef(new Map());
    const scheduleRemoval = useCallback(
        (toastId, duration, remove) => {
            const prev = durationTimers.current.get(toastId);
            if (prev !== undefined) {
                clearTimeout(prev);
                timers.current.delete(prev);
                durationTimers.current.delete(toastId);
            }
            if (duration > 0) {
                const timer = later(() => {
                    durationTimers.current.delete(toastId);
                    remove(toastId);
                }, scaled(duration));
                durationTimers.current.set(toastId, timer);
            }
        },
        [later],
    );
    useEffect(() => {
        const pending = timers.current;
        return () => {
            for (const id of pending) clearTimeout(id);
            pending.clear();
        };
    }, []);

    const removeToast = useCallback((id) => {
        setRemovingToasts((prev) => new Set([...prev, id]));
        later(() => {
            setToasts((prev) => prev.filter((t) => t.id !== id));
            setRemovingToasts((prev) => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
        }, 300);
    }, [later]);

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
            scheduleRemoval(id, duration, removeToast);

            return id; // ID 반환하여 나중에 업데이트 가능
        };

        // 토스트 업데이트 기능
        window.updateToast = (id, message, type, duration = 3000) => {
            setToasts((prev) => prev.map((toast) => (toast.id === id ? { ...toast, message, type } : toast)));

            // 기존 타이머 취소하고 새 타이머 설정
            scheduleRemoval(id, duration, removeToast);
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
    }, [removeToast, scheduleRemoval]);

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
