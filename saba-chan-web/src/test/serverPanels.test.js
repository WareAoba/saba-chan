import { describe, expect, it } from 'vitest';
import { resultText } from '../components/CommandsPanel';
import { commandArgs, mergeCommands } from '../components/commands';
import { sabaPatch, sabaValues } from '../components/SabaSettings';
import { uptimeText } from '../components/ServerDetail';

describe('사바쨩 설정', () => {
    const server = {
        memory_limit_mb: 4096,
        game_chat: 'keyword',
        game_chat_keyword: '!',
        module_settings: { stop_timeout: 30, auto_restart: 'false' },
    };

    it('저장된 값을 칸 값으로 읽는다 (재시작 기본값은 켜짐·꺼짐)', () => {
        expect(sabaValues(server)).toEqual({
            memory_limit_mb: '4096',
            stop_timeout: '30',
            game_chat: 'keyword',
            game_chat_keyword: '!',
            auto_restart: false,
            restart_on_unknown_exit: false,
        });
        expect(sabaValues({}).auto_restart).toBe(true);
    });

    it('바꾼 칸만 보내고, 비운 숫자 칸은 null 로 보낸다', () => {
        const values = { ...sabaValues(server), memory_limit_mb: '', auto_restart: true, game_chat_keyword: ' ! ' };
        expect(sabaPatch(sabaValues(server), values)).toEqual({ memory_limit_mb: null, auto_restart: true });
    });

    it('바뀐 것이 없으면 빈 객체', () => {
        expect(sabaPatch(sabaValues(server), sabaValues(server))).toEqual({});
    });

    it('다른 곳에서 바뀐 값은 고친 것으로 보지 않는다 (기준값과만 비교)', () => {
        const opened = sabaValues(server);
        const changedElsewhere = { ...server, memory_limit_mb: 8192 };
        expect(sabaPatch(opened, opened)).toEqual({});
        expect(sabaPatch(sabaValues(changedElsewhere), opened)).toEqual({ memory_limit_mb: '4096' });
    });
});

describe('게임 명령', () => {
    it('카탈로그에 모듈 정의를 입히고, 익스텐션 명령은 데몬이 위험하다고 한 것만 믿는다', () => {
        const catalog = [
            { name: 'kick', risk: 'caution' },
            { name: 'wipe', source: 'extension', risk: 'caution' },
            { name: 'nuke', source: 'extension', risk: 'danger' },
        ];
        const own = [{ name: 'kick', risk: 'danger', inputs: [{ name: 'player' }] }];
        expect(mergeCommands(catalog, own)).toEqual([
            { name: 'kick', risk: 'danger', inputs: [{ name: 'player' }] },
            { name: 'wipe', source: 'extension', risk: undefined },
            { name: 'nuke', source: 'extension', risk: 'danger' },
        ]);
        expect(mergeCommands(null, own)).toBe(own);
    });

    it('빈 칸은 빼고 숫자 칸은 숫자로, 켜고 끄는 칸은 참거짓으로 보낸다', () => {
        const inputs = [
            { name: 'player', type: 'text' },
            { name: 'minutes', type: 'number' },
            { name: 'silent', type: 'boolean' },
            { name: 'reason', type: 'text' },
        ];
        expect(commandArgs(inputs, { player: ' Steve ', minutes: '5', silent: true, reason: '' })).toEqual({
            player: 'Steve',
            minutes: 5,
            silent: true,
        });
        expect(
            commandArgs(
                [
                    { name: 'on', type: 'boolean', required: true },
                    { name: 'opt', type: 'boolean' },
                ],
                {},
            ),
        ).toEqual({
            on: false,
        });
    });
});

describe('명령 결과', () => {
    it('전송 안내보다 게임의 실제 응답을 보인다', () => {
        expect(resultText({ message: 'Sent over RCON: list', data: { response: 'There are 2 players online' } })).toBe(
            'There are 2 players online',
        );
        expect(resultText({ message: 'ok', data: { players: 2 } })).toBe('{"players":2}');
        expect(resultText({ message: 'Sent' })).toBe('Sent');
    });
});

describe('켜진 시간', () => {
    it('시작 시각(초) → 시:분:초', () => {
        expect(uptimeText(1000, (1000 + 3600 * 2 + 14 * 60 + 5) * 1000)).toBe('2:14:05');
        expect(uptimeText(null, 0)).toBeNull();
    });
});

describe('버튼', () => {
    it('기다리는 동안은 disabled={false} 를 넘겨도 누를 수 없다', async () => {
        const { renderToStaticMarkup } = await import('react-dom/server');
        const { createElement } = await import('react');
        const { Button } = await import('../components/ui');
        expect(renderToStaticMarkup(createElement(Button, { busy: true, disabled: false }, 'x'))).toContain('disabled');
    });
});
