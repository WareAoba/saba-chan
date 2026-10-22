import { describe, expect, it } from 'vitest';
import { installFailureText } from '../utils/helpers';

// 데몬 #533: 설치 전 실행 파일 백업을 못 하면 backup_failed + file, 되돌리지 못한 파일은 restore_failed와 같은 순서의 kept_backups
const t = (k, o) => `${k}${o ? JSON.stringify(o) : ''}`;

describe('설치 실패 문구', () => {
    it('백업하지 못해 설치하지 않았으면 그 파일 이름으로 알린다', () => {
        expect(installFailureText(t, { error_code: 'backup_failed', file: 'PalServer' }, 'base')).toBe(
            'server_settings.install_backup_failed{"file":"PalServer"}',
        );
        // 백업 만들기의 backup_failed처럼 file이 없으면 원래 문구
        expect(installFailureText(t, { error_code: 'backup_failed' }, 'base')).toBe('base');
    });

    it('되돌리지 못한 파일은 남긴 사본 이름을 덧붙인다', () => {
        const res = {
            success: false,
            restore_failed: ['PalServer', 'java'],
            kept_backups: ['PalServer.1.bak', 'java.bak'],
        };
        expect(installFailureText(t, res, 'base')).toBe(
            'base server_settings.install_kept_backups{"files":"PalServer → PalServer.1.bak, java → java.bak"}',
        );
        // 옛 데몬은 kept_backups 없이 restore_failed만 준다
        expect(installFailureText(t, { restore_failed: ['PalServer'] }, 'base')).toBe('base');
    });
});
