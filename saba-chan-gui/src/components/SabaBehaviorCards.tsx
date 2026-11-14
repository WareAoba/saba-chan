import { useId } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../stores/useServerStore';
import { memoryLimitState } from '../utils/serverView';
import { CustomDropdown, Icon, SabaToggle } from './index';
import { SabaNumberInput } from './ui/SabaUI';
import { LinkedText } from './ui/ScreenLink';
import { Card, type SettingValues, SettingRow } from './ServerSettingsParts';
import './ServerSettings.css';

/** 사바쨩이 서버를 다루는 설정 카드: 크래시 재시작, 응답 없음 감시, 자동 업데이트, 자동 정지, 램 상한, 게임 채팅, 정지 대기 */
export function SabaBehaviorCards({
    settingsServer,
    settingsValues,
    handleSettingChange,
    handleSaveSettings,
    autoUpdatable,
}: {
    settingsServer: Server;
    settingsValues: SettingValues;
    handleSettingChange: (name: string, value: unknown) => void;
    /** 지금 값을 저장한다 (걸리지 않는 메모리 상한 "비우기") */
    handleSaveSettings?: () => void;
    autoUpdatable: boolean;
}) {
    const { t } = useTranslation('gui');
    const rowId = useId();
    const crashOff = String(settingsValues.auto_restart) === 'false';
    const idleStopOn = String(settingsValues.idle_stop) === 'true';
    const watchAction = String(settingsValues.unresponsive_action ?? 'notify');
    const memoryLimit = memoryLimitState(settingsServer);

    return (
        <>
            {/* 크래시 자동 재시작 (데몬 A1: instance module_settings 의 auto_restart / restart_on_unknown_exit) */}
            <Card icon="refresh" title={t('shell.restart_title')} desc={t('shell.restart_desc')} className="ss-restart">
                <SettingRow
                    label={t('shell.restart_on_crash')}
                    findKey="auto_restart"
                    htmlFor={`${rowId}-crash`}
                    desc={t('shell.restart_on_crash_hint')}
                    control={
                        <SabaToggle
                            id={`${rowId}-crash`}
                            checked={!crashOff}
                            onChange={(checked) => handleSettingChange('auto_restart', checked)}
                        />
                    }
                />
                <SettingRow
                    label={t('shell.restart_on_unknown')}
                    findKey="restart_on_unknown_exit"
                    htmlFor={`${rowId}-unknown`}
                    desc={t('shell.restart_on_unknown_hint')}
                    disabled={crashOff}
                    control={
                        <SabaToggle
                            id={`${rowId}-unknown`}
                            checked={String(settingsValues.restart_on_unknown_exit) === 'true'}
                            disabled={crashOff}
                            onChange={(checked) => handleSettingChange('restart_on_unknown_exit', checked)}
                        />
                    }
                />
            </Card>

            {/* 응답 없는 서버 감시 (데몬 module_settings unresponsive_action·unresponsive_minutes) */}
            <Card
                icon="alertTriangle"
                title={t('server_settings.unresponsive_title')}
                desc={t('server_settings.unresponsive_desc')}
                className="ss-unresponsive"
            >
                <SettingRow
                    label={t('server_settings.unresponsive_action')}
                    findKey="unresponsive_action"
                    control={
                        <CustomDropdown
                            className="ss-select"
                            ariaLabel={t('server_settings.unresponsive_action')}
                            value={watchAction}
                            onChange={(v: string) => handleSettingChange('unresponsive_action', v)}
                            options={(['off', 'notify', 'restart'] as const).map((a) => ({
                                value: a,
                                label: t(`server_settings.unresponsive_action_${a}`),
                            }))}
                        />
                    }
                />
                <SettingRow
                    label={t('server_settings.unresponsive_minutes')}
                    findKey="unresponsive_minutes"
                    desc={t('server_settings.unresponsive_minutes_hint')}
                    disabled={watchAction === 'off'}
                    control={
                        <span className="ss-with-unit">
                            <SabaNumberInput
                                className="input ss-short"
                                min={5}
                                max={1440}
                                value={settingsValues.unresponsive_minutes ?? ''}
                                disabled={watchAction === 'off'}
                                // 자동 정지 카드에도 같은 이름의 칸이 있어, 읽어 주는 이름에는 카드 제목을 붙인다
                                aria-label={`${t('server_settings.unresponsive_title')}: ${t('server_settings.unresponsive_minutes')}`}
                                onChange={(e) => handleSettingChange('unresponsive_minutes', e.target.value)}
                            />
                            <span className="muted">{t('server_settings.idle_stop_unit')}</span>
                        </span>
                    }
                />
            </Card>

            {/* 게임 서버 자동 업데이트 (데몬 인스턴스 auto_update): 설치 방식 익스텐션으로 설치한 서버만 새 빌드를 확인할 수 있다 */}
            {autoUpdatable && (
                <Card icon="download" title={t('server_settings.auto_update_title')} className="ss-auto-update">
                    <SettingRow
                        label={t('server_settings.auto_update_label')}
                        htmlFor={`${rowId}-auto-update`}
                        desc={t('server_settings.auto_update_hint')}
                        control={
                            <SabaToggle
                                id={`${rowId}-auto-update`}
                                checked={String(settingsValues.auto_update) === 'true'}
                                onChange={(checked) => handleSettingChange('auto_update', checked)}
                            />
                        }
                    />
                </Card>
            )}

            {/* 접속자 없을 때 자동 정지 (데몬 module_settings idle_stop·idle_stop_minutes·idle_stop_grace_minutes).
                데몬이 접속자 수를 알 길이 없는 서버(idle_stop_supported 거짓)에서는 고를 수 없다 */}
            <Card icon="moon" title={t('server_settings.idle_stop_title')} desc={t('server_settings.idle_stop_desc')} className="ss-idle-stop">
                <SettingRow
                    label={t('server_settings.idle_stop_label')}
                    findKey="idle_stop"
                    htmlFor={`${rowId}-idle`}
                    disabled={!settingsServer.idle_stop_supported}
                    control={
                        <SabaToggle
                            id={`${rowId}-idle`}
                            checked={settingsServer.idle_stop_supported === true && idleStopOn}
                            disabled={!settingsServer.idle_stop_supported}
                            onChange={(checked) => handleSettingChange('idle_stop', checked)}
                        />
                    }
                />
                {!settingsServer.idle_stop_supported && (
                    <div className="notice tone-info">
                        <Icon name="info" size="sm" />
                        <span>{t('server_settings.idle_stop_unsupported')}</span>
                    </div>
                )}
                {settingsServer.idle_stop_supported &&
                    (
                        [
                            ['idle_stop_minutes', 'idle_stop_minutes', 1],
                            ['idle_stop_grace_minutes', 'idle_stop_grace', 0],
                        ] as const
                    ).map(([key, text, min]) => (
                        <SettingRow
                            key={key}
                            label={t(`server_settings.${text}`)}
                            findKey={key}
                            desc={t(`server_settings.${text}_hint`)}
                            disabled={!idleStopOn}
                            control={
                                <span className="ss-with-unit">
                                    <SabaNumberInput
                                        className="input ss-short"
                                        min={min}
                                        max={1440}
                                        value={settingsValues[key] ?? ''}
                                        disabled={!idleStopOn}
                                        aria-label={t(`server_settings.${text}`)}
                                        onChange={(e) => handleSettingChange(key, e.target.value)}
                                    />
                                    <span className="muted">{t('server_settings.idle_stop_unit')}</span>
                                </span>
                            }
                        />
                    ))}
            </Card>

            {/* 램 상한 (데몬 인스턴스 memory_limit_mb, MiB). 비우면 제한 없음, 다음 시작부터 적용.
                데몬이 상한을 걸 수 없다고 알리면(memory_limit_supported 거짓) 칸을 잠그고, 남은 예전 값은 비우게 한다 */}
            <Card icon="monitor" title={t('memory_limit.title')} desc={t('memory_limit.desc')} className="ss-memory">
                <SettingRow
                    label={t('memory_limit.label')}
                    findKey="memory_limit_mb"
                    htmlFor={`${rowId}-mem`}
                    desc={t(memoryLimit === 'editable' ? 'memory_limit.hint' : 'memory_limit.hint_locked')}
                    disabled={memoryLimit !== 'editable'}
                    control={
                        <input
                            id={`${rowId}-mem`}
                            type="text"
                            inputMode="numeric"
                            className="input ss-short"
                            value={settingsValues.memory_limit_mb ?? ''}
                            placeholder={t('memory_limit.placeholder')}
                            aria-label={t('memory_limit.label')}
                            disabled={memoryLimit !== 'editable'}
                            onChange={(e) => handleSettingChange('memory_limit_mb', e.target.value)}
                        />
                    }
                />
                {memoryLimit === 'locked' && (
                    <div className="notice tone-info">
                        <Icon name="info" size="sm" />
                        <span>{t('memory_limit.locked')}</span>
                    </div>
                )}
                {memoryLimit === 'stale' && (
                    <div className="notice tone-warn">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('memory_limit.stale')}</span>
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => {
                                handleSettingChange('memory_limit_mb', '');
                                handleSaveSettings?.();
                            }}
                        >
                            {t('memory_limit.clear')}
                        </button>
                    </div>
                )}
                {settingsServer.status === 'running' &&
                    String(settingsValues.memory_limit_mb ?? '').trim() !==
                        (settingsServer.memory_limit_mb == null ? '' : String(settingsServer.memory_limit_mb)) && (
                        <p className="dc-hint">{t('memory_limit.restart_note')}</p>
                    )}
            </Card>

            {/* 게임 채팅 전달 (데몬 인스턴스 game_chat·game_chat_keyword). 모듈이 채팅 줄을 알려 주고 데몬이 콘솔을 읽을 때만 */}
            {settingsServer.game_chat_supported && (
                <Card icon="messageSquare" title={t('game_chat.title')} desc={t('game_chat.desc')} className="ss-game-chat" spot="game_chat">
                    <SettingRow
                        label={t('game_chat.mode_label')}
                        findKey="game_chat"
                        control={
                            <CustomDropdown
                                className="ss-select"
                                ariaLabel={t('game_chat.mode_label')}
                                value={settingsValues.game_chat || 'off'}
                                onChange={(val) => handleSettingChange('game_chat', val)}
                                options={['off', 'all', 'keyword'].map((mode) => ({ value: mode, label: t(`game_chat.mode_${mode}`) }))}
                            />
                        }
                    />
                    {settingsValues.game_chat === 'keyword' && (
                        <SettingRow
                            label={t('game_chat.keyword_label')}
                            findKey="game_chat_keyword"
                            htmlFor={`${rowId}-keyword`}
                            desc={t('game_chat.keyword_hint')}
                            control={
                                <input
                                    id={`${rowId}-keyword`}
                                    type="text"
                                    className="input ss-short"
                                    maxLength={32}
                                    value={settingsValues.game_chat_keyword ?? ''}
                                    placeholder={t('game_chat.keyword_placeholder')}
                                    aria-label={t('game_chat.keyword_label')}
                                    onChange={(e) => handleSettingChange('game_chat_keyword', e.target.value)}
                                />
                            }
                        />
                    )}
                    <p className="dc-hint">
                        <LinkedText i18nKey="game_chat.channel_note" to="discord/*/moderation/channels" />
                    </p>
                </Card>
            )}

            {/* 정지 대기 시간 (데몬 MK14 module_settings.stop_timeout, 초). 비우면 모듈 값, 지나면 강제로 끈다.
                데몬이 띄우지 않는 서버(managed:false)는 모듈 stop()이 정지를 맡아 이 값을 쓰지 않는다 */}
            {settingsServer.managed !== false && (
                <Card icon="power" title={t('stop_timeout.title')} desc={t('stop_timeout.desc')} className="ss-stop-timeout">
                    <SettingRow
                        label={t('stop_timeout.label')}
                        findKey="stop_timeout"
                        htmlFor={`${rowId}-stop`}
                        desc={t('stop_timeout.hint')}
                        control={
                            <input
                                id={`${rowId}-stop`}
                                type="text"
                                inputMode="numeric"
                                className="input ss-short"
                                value={settingsValues.stop_timeout ?? ''}
                                placeholder={t('stop_timeout.placeholder')}
                                aria-label={t('stop_timeout.label')}
                                onChange={(e) => handleSettingChange('stop_timeout', e.target.value)}
                            />
                        }
                    />
                </Card>
            )}
        </>
    );
}

