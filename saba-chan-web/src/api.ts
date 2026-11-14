/**
 * GUI 의 window.api(createApi)를 그대로 쓰고, 데몬 요청만 릴레이 세션으로 보낸다.
 * 응답 가공(오류 문구·모듈 오류)은 GUI 와 같다. PC 의 창·파일·프로세스를 다루는 명령은 웹에 없다.
 */
import { createApi } from '@gui/api/createApi';
import { createEventHub } from '@gui/api/events';
import type { PcEvent, SessionReply, WebSession } from './types';

/** 0.1.14까지의 데몬은 판정을 `ok`로 알린 경로가 있다: 화면이 읽는 `success`로 옮긴다 (웹은 옛 데몬에도 붙는다) */
function current(reply: SessionReply): SessionReply {
    const data = reply.data as { ok?: unknown; success?: unknown } | null | undefined;
    if (!data || typeof data !== 'object' || Array.isArray(data)) return reply;
    if (typeof data.ok !== 'boolean' || data.success !== undefined) return reply;
    return { ...reply, data: { ...data, success: data.ok } };
}

export function createWebApi(session: Pick<WebSession, 'request'> & Partial<Pick<WebSession, 'stream'>>) {
    const hub = createEventHub();
    // 연 콘솔 스트림 (셸의 daemon_stream_open과 같은 약속: 조각은 hub 'daemon:stream' {id, event, data})
    const streams = new Map<string, () => void>();
    const openStream = (id: string, path: string) => {
        const cancel = session.stream?.(
            'GET',
            path,
            (data) => hub.dispatch('daemon:stream', { id, event: 'message', data }),
            (reply) => {
                streams.delete(id);
                hub.dispatch('daemon:stream', { id, event: reply.status === 200 ? 'end' : 'error', data: reply.data });
            },
        );
        if (!cancel) return { status: 0 };
        streams.set(id, cancel);
        return { status: 200 };
    };
    const invoke = async (
        cmd: string,
        args: { method?: string; path?: string; body?: unknown; timeoutMs?: number; id?: string } = {},
    ) => {
        if (cmd === 'daemon_stream_open') return openStream(args.id as string, args.path as string);
        if (cmd === 'daemon_stream_close') {
            streams.get(args.id as string)?.();
            streams.delete(args.id as string);
            return null;
        }
        if (cmd !== 'daemon_request') throw new Error('not_supported');
        return current(await session.request(args.method as string, args.path as string, args.body, args.timeoutMs));
    };
    return {
        api: createApi({ invoke, hub }).api,
        /** 세션이 받은 PC 이벤트 {type, data} */
        emit: (event: PcEvent) => hub.dispatch('events:message', event),
    };
}
