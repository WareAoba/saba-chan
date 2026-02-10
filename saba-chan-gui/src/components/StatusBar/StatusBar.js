import React, { useState, useEffect } from 'react';
import './StatusBar.css';
import { Icon } from '../Icon';

function StatusBar() {
    const [status, setStatus] = useState(null);
    const [visible, setVisible] = useState(false);
    const [fadeOut, setFadeOut] = useState(false);

    useEffect(() => {
        // 메인 프로세스로부터 상태 업데이트 수신
        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Status Update]', data.step, ':', data.message);
                setStatus(data);
                setFadeOut(false);
                setVisible(true);

                // 'ready' 상태일 때만 5초 후 사라지기
                if (data.step === 'ready') {
                    setTimeout(() => {
                        setFadeOut(true);
                        setTimeout(() => setVisible(false), 500); // CSS 애니메이션 대기
                    }, 3000);
                }
            });
        }
    }, []);

    if (!visible || !status) {
        return null;
    }

    const getIcon = (step) => {
        const iconMap = {
            daemon: 'settings',
            modules: 'package',
            instances: 'server',
            init: 'refresh',
            ready: 'checkCircle',
            ui: 'palette'
        };
        return iconMap[step] || 'loader';
    };

    return (
        <div className={`status-bar ${fadeOut ? 'fade-out' : ''}`}>
            <span className="status-icon"><Icon name={getIcon(status.step)} size="sm" /></span>
            <span className="status-message">{status.message}</span>
        </div>
    );
}

export default StatusBar;
