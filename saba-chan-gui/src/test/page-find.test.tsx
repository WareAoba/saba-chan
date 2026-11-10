import { type ComponentProps, useRef, useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { InstanceDiscordTab } from '../components/discord/InstanceWordsTab';
import { PageFind } from '../components/servers/PageFind';
import type { Server } from '../stores/useServerStore';

// 서버 상세 설정·사바쨩·디스코드 탭의 Ctrl+F 찾기 (사용자 11-10 시안 승인):
// 오른쪽 위 작은 찾기 창, 지금 탭의 칸 이름·입력된 값·원문 키에서 찾아 그 자리에서 칠한다
const modules = [
    {
        name: 'mc',
        settings: {
            fields: [
                { name: 'motd', label: 'MOTD', field_type: 'text' },
                { name: 'level_name', label: 'World name', field_type: 'text' },
                { name: 'view_distance', label: 'View distance', field_type: 'number', group: 'advanced' },
            ],
        },
    },
];
const server = { id: 's1', name: 'S', module: 'mc', status: 'stopped' };

function Harness({ enabled = true }: { enabled?: boolean }) {
    const ref = useRef<HTMLDivElement>(null);
    const [advanced, setAdvanced] = useState(false);
    const props = {
        section: 'server',
        embedded: true,
        settingsServer: server,
        settingsValues: { motd: 'Hello world', level_name: 'wild' },
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        advancedExpanded: advanced,
        setAdvancedExpanded: setAdvanced,
        availableVersions: [],
        handleSettingChange: vi.fn(),
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        onClose: vi.fn(),
        servers: [server],
    } as unknown as ComponentProps<typeof ServerSettingsModal>;
    return (
        <>
            <PageFind rootRef={ref} enabled={enabled} pageKey="settings" />
            <div ref={ref}>
                <ServerSettingsModal {...props} />
            </div>
        </>
    );
}

const ctrlF = () => fireEvent.keyDown(window, { key: 'f', ctrlKey: true });
const box = () => screen.getByRole('search');
const count = () => box().querySelector('[aria-live]')?.textContent;
const type = (q: string) => fireEvent.change(screen.getByRole('searchbox'), { target: { value: q } });
const settle = () => act(() => new Promise((r) => setTimeout(r, 30)));

describe('설정 찾기 (Ctrl+F)', () => {
    it('Ctrl+F로 오른쪽 위 찾기 창을 열고, 칸 이름과 입력된 값을 함께 센다', async () => {
        render(<Harness />);
        expect(screen.queryByRole('search')).toBeNull();
        const ev = ctrlF();
        expect(ev).toBe(false); // 웹뷰 기본 찾기 대신 앱 찾기
        type('world');
        await settle();
        // "World name" 이름 1 + MOTD 칸의 "Hello world" 1
        expect(count()).toBe('1/2');
    });

    it('Enter·Shift+Enter로 다음·이전 결과로 간다', async () => {
        render(<Harness />);
        ctrlF();
        type('world');
        await settle();
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter' });
        expect(count()).toBe('2/2');
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter' });
        expect(count()).toBe('1/2');
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter', shiftKey: true });
        expect(count()).toBe('2/2');
    });

    it('글자를 칠 때 설정 화면을 다시 만들지 않는다 (입력 칸이 그대로 남는다)', async () => {
        render(<Harness />);
        const motd = screen.getByDisplayValue('Hello world');
        ctrlF();
        type('wor');
        await settle();
        type('world');
        await settle();
        expect(motd.isConnected).toBe(true);
        expect(screen.getByDisplayValue('Hello world')).toBe(motd);
    });

    it('원문 키로 찾으면 그 줄에 키 꼬리표가 뜬다 (- _ 공백은 같게 본다)', async () => {
        render(<Harness />);
        expect(screen.queryByText('level_name')).toBeNull();
        ctrlF();
        type('level-na');
        await settle();
        expect(screen.getByText('level_name')).toBeTruthy();
        expect(count()).toBe('1/1');
    });

    it('접힌 고급 설정 안의 결과는 머리에 개수를 띄우고, 그 결과로 가면 펼친다', async () => {
        render(<Harness />);
        const toggle = document.querySelector('.ss-card-toggle') as HTMLElement;
        expect(toggle.getAttribute('aria-expanded')).toBe('false');
        ctrlF();
        type('view distance');
        await settle();
        expect(count()).toBe('1/1');
        expect(toggle.getAttribute('data-find-badge')).toBe('Matches: 1');
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter' });
        await settle();
        expect(toggle.getAttribute('aria-expanded')).toBe('true');
    });

    it('찾는 것이 없으면 "결과 없음"', async () => {
        render(<Harness />);
        ctrlF();
        type('zzzz');
        await settle();
        expect(count()).toBe('No results');
    });

    it('Esc로 닫으면 칠한 것을 지우고 마지막 결과의 입력 칸에 커서를 둔다', async () => {
        render(<Harness />);
        ctrlF();
        type('hello');
        await settle();
        fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
        expect(screen.queryByRole('search')).toBeNull();
        expect(document.activeElement).toBe(screen.getByDisplayValue('Hello world'));
    });

    it('찾기를 쓰지 않는 탭에서는 Ctrl+F를 가로채지 않는다', () => {
        render(<Harness enabled={false} />);
        expect(ctrlF()).toBe(true);
        expect(screen.queryByRole('search')).toBeNull();
    });
});

describe('디스코드 탭 찾기', () => {
    function DiscordHarness() {
        const ref = useRef<HTMLDivElement>(null);
        return (
            <>
                <PageFind rootRef={ref} enabled pageKey="discord" />
                <div ref={ref}>
                    <InstanceDiscordTab server={{ id: 's1', name: '야생', module: 'minecraft' } as Server} />
                </div>
            </>
        );
    }

    it('명령 이름·부르는 말 칩·명령 원문을 찾는다', async () => {
        window.api = {
            instanceWords: vi.fn().mockResolvedValue({
                id: 's1',
                name: '야생',
                module: 'minecraft',
                words: ['크리'],
                commands: [
                    {
                        name: 'kick',
                        label: 'Kick',
                        risk: 'caution',
                        builtin: false,
                        moduleAliases: [],
                        aliases: ['추방'],
                    },
                    {
                        name: 'save-all',
                        label: 'World save',
                        risk: 'safe',
                        builtin: false,
                        moduleAliases: [],
                        aliases: [],
                    },
                ],
            }),
        } as unknown as typeof window.api;
        await act(async () => render(<DiscordHarness />));
        await screen.findByText('Kick');
        ctrlF();
        type('kick');
        await settle();
        expect(count()).toBe('1/1');
        type('추방');
        await settle();
        expect(count()).toBe('1/1');
        expect(screen.queryByText('save-all')).toBeNull();
        type('save_a');
        await settle();
        expect(screen.getByText('save-all')).toBeTruthy();
        expect(count()).toBe('1/1');
    });
});
