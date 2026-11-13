import i18n from 'i18next';
import type { ApiBody, DaemonError, DaemonResponse, HttpMethod, RequestOptions } from '../types/daemon';

/** Rust 명령 호출 (`invoke(cmd, args)`) */
// biome-ignore lint/suspicious/noExplicitAny: Rust 명령마다 결과 모양이 다르다
export type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<any>;

function moduleErrorBody(body: ApiBody): ApiBody {
    // 고른 원격 데몬에 닿지 못한 것 같은 중계 오류 (01 §1.4.2): 화면마다 따로 번역하지 않게 여기서 문구로
    // 최소 판(`MIN_REMOTE_DAEMON`)보다 낮은 원격 데몬: 데몬이 중계하지 않고 그 판을 알린다
    if (body?.error_code === 'remote_too_old') {
        return { ...body, error: i18n.t('remote_daemon.update_needed', { ns: 'gui', version: body.min_version }) };
    }
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
export function createDaemonClient(invoke: Invoke) {
    // target: 이 요청만 그 데몬에 보낸다 (원격 데몬 hostId, 이 PC는 'local'). 없으면 상단 바에서 고른 데몬
    async function request<T = ApiBody>(
        method: HttpMethod,
        path: string,
        body?: unknown,
        { timeout = 5000, responseType, target }: RequestOptions = {},
    ): Promise<DaemonResponse<T>> {
        const args: Record<string, unknown> = { method: method.toUpperCase(), path, timeoutMs: timeout };
        if (body !== undefined) args.body = body;
        if (responseType) args.responseType = responseType;
        if (target) args.target = target;
        let res: { status?: number; data?: ApiBody; error?: string; code?: string } | undefined;
        try {
            res = await invoke('daemon_request', args);
        } catch (e) {
            // 명령 자체가 실패 (Rust 쪽 오류)
            const err: DaemonError = new Error(typeof e === 'string' ? e : (e as Error)?.message || String(e));
            err.code = 'EOTHER';
            throw err;
        }
        const status = res?.status ?? 0;
        if (status === 0) {
            const err: DaemonError = new Error(res?.error || 'Network Error');
            err.code = res?.code || 'EOTHER';
            throw err;
        }
        if (status < 200 || status >= 300) {
            const err: DaemonError = new Error(`Request failed with status code ${status}`);
            err.response = { status, data: moduleErrorBody(res?.data as ApiBody) };
            throw err;
        }
        return { status, data: moduleErrorBody(res?.data as ApiBody) as T };
    }

    // 데몬 응답 본문을 그대로 돌려준다. 실패하면 던지지 않고 failureBody를 돌려준다
    async function call(method: HttpMethod, path: string, body?: unknown, timeout = 30000): Promise<ApiBody> {
        try {
            return (await request(method, path, body, { timeout })).data;
        } catch (e) {
            return failureBody(e as DaemonError);
        }
    }

    return { request, call };
}

/**
 * 브리지의 실패 모양 (하나뿐이다): 데몬이 준 오류 본문(error_code, details 등)을 버리지 않고 error 문구를 채운다.
 * error가 없으면 모듈이 준 message, 그것도 없으면 오류 문구. 연결 실패면 error_code에 연결 오류 코드(ECONNREFUSED 등).
 * 판정은 utils/apiResult `failed`.
 */
export function failureBody(err: DaemonError): ApiBody {
    const data = err.response?.data;
    const body: ApiBody = data && typeof data === 'object' && !Array.isArray(data) ? data : {};
    const out: ApiBody = { ...body, error: body.error || body.message || err.message };
    const code = body.error_code ?? err.code;
    if (code !== undefined) out.error_code = code;
    return out;
}

/** axios params처럼 값이 있는 것만 쿼리 문자열로 ('?' 포함, 없으면 빈 문자열) */
export function queryString(params: Record<string, unknown>): string {
    const qs = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
        if (v !== undefined && v !== null) qs.set(k, String(v));
    }
    const s = qs.toString();
    return s ? `?${s}` : '';
}
