import React, { useState, useEffect } from 'react';
import './Toast.css';

function Toast() {
    const [toasts, setToasts] = useState([]);
    const [removingToasts, setRemovingToasts] = useState(new Set());

    // 전역 토스트 이벤트 리스너 등록
    useEffect(() => {
        // 일반 토스트 표시 (자동 사라짐)
        window.showToast = (message, type = 'info', duration = 3000) => {
            const id = Date.now() + Math.random();
            const newToast = { id, message, type, isStatus: false };
            setToasts((prev) => [...prev, newToast]);

            // 모든 토스트는 자동으로 사라짐
            if (duration > 0) {
                setTimeout(() => {
                    removeToast(id);
                }, duration);
            }
        };

        // 상태 업데이트 토스트 표시 (백그라운드 초기화 메시지만 표시)
        window.showStatus = (step, message, duration = 3000) => {
            // daemon, modules, instances는 무시 (display: none으로 처리)
            // init, ready, ui만 표시
            if (['daemon', 'modules', 'instances'].includes(step)) {
                return;
            }

            const id = Date.now() + Math.random();
            const typeMap = {
                init: 'status-init',
                ready: 'status-ready',
                ui: 'status-ui'
            };
            const type = typeMap[step] || 'info';
            const statusIcon = {
                init: '⚙️',
                ready: '✅',
                ui: '🎨'
            };
            const fullMessage = statusIcon[step] ? `${statusIcon[step]} ${message}` : message;
            const newToast = { id, message: fullMessage, type, isStatus: true, step };
            setToasts((prev) => [...prev, newToast]);

            // 모든 상태 토스트도 자동으로 사라짐
            if (duration > 0) {
                setTimeout(() => {
                    removeToast(id);
                }, duration);
            }
        };

        // StatusBar의 상태 업데이트 신호를 받아 showStatus 호출
        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Status Update]', data.step, ':', data.message);
                window.showStatus(data.step, data.message, 3000); // 모든 상태 메시지는 3초 후 자동 사라짐
            });
        }

        return () => {
            delete window.showToast;
            delete window.showStatus;
        };
    }, []);

    const removeToast = (id) => {
        // 클릭하면 즉시 사라지기 위해 제거 애니메이션 추가
        setRemovingToasts((prev) => new Set([...prev, id]));
        
        // 애니메이션 완료 후 제거
        setTimeout(() => {
            setToasts((prev) => prev.filter((t) => t.id !== id));
            setRemovingToasts((prev) => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
        }, 300);
    };

    return (
        <div className="toast-container">
            {toasts.map((toast) => (
                <div 
                    key={toast.id} 
                    className={`toast toast-${toast.type} ${removingToasts.has(toast.id) ? 'toast-removing' : ''}`}
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
