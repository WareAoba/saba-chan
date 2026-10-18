
import './api/install'; // Tauri 셸이면 window.api 설치 (가장 먼저)
import React from 'react';
import ReactDOM from 'react-dom/client';
import { ErrorBoundary } from 'react-error-boundary';
import App from './App';
import { ErrorFallback } from './components/ErrorFallback';
import { installMockApi } from './dev/mockApi';
import './i18n'; // Initialize i18n
import './styles/tokens.css'; // 디자인 토큰 (다른 CSS보다 먼저)
import './styles/base.css';
import { initTheme } from './utils/themeManager';

// 익스텐션 UMD 번들이 window.React / window.ReactDOM 을 참조하므로 전역 노출
window.React = React;
window.ReactDOM = ReactDOM;

// 브라우저 미리보기(`npm run dev`, Electron 없이)용 가짜 API
if (import.meta.env?.DEV && !window.api) installMockApi();

// Apply saved theme before first render
initTheme();

const handleGlobalError = (error, info) => {
    console.error('[ErrorBoundary] Uncaught error:', error);
    console.error('[ErrorBoundary] Component stack:', info.componentStack);
};

const root = ReactDOM.createRoot(document.getElementById('root'));
root.render(
    <ErrorBoundary FallbackComponent={ErrorFallback} onError={handleGlobalError}>
        <App />
    </ErrorBoundary>,
);
