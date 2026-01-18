import React from 'react';
import './Modals.css';
import {
    Lightbulb24Regular
} from '@fluentui/react-icons';

function BackgroundModal({ isOpen, onClose }) {
    if (!isOpen) {
        return null;
    }

    return (
        <div className="background-modal-container" onClick={(e) => e.stopPropagation()}>
            <div className="background-modal-header">
                <div className="background-modal-title">
                    <span className="status-indicator status-online"></span>
                    <h2>Background Services</h2>
                </div>
                <button className="background-modal-close" onClick={onClose}>✕</button>
            </div>

            <div className="background-modal-content">
                <div className="background-status-section">
                    <span className="status-label">Core Daemon:</span>
                    <span className="status-value status-running">
                        Running
                    </span>
                </div>

                <div className="background-info-section">
                    <div className="info-row">
                        <span className="info-label">🔌 IPC Host:</span>
                        <span className="info-value">localhost</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label">🔢 IPC Port:</span>
                        <span className="info-value">57474</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label">📡 Protocol:</span>
                        <span className="info-value">HTTP/1.1</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label">⏱️ Uptime:</span>
                        <span className="info-value">Connected</span>
                    </div>
                </div>

                <div className="background-info-box">
                    <h4><Lightbulb24Regular /> About Core Daemon</h4>
                    <p>Core Daemon은 게임 서버 프로세스 관리, IPC 통신, 모듈 로딩을 담당하는 백그라운드 서비스입니다.</p>
                    <p className="info-note">
                        GUI를 닫아도 데몬은 계속 실행되며, 시스템 트레이에서 관리할 수 있습니다.
                    </p>
                </div>
            </div>
        </div>
    );
}

export default BackgroundModal;
