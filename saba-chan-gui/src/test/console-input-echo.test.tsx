/**
 * #18: 콘솔에 보낸 명령이 콘솔에 남지 않았고, 명령 탭의 실행 기록에도 콘솔·디스코드에서 보낸 명령이 없었다
 */
import { act, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServerConsole } from '../components/servers/ServerConsole';
import { DaemonEvents } from '../components/shell/DaemonEvents';
import '../i18n';
import type { Server } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';
import type { ConsoleEntry } from '../stores/useConsoleStore';

const consoleOf = (lines: object[]) =>
    render(
        <ServerConsole
            server={{ id: 'm', name: 'MC', status: 'running' } as Server}
            state={{ lines, input: '' } as ConsoleEntry}
            openConsole={() => {}}
            setConsoleInput={() => {}}
            sendConsoleCommand={() => {}}
        />,
    );

describe('보낸 명령', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).onDaemonEvent;
        useUIStore.setState({ commandHistoryMap: {} });
    });

    it('콘솔에 › 명령과 보낸 곳을 다른 색으로 남긴다', () => {
        consoleOf([
            { id: 1, source: 'input', content: 'list', by: { source: 'gui' } },
            { id: 2, source: 'stdout', content: 'There are 0 players' },
            { id: 3, source: 'input', content: 'say hi', by: { source: 'discord', userName: 'jace' } },
        ]);
        const echoes = [...document.querySelectorAll('.console-input')].map((el) => el.textContent);
        expect(echoes).toEqual(['› list · App', '› say hi · Discord · jace']);
        expect(document.querySelector('.console-input .console-echo-by')?.textContent).toBe(' · App');
    });

    it('콘솔·다른 곳에서 보낸 명령도 실행 기록에 들어간다 (앱 명령 창은 스스로 남긴다)', async () => {
        const subs = new Set<(ev: unknown) => void>();
        window.api.onDaemonEvent = vi.fn((cb) => {
            subs.add(cb);
            return () => subs.delete(cb);
        }) as never;
        render(<DaemonEvents onServersChanged={() => {}} />);
        const emit = (data: object) =>
            act(async () => {
                for (const cb of subs) cb({ type: 'action.run', data: { instanceId: 'srv-1', instance: 'MC', ok: true, status: 200, ...data } });
            });
        await emit({ source: 'gui', action: 'stdin', command: 'list' });
        await emit({ source: 'discord', userName: 'Kimchi', action: 'command', command: 'say', args: '{"message":"hi"}' });
        await emit({ source: 'gui', action: 'command', command: 'save-all', args: '{}' });
        await emit({ source: 'web', action: 'stdin', command: 'kick Steve', ok: false, status: 503, code: 'console_input_full' });
        const history = useUIStore.getState().commandHistoryMap['srv-1'];
        expect(history.map((e) => [e.command, e.via, e.status])).toEqual([
            ['list', 'Console', 'success'],
            ['say “hi”', 'Discord · Kimchi', 'success'],
            ['kick Steve', 'Console · Saba-chan web', 'failure'],
        ]);
        expect(history[2].detail).toBe('The server is still processing earlier input. Try again in a moment.');
    });
});
