import { render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import NoticeModal from '../components/Modals/NoticeModal';
import '../i18n';
import { useServerStore } from '../stores/useServerStore';

// #14: 알림은 서버마다 한 묶음, 한 줄은 짧게 + 이유 한 줄
describe('알림 모음', () => {
    beforeEach(() => {
        localStorage.clear();
        useServerStore.setState({ servers: [{ id: 'mc', name: 'Our MC', module: 'minecraft', status: 'running' }] as never });
    });

    it('서버별로 묶고, 이름은 지금 이름을 쓰며, 이유를 둘째 줄로 보인다', () => {
        const add = window.__sabaNotice!.addNotice;
        add({ message: 'Stopped · App', source: 'Old MC name', instanceId: 'mc' });
        add({ message: 'Failed to start', detail: 'Ended 4s after starting', type: 'error', source: 'Pal', instanceId: 'pal' });
        add({ message: 'Crashed', detail: 'Hit the memory limit (4096 MiB)', type: 'error', source: 'Our MC', instanceId: 'mc' });
        add({ message: '📦 1 update(s) found: gui', source: 'Updater', action: 'openUpdateModal' });
        render(<NoticeModal isOpen onClose={() => {}} />);
        const groups = document.querySelectorAll('.notice-group');
        expect([...groups].map((g) => g.querySelector('.notice-group-label')?.textContent)).toEqual(['Updater', 'Our MC', 'Pal']);
        const mc = groups[1] as HTMLElement;
        expect(within(mc).getByText('2')).toBeInTheDocument();
        expect(within(mc).getByText('Crashed')).toBeInTheDocument();
        expect(within(mc).getByText('Hit the memory limit (4096 MiB)')).toBeInTheDocument();
        expect(within(mc).getByText('Stopped · App')).toBeInTheDocument();
        expect(screen.queryByText('Old MC name')).toBeNull();
    });
});
