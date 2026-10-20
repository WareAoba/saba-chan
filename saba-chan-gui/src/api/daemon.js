/**
 * 데몬 HTTP 요청 (Rust `daemon_request` 명령). main.js가 쓰던 axios 의미를 그대로 흉내 낸다:
 * 2xx면 {status, data}를 돌려주고, 그 밖의 HTTP 상태면 err.response = {status, data}인 오류를,
 * 연결 실패면 err.code(ECONNREFUSED·ETIMEDOUT·…)가 있는 오류를 던진다.
 */
export function createDaemonClient(invoke) {
    async function request(method, path, body, { timeout = 5000, responseType } = {}) {
        const args = { method: method.toUpperCase(), path, timeoutMs: timeout };
        if (body !== undefined) args.body = body;
        if (responseType) args.responseType = responseType;
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
            err.response = { status, data: res.data };
            throw err;
        }
        return { status, data: res.data };
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
