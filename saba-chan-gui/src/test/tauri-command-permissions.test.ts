import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// 셸에 등록한 명령은 build.rs COMMANDS와 capabilities 둘 다에 있어야 렌더러가 부를 수 있다 (빠지면 "not allowed")
const TAURI = join(__dirname, '../../src-tauri');
const read = (p: string) => readFileSync(join(TAURI, p), 'utf8');

describe('Tauri 명령 권한', () => {
    it('invoke_handler에 등록한 명령은 모두 허용 목록에 있다', () => {
        const handler = read('src/lib.rs').match(/generate_handler!\[([\s\S]*?)\]/)?.[1] ?? '';
        const registered = [...handler.matchAll(/\b\w+::(\w+)/g)].map((m) => m[1]);
        const commands = read('build.rs').match(/COMMANDS[^=]*=\s*&\[([\s\S]*?)\];/)?.[1] ?? '';
        const capabilities = readdirSync(join(TAURI, 'capabilities'))
            .map((f) => read(`capabilities/${f}`))
            .join('\n');
        expect(registered.length).toBeGreaterThan(10);
        const missing = registered.filter(
            (c) => !commands.includes(`"${c}"`) || !capabilities.includes(`"allow-${c.replace(/_/g, '-')}"`),
        );
        expect(missing).toEqual([]);
    });
});
