import clsx from 'clsx';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { useSettingsStore } from '../../stores/useSettingsStore';
import type { ToastOptions, ToastType } from '../../types/global';
import { deferDuringOpening } from '../../utils/opening';
import { Icon } from '../Icon';
import './Toast.css';

/** 종류별 앱 아이콘 (문구 앞 이모지 대신) */
const TYPE_ICON: Record<string, string> = {
    success: 'checkCircle',
    error: 'xCircle',
    warning: 'alertTriangle',
    info: 'info',
};

// 알림 표시 시간 설정 (toastDuration)에 따른 배율. 사라지지 않는 토스트(0)는 그대로
const DURATION_SCALE: Record<string, number> = { short: 0.6, normal: 1, long: 2 };
const scaled = (duration: number) =>
    duration > 0 ? Math.round(duration * (DURATION_SCALE[useSettingsStore.getState().toastDuration] ?? 1)) : duration;

interface ToastItem {
    id: number;
    message: string;
    type: ToastType;
    isStatus: boolean;
    icon?: ReactNode;
    action?: ToastOptions['action'];
}

function Toast() {
    const [toasts, setToasts] = useState<ToastItem[]>([]);
    const [removingToasts, setRemovingToasts] = useState<Set<number>>(new Set());
    // 사라지기 타이머: 언마운트 뒤에 setState가 돌지 않도록 모아 두었다가 지운다
    const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
    const later = useCallback((fn: () => void, ms: number) => {
        const id = setTimeout(() => {
            timers.current.delete(id);
            fn();
        }, ms);
        timers.current.add(id);
        return id;
    }, []);
    // 토스트마다 걸린 자동 사라짐 타이머 (갱신할 때 이전 것을 지운다)
    const durationTimers = useRef(new Map<number, ReturnType<typeof setTimeout>>());
    const scheduleRemoval = useCallback(
        (toastId: number, duration: number, remove: (id: number) => void) => {
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

    const removeToast = useCallback((id: number) => {
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
        window.showToast = (message, type = 'info', duration = 3000, options = {}) => {
            // 오프닝 중에는 끝난 뒤에 띄운다
            if (deferDuringOpening(() => window.showToast?.(message, type, duration, options))) return -1;
            const id = Date.now() + Math.random();
            const newToast = { id, message, type, isStatus: false, action: options.action };
            setToasts((prev) => [...prev, newToast]);

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

        return () => {
            delete window.showToast;
            delete window.updateToast;
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
                    {toast.icon ? (
                        <span className="toast-icon">{toast.icon}</span>
                    ) : (
                        TYPE_ICON[toast.type] && (
                            <span className={`toast-icon tone-${toast.type}`} aria-hidden="true">
                                <Icon name={TYPE_ICON[toast.type]} size="sm" />
                            </span>
                        )
                    )}
                    <span className="toast-message">{toast.message}</span>
                    {toast.action && (
                        <button
                            type="button"
                            className="btn btn-sm btn-ghost toast-action"
                            onClick={(e) => {
                                e.stopPropagation();
                                toast.action?.onClick();
                                removeToast(toast.id);
                            }}
                        >
                            {toast.action.label}
                        </button>
                    )}
                </div>
            ))}
        </div>
    );
}

export default Toast;
