import React from 'react';
import './Modals.css';

function DiscordBotModal({ 
    isOpen, 
    onClose, 
    discordBotStatus,
    discordToken,
    setDiscordToken,
    discordPrefix,
    setDiscordPrefix,
    discordAutoStart,
    setDiscordAutoStart,
    handleStartDiscordBot,
    handleStopDiscordBot,
    saveCurrentSettings
}) {
    if (!isOpen) {
        return null;
    }

    return (
        <div className="discord-modal-container" onClick={(e) => e.stopPropagation()}>
            <div className="discord-modal-header">
                    <div className="discord-modal-title">
                        <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : 'status-offline'}`}></span>
                        <h2>Discord Bot</h2>
                    </div>
                    <button className="discord-modal-close" onClick={onClose}>✕</button>
                </div>

                <div className="discord-modal-content">
                    <div className="discord-status-section">
                        <span className="status-label">상태:</span>
                        <span className={`status-value status-${discordBotStatus}`}>
                            {discordBotStatus === 'running' ? 'Online' : discordBotStatus === 'error' ? 'Error' : 'Offline'}
                        </span>
                    </div>

                    <div className="discord-config-section">
                        <div className="discord-form-group">
                            <label>Bot Token</label>
                            <input
                                type="password"
                                placeholder="Discord Bot Token을 입력하세요"
                                value={discordToken}
                                onChange={(e) => setDiscordToken(e.target.value)}
                                className="discord-input"
                            />
                        </div>

                        <div className="discord-form-group">
                            <label>봇 별명 (Prefix) *</label>
                            <input
                                type="text"
                                placeholder="예: !pal, !mc, !서버 등"
                                value={discordPrefix}
                                onChange={(e) => setDiscordPrefix(e.target.value)}
                                className="discord-input"
                            />
                            <small>봇이 반응할 명령어 접두사 (필수)</small>
                            {!discordPrefix && <small className="warning-text">⚠️ Prefix를 설정해주세요</small>}
                        </div>

                        <div className="discord-form-group">
                            <label className="discord-checkbox-label">
                                <input
                                    type="checkbox"
                                    checked={discordAutoStart}
                                    onChange={(e) => setDiscordAutoStart(e.target.checked)}
                                />
                                GUI 시작 시 봇 자동 실행
                            </label>
                        </div>
                    </div>

                    <div className="discord-info-box">
                        <h4>💡 봇 사용 방법</h4>
                        <p>Discord에서 다음 형식으로 명령어를 사용하세요:</p>
                        <code>{discordPrefix || '!saba'} [모듈명] [명령어]</code>
                        <p className="info-note">
                            모듈별 별명과 명령어 별명은 각 서버의 <strong>Settings → Discord 별명</strong> 탭에서 설정할 수 있습니다.
                        </p>
                    </div>
                </div>

                <div className="discord-modal-footer">
                    <button
                        className={`discord-btn ${
                            discordBotStatus === 'running' 
                                ? 'discord-btn-stop' 
                                : 'discord-btn-start'
                        }`}
                        onClick={() => {
                            if (discordBotStatus === 'running') {
                                handleStopDiscordBot();
                            } else {
                                handleStartDiscordBot();
                            }
                        }}
                    >
                        {discordBotStatus === 'running' ? '⏹ Stop Bot' : '▶ Start Bot'}
                    </button>
                    <button
                        className="discord-btn discord-btn-save"
                        onClick={saveCurrentSettings}
                    >
                        💾 저장
                    </button>
                </div>
            </div>
    );
}

export default DiscordBotModal;
