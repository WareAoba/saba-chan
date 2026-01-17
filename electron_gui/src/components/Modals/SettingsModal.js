import React, { useState } from 'react';
import './Modals.css';

function SettingsModal({ isOpen, onClose }) {
    const [activeTab, setActiveTab] = useState('general');

    if (!isOpen) {
        return null;
    }

    return (
        <div className="settings-modal-overlay" onClick={onClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                <div className="settings-modal-header">
                    <h2>⚙️ GUI 설정</h2>
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
                            <p>여기에 일반 설정 항목을 추가할 예정입니다.</p>
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
