import React, { useState, useEffect } from 'react';
import './Toast.css';
import { Icon } from '../Icon';

function Toast() {
    const [toasts, setToasts] = useState([]);
    const [removingToasts, setRemovingToasts] = useState(new Set());

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
                const noticeType = type === 'warning' ? 'info' : (type === 'error' ? 'error' : type === 'success' ? 'success' : 'info');
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
            setToasts((prev) => prev.map(toast => 
                toast.id === id ? { ...toast, message, type } : toast
            ));
            
            // 기존 타이머 취소하고 새 타이머 설정
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
            const fullMessage = message;
            const newToast = { id, message: fullMessage, type, isStatus: true, step, icon: step };
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
            delete window.updateToast;
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
