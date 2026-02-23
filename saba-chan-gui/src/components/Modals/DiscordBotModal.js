import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';

// ── 릴레이 서버 기본 URL (고급 설정에서 오버라이드 가능) ──
const DEFAULT_RELAY_URL = 'http://localhost:3000';

function DiscordBotModal({ 
    isOpen, 
    onClose, 
    isClosing,
    discordBotStatus,
    discordToken,
    setDiscordToken,
    discordPrefix,
    setDiscordPrefix,
    discordAutoStart,
    setDiscordAutoStart,
    discordMusicEnabled,
    setDiscordMusicEnabled,
    discordBotMode,
    setDiscordBotMode,
    discordCloudRelayUrl,
    setDiscordCloudRelayUrl,
    discordCloudHostId,
    setDiscordCloudHostId,
    handleStartDiscordBot,
    handleStopDiscordBot,
    saveCurrentSettings
}) {
    const { t } = useTranslation('gui');
    const isCloud = discordBotMode === 'cloud';

    // ── 릴레이 URL 결정 (커스텀 > 기본값) ──
    const effectiveRelayUrl = discordCloudRelayUrl || DEFAULT_RELAY_URL;

    // ── 연결 상태 ──
    const [cloudConnected, setCloudConnected] = useState(false);
    const [cloudConnecting, setCloudConnecting] = useState(false);
    const [cloudError, setCloudError] = useState('');

    // ── 노드 목록 (연결 후 표시) ──
    const [nodes, setNodes] = useState([]);
    const [expandedNode, setExpandedNode] = useState(null);
    const [nodeMembers, setNodeMembers] = useState({}); // { guildId: [member...] }
    const [nodeInstances, setNodeInstances] = useState({}); // { guildId: [instance...] }

    // ── 방법 3: 수동 입력용 로컬 스테이트 (타이핑 중 사라지는 버그 방지) ──
    const [manualHostIdInput, setManualHostIdInput] = useState('');

    // ── 페어링 상태 ──
    const [showPairing, setShowPairing] = useState(false);
    const [pairCode, setPairCode] = useState('');
    const [pairStatus, setPairStatus] = useState('idle'); // idle | waiting | success | expired | error
    const [pairExpiresAt, setPairExpiresAt] = useState(null);
    const [pairRemaining, setPairRemaining] = useState(0);
    const [pairCopied, setPairCopied] = useState(false);
    const pairPollRef = useRef(null);
    const pairTimerRef = useRef(null);

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
        return () => { if (pairTimerRef.current) clearInterval(pairTimerRef.current); };
    }, [pairStatus, pairExpiresAt]);

    // ── 클라우드 연결 확인 (hostId 존재 시) ──
    const checkCloudConnection = useCallback(async () => {
        if (!discordCloudHostId) {
            setCloudConnected(false);
            return;
        }
        setCloudConnecting(true);
        setCloudError('');
        try {
            const resp = await fetch(`${effectiveRelayUrl}/api/hosts/${encodeURIComponent(discordCloudHostId)}`);
            if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
            setCloudConnected(true);

            // 노드 목록 로드
            const nodesResp = await fetch(`${effectiveRelayUrl}/api/hosts/${encodeURIComponent(discordCloudHostId)}/nodes`);
            if (nodesResp.ok) {
                const nodesData = await nodesResp.json();
                setNodes(Array.isArray(nodesData) ? nodesData : []);
            }
        } catch (e) {
            setCloudConnected(false);
            setCloudError(e.message);
        } finally {
            setCloudConnecting(false);
        }
    }, [discordCloudHostId, effectiveRelayUrl]);

    // 모달 열릴 때 + hostId 변경 시 연결 확인
    useEffect(() => {
        if (isOpen && isCloud && discordCloudHostId) {
            checkCloudConnection();
        }
    }, [isOpen, isCloud, discordCloudHostId, checkCloudConnection]);

    // ── 노드 상세 로드 (멤버/인스턴스) ──
    const loadNodeDetails = useCallback(async (guildId) => {
        try {
            const [membersResp, instancesResp] = await Promise.all([
                fetch(`${effectiveRelayUrl}/api/nodes/${guildId}/members`),
                fetch(`${effectiveRelayUrl}/api/nodes/${guildId}/instances`),
            ]);
            if (membersResp.ok) {
                const m = await membersResp.json();
                setNodeMembers(prev => ({ ...prev, [guildId]: Array.isArray(m) ? m : [] }));
            }
            if (instancesResp.ok) {
                const i = await instancesResp.json();
                setNodeInstances(prev => ({ ...prev, [guildId]: Array.isArray(i) ? i : [] }));
            }
        } catch (e) {
            console.warn('[Cloud] Failed to load node details:', e.message);
        }
    }, [effectiveRelayUrl]);

    const toggleNodeExpand = useCallback((guildId) => {
        if (expandedNode === guildId) {
            setExpandedNode(null);
        } else {
            setExpandedNode(guildId);
            if (!nodeMembers[guildId]) {
                loadNodeDetails(guildId);
            }
        }
    }, [expandedNode, nodeMembers, loadNodeDetails]);

    // ── 페어링 ──
    const startPairing = useCallback(async () => {
        try {
            setPairStatus('idle');
            const resp = await fetch(`${effectiveRelayUrl}/api/pair/initiate`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ relayUrl: effectiveRelayUrl }),
            });
            if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
            const data = await resp.json();
            setPairCode(data.code);
            setPairExpiresAt(data.expiresAt);
            setPairStatus('waiting');
            setPairCopied(false);

            // 폴링 시작
            if (pairPollRef.current) clearInterval(pairPollRef.current);
            pairPollRef.current = setInterval(async () => {
                try {
                    const r = await fetch(`${effectiveRelayUrl}/api/pair/${encodeURIComponent(data.code)}/status`);
                    if (!r.ok) throw new Error(`HTTP ${r.status}`);
                    const s = await r.json();
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
                        } else if (!s.nodeToken) {
                            console.warn('[Pairing] Claimed but no nodeToken in response — token may have been collected already');
                        }
                        setTimeout(() => {
                            saveCurrentSettings();
                            checkCloudConnection();
                        }, 500);
                    } else if (s.status === 'expired') {
                        clearInterval(pairPollRef.current);
                        pairPollRef.current = null;
                        setPairStatus('expired');
                    }
                } catch { /* 네트워크 에러 — 폴링 계속 */ }
            }, 3000);
        } catch (e) {
            console.error('[Pairing] initiate failed:', e);
            setPairStatus('error');
        }
    }, [effectiveRelayUrl, setDiscordCloudHostId, saveCurrentSettings, checkCloudConnection]);

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

    // ── 연결 초기화 (hostId + 상태 전체 리셋) ──
    const disconnectCloud = useCallback(() => {
        resetPairing();
        setDiscordCloudHostId('');
        setCloudConnected(false);
        setCloudError('');
        setNodes([]);
        setExpandedNode(null);
        setNodeMembers({});
        setNodeInstances({});
        setManualHostIdInput('');
    }, [resetPairing, setDiscordCloudHostId]);

    if (!isOpen) return null;

    // ── 클라우드 모드: 셋업 완료 여부 판단 ──
    const cloudSetupDone = isCloud && discordCloudHostId && cloudConnected;
    const needsSetup = isCloud && (!discordCloudHostId || !cloudConnected);

    // ── 인라인 페어링 블록 ──
    const renderPairingBlock = () => (
        <div className="discord-pair-section" style={{ marginTop: 12 }}>
            {pairStatus === 'idle' && (
                <div className="discord-cloud-connecting">
                    <div className="discord-pair-spinner"></div>
                    <span>{t('discord_modal.cloud_connecting')}</span>
                </div>
            )}
            {pairStatus === 'waiting' && pairCode && (
                <div className="discord-pair-code-area">
                    <span className="discord-pair-code-label">{t('discord_modal.pair_code_label')}</span>
                    <div className="discord-pair-code-row">
                        <span className="discord-pair-code-value">{pairCode}</span>
                        <button className={`discord-pair-copy-btn ${pairCopied ? 'copied' : ''}`} onClick={copyPairCode}>
                            {pairCopied ? `✓ ${t('discord_modal.pair_code_copied')}` : `📋 ${t('discord_modal.pair_copy_button')}`}
                        </button>
                    </div>
                    <p className="discord-pair-instruction">{t('discord_modal.pair_instruction')}</p>
                    <code className="discord-pair-command">/사바쨩 연결 코드:{pairCode}</code>
                    <div className="discord-pair-waiting">
                        <div className="discord-pair-spinner"></div>
                        <span>{t('discord_modal.pair_waiting')}</span>
                        <span className="discord-pair-timer">{t('discord_modal.pair_expires_in', { seconds: pairRemaining })}</span>
                    </div>
                </div>
            )}
            {pairStatus === 'success' && (
                <div className="discord-pair-result success">✅ {t('discord_modal.pair_success')}</div>
            )}
            {pairStatus === 'expired' && (
                <div className="discord-pair-result error">
                    ⏰ {t('discord_modal.pair_expired')}
                    <button className="discord-pair-start-btn" style={{ marginLeft: 12, fontSize: 12, padding: '4px 12px' }}
                        onClick={() => { resetPairing(); setShowPairing(true); setTimeout(startPairing, 100); }}>
                        🔄 {t('discord_modal.cloud_retry')}
                    </button>
                </div>
            )}
            {pairStatus === 'error' && (
                <div className="discord-pair-result error">
                    ❌ {t('discord_modal.pair_error')}
                    <button className="discord-pair-start-btn" style={{ marginLeft: 12, fontSize: 12, padding: '4px 12px' }}
                        onClick={() => { resetPairing(); setShowPairing(true); setTimeout(startPairing, 100); }}>
                        🔄 {t('discord_modal.cloud_retry')}
                    </button>
                </div>
            )}
        </div>
    );

    return (
        <div className={`discord-modal-container ${isClosing ? 'closing' : ''}`} onClick={(e) => e.stopPropagation()}>
            <div className="discord-modal-header">
                <div className="discord-modal-title">
                    <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : 'status-offline'}`}></span>
                    <h2>{t('discord_modal.title')}</h2>
                </div>
                <button className="discord-modal-close" onClick={onClose}><Icon name="close" size="sm" /></button>
            </div>

            <div className="discord-modal-content">
                {/* ── 상태 표시 ── */}
                <div className="discord-status-section">
                    <span className="status-label">{t('discord_modal.status_label')}</span>
                    <span className={`status-value status-${discordBotStatus}`}>
                        {discordBotStatus === 'running' ? t('discord_modal.status_online') : discordBotStatus === 'error' ? t('discord_modal.status_error') : t('discord_modal.status_offline')}
                    </span>
                    {isCloud && <span className="discord-mode-badge cloud">☁️ {t('discord_modal.mode_cloud')}</span>}
                    {!isCloud && <span className="discord-mode-badge local">🏠 {t('discord_modal.mode_local')}</span>}
                </div>

                {/* ── 모드 전환 카드 ── */}
                <div className="discord-mode-toggle-card">
                    <div className="discord-mode-toggle-info">
                        <span className="discord-mode-toggle-icon">{isCloud ? '☁️' : '🏠'}</span>
                        <div className="discord-mode-toggle-text">
                            <span className="discord-mode-toggle-label">{t('discord_modal.mode_label')}</span>
                            <span className="discord-mode-toggle-desc">
                                {isCloud ? t('discord_modal.mode_cloud_desc') : t('discord_modal.mode_local_desc')}
                            </span>
                        </div>
                    </div>
                    <label className="toggle-switch">
                        <input
                            type="checkbox"
                            checked={isCloud}
                            onChange={(e) => {
                                const newMode = e.target.checked ? 'cloud' : 'local';
                                setDiscordBotMode(newMode);
                                if (newMode === 'cloud' && discordBotStatus === 'running') {
                                    handleStopDiscordBot();
                                }
                            }}
                        />
                        <span className="toggle-slider"></span>
                    </label>
                </div>

                {/* ══════════════════════════════════════════════ */}
                {/* ── 클라우드 모드: 셋업 필요 (페어링/연결) ── */}
                {/* ══════════════════════════════════════════════ */}
                {needsSetup && (
                    <div className="discord-cloud-section">
                        {/* ── 페어링 진행 중이면 페어링 블록만 표시 ── */}
                        {showPairing ? (
                            <>
                                {renderPairingBlock()}
                                <button className="discord-pair-start-btn discord-btn-secondary" style={{ marginTop: 8, width: '100%', fontSize: 12 }}
                                    onClick={resetPairing}>
                                    ← {t('discord_modal.back_to_setup')}
                                </button>
                            </>
                        ) : (
                            <>
                                {/* ── 연결 중 스피너 ── */}
                                {cloudConnecting && (
                                    <div className="discord-cloud-connecting">
                                        <div className="discord-pair-spinner"></div>
                                        <span>{t('discord_modal.cloud_connecting')}</span>
                                    </div>
                                )}

                                {/* ── 연결 오류 (hostId가 있는데 연결 실패) ── */}
                                {!cloudConnecting && cloudError && discordCloudHostId && (
                                    <div className="discord-cloud-error-card">
                                        <div className="discord-cloud-error-icon">⚠️</div>
                                        <div className="discord-cloud-error-body">
                                            <strong>{t('discord_modal.cloud_connection_failed_title')}</strong>
                                            <p>{t('discord_modal.cloud_connection_error', { error: cloudError })}</p>
                                            <small className="discord-cloud-error-hint">
                                                Host ID: {discordCloudHostId} → {effectiveRelayUrl}
                                            </small>
                                        </div>
                                        <div className="discord-cloud-error-actions">
                                            <button className="discord-pair-start-btn" onClick={checkCloudConnection}>
                                                🔄 {t('discord_modal.cloud_retry')}
                                            </button>
                                            <button className="discord-pair-start-btn" onClick={() => { setShowPairing(true); startPairing(); }}>
                                                🔗 {t('discord_modal.cloud_re_pair')}
                                            </button>
                                            <button className="discord-pair-start-btn discord-btn-danger" onClick={disconnectCloud}>
                                                🗑️ {t('discord_modal.cloud_disconnect')}
                                            </button>
                                        </div>
                                    </div>
                                )}

                                {/* ── 연결 오류 (hostId 없음 — 첫 설정) ── */}
                                {!cloudConnecting && cloudError && !discordCloudHostId && (
                                    <div className="discord-cloud-warning">
                                        <Icon name="warning" size="sm" />
                                        <span>{t('discord_modal.cloud_connection_error', { error: cloudError })}</span>
                                    </div>
                                )}



                                {/* ── 셋업 카드 (hostId 미등록 또는 연결 실패) ── */}
                                {!cloudConnecting && !cloudError && (
                                    <div className="discord-cloud-setup-card">
                                        <div className="discord-cloud-setup-icon">🔗</div>
                                        <h4>{t('discord_modal.cloud_setup_title')}</h4>
                                        <p>{t('discord_modal.cloud_setup_desc')}</p>

                                        <div className="discord-cloud-setup-method">
                                            <span className="discord-cloud-setup-badge">1</span>
                                            <div>
                                                <strong>{t('discord_modal.cloud_method_discord')}</strong>
                                                <p>{t('discord_modal.cloud_method_discord_desc')}</p>
                                                <code>/사바쨩 등록</code>
                                            </div>
                                        </div>

                                        <div className="discord-cloud-setup-method">
                                            <span className="discord-cloud-setup-badge">2</span>
                                            <div>
                                                <strong>{t('discord_modal.cloud_method_pair')}</strong>
                                                <p>{t('discord_modal.cloud_method_pair_desc')}</p>
                                            </div>
                                        </div>

                                        <div className="discord-cloud-setup-method">
                                            <span className="discord-cloud-setup-badge">3</span>
                                            <div>
                                                <strong>{t('discord_modal.cloud_method_manual')}</strong>
                                                <div className="discord-form-group" style={{ marginTop: 8 }}>
                                                    <input
                                                        type="text"
                                                        placeholder={t('discord_modal.host_id_placeholder')}
                                                        value={manualHostIdInput}
                                                        onChange={(e) => setManualHostIdInput(e.target.value)}
                                                        onKeyDown={(e) => {
                                                            if (e.key === 'Enter' && manualHostIdInput.trim()) {
                                                                setDiscordCloudHostId(manualHostIdInput.trim());
                                                            }
                                                        }}
                                                        onBlur={() => {
                                                            if (manualHostIdInput.trim()) {
                                                                setDiscordCloudHostId(manualHostIdInput.trim());
                                                            }
                                                        }}
                                                        className="discord-input"
                                                        style={{ width: '100%' }}
                                                    />
                                                </div>
                                            </div>
                                        </div>
                                    </div>
                                )}

                                {/* ── 페어링 시작 버튼 (셋업 카드 아래, 에러+hostId 시 에러카드에 이미 있음) ── */}
                                {!cloudConnecting && !(cloudError && discordCloudHostId) && (
                                    <button className="discord-pair-start-btn" style={{ marginTop: 12, width: '100%' }}
                                        onClick={() => { setShowPairing(true); startPairing(); }}>
                                        🔗 {t('discord_modal.pair_start_button')}
                                    </button>
                                )}
                            </>
                        )}
                    </div>
                )}

                {/* ══════════════════════════════════════════════ */}
                {/* ── 클라우드 모드: 연결 완료 → 노드 카드 ──── */}
                {/* ══════════════════════════════════════════════ */}
                {cloudSetupDone && (
                    <div className="discord-cloud-section">
                        <div className="discord-cloud-connected-banner">
                            <span className="discord-cloud-connected-icon">✅</span>
                            <div>
                                <strong>{t('discord_modal.cloud_connected_title')}</strong>
                                <span className="discord-cloud-host-id">Host: {discordCloudHostId}</span>
                            </div>
                            <div style={{ display: 'flex', gap: 4, marginLeft: 'auto' }}>
                                <button className="discord-pair-start-btn" style={{ fontSize: 11, padding: '3px 10px' }}
                                    onClick={checkCloudConnection}>🔄</button>
                                <button className="discord-pair-start-btn discord-btn-danger" style={{ fontSize: 11, padding: '3px 10px' }}
                                    onClick={disconnectCloud} title={t('discord_modal.cloud_disconnect')}>🔌</button>
                            </div>
                        </div>

                        {nodes.length === 0 ? (
                            <div className="discord-cloud-empty-nodes">
                                <p>{t('discord_modal.cloud_no_nodes')}</p>
                                <small>{t('discord_modal.cloud_no_nodes_hint')}</small>
                                <button className="discord-pair-start-btn" style={{ marginTop: 8 }}
                                    onClick={() => { setShowPairing(true); startPairing(); }}>
                                    🔗 {t('discord_modal.pair_start_button')}
                                </button>
                                {showPairing && renderPairingBlock()}
                            </div>
                        ) : (
                            <div className="discord-node-list">
                                <h4>📡 {t('discord_modal.cloud_nodes_title')} ({nodes.length})</h4>
                                {nodes.map(node => (
                                    <div key={node.guildId} className={`discord-node-card ${expandedNode === node.guildId ? 'expanded' : ''}`}>
                                        <div className="discord-node-card-header" onClick={() => toggleNodeExpand(node.guildId)}>
                                            <div className="discord-node-card-info">
                                                <span className="discord-node-guild-name">{node.guildName || node.guildId}</span>
                                                <span className="discord-node-guild-id">{node.guildId}</span>
                                            </div>
                                            <Icon name={expandedNode === node.guildId ? 'chevronDown' : 'chevronRight'} size="sm" />
                                        </div>

                                        {expandedNode === node.guildId && (
                                            <div className="discord-node-card-body">
                                                <div className="discord-node-section">
                                                    <h5>👥 {t('discord_modal.cloud_node_members')}</h5>
                                                    {(!nodeMembers[node.guildId] || nodeMembers[node.guildId].length === 0) ? (
                                                        <p className="discord-node-empty">{t('discord_modal.cloud_node_no_members')}</p>
                                                    ) : (
                                                        <div className="discord-node-member-list">
                                                            {nodeMembers[node.guildId].map((member, idx) => (
                                                                <div key={idx} className="discord-node-member-row">
                                                                    <span className="discord-node-member-id">{member.userDiscordId}</span>
                                                                    <span className="discord-node-member-cmds">
                                                                        {Array.isArray(member.allowedCommands) && member.allowedCommands.length > 0
                                                                            ? member.allowedCommands.join(', ')
                                                                            : t('discord_modal.cloud_node_all_commands')}
                                                                    </span>
                                                                </div>
                                                            ))}
                                                        </div>
                                                    )}
                                                </div>

                                                <div className="discord-node-section">
                                                    <h5>🖥️ {t('discord_modal.cloud_node_instances')}</h5>
                                                    {(!nodeInstances[node.guildId] || nodeInstances[node.guildId].length === 0) ? (
                                                        <p className="discord-node-empty">{t('discord_modal.cloud_node_no_instances')}</p>
                                                    ) : (
                                                        <div className="discord-node-instance-list">
                                                            {nodeInstances[node.guildId].map((inst, idx) => (
                                                                <div key={idx} className="discord-node-instance-row">
                                                                    <span className="discord-node-instance-type">{inst.instanceType}</span>
                                                                    <span className={`discord-node-instance-status ${inst.enabled ? 'enabled' : 'disabled'}`}>
                                                                        {inst.enabled ? '✅' : '⛔'}
                                                                    </span>
                                                                </div>
                                                            ))}
                                                        </div>
                                                    )}
                                                </div>
                                            </div>
                                        )}
                                    </div>
                                ))}

                                <button className="discord-pair-start-btn" style={{ marginTop: 8, width: '100%' }}
                                    onClick={() => { if (showPairing) { resetPairing(); } else { setShowPairing(true); startPairing(); } }}>
                                    {showPairing ? '✕ ' + t('discord_modal.pair_section_title') : '➕ ' + t('discord_modal.cloud_add_node')}
                                </button>
                                {showPairing && renderPairingBlock()}
                            </div>
                        )}
                    </div>
                )}

                {/* ══════════════════════════════════════════════ */}
                {/* ── 공통 설정 섹션 ─────────────────────────── */}
                {/* ══════════════════════════════════════════════ */}
                <div className="discord-config-section">
                    {!isCloud && (
                        <div className="discord-form-group">
                            <label>{t('discord_modal.token_label')}</label>
                            <input
                                type="password"
                                placeholder={t('discord_modal.token_placeholder')}
                                value={discordToken}
                                onChange={(e) => setDiscordToken(e.target.value)}
                                className="discord-input"
                            />
                        </div>
                    )}

                    <div className="discord-form-group">
                        <label>{t('discord_modal.prefix_label')}</label>
                        <input
                            type="text"
                            placeholder={t('discord_modal.prefix_placeholder')}
                            value={discordPrefix}
                            onChange={(e) => setDiscordPrefix(e.target.value)}
                            className="discord-input"
                        />
                        <small>{t('discord_modal.prefix_description')}</small>
                        {!discordPrefix && <small className="warning-text">{t('discord_modal.prefix_warning')}</small>}
                    </div>

                    {!isCloud && (
                        <div className="discord-form-group">
                            <label className="discord-checkbox-label">
                                <input
                                    type="checkbox"
                                    checked={discordAutoStart}
                                    onChange={(e) => setDiscordAutoStart(e.target.checked)}
                                />
                                {t('discord_modal.auto_start_label')}
                            </label>
                        </div>
                    )}
                </div>

                <div className="discord-info-box">
                    <h4><Icon name="lightbulb" size="sm" /> {t('discord_modal.usage_title')}</h4>
                    <p>{t('discord_modal.usage_instruction')}</p>
                    <code>{discordPrefix || '!saba'} [module] [command]</code>
                    <p className="info-note">{t('discord_modal.usage_note')}</p>
                </div>

                {!isCloud && (
                    <div className="discord-music-toggle-card">
                        <div className="discord-music-toggle-info">
                            <span className="discord-music-toggle-icon">🎵</span>
                            <div className="discord-music-toggle-text">
                                <span className="discord-music-toggle-label">{t('discord_modal.music_toggle_label')}</span>
                                <span className="discord-music-toggle-desc">{t('discord_modal.music_toggle_description')}</span>
                            </div>
                        </div>
                        <label className="toggle-switch">
                            <input type="checkbox" checked={discordMusicEnabled} onChange={(e) => setDiscordMusicEnabled(e.target.checked)} />
                            <span className="toggle-slider"></span>
                        </label>
                    </div>
                )}

                {isCloud && (
                    <div className="discord-cloud-music-notice">
                        <span>🎵</span>
                        <span>{t('discord_modal.music_cloud_notice')}</span>
                    </div>
                )}
            </div>

            <div className="discord-modal-footer">
                {!isCloud && (
                    <button
                        className={`discord-btn ${discordBotStatus === 'running' ? 'discord-btn-stop' : 'discord-btn-start'}`}
                        onClick={() => discordBotStatus === 'running' ? handleStopDiscordBot() : handleStartDiscordBot()}
                    >
                        {discordBotStatus === 'running' ? t('discord_modal.stop_button') : t('discord_modal.start_button')}
                    </button>
                )}
                {isCloud && cloudSetupDone && (
                    <button
                        className={`discord-btn ${discordBotStatus === 'running' ? 'discord-btn-stop' : 'discord-btn-start'}`}
                        onClick={() => discordBotStatus === 'running' ? handleStopDiscordBot() : handleStartDiscordBot()}
                    >
                        {discordBotStatus === 'running' ? t('discord_modal.agent_stop_button') : t('discord_modal.agent_start_button')}
                    </button>
                )}
                <button className="discord-btn discord-btn-save" onClick={saveCurrentSettings}>
                    {t('discord_modal.save_button')}
                </button>
            </div>
        </div>
    );
}

export default DiscordBotModal;
