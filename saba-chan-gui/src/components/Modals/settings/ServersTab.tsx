import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { SabaToggle } from '../../ui/SabaUI';
import { NetTraceSetting, netTraceAvailable } from '../NetTraceSetting';
import { setOption } from './options';

/** 설정 › 서버: 비밀번호 자동 생성·포트 검사·네트워크 측정·새 서버 기본값·활동 기록 보관 */
export function ServersTab() {
    const { t } = useTranslation(['gui', 'common']);
    const autoGeneratePasswords = useSettingsStore((s) => s.autoGeneratePasswords);
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);
    const autoPortCheck = useSettingsStore((s) => s.autoPortCheck);
    const newServerBackupKeep = useSettingsStore((s) => s.newServerBackupKeep);
    const newServerAutoRestart = useSettingsStore((s) => s.newServerAutoRestart);
    const auditKeepDays = useSettingsStore((s) => s.auditKeepDays);
    return (
        <section className="saba-card">
            {/* 비밀번호 자동 생성 설정 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="key" size="sm" />{' '}
                        {t('gui:settings_modal.auto_generate_passwords_label', 'Auto-generate Passwords')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.auto_generate_passwords_description',
                            "If a server's REST password is empty, fill it with a random one when the server starts. Existing passwords are never changed.",
                        )}
                    </span>
                </label>
                <SabaToggle
                    checked={autoGeneratePasswords}
                    onChange={(checked) => useSettingsStore.getState().update({ autoGeneratePasswords: checked })}
                />
            </div>

            {/* 포트 충돌 검사 설정 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="alertCircle" size="sm" />{' '}
                        {t('gui:settings_modal.port_conflict_check_label', 'Port Conflict Detection')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.port_conflict_check_description',
                            'Show warnings when multiple server instances use the same port.',
                        )}
                    </span>
                </label>
                <SabaToggle
                    checked={portConflictCheck}
                    onChange={(checked) => useSettingsStore.getState().update({ portConflictCheck: checked })}
                />
            </div>

            {/* 자동 포트포워딩 확인 (데몬이 서버를 만들 때·켜질 때) */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="globe" size="sm" />{' '}
                        {t('gui:settings_modal.auto_port_check_label', 'Automatic Port Forwarding Check')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.auto_port_check_description',
                            'When a server is created and when Saba-chan starts, quietly check through the relay whether friends can reach your servers.',
                        )}
                    </span>
                </label>
                <SabaToggle
                    checked={autoPortCheck}
                    onChange={(checked) => useSettingsStore.getState().update({ autoPortCheck: checked })}
                />
            </div>

            {/* 포트·서버별 네트워크 (기본 꺼짐, Windows만): 시스템 모니터를 펼쳐 둔 동안 데몬이 센다 */}
            {netTraceAvailable() && <NetTraceSetting />}

            {/* 새 서버 기본값: 서버를 만들 때 데몬이 채운다. 만든 뒤에는 서버마다 바꾼다 */}
            <div className="setting-item setting-item-group">
                <div className="setting-item-row">
                    <label className="setting-label">
                        <span className="setting-title">
                            <Icon name="server" size="sm" /> {t('gui:settings_modal.new_server_defaults_label')}
                        </span>
                        <span className="setting-description">
                            {t('gui:settings_modal.new_server_defaults_description')}
                        </span>
                    </label>
                </div>
                <div className="setting-sublist">
                    <div className="setting-subitem">
                        <span>{t('gui:settings_modal.new_server_backup_keep')}</span>
                        <CustomDropdown
                            className="setting-select"
                            value={newServerBackupKeep}
                            onChange={(val) => setOption({ newServerBackupKeep: Number(val) })}
                            options={[...new Set([3, 5, 10, 20, 30, 50, newServerBackupKeep])]
                                .sort((a, b) => a - b)
                                .map((value) => ({
                                    value,
                                    label: t('gui:settings_modal.new_server_backup_keep_count', {
                                        n: value,
                                    }),
                                }))}
                        />
                    </div>
                    <div className="setting-subitem">
                        <span>{t('gui:settings_modal.new_server_auto_restart')}</span>
                        <SabaToggle
                            size="sm"
                            checked={newServerAutoRestart}
                            onChange={(on) => setOption({ newServerAutoRestart: on })}
                        />
                    </div>
                </div>
            </div>

            {/* 활동 기록 보관 기간: 지난 기록은 데몬이 지운다 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="file" size="sm" /> {t('gui:settings_modal.audit_keep_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.audit_keep_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={auditKeepDays}
                    onChange={(val) => setOption({ auditKeepDays: Number(val) })}
                    options={[...new Set([0, 7, 30, 90, 365, auditKeepDays])]
                        .sort((a, b) => a - b)
                        .map((value) => ({
                            value,
                            label: value
                                ? t('gui:settings_modal.audit_keep_days', { n: value })
                                : t('gui:settings_modal.audit_keep_all'),
                        }))}
                />
            </div>
        </section>
    );
}
