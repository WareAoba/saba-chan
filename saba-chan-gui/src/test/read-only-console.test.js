/**
 * 콘솔 입력을 받지 않는 서버(REST 전용)는 콘솔 출력만 보이고 입력칸은 닫혀 있다.
 */
import { render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { describe, expect, it } from 'vitest';
import { ServerConsole } from '../components/servers/ServerConsole';

const server = { id: 'p', name: 'Pal', status: 'running' };
const state = { lines: [{ id: 0, content: '[LogInit] started', level: 'info', _key: 'k0' }], input: '' };

const consoleOf = (props) =>
    render(
        <ServerConsole
            server={server}
            state={state}
            quickCommands={[]}
            openConsole={() => {}}
            setConsoleInput={() => {}}
            sendConsoleCommand={() => {}}
            {...props}
        />,
    );

describe('읽기 전용 콘솔', () => {
    it('출력은 보이고 입력칸은 막히며 명령 탭을 안내한다', () => {
        consoleOf({ readOnly: true });
        expect(screen.getByText('[LogInit] started')).toBeTruthy();
        const input = screen.getByRole('textbox');
        expect(input.disabled).toBe(true);
        expect(i18n.t('gui:shell.console_read_only')).not.toBe('shell.console_read_only');
        expect(input.getAttribute('placeholder')).toBe(i18n.t('gui:shell.console_read_only'));
    });

    it('입력을 받는 서버는 그대로 입력칸이 열린다', () => {
        consoleOf({});
        expect(screen.getByRole('textbox').disabled).toBe(false);
    });
});
