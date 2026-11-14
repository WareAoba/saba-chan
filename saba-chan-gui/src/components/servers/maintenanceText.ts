import type { TFunction } from 'i18next';
import type { ApiBody } from '../../types/daemon';

// 데몬 원문(영어) 대신 보일 문구가 있는 거절 코드
const CODE_TEXT: Record<string, string> = {
    restore_outside: 'maintenance.err_restore_outside',
    backup_busy: 'maintenance.err_backup_busy',
    // 켜진 서버가 쥐고 있는 파일 (`file`: 그 이름)
    backup_file_locked: 'maintenance.err_backup_file_locked',
    // 백업은 만들었지만 자동 저장을 되돌리지 못했다 ([backup] resume_command)
    autosave_not_resumed: 'maintenance.err_autosave_not_resumed',
    // 백업도 못 했고 자동 저장도 되돌리지 못했다 (`cause`: 백업 실패 코드)
    backup_failed_autosave_off: 'maintenance.err_backup_failed_autosave_off',
    // 켜진 서버가 저장 명령을 거절했거나 저장 완료 줄이 나오지 않아 백업하지 않았다 ([backup] save_done_pattern)
    backup_save_rejected: 'maintenance.err_backup_save_rejected',
    backup_save_unconfirmed: 'maintenance.err_backup_save_unconfirmed',
    // 켜진 서버를 묶는 사이 서버가 꺼져 그 백업을 지웠다
    backup_server_stopped: 'maintenance.err_backup_server_stopped',
    snapshot_not_found: 'maintenance.err_snapshot_not_found',
    // 가져오려는 zip이 사바쨩이 내보낸 백업이 아니거나, 내용이 목록과 다르거나, 다른 모듈의 서버 것이다
    import_not_backup: 'maintenance.err_import_not_backup',
    import_damaged: 'maintenance.err_import_damaged',
    import_other_module: 'maintenance.err_import_other_module',
    export_old_backup: 'maintenance.err_export_old_backup',
    // 백업 폴더나 그 안의 백업 파일을 읽지 못했다
    backups_unreadable: 'maintenance.err_backups_unreadable',
    schedule_changed: 'maintenance.err_schedule_changed',
    server_running: 'maintenance.restore_stop_first',
};

export const errorText = (res: ApiBody | null | undefined, fallback: string, t: TFunction) => {
    const details = Array.isArray(res?.details) ? res.details.filter(Boolean) : [];
    const known = CODE_TEXT[res?.error_code as string];
    return [known ? t(known, { file: res?.file || '' }) : res?.error || fallback, ...details]
        .filter(Boolean)
        .join('\n');
};
