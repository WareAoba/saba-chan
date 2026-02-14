import React from 'react';

/**
 * SVG 아이콘 컴포넌트
 * @param {string} name - 아이콘 이름
 * @param {string} size - 아이콘 크기 ('sm', 'md', 'lg', 숫자)
 * @param {string} color - 아이콘 색상 (CSS 색상값)
 */
export const Icon = ({ name, size = 'md', color = 'currentColor' }) => {
    const sizeMap = {
        xs: 12,
        sm: 16,
        md: 20,
        lg: 24,
        xl: 32
    };

    const iconSize = typeof size === 'number' ? size : (sizeMap[size] || 20);

    const icons = {
        rocket: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z" />
                <path d="M9 3L5 7" />
            </svg>
        ),
        settings: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12.012 2.25c.734.008 1.465.093 2.182.253a.75.75 0 0 1 .582.649l.17 1.527a1.384 1.384 0 0 0 1.927 1.116l1.401-.615a.75.75 0 0 1 .85.174 9.792 9.792 0 0 1 2.204 3.792.75.75 0 0 1-.271.825l-1.242.916a1.381 1.381 0 0 0 0 2.226l1.243.915a.75.75 0 0 1 .272.826 9.797 9.797 0 0 1-2.204 3.792.75.75 0 0 1-.848.175l-1.407-.617a1.38 1.38 0 0 0-1.926 1.114l-.169 1.526a.75.75 0 0 1-.572.647 9.518 9.518 0 0 1-4.406 0 .75.75 0 0 1-.572-.647l-.168-1.524a1.382 1.382 0 0 0-1.926-1.11l-1.406.616a.75.75 0 0 1-.849-.175 9.798 9.798 0 0 1-2.204-3.796.75.75 0 0 1 .272-.826l1.243-.916a1.38 1.38 0 0 0 0-2.226l-1.243-.914a.75.75 0 0 1-.271-.826 9.793 9.793 0 0 1 2.204-3.792.75.75 0 0 1 .85-.174l1.4.615a1.387 1.387 0 0 0 1.93-1.118l.17-1.526a.75.75 0 0 1 .583-.65c.717-.159 1.45-.243 2.201-.252Zm0 1.5a9.135 9.135 0 0 0-1.354.117l-.109.977A2.886 2.886 0 0 1 6.525 7.17l-.898-.394a8.293 8.293 0 0 0-1.348 2.317l.798.587a2.881 2.881 0 0 1 0 4.643l-.799.588c.32.842.776 1.626 1.348 2.322l.905-.397a2.882 2.882 0 0 1 4.017 2.318l.11.984c.889.15 1.798.15 2.687 0l.11-.984a2.881 2.881 0 0 1 4.018-2.322l.905.396a8.296 8.296 0 0 0 1.347-2.318l-.798-.588a2.881 2.881 0 0 1 0-4.643l.796-.587a8.293 8.293 0 0 0-1.348-2.317l-.896.393a2.884 2.884 0 0 1-4.023-2.324l-.11-.976a8.988 8.988 0 0 0-1.333-.117ZM12 8.25a3.75 3.75 0 1 1 0 7.5 3.75 3.75 0 0 1 0-7.5Zm0 1.5a2.25 2.25 0 1 0 0 4.5 2.25 2.25 0 0 0 0-4.5Z" fill="currentColor"/>
            </svg>
        ),
        package: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <line x1="16.5" y1="9.4" x2="7.5" y2="4.21" />
                <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
                <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
                <line x1="12" y1="22.08" x2="12" y2="12" />
            </svg>
        ),
        database: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M4 6c0-.69.315-1.293.774-1.78.455-.482 1.079-.883 1.793-1.202C7.996 2.377 9.917 2 12 2c2.083 0 4.004.377 5.433 1.018.714.32 1.338.72 1.793 1.202.459.487.774 1.09.774 1.78v12c0 .69-.315 1.293-.774 1.78-.455.482-1.079.883-1.793 1.203C16.004 21.623 14.083 22 12 22c-2.083 0-4.004-.377-5.433-1.017-.714-.32-1.338-.72-1.793-1.203C4.315 19.293 4 18.69 4 18V6Zm1.5 0c0 .207.09.46.365.75.279.296.717.596 1.315.864 1.195.535 2.899.886 4.82.886 1.921 0 3.625-.35 4.82-.886.598-.268 1.036-.568 1.315-.864.275-.29.365-.543.365-.75 0-.207-.09-.46-.365-.75-.279-.296-.717-.596-1.315-.864C15.625 3.851 13.92 3.5 12 3.5c-1.921 0-3.625.35-4.82.886-.598.268-1.036.568-1.315.864-.275.29-.365.543-.365.75Zm13 2.392c-.32.22-.68.417-1.067.59C16.004 9.623 14.083 10 12 10c-2.083 0-4.004-.377-5.433-1.018a6.801 6.801 0 0 1-1.067-.59V18c0 .207.09.46.365.75.279.296.717.596 1.315.864 1.195.535 2.899.886 4.82.886 1.921 0 3.625-.35 4.82-.886.598-.268 1.036-.568 1.315-.864.275-.29.365-.543.365-.75V8.392Z" fill="currentColor"/>
            </svg>
        ),
        checkCircle: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
                <polyline points="22 4 12 14.01 9 11.01" />
            </svg>
        ),
        xCircle: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="10" />
                <line x1="15" y1="9" x2="9" y2="15" />
                <line x1="9" y1="9" x2="15" y2="15" />
            </svg>
        ),
        alertCircle: (
            <svg xmlns="http://www.w3.org/2000/svg" class="ionicon" viewBox="0 0 512 512">
                <path d="M448 256c0-106-86-192-192-192S64 150 64 256s86 192 192 192 192-86 192-192z" fill="none" stroke="currentColor" stroke-miterlimit="10" stroke-width="32"/>
                <path d="M250.26 166.05L256 288l5.73-121.95a5.74 5.74 0 00-5.79-6h0a5.74 5.74 0 00-5.68 6z" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="32"/>
                <path d="M256 367.91a20 20 0 1120-20 20 20 0 01-20 20z"/>
            </svg>
        ),
        x: (
            <svg xmlns="http://www.w3.org/2000/svg" class="ionicon" viewBox="0 0 512 512">
                <path d="M448 256c0-106-86-192-192-192S64 150 64 256s86 192 192 192 192-86 192-192z" fill="none" stroke="currentColor" stroke-miterlimit="10" stroke-width="32"/>
                <path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="32" d="M320 320L192 192M192 320l128-128"/>
            </svg>
        ),
        loader: (
            <svg xmlns="http://www.w3.org/2000/svg" class="ionicon" viewBox="0 0 512 512">
                <path d="M400 148l-21.12-24.57A191.43 191.43 0 00240 64C134 64 48 150 48 256s86 192 192 192a192.09 192.09 0 00181.07-128" fill="none" stroke="currentColor" stroke-linecap="round" stroke-miterlimit="10" stroke-width="32"/>
                <path d="M464 97.42V208a16 16 0 01-16 16H337.42c-14.26 0-21.4-17.23-11.32-27.31L436.69 86.1C446.77 76 464 83.16 464 97.42z"/>
            </svg>
        ),
        play: (
            <svg viewBox="0 0 24 24" fill="currentColor">
                <polygon points="5 3 19 12 5 21" />
            </svg>
        ),
        pause: (
            <svg viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="4" width="4" height="16" />
                <rect x="14" y="4" width="4" height="16" />
            </svg>
        ),
        stop: (
            <svg viewBox="0 0 24 24" fill="currentColor">
                <rect x="4" y="4" width="16" height="16" />
            </svg>
        ),
        edit: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
            </svg>
        ),
        trash: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 1.75a3.25 3.25 0 0 1 3.245 3.066L15.25 5h5.25a.75.75 0 0 1 .102 1.493L20.5 6.5h-.796l-1.28 13.02a2.75 2.75 0 0 1-2.561 2.474l-.176.006H8.313a2.75 2.75 0 0 1-2.714-2.307l-.023-.174L4.295 6.5H3.5a.75.75 0 0 1-.743-.648L2.75 5.75a.75.75 0 0 1 .648-.743L3.5 5h5.25A3.25 3.25 0 0 1 12 1.75Zm6.197 4.75H5.802l1.267 12.872a1.25 1.25 0 0 0 1.117 1.122l.127.006h7.374c.6 0 1.109-.425 1.225-1.002l.02-.126L18.196 6.5ZM13.75 9.25a.75.75 0 0 1 .743.648L14.5 10v7a.75.75 0 0 1-1.493.102L13 17v-7a.75.75 0 0 1 .75-.75Zm-3.5 0a.75.75 0 0 1 .743.648L11 10v7a.75.75 0 0 1-1.493.102L9.5 17v-7a.75.75 0 0 1 .75-.75Zm1.75-6a1.75 1.75 0 0 0-1.744 1.606L10.25 5h3.5A1.75 1.75 0 0 0 12 3.25Z" fill="currentColor"/>
            </svg>
        ),
        plus: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <line x1="12" y1="5" x2="12" y2="19" />
                <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
        ),
        minus: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
        ),
        chevronDown: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="6 9 12 15 18 9" />
            </svg>
        ),
        chevronUp: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="18 15 12 9 6 15" />
            </svg>
        ),
        chevronLeft: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="15 18 9 12 15 6" />
            </svg>
        ),
        chevronRight: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="9 18 15 12 9 6" />
            </svg>
        ),
        copy: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
            </svg>
        ),
        info: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 1.999c5.524 0 10.002 4.478 10.002 10.002 0 5.523-4.478 10.001-10.002 10.001-5.524 0-10.002-4.478-10.002-10.001C1.998 6.477 6.476 1.999 12 1.999Zm0 1.5a8.502 8.502 0 1 0 0 17.003A8.502 8.502 0 0 0 12 3.5Zm-.004 7a.75.75 0 0 1 .744.648l.007.102.003 5.502a.75.75 0 0 1-1.493.102l-.007-.101-.003-5.502a.75.75 0 0 1 .75-.75ZM12 7.003a.999.999 0 1 1 0 1.997.999.999 0 0 1 0-1.997Z" fill="currentColor"/>
            </svg>
        ),
        download: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
        ),
        discord: (
            <svg viewBox="0 0 192 192" fill="none">
                <path stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="12" d="m68 138-8 16c-10.19-4.246-20.742-8.492-31.96-15.8-3.912-2.549-6.284-6.88-6.378-11.548-.488-23.964 5.134-48.056 19.369-73.528 1.863-3.334 4.967-5.778 8.567-7.056C58.186 43.02 64.016 40.664 74 39l6 11s6-2 16-2 16 2 16 2l6-11c9.984 1.664 15.814 4.02 24.402 7.068 3.6 1.278 6.704 3.722 8.567 7.056 14.235 25.472 19.857 49.564 19.37 73.528-.095 4.668-2.467 8.999-6.379 11.548-11.218 7.308-21.769 11.554-31.96 15.8l-8-16m-68-8s20 10 40 10 40-10 40-10"/>
                <ellipse cx="71" cy="101" fill="currentColor" rx="13" ry="15"/><ellipse cx="121" cy="101" fill="currentColor" rx="13" ry="15"/>
            </svg>
        ),
        terminal: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="4 17 10 11 4 5" />
                <line x1="12" y1="19" x2="20" y2="19" />
            </svg>
        ),
        command: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18 3a3 3 0 0 0-3 3v12a3 3 0 0 0 3 3 3 3 0 0 0 3-3 3 3 0 0 0-3-3H6a3 3 0 0 0-3 3 3 3 0 0 0 3 3 3 3 0 0 0 3-3V6a3 3 0 0 0-3-3 3 3 0 0 0-3 3 3 3 0 0 0 3 3h12a3 3 0 0 0 3-3 3 3 0 0 0-3-3z" />
            </svg>
        ),
        server: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
                <rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
                <line x1="6" y1="6" x2="6.01" y2="6" />
                <line x1="6" y1="18" x2="6.01" y2="18" />
            </svg>
        ),
        gamepad: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M14.998 5a7 7 0 0 1 .24 13.996l-.24.004H9.002a7 7 0 0 1-.24-13.996L9.001 5h5.996Zm0 1.5H9.002a5.5 5.5 0 0 0-.221 10.996l.221.004h5.996a5.5 5.5 0 0 0 .221-10.996l-.221-.004ZM8 9a.75.75 0 0 1 .75.75v1.498h1.5a.75.75 0 0 1 0 1.5h-1.5v1.502a.75.75 0 0 1-1.5 0v-1.502h-1.5a.75.75 0 1 1 0-1.5h1.5V9.75A.75.75 0 0 1 8 9Zm6.75 3.5a1.25 1.25 0 1 1 0 2.5 1.25 1.25 0 0 1 0-2.5Zm2-3.5a1.25 1.25 0 1 1 0 2.5 1.25 1.25 0 0 1 0-2.5Z" fill="currentColor"/>
            </svg>
        ),
        folder: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M8.207 4c.46 0 .908.141 1.284.402l.156.12L12.022 6.5h7.728a2.25 2.25 0 0 1 2.229 1.938l.016.158.005.154v9a2.25 2.25 0 0 1-2.096 2.245L19.75 20H4.25a2.25 2.25 0 0 1-2.245-2.096L2 17.75V6.25a2.25 2.25 0 0 1 2.096-2.245L4.25 4h3.957Zm1.44 5.979a2.25 2.25 0 0 1-1.244.512l-.196.009-4.707-.001v7.251c0 .38.282.694.648.743l.102.007h15.5a.75.75 0 0 0 .743-.648l.007-.102v-9a.75.75 0 0 0-.648-.743L19.75 8h-7.729L9.647 9.979ZM8.207 5.5H4.25a.75.75 0 0 0-.743.648L3.5 6.25v2.749L8.207 9a.75.75 0 0 0 .395-.113l.085-.06 1.891-1.578-1.89-1.575a.75.75 0 0 0-.377-.167L8.207 5.5Z" fill="currentColor"/>
            </svg>
        ),
        network: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="5" r="3" />
                <path d="M12 8v6" />
                <path d="M8 11l-2.828 2.828" />
                <path d="M16 11l2.828 2.828" />
                <path d="M6 18l-2.828 2.828" />
                <path d="M18 18l2.828 2.828" />
                <path d="M6 18h12" />
            </svg>
        ),
        power: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
                <line x1="12" y1="2" x2="12" y2="12" />
            </svg>
        ),
        refresh: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polyline points="23 4 23 10 17 10" />
                <polyline points="1 20 1 14 7 14" />
                <path d="M3.51 9a9 9 0 0 1 14.85-3.36M20.49 15a9 9 0 0 1-14.85 3.36" />
            </svg>
        ),
        clock: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="10" />
                <polyline points="12 6 12 12 16 14" />
            </svg>
        ),
        messageSquare: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
            </svg>
        ),
        palette: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10Zm0-1.5v-17a8.5 8.5 0 0 1 0 17Z" fill="currentColor"/>
            </svg>
        ),
        close: (
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 640">
                <path fill="currentColor" d="M183.1 137.4C170.6 124.9 150.3 124.9 137.8 137.4C125.3 149.9 125.3 170.2 137.8 182.7L275.2 320L137.9 457.4C125.4 469.9 125.4 490.2 137.9 502.7C150.4 515.2 170.7 515.2 183.2 502.7L320.5 365.3L457.9 502.6C470.4 515.1 490.7 515.1 503.2 502.6C515.7 490.1 515.7 469.8 503.2 457.3L365.8 320L503.1 182.6C515.6 170.1 515.6 149.8 503.1 137.3C490.6 124.8 470.3 124.8 457.8 137.3L320.5 274.7L183.1 137.4z"/>
            </svg>
        ),
        check: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 2c5.523 0 10 4.477 10 10s-4.477 10-10 10S2 17.523 2 12 6.477 2 12 2Zm0 1.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17Zm-1.25 9.94 4.47-4.47a.75.75 0 0 1 1.133.976l-.073.084-5 5a.75.75 0 0 1-.976.073l-.084-.073-2.5-2.5a.75.75 0 0 1 .976-1.133l.084.073 1.97 1.97 4.47-4.47-4.47 4.47Z" fill="currentColor"/>
            </svg>
        ),
        globe: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="m10.946 2.047.005.007C11.296 2.018 11.646 2 12 2c5.522 0 10 4.477 10 10s-4.478 10-10 10a9.983 9.983 0 0 1-7.896-3.862h-.003v-.003A9.957 9.957 0 0 1 2 12c0-5.162 3.911-9.41 8.932-9.944l.014-.009ZM12 3.5l-.16.001c.123.245.255.533.374.85.347.923.666 2.282.1 3.487-.522 1.113-1.424 1.4-2.09 1.573l-.084.021c-.657.17-.91.235-1.093.514-.17.257-.144.582.061 1.25l.046.148c.082.258.18.57.23.863.064.364.082.827-.152 1.275a2.187 2.187 0 0 1-.9.945c-.341.185-.694.256-.958.302l-.093.017c-.515.09-.761.134-1 .39-.187.2-.307.553-.377 1.079-.029.214-.046.427-.064.646l-.01.117c-.02.242-.044.521-.099.76v.002a8.478 8.478 0 0 0 6.27 2.76c1.576 0 3.053-.43 4.319-1.178a4.47 4.47 0 0 1-.31-.35c-.34-.428-.786-1.164-.631-2.033.074-.418.298-.768.515-1.036a7.12 7.12 0 0 1 .72-.74l.158-.146c.179-.163.33-.301.46-.437.172-.18.21-.262.212-.267.068-.224-.015-.384-.106-.454a.304.304 0 0 0-.19-.061c-.084 0-.22.024-.401.14a.912.912 0 0 1-.836.085 1.025 1.025 0 0 1-.486-.432c-.144-.237-.225-.546-.278-.772-.04-.174-.08-.372-.115-.553l-.04-.206a4.127 4.127 0 0 0-.134-.54l-.02-.037a1.507 1.507 0 0 0-.064-.105 6.233 6.233 0 0 0-.227-.317l-.11-.143a12.686 12.686 0 0 1-.516-.712c-.196-.298-.417-.688-.487-1.104a1.46 1.46 0 0 1 .055-.734c.094-.264.265-.482.487-.649.483-.362 1.193-1.172 1.823-1.959.288-.359.544-.695.736-.95A8.46 8.46 0 0 0 12 3.5Zm5.727 2.22c-.197.263-.461.608-.757.978-.602.751-1.4 1.685-2.05 2.187.026.1.1.262.255.498.131.2.281.396.44.604l.129.17c.172.229.411.548.52.844.087.234.149.519.198.762l.049.246c.025.13.049.253.075.37.601-.172 1.201-.068 1.67.294.608.47.862 1.286.624 2.074-.11.362-.364.66-.563.869-.17.177-.372.362-.556.53l-.132.12c-.23.212-.423.4-.568.579-.148.184-.195.299-.205.356-.04.219.067.51.328.838a3.138 3.138 0 0 0 .374.392A8.48 8.48 0 0 0 20.5 12a8.478 8.478 0 0 0-2.773-6.28ZM3.5 12c0 1.398.338 2.718.936 3.881.085-.557.262-1.248.748-1.768.6-.642 1.335-.763 1.798-.839l.13-.021c.248-.044.391-.083.502-.143.088-.049.188-.128.288-.321.015-.028.042-.107.004-.325a5.236 5.236 0 0 0-.172-.636c-.02-.06-.04-.125-.06-.192-.185-.604-.48-1.602.12-2.515.522-.792 1.36-.994 1.893-1.123l.162-.04c.563-.145.883-.28 1.108-.758.295-.629.168-1.485-.146-2.32a7.615 7.615 0 0 0-.58-1.196A8.503 8.503 0 0 0 3.502 12Z" fill="currentColor"/>
            </svg>
        ),
        monitor: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                <line x1="8" y1="21" x2="16" y2="21" />
                <line x1="12" y1="17" x2="12" y2="21" />
            </svg>
        ),
        sun: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="5" />
                <line x1="12" y1="1" x2="12" y2="3" />
                <line x1="12" y1="21" x2="12" y2="23" />
                <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
                <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
                <line x1="1" y1="12" x2="3" y2="12" />
                <line x1="21" y1="12" x2="23" y2="12" />
                <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
                <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
            </svg>
        ),
        moon: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
            </svg>
        ),
        plug: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 22v-5" />
                <path d="M9 7V2" />
                <path d="M15 7V2" />
                <path d="M6 13V8a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4z" />
            </svg>
        ),
        hash: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <line x1="4" y1="9" x2="20" y2="9" />
                <line x1="4" y1="15" x2="20" y2="15" />
                <line x1="10" y1="3" x2="8" y2="21" />
                <line x1="16" y1="3" x2="14" y2="21" />
            </svg>
        ),
        broadcast: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="2" />
                <path d="M16.24 7.76a6 6 0 0 1 0 8.49" />
                <path d="M7.76 16.24a6 6 0 0 1 0-8.49" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                <path d="M4.93 19.07a10 10 0 0 1 0-14.14" />
            </svg>
        ),
        pin: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <line x1="12" y1="17" x2="12" y2="22" />
                <path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24z" />
            </svg>
        ),
        lightbulb: (
            <svg xmlns="http://www.w3.org/2000/svg" class="ionicon" viewBox="0 0 512 512">
                <path d="M304 384v-24c0-29 31.54-56.43 52-76 28.84-27.57 44-64.61 44-108 0-80-63.73-144-144-144a143.6 143.6 0 00-144 144c0 41.84 15.81 81.39 44 108 20.35 19.21 52 46.7 52 76v24M224 480h64M208 432h96M256 384V256" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="32"/>
                <path d="M294 240s-21.51 16-38 16-38-16-38-16" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="32"/>
            </svg>
        ),
        zap: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
            </svg>
        ),
        save: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M3 5.75A2.75 2.75 0 0 1 5.75 3h9.964a3.25 3.25 0 0 1 2.299.952l2.035 2.035c.61.61.952 1.437.952 2.299v9.964A2.75 2.75 0 0 1 18.25 21H5.75A2.75 2.75 0 0 1 3 18.25V5.75ZM5.75 4.5c-.69 0-1.25.56-1.25 1.25v12.5c0 .69.56 1.25 1.25 1.25H6v-5.25A2.25 2.25 0 0 1 8.25 12h7.5A2.25 2.25 0 0 1 18 14.25v5.25h.25c.69 0 1.25-.56 1.25-1.25V8.286c0-.465-.184-.91-.513-1.238l-2.035-2.035a1.75 1.75 0 0 0-.952-.49V7.25a2.25 2.25 0 0 1-2.25 2.25h-4.5A2.25 2.25 0 0 1 7 7.25V4.5H5.75Zm10.75 15v-5.25a.75.75 0 0 0-.75-.75h-7.5a.75.75 0 0 0-.75.75v5.25h9Zm-8-15v2.75c0 .414.336.75.75.75h4.5a.75.75 0 0 0 .75-.75V4.5h-6Z" fill="currentColor"/>
            </svg>
        ),
        enter: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M9 10l-5 5 5 5" />
                <path d="M20 4v7a4 4 0 0 1-4 4H4" />
            </svg>
        ),
        minimize: (
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
                <path fill="currentColor" d="M0 416c0-17.7 14.3-32 32-32l448 0c17.7 0 32 14.3 32 32s-14.3 32-32 32L32 448c-17.7 0-32-14.3-32-32z"/>
            </svg>
        ),
        maximize: (
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512">
                <path fill="currentColor" d="M32 32C14.3 32 0 46.3 0 64l0 96c0 17.7 14.3 32 32 32s32-14.3 32-32l0-64 64 0c17.7 0 32-14.3 32-32s-14.3-32-32-32L32 32zM64 352c0-17.7-14.3-32-32-32S0 334.3 0 352l0 96c0 17.7 14.3 32 32 32l96 0c17.7 0 32-14.3 32-32s-14.3-32-32-32l-64 0 0-64zM320 32c-17.7 0-32 14.3-32 32s14.3 32 32 32l64 0 0 64c0 17.7 14.3 32 32 32s32-14.3 32-32l0-96c0-17.7-14.3-32-32-32l-96 0zM448 352c0-17.7-14.3-32-32-32s-32 14.3-32 32l0 64-64 0c-17.7 0-32 14.3-32 32s14.3 32 32 32l96 0c17.7 0 32-14.3 32-32l0-96z"/>
            </svg>
        ),
        bell: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 2a7 7 0 0 1 7 7v3.528l1.8 3.6a1 1 0 0 1-.894 1.447L16 17.601V18a4 4 0 0 1-8 0v-.399l-3.906-.026a1 1 0 0 1-.894-1.447l1.8-3.6V9a7 7 0 0 1 7-7Zm2 15.601h-4V18a2 2 0 1 0 4 0v-.399ZM12 4a5 5 0 0 0-5 5v3.528a1 1 0 0 1-.106.447L5.618 15.6h12.764l-1.276-2.625a1 1 0 0 1-.106-.447V9a5 5 0 0 0-5-5Z" fill="currentColor"/>
            </svg>
        ),
        bellOff: (
            <svg width="24" height="24" fill="none" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 2a7 7 0 0 1 7 7v3.528l1.8 3.6a1 1 0 0 1-.894 1.447L16 17.601V18a4 4 0 0 1-8 0v-.399l-3.906-.026a1 1 0 0 1-.894-1.447l1.8-3.6V9a7 7 0 0 1 7-7Zm2 15.601h-4V18a2 2 0 1 0 4 0v-.399ZM12 4a5 5 0 0 0-5 5v3.528a1 1 0 0 1-.106.447L5.618 15.6h12.764l-1.276-2.625a1 1 0 0 1-.106-.447V9a5 5 0 0 0-5-5Z" fill="currentColor"/>
            </svg>
        ),
        externalLink: (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                <polyline points="15 3 21 3 21 9" />
                <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
        ),
    };

    const svgIcon = icons[name];
    if (!svgIcon) {
        return <span>?</span>;
    }

    return (
        <span
            style={{
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                width: iconSize,
                height: iconSize,
                color: color,
                flexShrink: 0,
                verticalAlign: '-0.125em',
            }}
        >
            <svg
                width={iconSize}
                height={iconSize}
                viewBox="0 0 24 24"
                style={{ display: 'block' }}
            >
                {svgIcon}
            </svg>
        </span>
    );
};

export default Icon;
