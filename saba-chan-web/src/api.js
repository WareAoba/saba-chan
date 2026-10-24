/**
 * GUI 의 window.api(createApi)를 그대로 쓰고, 데몬 요청만 릴레이 세션으로 보낸다.
 * 응답 가공(오류 문구·모듈 오류)은 GUI 와 같다. PC 의 창·파일·프로세스를 다루는 명령은 웹에 없다.
 */
import { createApi } from '@gui/api/createApi';
import { createEventHub } from '@gui/api/events';

export function createWebApi(session) {
    const hub = createEventHub();
    const invoke = async (cmd, args = {}) => {
        if (cmd !== 'daemon_request') throw new Error('not_supported');
        return session.request(args.method, args.path, args.body, args.timeoutMs);
    };
    return {
        api: createApi({ invoke, hub }).api,
        /** 세션이 받은 PC 이벤트 {type, data} */
        emit: (event) => hub.dispatch('events:message', event),
    };
}
