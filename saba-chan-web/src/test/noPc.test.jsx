import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// PC 없는 계정 세션 (hostId null): 디스코드 탭만 열리고, 디스코드 설정은 릴레이 API(/relay/*)로 바꾼다
const G = '111111111111111111';
const CH = '222222222222222222';
const U = '333333333333333333';
const ALL = { manage: true, channels: true, messages: true, members: true };

const settings = {
    locale: 'auto',
    sleeping: false,
    bigEmoji: false,
    music: { enabled: true, ui: true, queueLines: 5, refreshInterval: 5000, normalize: true },
    tts: { volume: 100, speed: 110, pitch: 0, names: 'multi' },
};

function fakeSession(can = ALL) {
    const routes = {
        'GET /relay/guilds': [{ id: G, name: 'Gather Craft', icon: null, hasPc: false, can }],
        [`GET /relay/guilds/${G}/settings`]: settings,
        [`GET /relay/guilds/${G}/channels`]: {
            chatbot: [],
            music: null,
            notify: null,
            gameChat: null,
            board: null,
            botAvailable: true,
            channels: [{ id: CH, name: 'chat', category: null }],
        },
        [`GET /relay/guilds/${G}/chatbot`]: { settings: { enabled: false }, hasApiKey: true },
        [`GET /relay/guilds/${G}/moderation`]: {
            botAvailable: true,
            can: { manageMessages: true, manageChannels: true, manageRoles: true, moderateMembers: true },
            channels: [
                { id: CH, name: 'chat', category: null, topic: null, slowmode: 10, locked: null, autoClear: null },
            ],
            mutes: [
                { userId: U, name: 'Acorn', forever: false, until: new Date(Date.now() + 1_800_000).toISOString() },
            ],
        },
    };
    return {
        extend: vi.fn(),
        request: vi.fn(async (method, path, body) => {
            if (method === 'GET') return { status: 200, data: routes[`GET ${path}`] };
            if (path.endsWith('/settings')) return { status: 200, data: { ...settings, ...body } };
            return { status: 200, data: {} };
        }),
    };
}

const flush = () => act(async () => {});

describe('PC 없는 사람의 웹', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => {
        useApp.setState({
            session: { phase: 'ready', reason: null, expiresAt: Date.now() + 900_000, hostId: null, warning: false },
            tab: 'servers',
            installBandClosed: false,
        });
    });
    afterEach(cleanup);

    it('디스코드 탭으로 열고, 서버·모듈·PC 탭은 자물쇠를 달고 설치 안내를 보인다', async () => {
        render(<App api={{}} session={fakeSession()} />);
        await flush();
        const tabs = screen.getByRole('navigation');
        expect(
            within(tabs)
                .getByRole('button', { name: /Discord/ })
                .getAttribute('aria-current'),
        ).toBe('page');
        fireEvent.click(within(tabs).getByRole('button', { name: /Servers/ }));
        const locked = screen.getByRole('heading', { name: 'This needs a PC' }).closest('section');
        expect(within(locked).getByRole('link', { name: 'saba-chan.online' }).getAttribute('href')).toBe(
            'https://saba-chan.online',
        );
    });

    it('설치 안내 띠는 닫으면 이 세션 동안 다시 나오지 않는다', async () => {
        const view = render(<App api={{}} session={fakeSession()} />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Close install notice' }));
        expect(screen.queryByRole('button', { name: 'Close install notice' })).toBeNull();
        view.rerender(<App api={{}} session={fakeSession()} />);
        expect(screen.queryByRole('button', { name: 'Close install notice' })).toBeNull();
    });

    it('PC 없는 서버의 설정은 릴레이에 바로 저장한다', async () => {
        const session = fakeSession();
        render(<App api={{}} session={session} />);
        await flush();
        fireEvent.click(screen.getByRole('checkbox', { name: /Big emoji/ }));
        await flush();
        expect(session.request).toHaveBeenCalledWith('PATCH', `/relay/guilds/${G}/settings`, { bigEmoji: true });
        expect(screen.getByRole('checkbox', { name: /Big emoji/ }).checked).toBe(true);
    });

    it('음소거를 풀면 릴레이 서버 관리 경로로 보낸다', async () => {
        const session = fakeSession();
        render(<App api={{}} session={session} />);
        await flush();
        fireEvent.click(screen.getByRole('button', { name: 'Unmute Acorn' }));
        await flush();
        expect(session.request).toHaveBeenCalledWith('PUT', `/relay/guilds/${G}/moderation/mutes/${U}`, {
            duration: 'off',
        });
    });

    it('서버 관리 권한만 있으면 서버 관리 칸만 보인다', async () => {
        const session = fakeSession({ manage: false, channels: true, messages: true, members: true });
        render(<App api={{}} session={session} />);
        await flush();
        expect(screen.queryByRole('heading', { name: 'Basics' })).toBeNull();
        expect(screen.getByRole('heading', { name: 'Server management' })).toBeTruthy();
        expect(session.request.mock.calls.map((c) => c[1])).not.toContain(`/relay/guilds/${G}/settings`);
    });

    it('읽기에 실패하면 숨기지 않고 알리고, 다시 읽으면 화면을 채운다', async () => {
        const session = fakeSession();
        const ok = session.request.getMockImplementation();
        let down = true;
        session.request.mockImplementation((method, path, body) =>
            down && path.endsWith('/settings')
                ? Promise.resolve({ status: 0, code: 'ECONNRESET' })
                : ok(method, path, body),
        );
        render(<App api={{}} session={session} />);
        await flush();
        expect(screen.queryByRole('heading', { name: 'Basics' })).toBeNull();
        down = false;
        fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
        await flush();
        expect(screen.getByRole('heading', { name: 'Basics' })).toBeTruthy();
    });
});
