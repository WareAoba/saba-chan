/**
 * 인스턴스별 부르는 말: 서버 상세의 디스코드 탭에서 이 서버와 그 명령을 부르는 낱말을 고친다.
 * 낱말은 인스턴스에 딸려 어느 디스코드 서버에서나 같다. 데몬 GET·PUT /api/instance/:id/words.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { errorsByField, InstanceDiscordTab } from '../components/discord/InstanceWordsTab';

const server = { id: 's1', name: '야생', module: 'minecraft' };
const item = () => ({
    id: 's1',
    name: '야생',
    module: 'minecraft',
    words: ['크리'],
    commands: [
        { name: 'start', label: 'Start', risk: 'safe', builtin: true, moduleAliases: ['시작'], aliases: [] },
        { name: 'kick', label: 'Kick', risk: 'caution', builtin: false, moduleAliases: [], aliases: ['추방'] },
    ],
});

async function addChip(label, word) {
    const input = screen.getByLabelText(label);
    await act(async () => fireEvent.change(input, { target: { value: word } }));
    await act(async () => fireEvent.keyDown(input, { key: 'Enter' }));
}

describe('InstanceDiscordTab', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            instanceWords: vi.fn().mockResolvedValue(item()),
            // 데몬 응답 모양: 빈 명령 목록은 빠진다
            instanceWordsSave: vi.fn(async (id, body) => ({
                success: true,
                id,
                words: body.words,
                commands: Object.fromEntries(Object.entries(body.commands).filter(([, l]) => l.length)),
            })),
        };
    });

    it('이 서버의 부르는 말과 명령 낱말을 디스코드 서버와 상관없이 하나로 저장한다', async () => {
        await act(async () => render(<InstanceDiscordTab server={server} />));
        await screen.findByLabelText('Names for 야생');
        expect(window.api.instanceWords).toHaveBeenCalledWith('s1');
        expect(screen.getAllByLabelText('Names for 야생')).toHaveLength(1);
        await addChip('Names for 야생', '야섭');
        await addChip('Kick', '킥');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Save/ })));
        expect(window.api.instanceWordsSave).toHaveBeenCalledWith('s1', {
            words: ['크리', '야섭'],
            commands: { start: [], kick: ['추방', '킥'] },
        });
        // 저장한 값이 화면의 기준이 되어 더 고칠 것이 없다
        await waitFor(() => expect(screen.queryByRole('button', { name: /Save/ })).toBeNull());
        expect(screen.getByText('킥')).toBeTruthy();
    });

    it('검증 오류는 그 칸 아래에 겹치는 대상과 함께 보인다', async () => {
        window.api.instanceWordsSave = vi.fn().mockResolvedValue({
            ok: false,
            error_code: 'validation_failed',
            details: [{ field: 'words', reason: 'conflict', word: '팰', conflictWith: 'palworld' }],
        });
        await act(async () => render(<InstanceDiscordTab server={server} />));
        await screen.findByLabelText('Names for 야생');
        await addChip('Names for 야생', '팰');
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Save/ })));
        await waitFor(() => expect(screen.getByText(/palworld/)).toBeTruthy());
        expect(document.querySelector('.word-chip.bad').textContent).toContain('팰');
    });

    it('못 받으면 이유와 다시 시도를 보인다', async () => {
        window.api.instanceWords = vi.fn().mockResolvedValue({ ok: false, error: 'daemon down' });
        await act(async () => render(<InstanceDiscordTab server={server} />));
        expect(await screen.findByText(/daemon down/)).toBeTruthy();
    });

    it('details의 field를 칸별로 나눈다', () => {
        expect(
            Object.keys(
                errorsByField([{ field: 'words[1]' }, { field: 'commands.kick' }, { field: 'commands.kick[0]' }]),
            ),
        ).toEqual(['words', 'commands.kick']);
    });
});
