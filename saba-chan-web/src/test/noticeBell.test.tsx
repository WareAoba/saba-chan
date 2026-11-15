import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { NoticeBell } from '../components/NoticeBell';
import { initI18n } from '../i18n';
import { useData } from '../store';
import type { PcEvent } from '../types';
import { asServers, mockApi } from './mocks';

// 웹 콘솔 알림: 데몬 알림함(GET /api/notices)을 앱과 같은 목록·같은 읽음 상태로 보인다
const flush = () => act(async () => {});
const now = Math.floor(Date.now() / 1000);
const inbox = () => ({
    success: true,
    notices: [
        { id: 3, at: now - 60, level: 'error', message: 'Crashed', detail: 'exit code 1', source: 'old-name', instanceId: 'i1', read: false },
        { id: 2, at: now - 120, level: 'error', message: 'Not responding', source: 'old-name', instanceId: 'i1', read: false },
        { id: 1, at: now - 86_400 * 3, level: 'success', message: 'Updated', source: 'Saba-chan', read: true },
    ],
    unread: 2,
});

function setup(list = vi.fn(async () => inbox())) {
    let listener: ((e: PcEvent) => void) | undefined;
    const api = mockApi({
        noticesList: list,
        noticesRead: vi.fn(async () => ({ success: true, unread: 0 })),
        noticesClear: vi.fn(async () => ({ success: true, removed: 3 })),
        onDaemonEvent: (cb: (e: PcEvent) => void) => {
            listener = cb;
            return () => {};
        },
    });
    render(<NoticeBell api={api} />);
    return { api, list, emit: (e: PcEvent) => act(async () => listener?.(e)) };
}

describe('웹 콘솔 알림 종', () => {
    beforeEach(async () => {
        await initI18n('en');
        useData.setState({ servers: asServers([{ id: 'i1', name: 'survival' }]) });
    });
    afterEach(cleanup);

    it('읽지 않은 수를 종에 보이고, 열면 서버별로 묶어 보이고 모두 읽음으로 바꾼다', async () => {
        const { api } = setup();
        await flush();
        const bell = screen.getByRole('button', { name: 'Notifications' });
        expect(within(bell).getByText('2')).toBeTruthy();

        fireEvent.click(bell);
        await flush();
        const panel = screen.getByRole('dialog', { name: 'Notifications' });
        // 서버 묶음은 지금 이름, 나머지는 데몬이 준 이름
        expect(within(panel).getByText('survival')).toBeTruthy();
        expect(within(panel).queryByText('old-name')).toBeNull();
        expect(within(panel).getByText('Saba-chan')).toBeTruthy();
        expect(within(panel).getByText('exit code 1')).toBeTruthy();
        expect(api.noticesRead).toHaveBeenCalledTimes(1);
        expect(within(bell).queryByText('2')).toBeNull();
    });

    it('전체 삭제는 데몬 알림함을 비운다', async () => {
        const { api } = setup();
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Notifications' }));
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Clear All' }));
        await flush();
        expect(api.noticesClear).toHaveBeenCalledTimes(1);
        expect(screen.getByText('No notifications')).toBeTruthy();
    });

    it('알림이 생길 만한 PC 이벤트가 오면 다시 읽는다 (정상 정지는 아니다)', async () => {
        const { list, emit } = setup();
        await flush();
        expect(list).toHaveBeenCalledTimes(1);
        await emit({ type: 'instance.stopped', data: {} } as PcEvent);
        expect(list).toHaveBeenCalledTimes(1);
        await emit({ type: 'instance.exit', data: { kind: 'crash' } } as PcEvent);
        expect(list).toHaveBeenCalledTimes(2);
    });
});
