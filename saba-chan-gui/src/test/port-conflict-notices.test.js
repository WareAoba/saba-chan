/**
 * 포트 충돌 강제 정지 알림은 데몬이 읽어도 지우지 않고 id를 붙여 둔다(L50).
 * GUI는 본 마지막 id를 `notices_after`로 실어 그 뒤 알림만 받고, 같은 알림을 두 번 띄우지 않는다.
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

const stop = (id, port) => ({
    id,
    stopped_name: 'b',
    stopped_id: 'b-id',
    existing_name: 'a',
    existing_id: 'a-id',
    port,
    timestamp_ms: 1,
});

describe('포트 충돌 강제 정지 알림', () => {
    it('본 마지막 id 뒤의 알림만 받아 한 번씩 알린다', async () => {
        const replies = [
            { servers: [], port_conflict_stops: [stop(1760000000000001, 25565), stop(1760000000000002, 25566)] },
            { servers: [], port_conflict_stops: [] },
        ];
        const invoke = vi.fn(async (cmd) =>
            cmd === 'daemon_request' ? { status: 200, data: replies.shift() } : undefined,
        );
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });

        await api.serverList();
        await api.serverList();

        const paths = invoke.mock.calls.filter(([cmd]) => cmd === 'daemon_request').map(([, a]) => a.path);
        expect(paths).toEqual(['/api/servers?notices_after=0', '/api/servers?notices_after=1760000000000002']);
        expect(invoke.mock.calls.filter(([cmd]) => cmd === 'notify')).toHaveLength(2);
    });

    it('id가 없는 옛 데몬의 알림도 그대로 알리고 커서는 움직이지 않는다', async () => {
        const invoke = vi.fn(async (cmd) =>
            cmd === 'daemon_request'
                ? { status: 200, data: { servers: [], port_conflict_stops: [{ ...stop(0, 1), id: undefined }] } }
                : undefined,
        );
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await api.serverList();
        await api.serverList();
        const paths = invoke.mock.calls.filter(([cmd]) => cmd === 'daemon_request').map(([, a]) => a.path);
        expect(paths).toEqual(['/api/servers?notices_after=0', '/api/servers?notices_after=0']);
        expect(invoke.mock.calls.filter(([cmd]) => cmd === 'notify')).toHaveLength(2);
    });
});
