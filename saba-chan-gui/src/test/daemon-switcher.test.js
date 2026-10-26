import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DaemonSwitcher, REMOTE_DAEMONS_CHANGED } from '../components/shell/DaemonSwitcher';
import { pickOption } from './dropdown';

describe('상단 바: 다룰 데몬 고르기 (이 PC / 연결한 원격 데몬)', () => {
    let reload;
    beforeEach(() => {
        reload = vi.fn();
        window.api = {
            remoteDaemons: vi.fn().mockResolvedValue({ daemons: [{ hostId: 'host-b', name: '게임 PC', address: 'g.example', port: 57480 }] }),
            daemonTarget: vi.fn().mockResolvedValue({ target: null }),
            daemonTargetSet: vi.fn().mockResolvedValue({ success: true, target: 'host-b' }),
        };
    });

    it('연결한 원격 데몬이 없으면 보이지 않는다', async () => {
        window.api.remoteDaemons.mockResolvedValue({ daemons: [] });
        const { container } = await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        expect(container).toBeEmptyDOMElement();
    });

    it('원격 데몬을 고르면 셸에 대상을 알리고 화면을 다시 읽는다', async () => {
        await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        const select = screen.getByRole('combobox', { name: 'Daemon' });
        expect(select).toHaveTextContent('This PC');
        pickOption(select, '게임 PC');
        await act(async () => {});
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith('host-b');
        expect(reload).toHaveBeenCalled();
    });

    it('고른 원격 데몬을 보이고, 이 PC로 돌아갈 수 있다', async () => {
        window.api.daemonTarget.mockResolvedValue({ target: 'host-b' });
        window.api.daemonTargetSet.mockResolvedValue({ success: true, target: null });
        await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        const select = screen.getByRole('combobox', { name: 'Daemon' });
        expect(select).toHaveTextContent('게임 PC');
        pickOption(select, 'This PC');
        await act(async () => {});
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
    });

    it('잊은 원격 데몬이 남아 있으면 이 PC로 되돌린다', async () => {
        window.api.daemonTarget.mockResolvedValue({ target: 'host-gone' });
        window.api.daemonTargetSet.mockResolvedValue({ success: true, target: null });
        await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        expect(window.api.daemonTargetSet).toHaveBeenCalledWith(null);
        expect(reload).toHaveBeenCalled();
    });

    it('이 PC로 되돌리기를 저장하지 못하면 다시 읽지 않고 고르는 칸을 남긴다 (되풀이 새로고침이 없게)', async () => {
        window.api.daemonTarget.mockResolvedValue({ target: 'host-gone' });
        window.api.daemonTargetSet.mockResolvedValue({ success: false, target: 'host-gone' });
        await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        expect(reload).not.toHaveBeenCalled();
        expect(screen.getByRole('combobox', { name: 'Daemon' })).toBeInTheDocument();
    });

    it('설정에서 원격 데몬을 연결하면 다시 켜지 않아도 나타난다', async () => {
        window.api.remoteDaemons.mockResolvedValueOnce({ daemons: [] });
        const { container } = await act(async () => render(<DaemonSwitcher onSwitched={reload} />));
        expect(container).toBeEmptyDOMElement();
        await act(async () => window.dispatchEvent(new Event(REMOTE_DAEMONS_CHANGED)));
        expect(screen.getByRole('combobox', { name: 'Daemon' })).toBeInTheDocument();
    });
});
