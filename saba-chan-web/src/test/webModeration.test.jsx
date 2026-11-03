import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { parseDuration } from '../components/DiscordTab';
import { initI18n } from '../i18n';
import { useApp } from '../store';

// 웹 서버 관리 — /제재·슬로우 기간·자동 청소 매일·지금 청소 (2026-11-04 사용자 OK 시안)
const G = '111111111111111111';
const CH = '222222222222222222';
const PHOTOS = '222222222222222223';
const U = '333333333333333333';
const GONE = '333333333333333334';
const DOTORI = '333333333333333335';
const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const ALL = { manage: false, channels: true, messages: true, members: true };

const channel = (id, name, more = {}) => ({
    id,
    name,
    category: null,
    topic: null,
    slowmode: 0,
    slowUntil: null,
    locked: null,
    autoClear: null,
    autoClearDaily: null,
    autoClearTarget: 'all',
    clearing: null,
    ...more,
});

function fakeSession() {
    const routes = {
        'GET /relay/guilds': [{ id: G, name: 'Gather Craft', icon: null, hasPc: false, can: ALL }],
        [`GET /relay/guilds/${G}/moderation`]: {
            botAvailable: true,
            can: {},
            channels: [
                channel(CH, 'chat', { slowmode: 10, slowUntil: new Date(Date.now() + HOUR).toISOString() }),
                channel(PHOTOS, 'photos', {
                    autoClearDaily: '04:00',
                    autoClearTarget: 'bots',
                    clearing: { deleted: 1284, oldestAt: new Date(Date.now() - 23 * DAY).toISOString() },
                }),
            ],
            mutes: [],
            sanctions: [
                {
                    userId: U,
                    name: 'Acorn',
                    kind: 'chat',
                    until: new Date(Date.now() + 2 * DAY).toISOString(),
                    inGuild: true,
                },
                { userId: GONE, name: 'spam_bot', kind: 'ban', until: null, inGuild: false },
            ],
        },
        [`GET /relay/guilds/${G}/moderation/members`]: [
            { id: DOTORI, name: 'Dotori', username: 'dotori', avatarUrl: '' },
            { id: U, name: 'Acorn', username: 'acorn', avatarUrl: '' },
        ],
    };
    return {
        extend: vi.fn(),
        request: vi.fn(async (method, path) => {
            if (method === 'GET') return { status: 200, data: routes[`GET ${path}`] };
            return { status: 200, data: {} };
        }),
    };
}

const flush = () => act(async () => {});
const sent = (session, method) => session.request.mock.calls.filter((c) => c[0] === method);

async function open() {
    const session = fakeSession();
    render(<App api={{}} session={session} />);
    await flush();
    await flush();
    return session;
}

