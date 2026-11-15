import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import NoticeModal from '../components/Modals/NoticeModal';
import i18n from '../i18n';
import { useServerStore } from '../stores/useServerStore';
import { refreshNotices } from '../utils/noticeInbox';

// 알림함은 데몬이 보관한다 (GET /api/notices). 문장·묶음 이름도 데몬이 만든다
const daemonList = [
    { id: 4, at: 1700000300, level: 'success', message: 'Updated to Saba-chan 0.2.0', source: 'Saba-chan', action: 'openPatchNotes', read: false },
    { id: 3, at: 1700000200, level: 'error', message: 'Exited by crash', detail: 'Memory limit reached (4096 MiB)', source: 'Our MC', instanceId: 'mc', read: false },
    { id: 2, at: 1700000100, level: 'error', message: 'Start failed', detail: 'Exited 4s after start', source: 'Pal', instanceId: 'pal', read: true },
    { id: 1, at: 1700000000, level: 'info', message: 'Started outside Saba-chan', source: 'Old MC name', instanceId: 'mc', read: true },
];

describe('알림함 (#14: 서버마다 한 묶음, 한 줄은 짧게 + 이유 한 줄)', () => {
    beforeEach(async () => {
        useServerStore.setState({ servers: [{ id: 'mc', name: 'Our MC', module: 'minecraft', status: 'running' }] as never });
        window.api.noticesList = vi.fn().mockResolvedValue({ success: true, notices: daemonList, unread: 2 }) as never;
        window.api.noticesRead = vi.fn().mockResolvedValue({ success: true, unread: 0 }) as never;
        window.api.noticeRemove = vi.fn().mockResolvedValue({ success: true }) as never;
        window.api.noticesClear = vi.fn().mockResolvedValue({ success: true }) as never;
        await refreshNotices();
    });
    afterEach(() => {
        for (const k of ['noticesList', 'noticesRead', 'noticeRemove', 'noticesClear', 'updaterLastResult']) {
            delete (window.api as unknown as Record<string, unknown>)[k];
        }
    });

    it('데몬 목록을 서버별로 묶고, 이름은 지금 이름을 쓰며, 이유를 둘째 줄로 보인다', async () => {
        await act(async () => {
            render(<NoticeModal isOpen onClose={() => {}} />);
        });
        const groups = document.querySelectorAll('.notice-group');
        expect([...groups].map((g) => g.querySelector('.notice-group-label')?.textContent)).toEqual(['Saba-chan', 'Our MC', 'Pal']);
        const mc = groups[1] as HTMLElement;
        expect(within(mc).getByText('2')).toBeInTheDocument();
        expect(within(mc).getByText('Exited by crash')).toBeInTheDocument();
        expect(within(mc).getByText('Memory limit reached (4096 MiB)')).toBeInTheDocument();
        expect(screen.queryByText('Old MC name')).toBeNull();
        // 열면 데몬에 모두 읽음을 보낸다 (다른 화면도 같은 읽음 상태를 본다)
        expect(window.api.noticesRead).toHaveBeenCalled();
    });

    it('지우기·전체 삭제는 데몬에 보낸다', async () => {
        await act(async () => {
            render(<NoticeModal isOpen onClose={() => {}} />);
        });
        const pal = document.querySelectorAll('.notice-group')[2] as HTMLElement;
        await act(async () => fireEvent.click(within(pal).getByRole('button')));
        expect(window.api.noticeRemove).toHaveBeenCalledWith(2);
        expect(screen.queryByText('Start failed')).toBeNull();
        await act(async () => fireEvent.click(screen.getByText(i18n.t('gui:notice_modal.clear_all'))));
        expect(window.api.noticesClear).toHaveBeenCalled();
    });

    it('업데이트 결과 알림을 누르면 데몬의 마지막 결과로 패치노트를 연다', async () => {
        const result = { success: true, updated_components: ['gui'], failed: [] };
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ success: true, version: '0.2.0', notes: '- 새 기능', result }) as never;
        const onOpen = vi.fn();
        await act(async () => {
            render(<NoticeModal isOpen onClose={vi.fn()} onOpenPatchNotes={onOpen} />);
        });
        await act(async () => fireEvent.click(screen.getByText('Updated to Saba-chan 0.2.0')));
        expect(window.api.updaterLastResult).toHaveBeenCalledWith({ includeSeen: true });
        expect(onOpen).toHaveBeenCalledWith({ version: '0.2.0', notes: '- 새 기능', result });
    });

    it('목록은 화면 언어로 달라고 한다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { success: true, notices: [] } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn(), on: vi.fn() } as never });
        await act(async () => i18n.changeLanguage('ko'));
        await api.noticesList();
        expect(JSON.stringify(invoke.mock.calls)).toContain('/api/notices?locale=ko');
        await act(async () => i18n.changeLanguage('en'));
    });
});
