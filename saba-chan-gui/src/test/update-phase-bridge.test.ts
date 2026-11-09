/**
 * 실시간 업데이트 브리지 (계약 D): 완료 확인은 데몬 POST /api/updates/phase/ack,
 * "지금 다시 시작"은 셸 명령 updater_restart(다시 띄울 GUI 경로는 셸이 실어 데몬 POST /api/updates/restart로 보낸다)
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

const make = () => {
    const invoke = vi.fn().mockResolvedValue({ status: 200, data: { ok: true } });
    const { api } = createApi({ invoke, hub: { on: vi.fn(), dispatch: vi.fn() } as never });
    return { api, invoke };
};

describe('업데이트 단계 브리지', () => {
    it('완료 확인은 phase/ack 로 보낸다', async () => {
        const { api, invoke } = make();
        await api.updaterPhaseAck();
        expect(invoke).toHaveBeenCalledWith('daemon_request', expect.objectContaining({ method: 'POST', path: '/api/updates/phase/ack' }));
    });

    it('지금 다시 시작은 셸 명령으로 간다', async () => {
        const { api, invoke } = make();
        await api.updaterRestart();
        expect(invoke).toHaveBeenCalledWith('updater_restart');
    });
});
