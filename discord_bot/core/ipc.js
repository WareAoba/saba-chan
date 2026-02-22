/**
 * 🔌 IPC 통신 모듈
 * 
 * 메인 프로세스(saba-chan 데몬)와의 HTTP 기반 IPC 통신을 관리합니다.
 * - 토큰 관리 (로드, 갱신, 401 재시도)
 * - axios 인터셉터
 * - 서버/모듈 API 래퍼
 * - 범용 응답 포맷터
 */

const axios = require('axios');
const fs = require('fs');
const path = require('path');
const i18n = require('../i18n');

const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474';

// ── 토큰 관리 ──
let _cachedToken = '';
let _tokenRefreshPromise = null;

const _tokenPath = process.env.SABA_TOKEN_PATH
    || path.join(process.env.APPDATA || process.env.HOME || '.', 'saba-chan', '.ipc_token');

function loadToken() {
    if (!_cachedToken && process.env.SABA_TOKEN) {
        _cachedToken = process.env.SABA_TOKEN;
    }
    try {
        if (fs.existsSync(_tokenPath)) {
            const token = fs.readFileSync(_tokenPath, 'utf8').trim();
            if (token) {
                const prev = _cachedToken;
                _cachedToken = token;
                if (prev !== token) {
                    console.log(`[IPC] Auth token loaded: ${token.substring(0, 8)}… from ${_tokenPath}` +
                        (prev ? ` (was: ${prev.substring(0, 8)}…)` : ' (first load)'));
                }
                return true;
            }
        }
    } catch (e) {
        console.warn('[IPC] Could not read token file:', e.message);
    }
    return false;
}

// ── axios 설정 및 인터셉터 ──
function init() {
    axios.defaults.timeout = 15000;

    // 요청 전 토큰 주입
    axios.interceptors.request.use((config) => {
        let token = _cachedToken;
        if (!token) {
            loadToken();
            token = _cachedToken;
        }
        if (token) {
            if (typeof config.headers?.set === 'function') {
                config.headers.set('X-Saba-Token', token);
            } else if (config.headers) {
                config.headers['X-Saba-Token'] = token;
            }
        }
        return config;
    });

    // 401 응답 시 토큰 자동 재로드 + 재시도
    axios.interceptors.response.use(
        (response) => response,
        async (error) => {
            const originalRequest = error.config;
            if (error.response && error.response.status === 401 && !originalRequest._retried) {
                originalRequest._retried = true;

                if (!_tokenRefreshPromise) {
                    _tokenRefreshPromise = (async () => {
                        try {
                            const newToken = fs.readFileSync(_tokenPath, 'utf8').trim();
                            if (newToken) {
                                _cachedToken = newToken;
                                console.log(`[IPC] Token refreshed after 401: ${newToken.substring(0, 8)}…`);
                                return newToken;
                            }
                        } catch (_) { /* 토큰 파일 읽기 실패 */ }
                        return null;
                    })();
                    _tokenRefreshPromise.finally(() => {
                        setTimeout(() => { _tokenRefreshPromise = null; }, 300);
                    });
                }

                const refreshedToken = await _tokenRefreshPromise;
                if (refreshedToken) {
                    if (typeof originalRequest.headers?.set === 'function') {
                        originalRequest.headers.set('X-Saba-Token', refreshedToken);
                    } else {
                        originalRequest.headers['X-Saba-Token'] = refreshedToken;
                    }
                    return axios(originalRequest);
                }
            }
            return Promise.reject(error);
        }
    );

    // 최초 토큰 로드
    loadToken();
}

// ── API 래퍼 ──

async function getServers() {
    const res = await axios.get(`${IPC_BASE}/api/servers`);
    return res.data.servers || [];
}

async function getModules() {
    const res = await axios.get(`${IPC_BASE}/api/modules`);
    return res.data.modules || [];
}

async function getModuleDetail(moduleName) {
    const res = await axios.get(`${IPC_BASE}/api/module/${moduleName}`);
    return res.data.toml || {};
}

async function startServer(serverId, serverName, serverModule, useManaged) {
    if (useManaged) {
        return axios.post(`${IPC_BASE}/api/instance/${serverId}/managed/start`, {});
    }
    return axios.post(`${IPC_BASE}/api/server/${serverName}/start`, {
        module: serverModule,
        config: {}
    });
}

async function stopServer(serverName) {
    return axios.post(`${IPC_BASE}/api/server/${serverName}/stop`, { force: false });
}

async function sendStdin(serverId, command) {
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/stdin`, { command });
}

async function sendRcon(serverId, command) {
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/rcon`, {
        command,
        instance_id: serverId,
    });
}

async function sendRestCommand(serverId, endpoint, httpMethod, body, serverOpts) {
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/rest`, {
        endpoint,
        method: httpMethod,
        body,
        instance_id: serverId,
        rest_host: serverOpts.rest_host || '127.0.0.1',
        rest_port: serverOpts.rest_port || 8212,
        username: serverOpts.rest_username || 'admin',
        password: serverOpts.rest_password || '',
    });
}

async function sendModuleCommand(serverId, commandName, body) {
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/command`, {
        command: commandName,
        args: body,
        instance_id: serverId,
    });
}

// ── 범용 응답 포맷터 ──

function formatResponse(data) {
    if (data === null || data === undefined) {
        return i18n.t('bot:responses.command_complete');
    }
    if (typeof data === 'string') {
        return data || i18n.t('bot:responses.command_complete');
    }
    if (Array.isArray(data)) {
        if (data.length === 0) return i18n.t('bot:responses.empty_list');
        return formatArray(data);
    }
    if (typeof data === 'object' && Object.keys(data).length === 0) {
        return i18n.t('bot:responses.command_complete');
    }
    if (typeof data === 'object') {
        for (const [key, value] of Object.entries(data)) {
            if (Array.isArray(value)) {
                if (value.length === 0) return `📋 **${key}**: (empty)`;
                return `📋 **${key}** (${value.length}):\n${formatArray(value)}`;
            }
        }
        const entries = Object.entries(data)
            .filter(([_, v]) => v !== null && v !== undefined)
            .map(([k, v]) => `• **${k}**: ${v}`)
            .join('\n');
        return entries || i18n.t('bot:responses.command_complete');
    }
    return String(data);
}

function formatArray(arr) {
    return arr.map((item, idx) => {
        if (typeof item === 'string') return `${idx + 1}. ${item}`;
        if (typeof item === 'object' && item !== null) {
            const name = item.name || item.id || item.userid || `#${idx + 1}`;
            const extras = Object.entries(item)
                .filter(([k]) => !['name', 'id'].includes(k))
                .map(([k, v]) => `${k}: ${v}`)
                .join(', ');
            return extras ? `• **${name}** (${extras})` : `• **${name}**`;
        }
        return `• ${item}`;
    }).join('\n');
}

module.exports = {
    init,
    getServers,
    getModules,
    getModuleDetail,
    startServer,
    stopServer,
    sendStdin,
    sendRcon,
    sendRestCommand,
    sendModuleCommand,
    formatResponse,
};
