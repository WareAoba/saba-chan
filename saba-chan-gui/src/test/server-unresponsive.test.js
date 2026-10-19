import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { ServerDetail } from '../components/servers/ServerDetail';
import { ServerRow } from '../components/servers/ServerRow';
import { useServerStore } from '../stores/useServerStore';

const base = { id: 'p', name: 'Pal', module: 'pal', status: 'running', start_time: 1 };

const row = (server) =>
    render(
        <ServerRow
            server={server}
            index={0}
            moduleData={{ name: 'pal', game_name: 'Palworld' }}
            onSelect={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            onPointerDown={() => {}}
            skipNextClick={{ current: false }}
        />,
    );

const detail = (server) =>
    render(
        <ServerDetail
            server={server}
            moduleData={{ name: 'pal' }}
            modules={[]}
            servers={[server]}
            setServers={() => {}}
            onStart={() => {}}
            onStop={() => {}}
            setModal={() => {}}
            consoleProps={{ consoles: {}, popoutOpen: {} }}
            settingsProps={{}}
        />,
    );

describe('실행 중이지만 응답이 없는 서버', () => {
    it('상세 화면에 경고와 모듈이 알린 이유를 보인다', () => {
        useServerStore.setState({ modulesLoaded: true });
        detail({ ...base, unresponsive: 'connection refused' });
        expect(screen.getByText(/running but not responding.*connection refused/i)).toBeTruthy();
    });

    it('목록 한 줄에도 경고 표시를 두고, 이유는 툴팁에 담는다', () => {
        useServerStore.setState({ modulesLoaded: true });
        row({ ...base, unresponsive: 'connection refused' });
        const flag = document.querySelector('.server-row-flag.tone-warn');
        expect(flag).not.toBeNull();
        expect(flag.getAttribute('title')).toMatch(/connection refused/);
    });

    it('필드가 없으면(옛 데몬이거나 답하는 중) 아무것도 보이지 않는다', () => {
        useServerStore.setState({ modulesLoaded: true });
        detail(base);
        row(base);
        expect(screen.queryByText(/not responding/i)).toBeNull();
        expect(document.querySelector('.server-row-flag.tone-warn')).toBeNull();
    });
});
