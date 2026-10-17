/**
 * Utility functions extracted from App.js
 * Pure functions with no React hooks dependency.
 */

const isTestEnv = () => process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';

export const debugLog = (...args) => {
    if (!isTestEnv()) console.log(...args);
};

export const debugWarn = (...args) => {
    if (!isTestEnv()) console.warn(...args);
};

/**
 * Safe wrapper around window.showToast that won't throw if Toast isn't mounted yet.
 */
export const safeShowToast = (message, type, duration, options) => {
    if (!message) return null;
    if (typeof window.showToast === 'function') {
        return window.showToast(message, type, duration, options);
    }
    console.warn('[Toast] window.showToast not ready, message:', message);
    return null;
};

/**
 * Retry an async function with exponential backoff.
 */
export const retryWithBackoff = async (fn, maxRetries = 3, initialDelay = 500) => {
    for (let i = 0; i < maxRetries; i++) {
        try {
            return await fn();
        } catch (error) {
            if (i === maxRetries - 1) {
                throw error;
            }
            const delay = initialDelay * 2 ** i;
            debugWarn(`Attempt ${i + 1} failed, retrying in ${delay}ms...`, error.message);
            await new Promise((resolve) => setTimeout(resolve, delay));
        }
    }
};

/**
 * Wait for the daemon to be ready (polling daemonStatus).
 */
export const waitForDaemon = async (timeout = 10000) => {
    const start = Date.now();
    while (Date.now() - start < timeout) {
        try {
            const status = await window.api.daemonStatus();
            if (status.running) return true;
        } catch (_err) {
            // ignore
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('Daemon startup timeout');
};

/** 지정한 조건이 될 때까지 fn을 반복 호출한다. 조건을 만족한 결과를, 시간 초과 시 null을 반환한다. */
export const pollUntil = async (fn, isDone, { intervalMs = 500, maxAttempts = 20 } = {}) => {
    for (let attempt = 0; attempt < maxAttempts; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, intervalMs));
        try {
            const result = await fn();
            if (isDone(result)) return result;
        } catch (_err) {
            // 일시 오류 — 계속 폴링
        }
    }
    return null;
};

const TIMEOUT_PATTERN = /ETIMEDOUT|timeout of \d+ms exceeded/i;

/** axios/네트워크 타임아웃 메시지인지 — 반복 폴링에서 소음성 토스트를 거를 때 사용 */
export const isTimeoutError = (errorMessage) => TIMEOUT_PATTERN.test(String(errorMessage || ''));

/**
 * 오류 메시지를 사용자에게 보여줄 문자열로 바꾸는 함수를 만든다.
 * 데몬·main이 준 메시지는 그대로 보여준다. 원시 시스템 오류 코드(ECONNREFUSED 등)와
 * 빈 메시지만 번역 문구로 바꾼다. 반환값은 항상 비어 있지 않은 문자열이다.
 * @param {Function} t - i18next translation function
 */
export function createTranslateError(t) {
    return (errorMessage) => {
        const msg = errorMessage == null ? '' : String(errorMessage).trim();
        if (!msg || msg === 'null' || msg === 'undefined') return t('errors.unknown_error');
        if (msg.includes('ECONNREFUSED')) return t('errors.daemon_connection_refused');
        if (TIMEOUT_PATTERN.test(msg)) return t('errors.request_timeout');
        if (msg.includes('ENOTFOUND')) return t('errors.server_not_found');
        if (msg === 'Network Error') return t('errors.network_error');
        if (msg.includes('cloud_token_not_found')) return t('errors.cloud_token_not_found');
        return msg;
    };
}
