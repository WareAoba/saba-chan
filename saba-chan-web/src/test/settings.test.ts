import { describe, expect, it } from 'vitest';
import { patchOf } from '../components/SettingsPanel';

describe('설정 보내기', () => {
    it('비밀번호·경로 칸은 고쳐도 보내지 않는다 (데몬이 원격으로 받지 않는다)', () => {
        const fields = [
            { name: 'motd', field_type: 'text' },
            { name: 'rcon_password', field_type: 'password' },
            { name: 'world_dir', field_type: 'text', format: 'path' },
            { name: 'rest_password', field_type: 'text' },
        ];
        const values = { motd: 'hi', rcon_password: 'x', world_dir: 'C:/', rest_password: 'y' };
        const touched = new Set(Object.keys(values));
        expect(patchOf(fields, values, touched)).toEqual({ motd: 'hi' });
    });
});
