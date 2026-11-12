/**
 * 얇은 클라이언트 점검 2번 (2026-11-12): 목록 줄이 나타날 때마다(탭을 옮겨 올 때마다) 서버별 업데이트 확인을
 * 데몬에 시키던 것을 없앴다. 데몬이 주기적으로 확인하고 목록의 `update_check`에 결과를 싣는다.
 * 그 칸이 없는 옛 데몬에만 예전처럼 직접 확인을 시킨다.
 */
import { renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useInstanceMaintenance, useMaintenanceStore } from '../hooks/useInstanceMaintenance';
import type { Server } from '../stores/useServerStore';

const module = { name: 'palworld', install: { method: 'steamcmd' } } as never;
const steam = (extra: Partial<Server>) =>
    ({
        id: 's1',
        name: 'pal',
        module: 'palworld',
        status: 'stopped',
        extension_data: { install_method_steamcmd: true },
        ...extra,
    }) as Server;

beforeEach(() => {
    useMaintenanceStore.setState({ byId: {} });
    window.api = { instanceCheckUpdate: vi.fn().mockResolvedValue({ update_available: false }) } as unknown as typeof window.api;
});

describe('게임 서버 업데이트 확인', () => {
    it('데몬이 확인한 결과가 목록에 있으면 직접 확인을 시키지 않는다', () => {
        const check = { update_available: true, local_buildid: '1', remote_buildid: '2', checked_at: 10 };
        renderHook(() => useInstanceMaintenance(steam({ update_check: check }), module));
        expect(window.api.instanceCheckUpdate).not.toHaveBeenCalled();
        expect(useMaintenanceStore.getState().byId.s1?.updateInfo).toMatchObject({ remote_buildid: '2' });
    });

    it('데몬의 확인 실패는 줄의 경고로, 아직 확인 전이면 아무것도 보이지 않는다', () => {
        const { rerender } = renderHook(({ s }) => useInstanceMaintenance(s, module), {
            initialProps: { s: steam({ update_check: { update_available: false, reason: 'check_failed', error: 'timed out' } }) },
        });
        expect(useMaintenanceStore.getState().byId.s1).toMatchObject({ updateInfo: null, updateCheckError: 'timed out' });
        rerender({ s: steam({ update_check: null }) });
        expect(useMaintenanceStore.getState().byId.s1).toMatchObject({ updateInfo: null, updateCheckError: null });
        expect(window.api.instanceCheckUpdate).not.toHaveBeenCalled();
    });

    it('목록에 그 칸이 없는 옛 데몬이면 예전처럼 직접 확인을 시킨다', () => {
        renderHook(() => useInstanceMaintenance(steam({}), module));
        expect(window.api.instanceCheckUpdate).toHaveBeenCalledWith('s1');
    });
});
