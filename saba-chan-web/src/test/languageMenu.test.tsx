import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { mockApi, mockSession } from './mocks';

// 화면 언어는 PC 탭 안쪽이 아니라 머리(와 연결 화면)에서 바로 고른다: PC 없는 세션도 바꿀 수 있어야 한다
const flush = () => act(async () => {});
const relayOnly = () =>
    mockSession({
        extend: vi.fn(),
        request: vi.fn(async (_m: string, path: string) => ({ status: 200, data: path === '/relay/guilds' ? [] : {} })),
    });

describe('화면 언어 고르기', () => {
    beforeEach(async () => {
        localStorage.clear();
        await initI18n('en');
    });
    afterEach(cleanup);

    it('PC 없는 세션도 머리의 언어 버튼으로 바꾸고, 이 브라우저에 기억한다', async () => {
        useApp.setState({
            session: { phase: 'ready', reason: null, expiresAt: Date.now() + 900_000, hostId: null, warning: false },
            tab: 'discord',
        });
        const view = render(<App api={mockApi({})} session={relayOnly()} />);
        await flush();
        fireEvent.click(
            within(view.container.querySelector('.topbar') as HTMLElement).getByRole('button', { name: 'Language' }),
        );
        fireEvent.click(screen.getByRole('menuitemradio', { name: '日本語' }));
        await waitFor(() => expect(i18n.language).toBe('ja'));
        expect(localStorage.getItem('saba-web.language')).toBe('ja');
        expect(document.documentElement.lang).toBe('ja');
        expect(screen.queryByRole('menuitemradio', { name: '日本語' })).toBeNull();
    });

    it('연결이 끝난 화면에서도 언어를 바꿀 수 있다', async () => {
        useApp.setState({
            session: { phase: 'closed', reason: 'no_link', expiresAt: null, hostId: null, warning: false },
        });
        render(<App session={null} />);
        fireEvent.click(screen.getByRole('button', { name: 'Language' }));
        const current = screen.getByRole('menuitemradio', { name: 'English' });
        expect(current.getAttribute('aria-checked')).toBe('true');
        fireEvent.click(screen.getByRole('menuitemradio', { name: '한국어' }));
        await waitFor(() => expect(i18n.language).toBe('ko'));
    });
});
