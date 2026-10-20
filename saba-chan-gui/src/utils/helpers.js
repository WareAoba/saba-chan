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
    } else {
        console.warn('[Toast] window.showToast not ready, message:', message);
        return null;
    }
};

/**
 * 폴더 열기 같은 셸 호출: 셸은 실패를 {error}(이미 번역된 문구)로 돌려주므로 토스트로 알린다.
 */
export const openWithNotice = async (call) => {
    try {
        const res = await call();
        if (res?.error) safeShowToast(res.error, 'error', 4000);
    } catch (err) {
        safeShowToast(err?.message || String(err), 'error', 4000);
    }
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
            if (status.running) {
                console.log('✓ Daemon is ready');
                return true;
            }
        } catch (_err) {
            // ignore
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('Daemon startup timeout');
};

/** 데몬이 켜져 있거나(server_running) 다른 작업 중이라(instance_busy), 모듈에 그 기능이 없어(not_supported) 거절한 이유. 그 밖의 코드는 null */
export function busyMessage(t, code, runningKey) {
    if (code === 'server_running') return t(`server_settings.${runningKey}`);
    if (code === 'instance_busy') return t('server_settings.instance_busy');
    if (code === 'not_supported') return t('errors.not_supported');
    return null;
}

/** 모듈이 lifecycle 함수 `fn`을 구현했는가 (`GET /api/modules`의 `functions`). 목록을 모르면(null) 있다고 본다 */
export const moduleSupports = (module, fn) => !Array.isArray(module?.functions) || module.functions.includes(fn);

/**
 * Creates an error translator function bound to the given i18n `t` function.
 * Maps common backend error messages to user-friendly i18n translations.
 * @param {Function} t - i18next translation function
 * @returns {Function} translateError(errorMessage) => translated string
 */
export function createTranslateError(t) {
    return (errorMessage) => {
        if (!errorMessage) return t('errors.unknown_error');

        const msg = String(errorMessage);

        // File path errors
        if (msg.includes('Executable not found') || msg.includes('executable not found')) {
            return t('errors.executable_not_found');
        }
        if (msg.includes('No such file or directory')) {
            return t('errors.file_not_found');
        }
        if (msg.includes('Permission denied')) {
            return t('errors.permission_denied');
        }

        // Network errors
        if (msg.includes('ECONNREFUSED')) {
            return t('errors.daemon_connection_refused');
        }
        // 시간 초과는 숨기지 않는다: 숨기면 저장·설치가 끝나지 않은 채 아무 알림도 없다
        if (msg.includes('ETIMEDOUT') || /timed out|timeout.*exceeded/i.test(msg)) {
            return t('errors.request_timeout');
        }
        if (msg.includes('ENOTFOUND')) {
            return t('errors.server_not_found');
        }
        if (msg.includes('Network Error') || msg.includes('network error')) {
            return t('errors.network_error');
        }

        // Server start/stop errors
        // 시작 준비 중에 정지가 들어와 띄우지 않았다 (데몬 start_cancelled)
        if (msg === 'start_cancelled' || msg.includes('was stopped while it was starting')) {
            return t('errors.start_cancelled');
        }
        if (msg.includes('Module failed to start')) {
            return t('errors.module_failed_to_start');
        }
        if (msg.includes('Failed to stop')) {
            return t('errors.failed_to_stop');
        }
        if (msg.includes('Already running')) {
            return t('errors.already_running');
        }
        if (msg.includes('Not running')) {
            return t('errors.not_running');
        }

        // Process errors
        if (msg.includes('Process not found')) {
            return t('errors.process_not_found');
        }
        if (msg.includes('Process crashed')) {
            return t('errors.process_crashed');
        }

        // Config errors
        if (msg.includes('Invalid configuration') || msg.includes('invalid config')) {
            return t('errors.invalid_configuration');
        }
        if (msg.includes('Missing required field')) {
            return t('errors.missing_required_field');
        }

        // Module errors
        if (msg.includes('Module not found')) {
            return t('errors.module_not_found');
        }
        if (msg.includes('Failed to load module')) {
            return t('errors.failed_to_load_module');
        }

        // Discord errors
        if (msg.includes('Invalid token') || msg.includes('invalid token')) {
            return t('errors.invalid_token');
        }
        if (msg.includes('Bot connection failed')) {
            return t('errors.network_error');
        }
        if (msg.includes('cloud_token_not_found')) {
            return t('errors.cloud_token_not_found');
        }
        if (msg === 'invalid_relay_url') {
            return t('discord.invalid_relay_url');
        }
        // Bot process exit — translate to user-friendly crash message
        if (msg.includes('Bot process exited with code') || msg.includes('exited with code')) {
            return t('errors.process_crashed');
        }

        // 모르는 오류는 데몬이 보낸 문구를 그대로 보여 준다 (데몬·모듈 문구는 이미 현지화돼 있다)
        return msg;
    };
}
