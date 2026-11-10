import { useTranslation } from 'react-i18next';
import { useDiscordStore } from '../../../stores/useDiscordStore';
import { DEFAULT_IPC_PORT } from '../../../stores/useSettingsStore';
import { AgentLink, agentLinkAvailable } from '../../discord/AgentLink';
import { Icon } from '../../Icon';
import { SabaNumberInput } from '../../ui/SabaUI';
import { RemoteDaemonSection, remoteDaemonAvailable } from '../RemoteDaemonSection';

interface ConnectionTabProps {
    ipcPort: number;
    ipcPortDraft: string | null;
    ipcPortError: string;
    ipcPortChanged: boolean;
    handleIpcPortChange: (value: string) => void;
    discordCloudRelayUrl?: string;
    onDiscordCloudRelayUrlChange?: (value: string) => void;
}

/** 설정 › 연결: 원격 데몬·AI 에이전트·IPC 포트·릴레이 주소 */
export function ConnectionTab({
    ipcPort,
    ipcPortDraft,
    ipcPortError,
    ipcPortChanged,
    handleIpcPortChange,
    discordCloudRelayUrl,
    onDiscordCloudRelayUrlChange,
}: ConnectionTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    const discordHostId = useDiscordStore((s) => s.discordCloudHostId);
    return (
        <>
            {remoteDaemonAvailable() && <RemoteDaemonSection />}

            {/* AI 에이전트(MCP) 연결: PC 카드와 같은 카드로 보인다 */}
            {discordHostId && agentLinkAvailable() && <AgentLink />}

            {/* IPC 포트 설정 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="hash" size="sm" /> {t('gui:settings_modal.ipc_port_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.ipc_port_description')}</span>
                </label>
                <SabaNumberInput
                    className="input setting-num"
                    aria-invalid={!!ipcPortError || undefined}
                    value={ipcPortDraft ?? ipcPort}
                    onChange={(e) => handleIpcPortChange(e.target.value)}
                    min={1024}
                    max={65535}
                    placeholder={String(DEFAULT_IPC_PORT)}
                />
            </div>
            {ipcPortError && (
                <div className="setting-validation-error">
                    <Icon name="alertCircle" size="sm" /> {ipcPortError}
                </div>
            )}
            {ipcPortChanged && !ipcPortError && (
                <div className="setting-restart-notice">
                    <Icon name="info" size="sm" /> {t('gui:settings_modal.ipc_port_restart_notice')}
                </div>
            )}

            {/* 커스텀 릴레이 서버 URL */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="globe" size="sm" /> {t('gui:settings_modal.relay_url_label', '릴레이 서버 URL')}
                    </span>
                    <span className="setting-description">
                        {t(
                            'gui:settings_modal.relay_url_description',
                            '클라우드 모드에서 사용할 릴레이 서버의 URL입니다. 비어있으면 기본 서버를 사용합니다.',
                        )}
                    </span>
                </label>
                <input
                    type="text"
                    className="input setting-field"
                    value={discordCloudRelayUrl || ''}
                    onChange={(e) => onDiscordCloudRelayUrlChange && onDiscordCloudRelayUrlChange(e.target.value)}
                    placeholder="https://relay.saba-chan.app"
                />
            </div>
        </>
    );
}
