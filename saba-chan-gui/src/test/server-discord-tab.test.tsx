import type { ComponentProps, ComponentType } from 'react';
/** 서버마다 디스코드에서 부르는 말은 서버 상세의 디스코드 탭에서 고친다 (디스코드 화면에는 없다) */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail as ServerDetailFull } from '../components/servers/ServerDetail';
import { useServerStore } from '../stores/useServerStore';

// 이 시험이 보는 props만 넘기므로 나머지는 선택으로 느슨하게 받는다
const ServerDetail = ServerDetailFull as unknown as ComponentType<Partial<ComponentProps<typeof ServerDetailFull>>>;

const server = { id: 's1', name: '야생', module: 'minecraft', status: 'stopped' };

const detail = () =>
    render(
        <ServerDetail
            server={server as never}
            moduleData={{ name: 'minecraft' } as never}
            modules={[]}
            servers={[server] as never}
            setServers={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            setModal={() => {}}
            consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] } as never}
            settingsProps={{} as never}
        />,
    );

describe('서버 상세의 디스코드 탭', () => {
    afterEach(() => {
        window.api = undefined as never;
    });

    it('이 서버의 부르는 말을 보인다', async () => {
        useServerStore.setState({ modulesLoaded: true });
        window.api = {
            instanceWords: vi
                .fn()
                .mockResolvedValue({ id: 's1', name: '야생', module: 'minecraft', words: [], commands: [] }),
            instanceWordsSave: vi.fn(),
        } as unknown as typeof window.api;
        detail();
        await act(async () => fireEvent.click(screen.getByText('Discord')));
        expect(await screen.findByLabelText('Names for 야생')).toBeTruthy();
        expect(window.api.instanceWords).toHaveBeenCalledWith('s1');
    });

    it('데몬이 낱말 API를 모르면 탭을 두지 않는다', () => {
        useServerStore.setState({ modulesLoaded: true });
        window.api = {} as unknown as typeof window.api;
        detail();
        expect(screen.queryByText('Discord')).toBeNull();
    });
});
