import { describe, expect, it } from 'vitest';
import { installFailureText } from '../utils/helpers';

// 데몬 #533: 설치 전 실행 파일 백업을 못 하면 backup_failed + file, 되돌리지 못한 파일은 restore_failed와 같은 순서의 kept_backups.
// 모듈이 오류로 끝나 500이면 error가 `could not put back X (kept as Y): <모듈 오류>`다 (01 install 행)
const t = (k, o) => `${k}${o ? JSON.stringify(o) : ''}`;
// translateError 자리: 무엇을 번역하라고 넘겼는지 보인다
const translate = (msg) => `tr(${msg ?? ''})`;

describe('설치 실패 문구', () => {
    it('백업하지 못해 설치하지 않았으면 그 파일 이름으로 알린다', () => {
        expect(installFailureText(t, { error_code: 'backup_failed', file: 'PalServer', message: 'x' }, translate)).toBe(
            'server_settings.install_backup_failed{"file":"PalServer"}',
        );
        // 백업 만들기의 backup_failed처럼 file이 없으면 원래 문구
        expect(installFailureText(t, { error_code: 'backup_failed', error: 'disk' }, translate)).toBe('tr(disk)');
    });

    it('되돌리지 못한 파일은 남긴 사본 이름을 덧붙인다', () => {
        const res = {
            success: false,
            message: 'download failed',
            restore_failed: ['PalServer', 'java'],
            kept_backups: ['PalServer.1.bak', 'java.bak'],
        };
        expect(installFailureText(t, res, translate)).toBe(
            'tr(download failed) server_settings.install_kept_backups{"files":"PalServer → PalServer.1.bak, java → java.bak"}',
        );
        // 옛 데몬은 kept_backups 없이 restore_failed만 준다
        expect(installFailureText(t, { restore_failed: ['PalServer'], message: 'm' }, translate)).toBe('tr(m)');
    });

    // 리뷰 630 P3-1: 모듈 오류의 낱말(timed out 등) 때문에 토스트 전체가 일반 문구로 바뀌어 남긴 사본 안내가 사라졌다
    it('500 오류의 되돌리지 못한 파일 안내는 번역에 묻히지 않는다', () => {
        const res = {
            error: 'could not put back PalServer (kept as PalServer.1.bak), java (kept as java.bak): Plugin timed out after 1800s',
        };
        expect(installFailureText(t, res, translate)).toBe(
            'tr(Plugin timed out after 1800s) server_settings.install_kept_backups{"files":"PalServer → PalServer.1.bak, java → java.bak"}',
        );
        expect(installFailureText(t, { error: 'Plugin timed out' }, translate)).toBe('tr(Plugin timed out)');
    });
});
