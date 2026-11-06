/**
 * #4: 직접 치는 명령 줄을 데몬이 읽는 방식(이름·별명 + 칸 순서, 마지막 칸이 남은 말을 받음)대로 읽어
 * 맞게 쳤는지·무엇이 빠졌는지·무엇이 틀렸는지 알린다. 모듈이 알려 준 칸 정보만 쓴다.
 */
import { describe, expect, it } from 'vitest';
import type { CommandField } from '../utils/commandRisk';
import { applySuggestion, readCommandLine } from '../utils/commandSyntax';

const input = (name: string, extra: object = {}) => ({
    name,
    label: name,
    type: 'text',
    required: true,
    placeholder: null,
    default: null,
    ...extra,
});
const COMMANDS = [
    { name: 'say', method: 'console', inputs: [input('message')] },
    {
        name: 'give',
        method: 'console',
        aliases: ['지급'],
        inputs: [input('player'), input('item'), input('amount', { type: 'number', min: 1, max: 64, default: 1 })],
    },
    {
        name: 'weather',
        method: 'console',
        inputs: [input('type', { type: 'select', options: ['clear', 'rain', 'thunder'] })],
    },
    { name: 'list', method: 'console', inputs: [] },
    { name: 'whitelist', method: 'console', inputs: [input('action', { type: 'select', options: ['add', 'remove'] })] },
] as unknown as CommandField[];

const kinds = (text: string) =>
    readCommandLine(text, COMMANDS)
        .tokens.filter((t) => t.kind !== 'space')
        .map((t) => `${t.text}:${t.kind}`);

describe('명령 줄 읽기', () => {
    it('이름과 맞는 칸을 색으로 나눈다', () => {
        expect(kinds('give Steve diamond 10')).toEqual(['give:name', 'Steve:ok', 'diamond:ok', '10:ok']);
        expect(readCommandLine('give Steve diamond 10', COMMANDS)).toMatchObject({ problem: null, missing: null });
    });

    it('별명도 이름으로 읽는다 (대소문자 무시)', () => {
        expect(readCommandLine('지급 Steve', COMMANDS).command?.name).toBe('give');
        expect(readCommandLine('GIVE Steve', COMMANDS).command?.name).toBe('give');
    });

    it('범위 밖 숫자는 틀린 칸으로 표시하고 이유를 준다', () => {
        const state = readCommandLine('give Steve diamond 99', COMMANDS);
        expect(kinds('give Steve diamond 99').at(-1)).toBe('99:bad');
        expect(state.problem).toMatchObject({ rule: 'out_of_range', value: '99', input: { name: 'amount' } });
    });

    it('마지막 칸은 남은 말을 모두 받는다 (데몬과 같다)', () => {
        expect(kinds('say hello world')).toEqual(['say:name', 'hello:ok', 'world:ok']);
        const state = readCommandLine('give Steve diamond 10 more', COMMANDS);
        expect(state.problem).toMatchObject({ rule: 'not_number', value: '10 more' });
    });

    it('기본값 없는 필수 칸이 비었으면 그 칸을 알린다 (기본값이 있으면 데몬이 채운다)', () => {
        expect(readCommandLine('give Steve ', COMMANDS).missing?.name).toBe('item');
        expect(readCommandLine('give Steve diamond', COMMANDS).missing).toBeNull();
    });

    it('고를 값이 정해진 칸은 친 앞부분에 맞는 값을 권하고, 치는 중에는 틀렸다고 하지 않는다', () => {
        const state = readCommandLine('weather r', COMMANDS);
        expect(state.suggestions).toEqual({ kind: 'option', values: ['rain'] });
        expect(state).toMatchObject({ problem: null, picking: { name: 'type' } });
        expect(kinds('weather r').at(-1)).toBe('r:ok');
        expect(readCommandLine('weather x', COMMANDS).problem).toMatchObject({ rule: 'not_option', value: 'x' });
        expect(readCommandLine('weather r ', COMMANDS).problem).toMatchObject({ rule: 'not_option', value: 'r' });
        expect(readCommandLine('weather ', COMMANDS).suggestions?.values).toEqual(['clear', 'rain', 'thunder']);
    });

    it('칸이 없는 명령에 말을 붙이면 남는 말로 표시한다', () => {
        expect(kinds('list all')).toEqual(['list:name', 'all:extra']);
        expect(readCommandLine('list all', COMMANDS).problem).toMatchObject({ rule: 'too_many', value: 'all' });
    });

    it('목록에 없는 명령은 막지 않고, 첫 낱말을 치는 중이면 이어 칠 수 있는 이름을 권한다', () => {
        const state = readCommandLine('tp Steve Alex', COMMANDS);
        expect(state.command).toBeNull();
        expect(state.problem).toBeNull();
        expect(kinds('tp Steve Alex')).toEqual(['tp:free', 'Steve:free', 'Alex:free']);
        expect(readCommandLine('w', COMMANDS).suggestions).toEqual({
            kind: 'command',
            values: ['weather', 'whitelist'],
        });
    });

    it('지금 치는 칸을 알려 준다 (빈칸 뒤면 다음 칸)', () => {
        expect(readCommandLine('give Ste', COMMANDS).current).toBe(0);
        expect(readCommandLine('give Steve ', COMMANDS).current).toBe(1);
        expect(readCommandLine('say a b c', COMMANDS).current).toBe(0);
    });

    it('권하는 값을 누르면 치던 낱말을 바꾸거나 새 낱말로 붙인다', () => {
        const pick = (text: string, value: string) => applySuggestion(text, readCommandLine(text, COMMANDS), value);
        expect(pick('w', 'weather')).toBe('weather ');
        expect(pick('weather r', 'rain')).toBe('weather rain ');
        expect(pick('weather ', 'thunder')).toBe('weather thunder ');
    });
});
