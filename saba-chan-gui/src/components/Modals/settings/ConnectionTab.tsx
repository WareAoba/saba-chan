import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useDiscordStore } from '../../../stores/useDiscordStore';
import { DEFAULT_IPC_PORT } from '../../../stores/useSettingsStore';
import { AgentLink, agentLinkAvailable } from '../../discord/AgentLink';
import { Icon } from '../../Icon';
import { SabaNumberInput } from '../../ui/SabaUI';
import { usePendingChange } from '../../ui/SaveBar';
import { RemoteDaemonSection, remoteDaemonAvailable } from '../RemoteDaemonSection';

interface ConnectionTabProps {
    ipcPort: number;
    /** 앱을 켤 때의 IPC 포트 (바꾼 포트는 다시 켜야 적용된다) */
    runningIpcPort: number;
    onIpcPortSave: (port: number) => void;
    discordCloudRelayUrl?: string;
    onDiscordCloudRelayUrlChange?: (value: string) => void;
}

/** 저장된 값을 따라가는 입력 초안: 손대지 않은 동안 저장된 값이 바뀌면(늦게 읽힘·저장) 초안도 바뀐다 */
function useFollowingDraft(saved: string) {
    const [draft, setDraft] = useState(saved);
    const [base, setBase] = useState(saved);
    if (base !== saved) {
        setBase(saved);
        if (draft === base) setDraft(saved);
    }
    return [draft, setDraft] as const;
}

/** 받을 수 있는 IPC 포트면 숫자, 아니면 null */
function ipcPortOf(value: string) {
    const port = Number(value);
    return Number.isInteger(port) && port >= 1024 && port <= 65535 ? port : null;
}

/** 설정 › 연결: 원격 데몬·AI 에이전트·IPC 포트·릴레이 주소 */
export function ConnectionTab({
    ipcPort,
    runningIpcPort,
    onIpcPortSave,
    discordCloudRelayUrl,
    onDiscordCloudRelayUrlChange,
}: ConnectionTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    const discordHostId = useDiscordStore((s) => s.discordCloudHostId);

    // 포트·주소는 다 친 뒤에 적용해야 하므로 즉시 저장하지 않고 저장 막대로 묻는다
    const [portDraft, setPortDraft] = useFollowingDraft(String(ipcPort));
    const draftPort = ipcPortOf(portDraft);
    const ipcPortError = draftPort === null ? t('gui:settings_modal.ipc_port_invalid') : '';
    usePendingChange('ipc-port', {
        dirty: portDraft !== String(ipcPort),
        blocked: draftPort === null,
        save: () => draftPort !== null && onIpcPortSave(draftPort),
        revert: () => setPortDraft(String(ipcPort)),
    });
    const ipcPortChanged = ipcPort !== runningIpcPort && portDraft === String(ipcPort);

    const savedRelay = discordCloudRelayUrl || '';
    const [relayDraft, setRelayDraft] = useFollowingDraft(savedRelay);
    usePendingChange('relay-url', {
        dirty: relayDraft.trim() !== savedRelay,
        save: () => onDiscordCloudRelayUrlChange?.(relayDraft.trim()),
        revert: () => setRelayDraft(savedRelay),
    });
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
                    value={portDraft}
                    onChange={(e) => setPortDraft(e.target.value)}
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
                    value={relayDraft}
                    onChange={(e) => setRelayDraft(e.target.value)}
                    placeholder="https://relay.saba-chan.app"
                />
            </div>
        </>
    );
}
