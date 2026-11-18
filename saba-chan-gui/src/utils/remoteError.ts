import type { TFunction } from 'i18next';
import type { ApiBody } from '../types/daemon';

// 데몬 원격 데몬 오류 코드 중 문구가 있는 것 (01 §1.4.2)
const ERRORS = [
    'certificate_failed',
    'port_in_use',
    'bind_failed',
    'invalid_port',
    'invalid_address',
    'invalid_name',
    'remote_not_found',
    'remote_no_endpoint',
    'remote_unreachable',
    'remote_rejected',
    'remote_unknown',
    'relay_not_paired',
    'relay_unreachable',
    'remote_timeout',
    'remote_ambiguous',
    'remote_bad_path',
    'remote_unauthorized',
    'remote_local_only',
    'pair_code_not_found',
    'pair_code_used',
    'pair_code_expired',
    'remote_not_ready',
    'ssh_missing',
    'ssh_spawn_failed',
    'ssh_failed',
    'install_failed',
    'pair_code_missing',
    'unsupported',
    'cli_missing',
    'remote_uninstall_failed',
    'invalid_host',
    'invalid_user',
    'invalid_code',
];

/** 실패 → 한 줄 (아는 코드는 번역, 나머지는 데몬 문구) */
export function remoteErrorText(res: ApiBody | null | undefined, t: TFunction) {
    const code = String(res?.error_code || res?.error || '');
    if (ERRORS.includes(code)) return t(`remote_daemon.err_${code}`);
    return res?.error || t('remote_daemon.err_generic');
}
