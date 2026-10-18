import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CliLine, fillCli, resetCliCache } from '../components/CliLine';
import { summaryValue } from '../components/shell/CommandPalette';

const t = (k) => (k === 'palette.none' ? 'none' : k);

describe('설정 변경 요약: 이전 → 새 값 (P2 후속 §2)', () => {
    it('before가 있으면 이전 → 새 값, null이면 없음으로 보인다', () => {
        expect(summaryValue({ label: 'Max players', value: 20, before: 10 }, t)).toBe('10 → 20');
        expect(summaryValue({ label: 'MOTD', value: 'hi', before: null }, t)).toBe('none → hi');
        expect(summaryValue({ label: 'Server', value: 'A' }, t)).toBe('A');
    });
});

describe('CLI로 보기 (P2 후속 §5)', () => {
    beforeEach(() => {
        resetCliCache();
        window.api = {};
    });

    it('서버 이름(공백이면 따옴표)·필드 값을 채우고 바뀐 값을 덧붙인다', () => {
        expect(
            fillCli('saba do backup.restore --server <server> name=<name>', {
                server: 'My Server',
                values: { name: 'b1' },
            }),
        ).toBe('saba do backup.restore --server "My Server" name=b1');
        expect(
            fillCli('saba do instance.settings.set --server <server>', {
                server: 'mc',
                extra: { max_players: 20, motd: 'hello world', tags: ['a'] },
            }),
        ).toBe(`saba do instance.settings.set --server mc max_players=20 motd="hello world" tags='["a"]'`);
    });

    it('카탈로그에 그 액션의 cli가 있으면 한 줄과 복사 버튼을 보이고, 채널이 없으면 숨긴다', async () => {
        const { container } = render(<CliLine actionId="instance.settings.set" server="mc" />);
        expect(container).toBeEmptyDOMElement();

        resetCliCache();
        window.api.actionsList = vi.fn().mockResolvedValue({
            actions: [{ id: 'instance.settings.set', cli: 'saba do instance.settings.set --server <server>' }],
        });
        const writeText = vi.fn().mockResolvedValue();
        Object.assign(navigator, { clipboard: { writeText } });
        render(<CliLine actionId="instance.settings.set" server="mc" extra={{ port: 25566 }} />);
        await act(async () => {});
        expect(screen.getByText('saba do instance.settings.set --server mc port=25566')).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Copy/i })));
        expect(writeText).toHaveBeenCalledWith('saba do instance.settings.set --server mc port=25566');
    });
});
