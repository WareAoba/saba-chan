import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './i18n'; // Initialize i18n
import './theme.css'; // Theme CSS variables (must load before App.css)
import { initTheme } from './utils/themeManager';

// Apply saved theme before first render
initTheme();

const root = ReactDOM.createRoot(document.getElementById('root'));
root.render(<App />);
