import React, { useState, useEffect } from 'react';
import './Modals.css';
import { Icon } from '../Icon';

function SettingsModal({ isOpen, onClose, refreshInterval, onRefreshIntervalChange }) {
    const [activeTab, setActiveTab] = useState('general');
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);

    // refreshInterval prop이 변경되면 로컬 상태 업데이트
    useEffect(() => {
        setLocalRefreshInterval(refreshInterval);
    }, [refreshInterval]);

    // 리프레시 주기 변경 핸들러
    const handleRefreshIntervalChange = (value) => {
        setLocalRefreshInterval(value);
        if (onRefreshIntervalChange) {
            onRefreshIntervalChange(value);
        }
    };

    if (!isOpen) {
        return null;
    }

    return (
        <div className="settings-modal-overlay" onClick={onClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                <div className="settings-modal-header">
                    <h2><Icon name="settings" size="md" /> GUI 설정</h2>
                    <button className="settings-modal-close" onClick={onClose}>✕</button>
                </div>

                <div className="settings-modal-tabs">
                    <button
                        className={`settings-tab ${activeTab === 'general' ? 'active' : ''}`}
                        onClick={() => setActiveTab('general')}
                    >
                        일반
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'appearance' ? 'active' : ''}`}
                        onClick={() => setActiveTab('appearance')}
                    >
                        외형
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'advanced' ? 'active' : ''}`}
                        onClick={() => setActiveTab('advanced')}
                    >
                        고급
                    </button>
                </div>

                <div className="settings-modal-content">
                    {activeTab === 'general' && (
                        <div className="settings-tab-content">
                            <h3>일반 설정</h3>
                            
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">🔄 서버 상태 업데이트 주기</span>
                                    <span className="setting-description">서버 프로세스 상태를 확인하는 주기를 설정합니다</span>
                                </label>
                                <select 
                                    className="setting-select"
                                    value={localRefreshInterval}
                                    onChange={(e) => handleRefreshIntervalChange(Number(e.target.value))}
                                >
                                    <option value={1000}>1초</option>
                                    <option value={2000}>2초</option>
                                    <option value={3000}>3초</option>
                                    <option value={5000}>5초</option>
                                    <option value={10000}>10초</option>
                                </select>
                            </div>
                        </div>
                    )}

                    {activeTab === 'appearance' && (
                        <div className="settings-tab-content">
                            <h3>외형 설정</h3>
                            <p>여기에 외형 설정 항목을 추가할 예정입니다.</p>
                        </div>
                    )}

                    {activeTab === 'advanced' && (
                        <div className="settings-tab-content">
                            <h3>고급 설정</h3>
                            <p>여기에 고급 설정 항목을 추가할 예정입니다.</p>
                        </div>
                    )}
                </div>

                <div className="settings-modal-footer">
                    <button className="settings-btn-cancel" onClick={onClose}>
                        닫기
                    </button>
                </div>
            </div>
        </div>
    );
}

export default SettingsModal;
