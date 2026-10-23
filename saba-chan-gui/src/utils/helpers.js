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
 * 브리지는 데몬 실패를 `{ error }`로 돌려준다: 그 답도 실패로 보고 다시 시도하고, 끝까지 실패하면 마지막 답을 돌려준다.
 */
export const retryWithBackoff = async (fn, maxRetries = 3, initialDelay = 500) => {
    for (let i = 0; i < maxRetries; i++) {
        let failure;
        try {
            const result = await fn();
            if (!result?.error || i === maxRetries - 1) return result;
            failure = result.error;
        } catch (error) {
            if (i === maxRetries - 1) {
                throw error;
            }
            failure = error.message;
        }
        const delay = initialDelay * 2 ** i;
        debugWarn(`Attempt ${i + 1} failed, retrying in ${delay}ms...`, failure);
        await new Promise((resolve) => setTimeout(resolve, delay));
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

/**
 * 설치 실패 문구. 백업을 못 해 설치하지 않았으면 그 파일(`backup_failed`의 `file`), 되돌리지 못한 파일은 남긴 사본
 * (`restore_failed`와 같은 순서의 `kept_backups`, 422·500 모두)을 덧붙인다.
 * 500 `error`는 `could not put back X (kept as Y): <모듈 오류>`라서 그 앞부분을 떼고 모듈 오류만 `translate`(translateError)에 넘긴다:
 * 모듈 오류의 낱말로 문구 전체가 바뀌어 사본 안내가 사라지지 않게
 */
export function installFailureText(t, result, translate) {
    if (result?.error_code === 'backup_failed' && result.file)
        return t('server_settings.install_backup_failed', { file: result.file });
    let message = result?.error || result?.message;
    const files = Array.isArray(result?.restore_failed) ? result.restore_failed : [];
    const kept = Array.isArray(result?.kept_backups) ? result.kept_backups : [];
    const pairs = files.map((file, i) => kept[i] && [file, kept[i]]).filter(Boolean);
    if (pairs.length && typeof message === 'string') {
        const prefix = `could not put back ${pairs.map(([file, bak]) => `${file} (kept as ${bak})`).join(', ')}`;
        if (message === prefix) message = undefined;
        else if (message.startsWith(`${prefix}: `)) message = message.slice(prefix.length + 2);
    }
    const text = translate(message);
    if (!pairs.length) return text;
    const list = pairs.map(([file, bak]) => `${file} → ${bak}`).join(', ');
    return `${text} ${t('server_settings.install_kept_backups', { files: list })}`;
}

export function moduleFailureText(t, result) {
    const known = t(`common:module_errors.${result?.error_code}`, { defaultValue: '' });
    return (known && result?.error) || result?.message || result?.error || '';
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

/** install_server가 받은 실행 파일 (계약은 executable_path, 예전 모듈은 jar_path만 준다) */
export const installedExecutable = (result) => result?.executable_path || result?.jar_path || '';
