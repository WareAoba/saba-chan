import i18n from 'i18next';

function moduleErrorBody(body) {
    // 고른 원격 데몬에 닿지 못한 것 같은 중계 오류 (01 §1.4.2): 화면마다 따로 번역하지 않게 여기서 문구로
    const relayKey = `remote_daemon.err_${body?.error_code}`;
    if (String(body?.error_code || '').startsWith('remote_') && i18n.exists(relayKey, { ns: 'gui' })) {
        // PC 전용 칸 거절(`remote_field_blocked`)은 막힌 칸 이름을 함께 보인다
        const fields = Array.isArray(body.fields) ? body.fields.join(', ') : '';
        return { ...body, error: i18n.t(relayKey, { ns: 'gui', fields }) };
    }
    const key = `module_errors.${body?.error_code}`;
    if (!i18n.exists(key, { ns: 'common' })) return body;
    const details = [body.error || body.message];
    if (body.module || body.function) details.push([body.module, body.function].filter(Boolean).join('.'));
    if (Array.isArray(body.stderr_tail)) details.push(...body.stderr_tail);
    return { ...body, error: [i18n.t(key, { ns: 'common' }), ...details.filter(Boolean)].join('\n') };
}

/**
 * 데몬 HTTP 요청 (Rust `daemon_request` 명령). main.js가 쓰던 axios 의미를 그대로 흉내 낸다:
 * 2xx면 {status, data}를 돌려주고, 그 밖의 HTTP 상태면 err.response = {status, data}인 오류를,
 * 연결 실패면 err.code(ECONNREFUSED·ETIMEDOUT·…)가 있는 오류를 던진다.
 */
export function createDaemonClient(invoke) {
    // target: 이 요청만 그 데몬에 보낸다 (원격 데몬 hostId, 이 PC는 'local'). 없으면 상단 바에서 고른 데몬
    async function request(method, path, body, { timeout = 5000, responseType, target } = {}) {
        const args = { method: method.toUpperCase(), path, timeoutMs: timeout };
        if (body !== undefined) args.body = body;
        if (responseType) args.responseType = responseType;
        if (target) args.target = target;
        let res;
        try {
            res = await invoke('daemon_request', args);
        } catch (e) {
            // 명령 자체가 실패 (Rust 쪽 오류)
            const err = new Error(typeof e === 'string' ? e : e?.message || String(e));
            err.code = 'EOTHER';
            throw err;
        }
        const status = res?.status ?? 0;
        if (status === 0) {
            const err = new Error(res?.error || 'Network Error');
            err.code = res?.code || 'EOTHER';
            throw err;
        }
        if (status < 200 || status >= 300) {
            const err = new Error(`Request failed with status code ${status}`);
            err.response = { status, data: moduleErrorBody(res.data) };
            throw err;
        }
        return { status, data: moduleErrorBody(res.data) };
    }

    /** 데몬이 준 오류 본문(error_code, details 등)을 버리지 않고 렌더러에 넘긴다 (error가 없으면 모듈이 준 message) */
    function errorBody(err, extra = {}) {
        const data = err.response?.data;
        const body = data && typeof data === 'object' && !Array.isArray(data) ? data : {};
        return { ...body, ...extra, error: body.error || body.message || err.message };
    }

    // 데몬 응답 본문을 그대로 돌려준다. HTTP 오류면 데몬이 준 본문({ok, error, error_code})을 돌려준다.
    async function daemonCall(method, path, body, timeout = 30000) {
        try {
            return (await request(method, path, body, { timeout })).data;
        } catch (error) {
            if (error.response) {
                return error.response.data || { ok: false, error: `HTTP ${error.response.status}` };
            }
            return { ok: false, error: error.message, error_code: error.code };
        }
    }

    // 데몬 JSON 호출. 4xx·5xx도 데몬이 준 본문을 그대로 돌려준다 (error_code, details)
    async function daemonJson(method, path, data, timeout = 15000) {
        try {
            return (await request(method, path, data, { timeout })).data;
        } catch (err) {
            return err.response?.data || { success: false, error: err.message };
        }
    }

    return { request, errorBody, daemonCall, daemonJson };
}

/** axios params처럼 값이 있는 것만 쿼리 문자열로 ('?' 포함, 없으면 빈 문자열) */
export function queryString(params) {
    const qs = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
        if (v !== undefined && v !== null) qs.set(k, String(v));
    }
    const s = qs.toString();
    return s ? `?${s}` : '';
}
