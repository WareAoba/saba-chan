import React from 'react';
import './TitleBar.css';

function TitleBar() {
    const handleMinimize = () => {
        window.electron.minimizeWindow();
    };

    const handleMaximize = () => {
        window.electron.maximizeWindow();
    };

    const handleClose = () => {
        window.electron.closeWindow();
    };

    return (
        <div className="title-bar">
            <div className="title-bar-text">
                <span>🎮 Saba-Chan</span>
            </div>
            <div className="title-bar-controls">
                <button 
                    className="title-bar-btn minimize-btn"
                    onClick={handleMinimize}
                    title="최소화"
                >
                    −
                </button>
                <button 
                    className="title-bar-btn maximize-btn"
                    onClick={handleMaximize}
                    title="최대화"
                >
                    ▢
                </button>
                <button 
                    className="title-bar-btn close-btn"
                    onClick={handleClose}
                    title="종료"
                >
                    ✕
                </button>
            </div>
        </div>
    );
}

export default TitleBar;
