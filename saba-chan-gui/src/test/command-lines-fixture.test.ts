/**
 * 명령 줄 표본(tests/fixtures/command-lines.json): 데몬 시험(`commands::tests::command_lines_read_like_the_shared_fixture`)과
 * 같은 표본을 앱 명령 줄 표시가 같게 읽는다 — 어느 명령인지, 틀렸다면 어느 칸의 어떤 규칙인지
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import type { CommandField } from '../utils/commandRisk';
import { readCommandLine } from '../utils/commandSyntax';

const fixture = JSON.parse(
    fs.readFileSync(path.resolve(__dirname, '../../../tests/fixtures/command-lines.json'), 'utf8'),
) as {
    commands: CommandField[];
    lines: { line: string; command: string | null; args?: object; error?: { rule: string; field: string | null } }[];
};

describe('명령 줄 표본', () => {
    it.each(fixture.lines)('$line', ({ line, command, error }) => {
        // 줄을 다 친 뒤(빈칸)로 읽는다: 치는 중인 마지막 낱말은 아직 틀렸다고 하지 않는다
        const state = readCommandLine(`${line} `, fixture.commands);
        expect(state.command?.name ?? null).toBe(command);
        const read = state.problem
            ? { rule: state.problem.rule, field: state.problem.input?.name ?? null }
            : state.missing
              ? { rule: 'missing', field: state.missing.name }
              : null;
        expect(read).toEqual(error ?? null);
    });
});
