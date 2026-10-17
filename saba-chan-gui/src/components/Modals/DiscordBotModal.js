
import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import InstanceSettingsModal from './InstanceSettingsModal';

// ── 릴레이 서버 기본 URL (고급 설정에서 오버라이드 가능) ──
const DEFAULT_RELAY_URL = 'https://saba-chan.online';

function DiscordBotModal({
    isOpen,
    onClose,
    isClosing,
    displayMode = 'popup',
    discordPrefix,
    setDiscordPrefix,
    discordCloudRelayUrl,
    discordCloudHostId,
    setDiscordCloudHostId,
    relayConnected,
    relayStatusChecked,
    saveCurrentSettings,
    servers,
    moduleAliasesPerModule,
    nodeSettings,
    setNodeSettings,
    instanceSettings,
    setInstanceSettings,
    cloudNodes,
    setCloudNodes,
    cloudMembers,
    setCloudMembers,
}) {
    const { t } = useTranslation('gui');
    // 로컬 봇 기능이 제거되어 항상 클라우드(릴레이) 모드.

    // ── 릴레이 URL 결정 (커스텀 > 기본값) ──
    const effectiveRelayUrl = discordCloudRelayUrl || DEFAULT_RELAY_URL;

    // ── 연결 상태 (훅에서 전달받은 릴레이 상태 사용) ──
    const cloudConnected = relayConnected ?? false;
    // '연결 중' 표시는 첫 상태 확인 전에만 — 이후 5초 폴링의 relayConnecting 토글로
    // 미연결 상태 배지가 '연결 중 ↔ 끊김'으로 깜빡이는 것을 방지
    const cloudChecking = !cloudConnected && !relayStatusChecked;
    const [cloudError, setCloudError] = useState('');

    // ── 노드 UI 상태 (App에 저장할 필요 없는 일시적 UI 상태) ──
    const [expandedNode, setExpandedNode] = useState(null);

    // ── 길드 멤버 로딩 상태 ──
    const [membersLoading, setMembersLoading] = useState(false);
    const [membersError, setMembersError] = useState('');

    // ── 서버별(인스턴스별) 설정 팝업 ──
    const [showInstanceSettings, setShowInstanceSettings] = useState(false);

    // ── 페어링 상태 ──
    const [showPairing, setShowPairing] = useState(false);
    const [pairCode, setPairCode] = useState('');
    const [pairStatus, setPairStatus] = useState('idle');
    const [pairExpiresAt, setPairExpiresAt] = useState(null);
    const [pairRemaining, setPairRemaining] = useState(0);
    const [pairCopied, setPairCopied] = useState(false);
    const pairPollRef = useRef(null);
    const pairTimerRef = useRef(null);
    const syncNodeMusicSettings = useDiscordStore((s) => s.syncNodeMusicSettings);
    const updateNodeMusicSettings = useDiscordStore((s) => s.updateNodeMusicSettings);
    const checkRelayStatus = useDiscordStore((s) => s.checkStatus);

    // ── 페어링 타이머 & 폴링 클린업 ──
    useEffect(() => {
        return () => {
            if (pairPollRef.current) clearInterval(pairPollRef.current);
            if (pairTimerRef.current) clearInterval(pairTimerRef.current);
        };
    }, []);

    // 카운트다운 타이머
    useEffect(() => {
        if (pairStatus !== 'waiting' || !pairExpiresAt) return;
        const tick = () => {
            const remaining = Math.max(0, Math.floor((new Date(pairExpiresAt).getTime() - Date.now()) / 1000));
            setPairRemaining(remaining);
            if (remaining <= 0) {
                setPairStatus('expired');
                if (pairPollRef.current) clearInterval(pairPollRef.current);
                if (pairTimerRef.current) clearInterval(pairTimerRef.current);
            }
        };
        tick();
        pairTimerRef.current = setInterval(tick, 1000);
        return () => {
            if (pairTimerRef.current) clearInterval(pairTimerRef.current);
        };
    }, [pairStatus, pairExpiresAt]);

    // ── 모달 열릴 때 일시적 UI 상태 초기화 ──
    useEffect(() => {
        if (isOpen) {
            if (pairPollRef.current) {
                clearInterval(pairPollRef.current);
                pairPollRef.current = null;
            }
            if (pairTimerRef.current) {
                clearInterval(pairTimerRef.current);
                pairTimerRef.current = null;
            }
            setPairStatus('idle');
            setPairCode('');
            setPairExpiresAt(null);
            setPairRemaining(0);
            setShowPairing(false);
            setPairCopied(false);
            setMembersError('');
        }
    }, [isOpen]);

    // ══════════════════════════════════════════════
    // ── 길드 멤버 가져오기 ──
    // ══════════════════════════════════════════════

    /** 클라우드모드: 릴레이 서버 봇을 통해 디스코드 길드 멤버 가져오기 */
    const fetchCloudNodeMembers = useCallback(
        async (guildId) => {
            setMembersLoading(true);
            setMembersError('');
            try {
                // 데몬 프록시를 통해 멤버 조회 (discord-members → members 폴백은 데몬이 처리)
                const data = await window.api.relayListNodeMembers(guildId, effectiveRelayUrl);
                // 정상 응답은 배열 또는 {members: []} — 그 외는 에러 응답이므로
                // 기존 멤버 캐시를 빈 배열로 덮어쓰지 않고 에러로 처리한다.
                const members = Array.isArray(data) ? data : Array.isArray(data?.members) ? data.members : null;
                if (!members) {
                    throw new Error(data?.error || 'Invalid members response');
                }
                setCloudMembers((prev) => ({
                    ...prev,
                    [guildId]: members,
                }));
            } catch (e) {
                setMembersError(t('discord_modal.members_load_failed'));
                console.warn('[DiscordBotModal] Failed to fetch cloud members:', e);
            } finally {
                setMembersLoading(false);
            }
        },
        [effectiveRelayUrl, setCloudMembers, t],
    );

    // ══════════════════════════════════════════════
    // ── nodeSettings 헬퍼 함수들 ──
    // ══════════════════════════════════════════════

    /** 특정 노드의 설정 가져오기 (없으면 기본값) — 노드 단위 설정은 음악 전용 */
    const getNodeConfig = useCallback(
        (nodeKey) => {
            const cfg = nodeSettings[nodeKey];
            return {
                musicEnabled: true,
                musicUIEnabled: true,
                musicSettings: { queueLines: 5, refreshInterval: 4000, normalize: true },
                ...(cfg || {}),
            };
        },
        [nodeSettings],
    );

    const toggleNodeMusicOption = useCallback(
        async (nodeKey, key, value) => {
            const prevValue = nodeSettings[nodeKey]?.[key];
            setNodeSettings((current) => ({
                ...current,
                [nodeKey]: { ...(current[nodeKey] || {}), [key]: value },
            }));

            const result = await updateNodeMusicSettings(nodeKey, { [key]: value }, effectiveRelayUrl);
            if (!result?.success && result?.error) {
                setNodeSettings((current) => ({
                    ...current,
                    [nodeKey]: { ...(current[nodeKey] || {}), [key]: prevValue },
                }));
                if (window.showToast) window.showToast(result.error, 'error');
            }
        },
        [effectiveRelayUrl, nodeSettings, setNodeSettings, updateNodeMusicSettings],
    );

    // ══════════════════════════════════════════════
    // ── 클라우드 노드 목록 로드 (연결 상태는 훅에서 관리) ──
    // ══════════════════════════════════════════════
    const loadCloudNodes = useCallback(async () => {
        if (!discordCloudHostId) return;
        setCloudError('');
        try {
            // 노드 목록 로드 (데몬 프록시 경유)
            const nodesData = await window.api.relayListHostNodes(discordCloudHostId, effectiveRelayUrl);
            // 릴레이는 항상 배열을 반환한다 — 배열이 아니면 에러 응답이므로
            // 기존 노드 목록을 빈 배열로 덮어쓰지 않고 에러로 처리한다.
            if (!Array.isArray(nodesData)) {
                throw new Error(nodesData?.error || 'Invalid nodes response');
            }
            setCloudNodes(nodesData);
            await Promise.all(
                nodesData.map((node) =>
                    syncNodeMusicSettings(node.guildId, effectiveRelayUrl).catch((err) => {
                        console.warn('[DiscordBotModal] Node music settings fetch failed:', node.guildId, err);
                    }),
                ),
            );
        } catch (e) {
            setCloudError(t('errors.network_error', { defaultValue: 'Connection failed' }));
            console.error('[DiscordBotModal] Cloud nodes fetch error:', e.message);
        }
    }, [discordCloudHostId, effectiveRelayUrl, setCloudNodes, syncNodeMusicSettings, t]);

    // 모달 열릴 때 + 연결 확인 시 노드 로드
    useEffect(() => {
        if (isOpen && discordCloudHostId && cloudConnected) {
            loadCloudNodes();
        }
    }, [isOpen, discordCloudHostId, cloudConnected, loadCloudNodes]);

    // ── 노드 확장 ──
    // 멤버 로드는 서버별 설정 팝업(InstanceSettingsModal)이 필요 시 수행한다.
    const toggleNodeExpand = useCallback((guildId) => {
        setExpandedNode((prev) => (prev === guildId ? null : guildId));
    }, []);

    /** 서버별 설정 팝업에서 멤버를 요청할 때 — 캐시가 없을 때만 릴레이 조회 (force로 강제 새로고침) */
    const fetchNodeMembersCached = useCallback(
        (guildId, force = false) => {
            if (force || !(cloudMembers[guildId]?.length > 0)) {
                fetchCloudNodeMembers(guildId);
            }
        },
        [cloudMembers, fetchCloudNodeMembers],
    );

    // ── 페어링 ──
    const startPairing = useCallback(async () => {
        try {
            setPairStatus('idle');
            const data = await window.api.relayInitiatePairing({ relayUrl: effectiveRelayUrl });
            if (data.error) throw new Error(data.error);
            setPairCode(data.code);
            setPairExpiresAt(data.expiresAt);
            setPairStatus('waiting');
            setPairCopied(false);

            // 폴링 시작 (데몬 프록시 경유)
            if (pairPollRef.current) clearInterval(pairPollRef.current);
            const secret = data.pollSecret;
            pairPollRef.current = setInterval(async () => {
                try {
                    const s = await window.api.relayPollPairingStatus(data.code, secret, effectiveRelayUrl);
                    if (s.error) throw new Error(s.error);
                    if (s.status === 'claimed') {
                        clearInterval(pairPollRef.current);
                        pairPollRef.current = null;
                        setPairStatus('success');
                        if (s.hostId) setDiscordCloudHostId(s.hostId);
                        if (s.nodeToken && window.api?.saveNodeToken) {
                            try {
                                await window.api.saveNodeToken(s.nodeToken);
                                console.log('[Pairing] Node token saved successfully');
                            } catch (e) {
                                console.error('[Pairing] Failed to save node token:', e);
                            }
                        }
                        // 성공 메시지 잠시 표시 후 자동 전환
                        // ★ checkCloudConnection()을 직접 호출하면 stale closure 문제 발생
                        //   → useEffect가 discordCloudHostId 변경 감지 후 자동 실행
                        setTimeout(() => {
                            saveCurrentSettings();
                            setPairStatus('idle');
                            setPairCode('');
                            setShowPairing(false);
                            if (pairTimerRef.current) {
                                clearInterval(pairTimerRef.current);
                                pairTimerRef.current = null;
                            }
                        }, 2000);
                    } else if (s.status === 'expired') {
                        clearInterval(pairPollRef.current);
                        pairPollRef.current = null;
                        setPairStatus('expired');
                    }
                } catch {
                    /* 네트워크 에러 — 폴링 계속 */
                }
            }, 3000);
        } catch (e) {
            console.error('[Pairing] initiate failed:', e);
            setPairStatus('error');
        }
    }, [effectiveRelayUrl, setDiscordCloudHostId, saveCurrentSettings]);

    const copyPairCode = useCallback(() => {
        if (!pairCode) return;
        navigator.clipboard.writeText(pairCode).then(() => {
            setPairCopied(true);
            setTimeout(() => setPairCopied(false), 2000);
        });
    }, [pairCode]);

    const resetPairing = useCallback(() => {
        if (pairPollRef.current) clearInterval(pairPollRef.current);
        if (pairTimerRef.current) clearInterval(pairTimerRef.current);
        setPairCode('');
        setPairStatus('idle');
        setPairExpiresAt(null);
        setPairRemaining(0);
        setShowPairing(false);
    }, []);

    // ── 연결 초기화 ──
    const disconnectCloud = useCallback(() => {
        resetPairing();
        setDiscordCloudHostId('');
        setCloudError('');
        setCloudNodes([]);
        setExpandedNode(null);
        setCloudMembers({});
        // 페어링 토큰(.node_token)도 삭제해야 데몬-릴레이 연결이 실제로 끊긴다.
        // (토큰이 남으면 데몬은 계속 페어링 상태로 폴링하고, 데몬의 hostId
        //  자가치유가 다음 로드에서 연결 표시를 되살린다)
        window.api?.deleteNodeToken?.()?.catch?.(() => {});
    }, [resetPairing, setDiscordCloudHostId, setCloudNodes, setCloudMembers]);

    // ══════════════════════════════════════════════
    // ── 노드 설정 Body 렌더링 (노드 단위 설정 = 음악 전용) ──
    // 인스턴스 허용/유저 권한은 "서버별 설정" 팝업(InstanceSettingsModal)으로 이관됨.
    // ══════════════════════════════════════════════
    const renderNodeSettingsBody = (nodeKey, _nodeLabel) => {
        const cfg = getNodeConfig(nodeKey);
        const musicEnabled = cfg.musicEnabled !== false;
        const musicUIEnabled = cfg.musicUIEnabled !== false;

        return (
            <div className="discord-node-settings-body">
                <div className="discord-node-music-controls">
                    <div className="discord-node-music-control-row">
                        <div className="discord-node-music-control-copy">
                            <strong>{t('discord_modal.node_music_enabled', { defaultValue: 'Music' })}</strong>
                            <small>
                                {t('discord_modal.node_music_enabled_desc', { defaultValue: 'Voice music commands' })}
                            </small>
                        </div>
                        <SabaToggle
                            size="sm"
                            checked={musicEnabled}
                            onChange={(checked) => toggleNodeMusicOption(nodeKey, 'musicEnabled', checked)}
                        />
                    </div>
                    <div className="discord-node-music-control-row">
                        <div className="discord-node-music-control-copy">
                            <strong>{t('discord_modal.node_music_ui_enabled', { defaultValue: 'Player UI' })}</strong>
                            <small>
                                {t('discord_modal.node_music_ui_enabled_desc', {
                                    defaultValue: 'Dedicated channel controls',
                                })}
                            </small>
                        </div>
                        <SabaToggle
                            size="sm"
                            checked={musicUIEnabled}
                            onChange={(checked) => toggleNodeMusicOption(nodeKey, 'musicUIEnabled', checked)}
                        />
                    </div>
                </div>
            </div>
        );
    };

    if (!isOpen) return null;

    // ── 클라우드 모드 상태 머신 ──
    // no_host → pairing → pair_success → connecting → connected
    //                                               → error (상태 확인 완료 후에도 미연결)
    let cloudState;
        if (pairStatus === 'success') {
            cloudState = 'pair_success';
        } else if (showPairing && !cloudConnected) {
            cloudState = 'pairing';
        } else if (!discordCloudHostId) {
            cloudState = 'no_host';
        } else if (cloudConnected) {
            cloudState = 'connected';
        } else if (!relayStatusChecked) {
            // 아직 첫 상태 확인 전 → 스피너
            // (첫 확인 후에는 5초 폴링의 relayConnecting 토글로 스피너 ↔ 오류 카드가
            //  번갈아 깜빡이지 않도록, 재확인 중에도 오류 카드를 유지한다)
            cloudState = 'connecting';
        } else {
            // 상태 확인이 끝났는데 연결 안 됨 → 무한 스피너 대신 오류 카드
            cloudState = 'error';
        }

    // ── 인라인 페어링 블록 ──
    const renderPairingBlock = () => (
        <div className="discord-pair-section" style={{ marginTop: 12 }}>
            {pairStatus === 'idle' && (
                <div className="discord-cloud-connecting">
                    <SabaSpinner size="sm" />
                    <span>{t('discord_modal.cloud_connecting')}</span>
                </div>
            )}
            {pairStatus === 'waiting' && pairCode && (
                <div className="discord-pair-code-area">
                    <span className="discord-pair-code-label">{t('discord_modal.pair_code_label')}</span>
                    <div className="discord-pair-code-row">
                        <span className="discord-pair-code-value">{pairCode}</span>
                        <button type="button"
                            className={clsx('discord-pair-copy-btn', { copied: pairCopied })}
                            onClick={copyPairCode}
                        >
                            {pairCopied
                                ? `✓ ${t('discord_modal.pair_code_copied')}`
                                : `📋 ${t('discord_modal.pair_copy_button')}`}
                        </button>
                    </div>
                    <p className="discord-pair-instruction">{t('discord_modal.pair_instruction')}</p>
                    <code className="discord-pair-command">/사바쨩 연결 코드:{pairCode}</code>
                    <div className="discord-pair-waiting">
                        <SabaSpinner size="sm" />
                        <span>{t('discord_modal.pair_waiting')}</span>
                        <span className="discord-pair-timer">
                            {t('discord_modal.pair_expires_in', { seconds: pairRemaining })}
                        </span>
                    </div>
                </div>
            )}
            {pairStatus === 'success' && (
                <div className="discord-pair-result success">
                    ✅ {t('discord_modal.pair_success')}
                    <div className="discord-cloud-connecting" style={{ marginTop: 8, justifyContent: 'center' }}>
                        <SabaSpinner size="sm" />
                        <span>{t('discord_modal.cloud_connecting')}</span>
                    </div>
                </div>
            )}
            {pairStatus === 'expired' && (
                <div className="discord-pair-result error">
                    ⏰ {t('discord_modal.pair_expired')}
                    <button type="button"
                        className="discord-pair-start-btn"
                        style={{ marginLeft: 12, fontSize: 12, padding: '4px 12px' }}
                        onClick={() => {
                            resetPairing();
                            setShowPairing(true);
                            setTimeout(startPairing, 100);
                        }}
                    >
                        🔄 {t('discord_modal.cloud_retry')}
                    </button>
                </div>
            )}
            {pairStatus === 'error' && (
                <div className="discord-pair-result error">
                    ❌ {t('discord_modal.pair_error')}
                    <button type="button"
                        className="discord-pair-start-btn"
                        style={{ marginLeft: 12, fontSize: 12, padding: '4px 12px' }}
                        onClick={() => {
                            resetPairing();
                            setShowPairing(true);
                            setTimeout(startPairing, 100);
                        }}
                    >
                        🔄 {t('discord_modal.cloud_retry')}
                    </button>
                </div>
            )}
        </div>
    );

    const isSide = displayMode === 'side';

    return (
        <>
            {/* 팝업 모드 백드롭 — 반드시 모달의 형제(헤더 내부)로 렌더링해야
            모달(z-2000)이 백드롭(z-1999) 위에 온다. App 최상위에 깔면 헤더
            스태킹 컨텍스트(z-10) 전체가 백드롭 아래로 들어가 모달이 차단된다. */}
            {!isSide && <button aria-label={t('modals.close')} type="button"  className="discord-backdrop" onClick={(event) => { if (event.target === event.currentTarget) (onClose)(event); }} />}
            <div
                className={clsx('discord-modal-container', { closing: isClosing, 'side-panel': isSide })}

            >
                <div className="discord-modal-header">
                    <div className="discord-modal-title">
                        <span
                            className={clsx('status-indicator', {
                                'status-online': cloudConnected,
                                'status-connecting': cloudChecking,
                                'status-offline': !cloudConnected && !cloudChecking,
                            })}
                        ></span>
                        <h2>{t('discord_modal.title')}</h2>
                    </div>
                    <button type="button" className="discord-modal-close" onClick={onClose}>
                        <Icon name="close" size="sm" />
                    </button>
                </div>

                <div className="discord-modal-content">
                    {/* ── 상태 표시 ── */}
                    <div className="discord-status-section">
                        <div className="discord-status-rows">
                            <div className="discord-status-row">
                                <span className="status-label">{t('discord_modal.status_cloud_label')}</span>
                                <span
                                    className={clsx('status-value', {
                                        'status-running': cloudConnected,
                                        'status-connecting': cloudChecking,
                                        'status-needs-setup': !cloudConnected && !cloudChecking && !discordCloudHostId,
                                        'status-stopped': !cloudConnected && !cloudChecking && discordCloudHostId,
                                    })}
                                >
                                    {cloudConnected
                                        ? t('discord_modal.status_relay_connected')
                                        : cloudChecking
                                          ? t('discord_modal.status_relay_connecting')
                                          : !discordCloudHostId
                                            ? t('discord_modal.status_relay_needs_setup')
                                            : t('discord_modal.status_relay_disconnected')}
                                </span>
                            </div>
                        </div>
                        <span className="discord-mode-badge cloud">
                            <Icon name="cloud" size="sm" /> {t('discord_modal.mode_cloud')}
                        </span>
                    </div>

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드 모드: 길드(노드) 설정 ───────────── */}
                    {/* ══════════════════════════════════════════════ */}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 페어링 성공 (자동 전환 대기) ── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'pair_success' && (
                        <div className="discord-cloud-section">{renderPairingBlock()}</div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 호스트 미설정 → 셋업 카드 ───── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'no_host' && (
                        <div className="discord-cloud-section">
                            <div className="discord-cloud-setup-card">
                                <div className="discord-cloud-setup-icon">🔗</div>
                                <h4>{t('discord_modal.cloud_setup_title')}</h4>
                                <p>{t('discord_modal.cloud_setup_desc_simple')}</p>

                                <button type="button"
                                    className="discord-pair-start-btn"
                                    style={{ width: '100%', marginTop: 8 }}
                                    onClick={() => {
                                        setShowPairing(true);
                                        startPairing();
                                    }}
                                >
                                    🔗 {t('discord_modal.pair_start_button')}
                                </button>
                            </div>
                        </div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 페어링 진행 중 ────────────────── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'pairing' && (
                        <div className="discord-cloud-section">
                            {renderPairingBlock()}
                            <button type="button"
                                className="discord-pair-start-btn discord-btn-secondary"
                                style={{ marginTop: 8, width: '100%', fontSize: 12 }}
                                onClick={resetPairing}
                            >
                                ← {t('discord_modal.back_to_setup')}
                            </button>
                        </div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 연결 중 ──────────────────────── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'connecting' && (
                        <div className="discord-cloud-section">
                            <div className="discord-cloud-connecting">
                                <SabaSpinner size="sm" />
                                <span>{t('discord_modal.cloud_connecting')}</span>
                            </div>
                        </div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 연결 오류 ────────────────────── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'error' && (
                        <div className="discord-cloud-section">
                            <div className="discord-cloud-error-card">
                                <div className="discord-cloud-error-icon">⚠️</div>
                                <div className="discord-cloud-error-body">
                                    <strong>{t('discord_modal.cloud_connection_failed_title')}</strong>
                                    <p>
                                        {t('discord_modal.cloud_connection_error', {
                                            error: cloudError || t('discord_modal.status_relay_disconnected'),
                                        })}
                                    </p>
                                    <small className="discord-cloud-error-hint">
                                        Host ID: {discordCloudHostId} → {effectiveRelayUrl}
                                    </small>
                                </div>
                                <div className="discord-cloud-error-actions">
                                    <button type="button"
                                        className="discord-pair-start-btn"
                                        onClick={() => {
                                            setCloudError('');
                                            // 재연결 확인 — 연결이 복구되면 useEffect가 노드 목록을 다시 로드한다
                                            checkRelayStatus();
                                        }}
                                    >
                                        🔄 {t('discord_modal.cloud_retry')}
                                    </button>
                                    <button type="button"
                                        className="discord-pair-start-btn"
                                        onClick={() => {
                                            setShowPairing(true);
                                            startPairing();
                                        }}
                                    >
                                        🔗 {t('discord_modal.cloud_re_pair')}
                                    </button>
                                    <button type="button"
                                        className="discord-pair-start-btn discord-btn-danger"
                                        onClick={disconnectCloud}
                                    >
                                        🗑️ {t('discord_modal.cloud_disconnect')}
                                    </button>
                                </div>
                            </div>
                        </div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 클라우드: 연결 완료 → 노드 카드 ──── */}
                    {/* ══════════════════════════════════════════════ */}
                    {cloudState === 'connected' && (
                        <div className="discord-cloud-section">
                            <div className="discord-cloud-connected-banner">
                                <span className="discord-cloud-connected-icon">✅</span>
                                <div>
                                    <strong>{t('discord_modal.cloud_connected_title')}</strong>
                                    <span className="discord-cloud-host-id">Host: {discordCloudHostId}</span>
                                </div>
                                <div style={{ display: 'flex', gap: 4, marginLeft: 'auto' }}>
                                    <button type="button"
                                        className="discord-pair-start-btn"
                                        style={{ fontSize: 11, padding: '3px 10px' }}
                                        onClick={loadCloudNodes}
                                    >
                                        🔄
                                    </button>
                                    <button type="button"
                                        className="discord-pair-start-btn discord-btn-danger"
                                        style={{ fontSize: 11, padding: '3px 10px' }}
                                        onClick={disconnectCloud}
                                        title={t('discord_modal.cloud_disconnect')}
                                    >
                                        <Icon name="cloudOff" size="sm" />
                                    </button>
                                </div>
                            </div>

                            {/* 노드 카드 목록 */}
                            {cloudNodes.length > 0 && (
                                <div className="discord-node-list">
                                    <h4>
                                        📡 {t('discord_modal.cloud_nodes_title')} ({cloudNodes.length})
                                    </h4>
                                    {cloudNodes.map((node) => (
                                        <div
                                            key={node.guildId}
                                            className={clsx('discord-node-card', {
                                                expanded: expandedNode === node.guildId,
                                            })}
                                        >
                                            <button type="button"
                                                className="discord-node-card-header"
                                                onClick={() => toggleNodeExpand(node.guildId)}
                                            >
                                                <div className="discord-node-card-info">
                                                    <span className="discord-node-guild-name">
                                                        {node.guildName || node.guildId}
                                                    </span>
                                                    <span className="discord-node-guild-id">{node.guildId}</span>
                                                </div>
                                                <Icon
                                                    name={
                                                        expandedNode === node.guildId ? 'chevronDown' : 'chevronRight'
                                                    }
                                                    size="sm"
                                                />
                                            </button>

                                            {expandedNode === node.guildId && (
                                                <div className="discord-node-card-body">
                                                    {renderNodeSettingsBody(
                                                        node.guildId,
                                                        node.guildName || node.guildId,
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                    ))}
                                </div>
                            )}

                            {/* 노드 목록 로드 실패 — 기존 목록은 유지하고 경고만 표시 */}
                            {cloudError && (
                                <div className="discord-cloud-empty-nodes">
                                    <p>⚠️ {t('discord_modal.cloud_nodes_load_failed')}</p>
                                    <button type="button"
                                        className="discord-instance-select-btn"
                                        style={{ marginTop: 4 }}
                                        onClick={loadCloudNodes}
                                    >
                                        🔄 {t('discord_modal.cloud_retry')}
                                    </button>
                                </div>
                            )}

                            {/* 서버별(인스턴스별) 설정 — 별명/노드/유저 권한 팝업 */}
                            <button type="button"
                                className="discord-pair-start-btn"
                                style={{ marginTop: 8, width: '100%' }}
                                onClick={() => setShowInstanceSettings(true)}
                            >
                                🖥️ {t('discord_modal.is_button', { defaultValue: 'Per-instance settings' })}
                            </button>

                            {/* 노드 추가 버튼 (항상 표시) */}
                            <button type="button"
                                className="discord-pair-start-btn"
                                style={{ marginTop: 8, width: '100%' }}
                                onClick={() => {
                                    if (showPairing) {
                                        resetPairing();
                                    } else {
                                        setShowPairing(true);
                                        startPairing();
                                    }
                                }}
                            >
                                {showPairing
                                    ? `✕ ${t('discord_modal.pair_section_title')}`
                                    : `➕ ${t('discord_modal.cloud_add_node')}`}
                            </button>
                            {showPairing && renderPairingBlock()}
                        </div>
                    )}

                    {/* ══════════════════════════════════════════════ */}
                    {/* ── 공통 설정 섹션 ─────────────────────────── */}
                    {/* ══════════════════════════════════════════════ */}
                    <div className="discord-config-section">
                        <div className="discord-form-group">
                            <label htmlFor="discordbotmodal-783">{t('discord_modal.prefix_label')}</label>
                            <input id="discordbotmodal-783"
                                type="text"
                                placeholder={t('discord_modal.prefix_placeholder')}
                                value={discordPrefix}
                                onChange={(e) => setDiscordPrefix(e.target.value)}
                                className="discord-input"
                            />
                            <small>{t('discord_modal.prefix_description')}</small>
                            {!discordPrefix && (
                                <small className="warning-text">{t('discord_modal.prefix_warning')}</small>
                            )}
                        </div>
                    </div>

                    <div className="discord-info-box">
                        <h4>
                            <Icon name="lightbulb" size="sm" /> {t('discord_modal.usage_title')}
                        </h4>
                        <p>{t('discord_modal.usage_instruction')}</p>
                        <code>{discordPrefix || '!saba'} [module] [command]</code>
                        <p className="info-note">{t('discord_modal.usage_note')}</p>
                    </div>

                        <div className="discord-cloud-music-notice">
                            <span>🎵</span>
                            <span>{t('discord_modal.music_cloud_notice')}</span>
                        </div>
                </div>

                <div className="discord-modal-footer">
                    <button type="button" className="discord-btn discord-btn-save" onClick={saveCurrentSettings}>
                        {t('discord_modal.save_button')}
                    </button>
                </div>
            </div>

            {/* 서버별(인스턴스별) 설정 팝업 — 포털로 전체 화면 위에 렌더링 */}
            {showInstanceSettings && (
                <InstanceSettingsModal
                    onClose={() => setShowInstanceSettings(false)}
                    servers={servers}
                    moduleAliasesPerModule={moduleAliasesPerModule}
                    instanceSettings={instanceSettings}
                    setInstanceSettings={setInstanceSettings}
                    cloudNodes={cloudNodes}
                    cloudMembers={cloudMembers}
                    fetchNodeMembers={fetchNodeMembersCached}
                    membersLoading={membersLoading}
                    membersError={membersError}
                />
            )}
        </>
    );
}

export default DiscordBotModal;
