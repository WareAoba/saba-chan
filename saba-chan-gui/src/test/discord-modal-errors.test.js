/**
 * DiscordBotModal — 에러 위장 방지 회귀 테스트
 *
 * 릴레이 조회 실패가 "노드/멤버 없음"으로 위장되어 기존 데이터를
 * 빈 목록으로 덮어쓰던 버그의 재발을 방지한다.
 * - 노드 목록: 에러 응답 시 setCloudNodes 미호출 (기존 목록 유지) + 경고 표시
 * - 멤버 목록: 에러 응답 시 setCloudMembers 미호출 (기존 캐시 유지) + 경고 표시
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import DiscordBotModal from '../components/Modals/DiscordBotModal';
import { useDiscordStore } from '../stores/useDiscordStore';

function createApiMock(overrides = {}) {
    const base = {
        relayListHostNodes: vi.fn().mockResolvedValue([{ guildId: 'g1', guildName: 'TestGuild' }]),
        relayListNodeMembers: vi.fn().mockResolvedValue([]),
        relayGetNodeSettings: vi.fn().mockResolvedValue({ musicEnabled: true, musicUIEnabled: true }),
        relayUpdateNodeSettings: vi.fn().mockResolvedValue({ success: true }),
        ...overrides,
    };
    return new Proxy(base, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
            return target[prop];
        },
    });
}

function createProps(overrides = {}) {
    return {
        isOpen: true,
        onClose: vi.fn(),
        isClosing: false,
        displayMode: 'popup',
        discordPrefix: '!saba',
        setDiscordPrefix: vi.fn(),
        discordCloudRelayUrl: '',
        discordCloudHostId: 'host-1',
        setDiscordCloudHostId: vi.fn(),
        relayConnected: true,
        relayConnecting: false,
        relayStatusChecked: true,
        saveCurrentSettings: vi.fn(),
        servers: [{ id: 's1', name: 'PalServer', module: 'palworld' }],
        moduleAliasesPerModule: {
            palworld: {
                module_aliases: ['팰월드'],
                commands: { start: { aliases: ['실행'], label: 'Start', description: '' } },
            },
        },
        nodeSettings: {},
        setNodeSettings: vi.fn(),
        instanceSettings: {},
        setInstanceSettings: vi.fn(),
        cloudNodes: [{ guildId: 'g1', guildName: 'TestGuild' }],
        setCloudNodes: vi.fn(),
        cloudMembers: { g1: [{ id: 'u1', username: 'alice', displayName: 'Alice' }] },
        setCloudMembers: vi.fn(),
        ...overrides,
    };
}

beforeEach(() => {
    vi.clearAllMocks();
    useDiscordStore.getState()._resetForTest();
    window.showToast = vi.fn();
});

describe('DiscordBotModal — 노드 목록 에러 처리', () => {
    it('에러 응답이면 기존 노드 목록을 덮어쓰지 않고 경고를 표시한다', async () => {
        window.api = createApiMock({
            relayListHostNodes: vi.fn().mockResolvedValue({ error: 'rate limited' }),
        });
        const props = createProps();

        await act(async () => {
            render(<DiscordBotModal {...props} />);
        });

        await waitFor(() => {
            expect(window.api.relayListHostNodes).toHaveBeenCalled();
        });

        // 기존 목록을 빈 배열로 덮어쓰지 않음
        expect(props.setCloudNodes).not.toHaveBeenCalled();
        // props로 받은 기존 노드 카드는 그대로 표시
        expect(screen.getByText('TestGuild')).toBeInTheDocument();
        // 로드 실패 경고 표시 (en locale)
        expect(screen.getByText(/Failed to load node list/)).toBeInTheDocument();
    });

    it('정상 배열 응답이면 노드 목록을 갱신한다', async () => {
        window.api = createApiMock();
        const props = createProps();

        await act(async () => {
            render(<DiscordBotModal {...props} />);
        });

        await waitFor(() => {
            expect(props.setCloudNodes).toHaveBeenCalledWith([{ guildId: 'g1', guildName: 'TestGuild' }]);
        });
        expect(screen.queryByText(/Failed to load node list/)).not.toBeInTheDocument();
    });
});

describe('DiscordBotModal — 멤버 목록 에러 처리 (서버별 설정 팝업)', () => {
    /** 서버별 설정 팝업 열기 → 인스턴스 확장 → 명령어 펼치기 (유저 목록 노출) */
    async function openUserList() {
        // "서버별 설정" 팝업 열기 (en locale)
        await act(async () => {
            screen.getByText(/Per-instance Settings/).click();
        });
        // 인스턴스 카드 확장
        await act(async () => {
            screen.getByText('PalServer').click();
        });
        // 명령어(start) 펼치기 — 유저 차단 목록 표시
        const cmdHeader = document.querySelector('.instance-settings-command-header');
        expect(cmdHeader).not.toBeNull();
        await act(async () => {
            cmdHeader.click();
        });
    }

    /** 명령어 바디 내부의 멤버 새로고침 버튼 */
    function findRefreshBtn() {
        const body = document.querySelector('.instance-settings-command-body');
        expect(body).not.toBeNull();
        return [...body.querySelectorAll('.discord-instance-select-btn')].find((b) => b.textContent.includes('🔄'));
    }

    it('에러 응답이면 기존 멤버 캐시를 유지하고 경고를 표시한다', async () => {
        window.api = createApiMock({
            relayListNodeMembers: vi.fn().mockResolvedValue({ error: 'boom' }),
        });
        const props = createProps();

        await act(async () => {
            render(<DiscordBotModal {...props} />);
        });
        await openUserList();

        // 캐시가 있으므로 표시됨
        expect(screen.getByText('Alice')).toBeInTheDocument();

        // 새로고침(강제) → 에러 응답
        await act(async () => {
            findRefreshBtn().click();
        });

        await waitFor(() => {
            expect(window.api.relayListNodeMembers).toHaveBeenCalled();
        });

        // 기존 캐시를 빈 배열로 덮어쓰지 않음
        expect(props.setCloudMembers).not.toHaveBeenCalled();
        // 캐시된 멤버는 그대로 표시 + 에러 경고
        expect(screen.getByText('Alice')).toBeInTheDocument();
        expect(screen.getByText(/Failed to load member list/)).toBeInTheDocument();
    });

    it('정상 응답이면 멤버 캐시를 갱신하고 경고가 사라진다', async () => {
        window.api = createApiMock({
            relayListNodeMembers: vi.fn().mockResolvedValue([{ id: 'u2', username: 'bob' }]),
        });
        const props = createProps();

        await act(async () => {
            render(<DiscordBotModal {...props} />);
        });
        await openUserList();

        await act(async () => {
            findRefreshBtn().click();
        });

        await waitFor(() => {
            expect(props.setCloudMembers).toHaveBeenCalled();
        });
        expect(screen.queryByText(/Failed to load member list/)).not.toBeInTheDocument();
    });

    it('{members: [...]} 형태 응답도 정상 처리한다', async () => {
        window.api = createApiMock({
            relayListNodeMembers: vi.fn().mockResolvedValue({ members: [{ id: 'u3', username: 'carol' }] }),
        });
        const props = createProps();

        await act(async () => {
            render(<DiscordBotModal {...props} />);
        });
        await openUserList();

        await act(async () => {
            findRefreshBtn().click();
        });

        await waitFor(() => {
            expect(props.setCloudMembers).toHaveBeenCalled();
        });
        expect(screen.queryByText(/Failed to load member list/)).not.toBeInTheDocument();
    });
});
