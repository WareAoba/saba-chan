/**
 * Utility functions extracted from App.js
 * Pure functions with no React hooks dependency.
 */

export const isTestEnv = () =>
    process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';

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
    if (typeof window.showToast === 'function') {
        return window.showToast(message, type, duration, options);
    } else {
        console.warn('[Toast] window.showToast not ready, message:', message);
        return null;
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
            const delay = initialDelay * Math.pow(2, i);
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
        } catch (err) {
            // ignore
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('Daemon startup timeout');
};

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
        if (msg.includes('ETIMEDOUT')) {
            return t('errors.request_timeout');
        }
        if (msg.includes('ENOTFOUND')) {
            return t('errors.server_not_found');
        }
        if (msg.includes('Network Error') || msg.includes('network error')) {
            return t('errors.network_error');
        }

        // Server start/stop errors
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

        // Fallback: return original message
        return msg;
    };
}
