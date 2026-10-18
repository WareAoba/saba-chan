import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CliLine, changedForCli, commandCli, fillCli, resetCliCache, splitQuoted } from '../components/CliLine';
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
        ).toBe("saba do backup.restore --server 'My Server' name=b1");
        expect(
            fillCli('saba do instance.settings.set --server <server>', {
                server: 'mc',
                extra: { max_players: 20, motd: 'hello world', tags: ['a'] },
            }),
        ).toBe(`saba do instance.settings.set --server mc max_players=20 motd='hello world' tags='["a"]'`);
        // 셸이 풀어 버리는 글자는 작은따옴표 안에 두고, 안의 작은따옴표는 '\''로
        expect(fillCli('saba do x --server <server>', { server: "it's $HOME `x`" })).toBe(
            `saba do x --server 'it'\\''s $HOME \`x\`'`,
        );
    });

    it('비밀번호 칸과 *_password·*_token·*_secret 키는 값 대신 <이름>을 둔다', () => {
        const fields = [{ name: 'admin_pw', field_type: 'password' }];
        const changed = changedForCli(
            { admin_pw: 'hunter2', rcon_password: 'p', api_token: 't', max_players: 20, _launch_extra_args: 'x' },
            { max_players: 10 },
            fields,
        );
        expect(changed).toEqual({
            admin_pw: '<admin_pw>',
            rcon_password: '<rcon_password>',
            api_token: '<api_token>',
            max_players: 20,
        });
        expect(
            fillCli('saba do instance.settings.set --server <server>', { server: 'mc', extra: changed }),
        ).not.toMatch(/hunter2/);
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

describe('명령 화면의 CLI 줄', () => {
    const commands = [{ name: 'say' }, { name: 'tellraw' }, { name: 'kick', aliases: ['추방'] }];

    it('데몬 split_quoted와 같이 나눈다', () => {
        expect(splitQuoted('say "hello world" x')).toEqual(['say', 'hello world', 'x']);
        expect(splitQuoted('tellraw @a {"text":"hi"}')).toEqual(['tellraw', '@a', '{"text":"hi"}']);
        expect(splitQuoted('say "a"b c"')).toEqual(['say', 'a"b c']);
    });

    it('아는 명령은 server run, 칸마다 셸 따옴표로 지킨다', () => {
        expect(commandCli('My Server', 'say "hello world"', commands)).toBe(
            "saba-chan-cli server run 'My Server' say 'hello world'",
        );
        expect(commandCli('wild', 'tellraw @a {"text":"hi"}', commands)).toBe(
            `saba-chan-cli server run wild tellraw @a '{"text":"hi"}'`,
        );
        expect(commandCli('wild', 'say hi; rm -rf ~', commands)).toBe(
            "saba-chan-cli server run wild say 'hi;' rm -rf '~'",
        );
        expect(commandCli('wild', '추방 steve', commands)).toBe("saba-chan-cli server run wild '추방' steve");
    });

    it('모르는 명령은 줄 전체를 한 인자로 server send, 비었으면 줄이 없다', () => {
        expect(commandCli('wild', 'gamemode creative $HOME', commands)).toBe(
            "saba-chan-cli server send wild 'gamemode creative $HOME'",
        );
        expect(commandCli('wild', '  ', commands)).toBeNull();
    });
});
