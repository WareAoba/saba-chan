import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandPalette } from '../components/shell/CommandPalette';
import { useDiscordStore } from '../stores/useDiscordStore';
import type { Server } from '../stores/useServerStore';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

const API = ['actionsPlan', 'actionsRun', 'executeCommand', 'instanceCommands'];
const say = {
    name: 'say',
    label: 'Broadcast',
    risk: 'safe',
    source: 'module',
    ready: true,
    inputs: [{ name: 'message', label: 'Message', type: 'text', required: true }],
};
const kick = { name: 'kick', label: 'Kick player', risk: 'danger', source: 'module', ready: true, inputs: [] };
const flush = () => act(async () => {});
const wait = (ms: number) => act(() => new Promise((r) => setTimeout(r, ms)));
const open = async () => {
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    await flush();
};
const input = () => screen.getByRole('combobox');
const type = (text: string) => fireEvent.change(input(), { target: { value: text } });
const press = (key: string) => fireEvent.keyDown(input(), { key });
const option = (name: RegExp) => screen.getByRole('option', { name });

describe('명령 팔레트', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        useServerStore.setState({
            servers: [
                { id: 'mc-1', name: 'Survival', module: 'minecraft', status: 'running' },
                { id: 'pw-1', name: 'Palworld', module: 'palworld', status: 'stopped' },
            ] as Server[],
            modules: [],
        });
        useDiscordStore.setState({ cloudNodes: [{ guildId: 'g1', guildName: 'Mayo guild' }] });
        useUIStore.setState({ activeTab: 'servers', activeServerId: 'mc-1', openDiscordNodeId: null });
        window.api.instanceCommands = vi.fn(async (id: string) => ({
            commands: id === 'mc-1' ? [say, kick] : [{ ...say, ready: false }],
        })) as never;
        window.api.actionsPlan = vi.fn(async (id: string) => ({
            planId: `p-${id}`,
            needsConfirm: id === 'server.stop' || id === 'server.restart',
            summary: [],
        })) as never;
        window.api.actionsRun = vi.fn(async () => ({ success: true })) as never;
        window.api.executeCommand = vi.fn(async () => ({ success: true, message: 'ok' })) as never;
    });
    afterEach(() => {
        for (const k of API) delete (window.api as Partial<typeof window.api>)[k as keyof typeof window.api];
        window.showToast = undefined;
    });

    it('빈 검색: 지금 서버의 조작·보낼 수 있는 명령·바로 가기를 묶어 보인다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        expect(option(/^Stop/)).toBeInTheDocument();
        expect(option(/^Broadcast/)).toBeInTheDocument();
        expect(option(/^Settings/)).toBeInTheDocument();
        expect(screen.queryByRole('option', { name: /^Start/ })).toBeNull();
        // 카테고리 버튼: 조작 → 게임 명령 → 바로 가기
        const cats = screen.getAllByRole('button', { pressed: false }).map((b) => b.getAttribute('aria-label'));
        expect(cats).toEqual(['Controls', 'Game commands', 'Go to']);
    });

    it('꺼진 서버를 범위로 고르면 시작만 있고 명령 카테고리 버튼이 사라진다. ⌫로 칩을 지운다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        type('palw');
        fireEvent.click(option(/^Palworld/));
        await flush();
        expect(screen.getByRole('button', { name: /Palworld/ })).toBeInTheDocument(); // 칩
        expect(option(/^Start/)).toBeInTheDocument();
        expect(screen.queryByRole('option', { name: /^Stop/ })).toBeNull();
        expect(screen.queryByRole('button', { name: 'Game commands' })).toBeNull();
        press('Backspace');
        await wait(200);
        expect(screen.queryByRole('button', { name: /Palworld/ })).toBeNull();
        expect(option(/^Stop/)).toBeInTheDocument();
    });

    it('검색하면 같은 명령을 보낼 수 있는 다른 서버도 나온다 (꺼진 서버는 빠진다)', async () => {
        useUIStore.setState({ activeServerId: 'pw-1' });
        render(<CommandPalette />);
        await open();
        await flush();
        type('broad');
        const rows = screen.getAllByRole('option');
        expect(rows).toHaveLength(1);
        expect(within(rows[0]).getByText('Survival · On')).toBeInTheDocument();
        // 고르면 그 서버로 범위를 바꾸고 칸 입력을 이어간다
        press('Enter');
        await flush();
        expect(screen.getByRole('button', { name: /Survival/ })).toBeInTheDocument();
        expect(input()).toHaveAttribute('placeholder', 'Message');
    });

    it('명령은 칸을 하나씩 받고 확인 뒤 보낸다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        type('broad');
        press('Enter');
        expect(input()).toHaveAttribute('placeholder', 'Message');
        press('Enter'); // 필수 칸이 비면 넘어가지 않는다
        expect(input()).toHaveAttribute('placeholder', 'Message');
        type('hello');
        press('Enter');
        expect(screen.getByText('Send this command to Survival')).toBeInTheDocument();
        press('Enter');
        await flush();
        expect(window.api.executeCommand).toHaveBeenCalledWith('mc-1', { command: 'say', args: { message: 'hello' } });
        expect(screen.getByText('Sent')).toBeInTheDocument();
    });

    it('위험한 명령은 한 번 더 눌러야 보낸다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        type('kick');
        press('Enter');
        press('Enter');
        await flush();
        expect(window.api.executeCommand).not.toHaveBeenCalled();
        expect(screen.getByRole('button', { name: 'Press again to send' })).toBeInTheDocument();
        press('Enter');
        await flush();
        expect(window.api.executeCommand).toHaveBeenCalledWith('mc-1', { command: 'kick', args: {} });
    });

    it('조작은 데몬 액션으로 하고, 데몬이 되묻는 정지는 확인 뒤 실행한다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        type('backup');
        press('Enter');
        await flush();
        expect(window.api.actionsPlan).toHaveBeenCalledWith('backup.create', { input: {}, instance: 'mc-1' });
        expect(window.api.actionsRun).toHaveBeenCalledWith('backup.create', { planId: 'p-backup.create' });
        expect(screen.queryByRole('dialog')).toBeNull();

        await open();
        await flush();
        type('stop');
        press('Enter');
        await flush();
        expect(window.api.actionsRun).toHaveBeenCalledTimes(1);
        press('Enter');
        await flush();
        expect(window.api.actionsRun).toHaveBeenLastCalledWith('server.stop', { planId: 'p-server.stop' });
    });

    it('실행 중에는 Esc·Ctrl+K·바깥 클릭으로 닫히지 않고, 실패하면 이유를 보인다', async () => {
        let resolveRun!: (v: unknown) => void;
        window.api.actionsRun = vi.fn(() => new Promise((r) => (resolveRun = r))) as never;
        render(<CommandPalette />);
        await open();
        await flush();
        type('backup');
        press('Enter');
        await flush();
        expect(window.api.actionsRun).toHaveBeenCalled();
        fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
        fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
        fireEvent.mouseDown(document.querySelector('.palette-overlay') as HTMLElement);
        expect(screen.getByRole('dialog')).toBeTruthy();
        await act(async () => resolveRun({ success: false, error: 'boom' }));
        expect(screen.getByRole('dialog').textContent).toContain('boom');
    });

    it('화살표로 줄을 옮겨 고르고, Ctrl+숫자로 카테고리를 켜고 끈다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        const first = screen.getAllByRole('option')[0];
        expect(first).toHaveAttribute('aria-selected', 'true');
        press('ArrowDown');
        expect(screen.getAllByRole('option')[1]).toHaveAttribute('aria-selected', 'true');
        press('ArrowUp');
        press('ArrowUp'); // 맨 위에서 더 올라가지 않는다
        expect(screen.getAllByRole('option')[0]).toHaveAttribute('aria-selected', 'true');
        fireEvent.keyDown(input(), { key: '2', ctrlKey: true });
        expect(screen.getByRole('button', { name: 'Game commands' })).toHaveAttribute('aria-pressed', 'true');
        expect(screen.queryByRole('option', { name: /^Stop/ })).toBeNull();
        expect(option(/^Broadcast/)).toBeInTheDocument();
        fireEvent.keyDown(input(), { key: '2', ctrlKey: true });
        expect(screen.getByRole('button', { name: 'Game commands' })).toHaveAttribute('aria-pressed', 'false');
        expect(option(/^Stop/)).toBeInTheDocument();
    });

    it('Tab은 범위 고르기 목록을, Esc는 한 단계 뒤로 가고 목록에서는 닫는다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        press('Tab');
        expect(input()).toHaveAttribute('placeholder', 'Select scope');
        expect(option(/^Palworld/)).toBeInTheDocument();
        expect(option(/Mayo guild/)).toBeInTheDocument();
        press('Escape');
        expect(input()).toHaveAttribute('placeholder', 'What do you want to do? Server names also work');
        type('broad');
        press('Enter');
        expect(input()).toHaveAttribute('placeholder', 'Message');
        press('Escape');
        expect(input()).toHaveAttribute('placeholder', 'What do you want to do? Server names also work');
        expect(input()).toHaveValue('');
        press('Escape');
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('확인 단계의 ⌫는 마지막 칸으로 돌아가 다시 받는다', async () => {
        render(<CommandPalette />);
        await open();
        await flush();
        type('broad');
        press('Enter');
        type('hello');
        press('Enter');
        expect(screen.getByText('Send this command to Survival')).toBeInTheDocument();
        press('Backspace');
        expect(screen.queryByText('Send this command to Survival')).toBeNull();
        expect(input()).toHaveAttribute('placeholder', 'Message');
        type('bye');
        press('Enter');
        press('Enter');
        await flush();
        expect(window.api.executeCommand).toHaveBeenCalledWith('mc-1', { command: 'say', args: { message: 'bye' } });
    });

    it('바로 가기는 실행하지 않고 그 화면을 연다 (디스코드 서버 탭은 실제 메뉴 이름)', async () => {
        useUIStore.setState({ activeTab: 'discord', activeServerId: null, openDiscordNodeId: 'g1' });
        render(<CommandPalette />);
        await open();
        await flush();
        expect(option(/^Instances/)).toBeInTheDocument();
        fireEvent.click(option(/^Members/));
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(useUIStore.getState().requestedDiscordView).toMatchObject({ kind: 'node', id: 'g1', tab: 'members' });
    });
});
