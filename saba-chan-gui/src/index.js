
import './api/install'; // Tauri 셸이면 window.api 설치 (가장 먼저)
import React from 'react';
import ReactDOM from 'react-dom/client';
import { ErrorBoundary } from 'react-error-boundary';
import App from './App';
import { ErrorFallback } from './components/ErrorFallback';
import { TrayPopup } from './components/shell/TrayPopup';
import { installMockApi } from './dev/mockApi';
import { i18nReady } from './i18n'; // Initialize i18n
import './styles/tokens.css'; // 디자인 토큰 (다른 CSS보다 먼저)
import './styles/base.css';
import { installOverlayScrollbars } from './utils/overlayScrollbar';
import { initTheme } from './utils/themeManager';

// 익스텐션 UMD 번들이 window.React / window.ReactDOM 을 참조하므로 전역 노출
window.React = React;
window.ReactDOM = ReactDOM;

// 브라우저 미리보기(`npm run dev`, Electron 없이)용 가짜 API
if (import.meta.env?.DEV && !window.api) installMockApi();

// 트레이 아이콘을 눌러 뜬 작은 창은 앱 전체가 아니라 팝업만 그린다
const trayPopup = new URLSearchParams(window.location.search).has('tray-popup');
if (trayPopup) document.documentElement.dataset.trayPopup = '';

// Apply saved theme before first render (트레이 팝업 표시를 먼저 달아야 투명 창에 바탕을 칠하지 않는다)
initTheme();
// 스크롤바는 내용 위에 겹쳐 그린다 (레이아웃을 밀지 않게)
installOverlayScrollbars();

const handleGlobalError = (error, info) => {
    console.error('[ErrorBoundary] Uncaught error:', error);
    console.error('[ErrorBoundary] Component stack:', info.componentStack);
};

// 고른 언어 파일을 읽은 뒤 그린다 (실패해도 영어로 그린다)
i18nReady.finally(() => {
    const root = ReactDOM.createRoot(document.getElementById('root'));
    root.render(
        <ErrorBoundary FallbackComponent={ErrorFallback} onError={handleGlobalError}>
            {trayPopup ? <TrayPopup /> : <App />}
        </ErrorBoundary>,
    );
});