describe('웹 서버 관리', () => {
    beforeAll(() => initI18n('en'));
    beforeEach(() => {
        useApp.setState({
            session: { phase: 'ready', reason: null, expiresAt: Date.now() + 900_000, hostId: null, warning: false },
            tab: 'servers',
            installBandClosed: false,
        });
    });
    afterEach(cleanup);

    it('채널 줄에 슬로우 끝·매일 청소·이어 지우기를 보인다', async () => {
        await open();
        expect(screen.getByText(/Slow mode 10s · ends in 1 h/)).toBeTruthy();
        expect(screen.getByText('Clearing · back to 23 d ago')).toBeTruthy();
    });

    it('제재 중인 사람을 종류와 함께 보이고, 풀면 그 종류로 푼다', async () => {
        const session = await open();
        const list = within(screen.getByRole('list', { name: 'Sanctioned people' }));
        expect(list.getByText('Chat')).toBeTruthy();
        expect(list.getByText(/Not in the server/)).toBeTruthy();
        fireEvent.click(list.getByRole('button', { name: 'Lift sanction on Acorn' }));
        await flush();
        expect(session.request).toHaveBeenCalledWith('PUT', `/relay/guilds/${G}/moderation/sanctions/${U}`, {
            kind: 'chat',
            duration: 'off',
        });
    });

    it('제재하기: 사람을 찾아 고르고 종류·기간·사유로 건다. 밴일 때만 글 삭제를 보인다', async () => {
        const session = await open();
        fireEvent.click(screen.getByRole('button', { name: 'Sanction someone' }));
        await flush();
        const sheet = within(screen.getByRole('dialog'));
        fireEvent.change(sheet.getByRole('searchbox', { name: /^Member/ }), { target: { value: 'dot' } });
        fireEvent.click(sheet.getByRole('button', { name: /Dotori/ }));
        expect(sheet.queryByRole('combobox', { name: 'Delete their messages' })).toBeNull();
        fireEvent.click(sheet.getByRole('button', { name: 'Ban' }));
        expect(sheet.getByText('Delete their messages')).toBeTruthy();
        fireEvent.click(sheet.getByRole('button', { name: 'Voice' }));
        fireEvent.click(sheet.getByRole('button', { name: '1 h' }));
        fireEvent.change(sheet.getByLabelText(/Reason/), { target: { value: 'spam' } });
        fireEvent.click(sheet.getByRole('button', { name: 'Sanction' }));
        await flush();
        expect(session.request).toHaveBeenCalledWith('PUT', `/relay/guilds/${G}/moderation/sanctions/${DOTORI}`, {
            kind: 'voice',
            duration: 60,
            reason: 'spam',
        });
    });

    it('직접 쓴 기간을 분으로 바꾼다 (언어의 단위 낱말도 읽는다)', async () => {
        expect(parseDuration('3시간', '분,시간,일,주')).toBe(180);
        expect(parseDuration('30')).toBe(30);
        expect(parseDuration('2x')).toBeNull();
        const session = await open();
        fireEvent.click(screen.getByRole('button', { name: 'Sanction someone' }));
        await flush();
        const sheet = within(screen.getByRole('dialog'));
        fireEvent.change(sheet.getByRole('searchbox', { name: /^Member/ }), { target: { value: 'acorn' } });
        fireEvent.click(sheet.getByRole('button', { name: /Acorn/ }));
        fireEvent.change(sheet.getByLabelText(/Custom/), { target: { value: '2w' } });
        fireEvent.click(sheet.getByRole('button', { name: 'Sanction' }));
        await flush();
        expect(session.request).toHaveBeenCalledWith('PUT', `/relay/guilds/${G}/moderation/sanctions/${U}`, {
            kind: 'chat',
            duration: 14 * 1440,
        });
    });

    it('채널: 슬로우 기간과 자동 청소 매일·대상을 바꾼다', async () => {
        const session = await open();
        fireEvent.click(screen.getByRole('button', { name: /# chat/ }));
        const sheet = within(screen.getByRole('dialog'));
        fireEvent.click(sheet.getByRole('button', { name: '1 h' }));
        await flush();
        fireEvent.click(sheet.getByRole('button', { name: 'Daily' }));
        await flush();
        const path = `/relay/guilds/${G}/moderation/channels/${CH}`;
        expect(sent(session, 'PATCH')).toEqual([
            ['PATCH', path, { slowmode: 10, slowFor: 60 }],
            ['PATCH', path, { autoClearDaily: '04:00' }],
        ]);
    });

    it('시각 지정을 비워 두면 청소를 시작할 수 없다 (채널 전체가 지워지지 않게)', async () => {
        await open();
        fireEvent.click(screen.getByRole('button', { name: /# chat/ }));
        fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Clear now…' }));
        const sheet = within(screen.getByRole('dialog'));
        fireEvent.click(sheet.getByRole('button', { name: 'Pick times' }));
        expect(sheet.getByRole('button', { name: 'Start clearing' }).disabled).toBe(true);
        fireEvent.change(sheet.getByLabelText('From'), { target: { value: '2026-11-01T20:00' } });
        expect(sheet.getByRole('button', { name: 'Start clearing' }).disabled).toBe(false);
    });

    it('지금 청소: 대상·개수로 지우고, 이어 지우는 채널은 진행과 중지를 보인다', async () => {
        const session = await open();
        fireEvent.click(screen.getByRole('button', { name: /# chat/ }));
        fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Clear now…' }));
        const sheet = within(screen.getByRole('dialog'));
        fireEvent.click(sheet.getByRole('button', { name: 'All bots' }));
        fireEvent.change(sheet.getByLabelText(/How many/), { target: { value: '20' } });
        fireEvent.click(sheet.getByRole('button', { name: 'Start clearing' }));
        await flush();
        expect(sent(session, 'POST')).toEqual([
            ['POST', `/relay/guilds/${G}/moderation/channels/${CH}/clear`, { target: 'bots', count: 20 }],
        ]);
        cleanup();

        const again = await open();
        fireEvent.click(screen.getByRole('button', { name: /# photos/ }));
        fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Clear now…' }));
        const progress = within(screen.getByRole('dialog'));
        expect(progress.getByText(/1,284 deleted/)).toBeTruthy();
        fireEvent.click(progress.getByRole('button', { name: 'Stop' }));
        await flush();
        expect(sent(again, 'DELETE')).toEqual([
            ['DELETE', `/relay/guilds/${G}/moderation/channels/${PHOTOS}/clear`, undefined],
        ]);
    });
});
