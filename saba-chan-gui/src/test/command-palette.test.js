import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandPalette, filterActions, inputFields } from '../components/shell/CommandPalette';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const API = ['actionsList', 'actionsPlan', 'actionsRun'];
const actions = [
    { id: 'instance.list', kind: 'query', risk: 'safe', scope: 'none', title: 'List servers' },
    { id: 'server.restart', kind: 'command', risk: 'caution', scope: 'instance', title: 'Restart server' },
    {
        id: 'server.command:say',
        kind: 'command',
        risk: 'caution',
        scope: 'instance',
        title: 'Broadcast',
        input: { type: 'object', properties: { message: { type: 'string', title: 'Message' } }, required: ['message'] },
    },
    { id: 'instance.delete', kind: 'admin', risk: 'danger', scope: 'instance', title: 'Delete server' },
];
const flush = () => act(async () => {});
const open = async () => {
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    await flush();
};

describe('명령 팔레트 계산', () => {
    it('검색어 조각이 제목·id에 모두 있는 것만 남긴다', () => {
        expect(filterActions(actions, 'rest serv').map((a) => a.id)).toEqual(['server.restart']);
        expect(filterActions(actions, 'command:say').map((a) => a.id)).toEqual(['server.command:say']);
        expect(filterActions(actions, '  ')).toHaveLength(4);
    });

    it('입력 스키마에서 필드와 필수 여부를 읽는다', () => {
        expect(inputFields(actions[2].input)).toEqual([
            expect.objectContaining({ name: 'message', title: 'Message', required: true }),
        ]);
        expect(inputFields(undefined)).toEqual([]);
    });
});

describe('명령 팔레트', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        useServerStore.setState({ servers: [{ id: 'mc-1', name: 'Survival', module: 'minecraft' }] });
        useUIStore.setState({ activeServerId: 'mc-1' });
        window.api.actionsList = vi.fn().mockResolvedValue({ actions, locale: 'en' });
        window.api.actionsPlan = vi.fn(async (id) => ({
            planId: `p-${id}`,
            needsConfirm: id === 'instance.delete',
            summary: [{ labelKey: 'instance', label: 'Server', value: 'Survival' }],
        }));
        window.api.actionsRun = vi.fn(async (id) => ({
            success: true,
            result: id === 'instance.list' ? { message: '2 servers' } : {},
        }));
    });
    afterEach(() => {
        for (const k of API) delete window.api[k];
        window.showToast = undefined;
    });

    it('Ctrl+K로 열면 지금 서버의 액션을 불러오고, 고르면 plan 뒤 바로 실행한다', async () => {
        render(<CommandPalette />);
        await open();
        expect(window.api.actionsList).toHaveBeenCalledWith({ surface: 'gui', instance: 'mc-1', locale: 'en' });
        const input = screen.getByRole('combobox');
        fireEvent.change(input, { target: { value: 'restart' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        await flush();
        expect(window.api.actionsPlan).toHaveBeenCalledWith('server.restart', { input: {}, instance: 'mc-1' });
        expect(window.api.actionsRun).toHaveBeenCalledWith('server.restart', { planId: 'p-server.restart' });
        expect(window.showToast).toHaveBeenCalledWith('Done: Restart server', 'success', 3000, undefined);
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('실행 중에는 Esc·Ctrl+K·바깥 클릭으로 닫히지 않고, 끝나면 결과를 알린다', async () => {
        let resolveRun;
        window.api.actionsRun = vi.fn(() => new Promise((r) => (resolveRun = r)));
        render(<CommandPalette />);
        await open();
        const input = screen.getByRole('combobox');
        fireEvent.change(input, { target: { value: 'restart' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        await flush();
        expect(window.api.actionsRun).toHaveBeenCalled();
        fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
        fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
        fireEvent.mouseDown(document.querySelector('.palette-overlay'));
        expect(screen.getByRole('dialog')).toBeTruthy();
        await act(async () => resolveRun({ success: false, error: 'boom' }));
        expect(screen.getByRole('dialog').textContent).toContain('boom');
    });

    it('확인이 필요한 액션은 요약을 보여 주고 한 번 더 눌러야 실행한다', async () => {
        render(<CommandPalette />);
        await open();
        fireEvent.click(screen.getByText('Delete server'));
        await flush();
        expect(window.api.actionsRun).not.toHaveBeenCalled();
        expect(screen.getByText('Run Delete server?')).toBeInTheDocument();
        expect(screen.getByText('Survival')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Run' }));
        await flush();
        expect(window.api.actionsRun).toHaveBeenCalledWith('instance.delete', { planId: 'p-instance.delete' });
    });

    it('입력이 있는 액션은 칸을 채운 뒤 실행하고, 조회 결과는 팔레트에 보인다', async () => {
        render(<CommandPalette />);
        await open();
        fireEvent.click(screen.getByText('Broadcast'));
        const next = screen.getByRole('button', { name: 'Next' });
        expect(next).toBeDisabled();
        fireEvent.change(screen.getByLabelText(/Message/), { target: { value: 'hi' } });
        fireEvent.click(next);
        await flush();
        expect(window.api.actionsPlan).toHaveBeenCalledWith('server.command:say', {
            input: { message: 'hi' },
            instance: 'mc-1',
        });

        await open(); // 닫혔으니 다시 연다
        fireEvent.click(screen.getByText('List servers'));
        await flush();
        expect(window.api.actionsPlan).toHaveBeenLastCalledWith('instance.list', { input: {}, instance: undefined });
        expect(screen.getByText('2 servers')).toBeInTheDocument();
    });

    it('데몬이 거절하면 이유를 팔레트에 보이고 닫지 않는다', async () => {
        window.api.actionsPlan = vi
            .fn()
            .mockResolvedValue({ error_code: 'invalid_input', field: 'message', reason: 'too long' });
        render(<CommandPalette />);
        await open();
        fireEvent.click(screen.getByText('Restart server'));
        await flush();
        expect(screen.getByText('Check message: too long')).toBeInTheDocument();
        expect(window.api.actionsRun).not.toHaveBeenCalled();
    });
});
