/**
 * 콘솔을 별도 창으로 열지 못하면(셸이 {ok:false}로 답하거나 예외) 조용히 넘기지 않고 알린다 (G-C5).
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));
vi.mock('../components/servers/ServerConsole', () => ({
    ServerConsole: ({ onPopout }) => (
        <button type="button" onClick={onPopout}>
            popout
        </button>
    ),
}));

import { ServerDetail } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';

const server = { id: 'p', name: 'Pal', module: 'pal', status: 'running', start_time: 1, managed: true };

const detail = () =>
    render(
        <ServerDetail
            server={server}
            moduleData={{ name: 'pal', protocols: { supported: ['stdin'] } }}
            modules={[]}
            servers={[server]}
            setServers={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            setModal={() => {}}
            consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] }}
            settingsProps={{}}
        />,
    );

const failed = () => i18n.t('gui:console.popout_failed');

beforeEach(() => {
    window.showToast = vi.fn();
    useServerStore.setState({ modulesLoaded: true });
});

describe('콘솔 별도 창 열기 실패', () => {
    it('셸이 ok:false로 답하면 알린다', async () => {
        window.api.consolePopout = vi.fn().mockResolvedValue({ ok: false, error: 'window label taken' });
        detail();
        await act(async () => fireEvent.click(screen.getByText('popout')));
        expect(window.api.consolePopout).toHaveBeenCalledWith('p', 'Pal');
        expect(failed()).not.toBe('console.popout_failed');
        expect(window.showToast).toHaveBeenCalledWith(failed(), 'error', undefined, undefined);
    });

    it('부르다 예외가 나도 알린다', async () => {
        window.api.consolePopout = vi.fn().mockRejectedValue(new Error('ipc closed'));
        detail();
        await act(async () => fireEvent.click(screen.getByText('popout')));
        expect(window.showToast).toHaveBeenCalledWith(failed(), 'error', undefined, undefined);
    });
});
