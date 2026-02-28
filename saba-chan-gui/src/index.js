
import React from 'react';
import ReactDOM from 'react-dom/client';
import { ErrorBoundary } from 'react-error-boundary';
import App from './App';
import { ErrorFallback } from './components/ErrorFallback';
import './i18n'; // Initialize i18n
import './theme.css'; // Theme CSS variables (must load before App.css)
import { initTheme } from './utils/themeManager';

// 익스텐션 UMD 번들이 window.React / window.ReactDOM 을 참조하므로 전역 노출
window.React = React;
window.ReactDOM = ReactDOM;

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
