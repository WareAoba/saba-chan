import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import i18n from '../../i18n';
import './Modals.css';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { useExtensions } from '../../contexts/ExtensionContext';
import { useDiscordStore } from '../../stores/useDiscordStore';

// ── 릴레이 서버 기본 URL (고급 설정에서 오버라이드 가능) ──
const DEFAULT_RELAY_URL = 'https://saba-chan.online';

// ── 음악 명령어 키 목록 (범용 별명은 i18n bot:music_commands에서 로드) ──
const UNIVERSAL_COMMAND_ALIASES = {
    play:    ['p'],
    search:  ['find'],
    pause:   [],
    resume:  ['continue'],
    skip:    ['s', 'next'],
    stop:    ['leave', 'disconnect', 'dc'],
    queue:   ['q', 'list'],
    np:      ['nowplaying', 'now'],
    volume:  ['vol', 'v'],
    shuffle: ['random'],
    help:    [],
};
const MUSIC_COMMAND_KEYS = Object.keys(UNIVERSAL_COMMAND_ALIASES);

function DiscordBotModal({
    isOpen,
    onClose,
    isClosing,
    displayMode = 'popup',
    discordBotStatus,
    discordToken,
    setDiscordToken,
    discordPrefix,
    setDiscordPrefix,
    discordAutoStart,
    setDiscordAutoStart,
    discordMusicEnabled,
    setDiscordMusicEnabled,
    discordMusicChannelId,
    setDiscordMusicChannelId,
    discordMusicUISettings,
    setDiscordMusicUISettings,
    discordBotMode,
    setDiscordBotMode,
    discordCloudRelayUrl,
    discordCloudHostId,
    setDiscordCloudHostId,
    relayConnected,
    relayConnecting,
    handleStartDiscordBot,
    handleStopDiscordBot,
    saveCurrentSettings,
    servers,
    moduleAliasesPerModule,
    nodeSettings,
    setNodeSettings,
    cloudNodes,
    setCloudNodes,
    cloudMembers,
    setCloudMembers,
}) {
    const { t } = useTranslation('gui');
    const isCloud = discordBotMode === 'cloud';

    // 익스텐션 시스템에서 music 익스텐션 활성 여부 확인
    const { extensions: extList } = useExtensions();
    const musicExtEnabled = extList.some((e) => e.id === 'music' && e.enabled);

    // ── 릴레이 URL 결정 (커스텀 > 기본값) ──
    const effectiveRelayUrl = discordCloudRelayUrl || DEFAULT_RELAY_URL;

    // ── 연결 상태 (훅에서 전달받은 릴레이 상태 사용) ──
    const cloudConnected = relayConnected ?? false;
    const cloudConnecting = relayConnecting ?? false;
    const [cloudError, setCloudError] = useState('');

    // ── 노드 UI 상태 (App에 저장할 필요 없는 일시적 UI 상태) ──
    const [expandedNode, setExpandedNode] = useState(null);

    // ── 로컬 모드: 봇이 접속한 길드 목록 ──
    const [localGuilds, setLocalGuilds] = useState([]);

    // ── 길드 멤버 로딩 상태 ──
    const [membersLoading, setMembersLoading] = useState(false);

    // ── 노드별 탭 상태 (instances | members) ──
    const [nodeTab, setNodeTab] = useState({});
    // ── 멤버 확장 상태 ──
    const [expandedMember, setExpandedMember] = useState({});

    // ── 페어링 상태 ──
    const [showPairing, setShowPairing] = useState(false);
    const [pairCode, setPairCode] = useState('');
    const [pairStatus, setPairStatus] = useState('idle');
    const [pairExpiresAt, setPairExpiresAt] = useState(null);
    const [pairRemaining, setPairRemaining] = useState(0);
    const [pairCopied, setPairCopied] = useState(false);
    const [pairPollSecret, setPairPollSecret] = useState('');  // 폴링 인증용 시크릿
    const pairPollRef = useRef(null);
    const pairTimerRef = useRef(null);

    // ── 음악 설정 패널 상태 ──
    const [showMusicSettings, setShowMusicSettings] = useState(false);
    const [musicModuleAliases, setMusicModuleAliases] = useState('');
    const [musicCommandAliases, setMusicCommandAliases] = useState({});
    const musicSettingsRef = useRef(null);
    const [guildChannels, setGuildChannels] = useState(null); // { guildId: { guildName, channels: [...] } }
    const [channelsLoading, setChannelsLoading] = useState(false);
    const [channelsError, setChannelsError] = useState(null); // M3: 채널 로드 에러 상태

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
            setPairPollSecret('');
            setShowMusicSettings(false);
        }
    }, [isOpen]);

    // ══════════════════════════════════════════════
    // ── 음악 명령어 별명 로드 / 저장 / 초기화 ──
    // ══════════════════════════════════════════════

    /** 음악 설정 패널 열기 — bot-config에서 현재 별명 로드 */
    const openMusicSettings = useCallback(async () => {
        try {
            const cfg = await window.api.botConfigLoad();
            // 모듈 별명 로드
            const savedModAlias = cfg?.moduleAliases?.music || '';
            setMusicModuleAliases(savedModAlias);

            // 명령어 별명 로드
            const savedCmdAliases = cfg?.commandAliases?.music || {};
            const initial = {};
            for (const cmd of MUSIC_COMMAND_KEYS) {
                initial[cmd] = savedCmdAliases[cmd] || '';
            }
            setMusicCommandAliases(initial);
        } catch (e) {
            console.warn('[MusicSettings] Failed to load config:', e);
            // 기본값으로 초기화
            setMusicModuleAliases('');
            const initial = {};
            for (const cmd of MUSIC_COMMAND_KEYS) {
                initial[cmd] = '';
            }
            setMusicCommandAliases(initial);
        }
        setShowMusicSettings(true);
        // 봇이 실행 중이면 채널 목록 로드
        if (discordBotStatus === 'running') {
            loadGuildChannels();
        }
        // 다음 렌더 후 패널로 스크롤
        requestAnimationFrame(() => {
            musicSettingsRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' });
        });
    }, [discordBotStatus]);

    /** 서버 채널 목록 로드 (재시도 포함) */
    const loadGuildChannels = useCallback(async () => {
        setChannelsLoading(true);
        setChannelsError(null);
        const maxRetries = 3;
        for (let attempt = 1; attempt <= maxRetries; attempt++) {
            try {
                const resp = await window.api.discordGuildChannels();
                if (resp?.data && typeof resp.data === 'object' && Object.keys(resp.data).length > 0) {
                    setGuildChannels(resp.data);
                    setChannelsLoading(false);
                    return;
                }
                // BOT_NOT_READY 에러면 잠시 대기 후 재시도
                if (resp?.error === 'BOT_NOT_READY' && attempt < maxRetries) {
                    await new Promise(r => setTimeout(r, 2000));
                    continue;
                }
            } catch (e) {
                console.warn(`[MusicSettings] Channel load attempt ${attempt} failed:`, e);
                if (attempt < maxRetries) {
                    await new Promise(r => setTimeout(r, 1500));
                    continue;
                }
            }
            // 최종 실패
            setGuildChannels(null);
            setChannelsError('Failed to load channels');
        }
        setChannelsLoading(false);
    }, []);

    /** 음악 별명 저장 */
    const handleSaveMusicAliases = useCallback(async () => {
        try {
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };

            // 모듈 별명
            if (musicModuleAliases.trim()) {
                moduleAliases.music = musicModuleAliases.trim();
            } else {
                delete moduleAliases.music;
            }

            // 명령어 별명
            const cmdMap = {};
            let hasAny = false;
            for (const [cmd, val] of Object.entries(musicCommandAliases)) {
                const trimmed = (val || '').trim();
                if (trimmed) {
                    cmdMap[cmd] = trimmed;
                    hasAny = true;
                }
            }
            if (hasAny) {
                commandAliases.music = cmdMap;
            } else {
                delete commandAliases.music;
            }

            const payload = {
                ...current,
                moduleAliases,
                commandAliases,
                musicChannelId: discordMusicChannelId || '',
                musicUISettings: discordMusicUISettings || { queueLines: 5, refreshInterval: 4000 },
            };
            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                console.error('[MusicSettings] Save failed:', res.error);
                // M4: 사용자에게 저장 실패 피드백 표시
                if (window.showToast) window.showToast(res.error, 'error');
            } else {
                // zustand store 동기화 — startBot이 최신 별명을 전달하도록
                useDiscordStore.getState().update({
                    discordModuleAliases: moduleAliases,
                    discordCommandAliases: commandAliases,
                    discordMusicChannelId: discordMusicChannelId || '',
                    discordMusicUISettings: discordMusicUISettings || { queueLines: 5, refreshInterval: 4000 },
                });
                console.log('[MusicSettings] Aliases saved');
                if (window.showToast) window.showToast('Music settings saved', 'success');
            }
        } catch (e) {
            console.error('[MusicSettings] Save error:', e);
            if (window.showToast) window.showToast(String(e.message || e), 'error');
        }
    }, [musicModuleAliases, musicCommandAliases, discordMusicChannelId, discordMusicUISettings]);

    /** 음악 별명 초기화 */
    const handleResetMusicAliases = useCallback(async () => {
        setMusicModuleAliases('');
        const cleared = {};
        for (const cmd of MUSIC_COMMAND_KEYS) {
            cleared[cmd] = '';
        }
        setMusicCommandAliases(cleared);

        try {
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };
            delete moduleAliases.music;
            delete commandAliases.music;

            const payload = { ...current, moduleAliases, commandAliases };
            await window.api.botConfigSave(payload);
            // zustand store 동기화
            useDiscordStore.getState().update({
                discordModuleAliases: moduleAliases,
                discordCommandAliases: commandAliases,
            });
            console.log('[MusicSettings] Aliases reset');
        } catch (e) {
            console.error('[MusicSettings] Reset error:', e);
        }
    }, []);

    // ══════════════════════════════════════════════
    // ── 길드 멤버 가져오기 ──
    // ══════════════════════════════════════════════

    /** 로컬모드: 봇 프로세스에서 길드별 멤버 가져오기 */
    const fetchLocalGuildMembers = useCallback(async () => {
        if (!window.api?.discordGuildMembers) return;
        setMembersLoading(true);
        try {
            const resp = await window.api.discordGuildMembers();
            if (resp?.data) {
                // 길드 목록 저장
                const guilds = Object.entries(resp.data).map(([guildId, guildData]) => ({
                    guildId,
                    guildName: guildData.guildName || guildId,
                }));
                setLocalGuilds(guilds);

                // 길드별 멤버 저장 (guildId 키)
                setCloudMembers((prev) => {
                    const next = { ...prev };
                    for (const [guildId, guildData] of Object.entries(resp.data)) {
                        next[guildId] = guildData.members || [];
                    }
                    return next;
                });
            }
        } catch (e) {
            console.warn('[DiscordBotModal] Failed to fetch local guild members:', e);
        } finally {
            setMembersLoading(false);
        }
    }, [setCloudMembers]);

    /** 클라우드모드: 릴레이 서버 봇을 통해 디스코드 길드 멤버 가져오기 */
    const fetchCloudNodeMembers = useCallback(
        async (guildId) => {
            setMembersLoading(true);
            try {
                // 데몬 프록시를 통해 멤버 조회 (discord-members → members 폴백은 데몬이 처리)
                const data = await window.api.relayListNodeMembers(guildId, effectiveRelayUrl);
                const members = Array.isArray(data) ? data : data?.members || [];
                setCloudMembers((prev) => ({
                    ...prev,
                    [guildId]: members,
                }));
            } catch (e) {
                console.warn('[DiscordBotModal] Failed to fetch cloud members:', e);
            } finally {
                setMembersLoading(false);
            }
        },
        [effectiveRelayUrl, setCloudMembers],
    );

    // 로컬 모드 + 봇 실행 중일 때 길드 목록 자동 로드 (캐시 없을 때만)
    useEffect(() => {
        if (isOpen && !isCloud && discordBotStatus === 'running' && localGuilds.length === 0) {
            fetchLocalGuildMembers();
        }
    }, [isOpen, isCloud, discordBotStatus, fetchLocalGuildMembers, localGuilds.length]);

    // ══════════════════════════════════════════════
    // ── nodeSettings 헬퍼 함수들 ──
    // ══════════════════════════════════════════════

    /** 특정 노드의 설정 가져오기 (없으면 기본값) */
    const getNodeConfig = useCallback(
        (nodeKey) => {
            const cfg = nodeSettings[nodeKey];
            return cfg || { allowedInstances: [], memberPermissions: {} };
        },
        [nodeSettings],
    );

    /**
     * 인스턴스가 이미 다른 노드에 할당되어 있는지 확인.
     * @returns {string|null} 소유 노드 키, 없으면 null
     */
    const getInstanceOwnerNode = useCallback(
        (serverId, excludeNodeKey) => {
            // 모드별 형제 노드만 체크 (로컬 ↔ 클라우드 독립)
            const siblingKeys = isCloud
                ? cloudNodes.map((n) => n.guildId)
                : localGuilds.map((g) => g.guildId);
            for (const nodeKey of siblingKeys) {
                if (nodeKey === excludeNodeKey) continue;
                const cfg = nodeSettings[nodeKey];
                if (Array.isArray(cfg?.allowedInstances) && cfg.allowedInstances.includes(serverId)) {
                    return nodeKey;
                }
            }
            return null;
        },
        [nodeSettings, isCloud, cloudNodes, localGuilds],
    );

    /** 인스턴스 토글 (형제 노드 간 단일 할당 제약) */
    const toggleNodeInstance = useCallback(
        (nodeKey, serverId) => {
            setNodeSettings((prev) => {
                const next = { ...prev };
                const cfg = { ...(next[nodeKey] || { allowedInstances: [], memberPermissions: {} }) };
                const arr = Array.isArray(cfg.allowedInstances) ? [...cfg.allowedInstances] : [];
                const idx = arr.indexOf(serverId);
                if (idx >= 0) {
                    arr.splice(idx, 1); // 제거는 항상 허용
                } else {
                    // 형제 노드에 이미 할당되어 있으면 추가 불가 (로컬/클라우드 독립)
                    const siblingKeys = isCloud
                        ? cloudNodes.map((n) => n.guildId)
                        : localGuilds.map((g) => g.guildId);
                    for (const otherKey of siblingKeys) {
                        if (otherKey === nodeKey) continue;
                        const otherCfg = prev[otherKey];
                        if (Array.isArray(otherCfg?.allowedInstances) && otherCfg.allowedInstances.includes(serverId)) {
                            return prev; // 변경 없음
                        }
                    }
                    arr.push(serverId);
                }
                cfg.allowedInstances = arr;
                next[nodeKey] = cfg;
                return next;
            });
        },
        [setNodeSettings, isCloud, cloudNodes, localGuilds],
    );

    /** 전체 선택 / 해제 (형제 노드에 할당된 인스턴스 제외) */
    const setNodeAllInstances = useCallback(
        (nodeKey, selectAll) => {
            setNodeSettings((prev) => {
                const next = { ...prev };
                const cfg = { ...(next[nodeKey] || { allowedInstances: [], memberPermissions: {} }) };
                if (selectAll && servers) {
                    // 형제 노드에 할당된 인스턴스 제외 (로컬/클라우드 독립)
                    const siblingKeys = isCloud
                        ? cloudNodes.map((n) => n.guildId)
                        : localGuilds.map((g) => g.guildId);
                    const otherAssigned = new Set();
                    for (const otherKey of siblingKeys) {
                        if (otherKey === nodeKey) continue;
                        for (const id of prev[otherKey]?.allowedInstances || []) {
                            otherAssigned.add(id);
                        }
                    }
                    if (otherAssigned.size > 0) {
                        cfg.allowedInstances = servers.filter((s) => !otherAssigned.has(s.id)).map((s) => s.id);
                    } else {
                        cfg.allowedInstances = servers.map((s) => s.id);
                    }
                } else {
                    cfg.allowedInstances = [];
                }
                next[nodeKey] = cfg;
                return next;
            });
        },
        [setNodeSettings, servers, isCloud, cloudNodes, localGuilds],
    );

    /** 멤버 권한 토글 (멤버를 nodeSettings에 추가/제거) */
    const toggleMemberEnabled = useCallback(
        (nodeKey, userId) => {
            setNodeSettings((prev) => {
                const next = { ...prev };
                const cfg = { ...(next[nodeKey] || { allowedInstances: [], memberPermissions: {} }) };
                const perms = { ...cfg.memberPermissions };
                if (perms[userId]) {
                    delete perms[userId]; // 제거
                } else {
                    perms[userId] = {}; // 추가 (빈 권한)
                }
                cfg.memberPermissions = perms;
                next[nodeKey] = cfg;
                return next;
            });
        },
        [setNodeSettings],
    );

    /** 멤버의 특정 인스턴스에 대한 명령어 토글 */
    const toggleMemberCommand = useCallback(
        (nodeKey, userId, serverId, command) => {
            setNodeSettings((prev) => {
                const next = { ...prev };
                const cfg = { ...(next[nodeKey] || { allowedInstances: [], memberPermissions: {} }) };
                const perms = { ...cfg.memberPermissions };
                const userPerms = { ...perms[userId] };
                const cmds = Array.isArray(userPerms[serverId]) ? [...userPerms[serverId]] : [];
                const idx = cmds.indexOf(command);
                if (idx >= 0) cmds.splice(idx, 1);
                else cmds.push(command);
                userPerms[serverId] = cmds;
                perms[userId] = userPerms;
                cfg.memberPermissions = perms;
                next[nodeKey] = cfg;
                return next;
            });
        },
        [setNodeSettings],
    );

    /** 멤버의 특정 인스턴스 명령어 전체 허용/차단 */
    const setMemberAllCommands = useCallback(
        (nodeKey, userId, serverId, allCommands, allow) => {
            setNodeSettings((prev) => {
                const next = { ...prev };
                const cfg = { ...(next[nodeKey] || { allowedInstances: [], memberPermissions: {} }) };
                const perms = { ...cfg.memberPermissions };
                const userPerms = { ...perms[userId] };
                userPerms[serverId] = allow ? [] : [...allCommands];
                perms[userId] = userPerms;
                cfg.memberPermissions = perms;
                next[nodeKey] = cfg;
                return next;
            });
        },
        [setNodeSettings],
    );

    /** 모듈의 명령어 목록 가져오기 */
    const getCommandsForModule = useCallback(
        (moduleName) => {
            const modInfo = moduleAliasesPerModule?.[moduleName];
            if (!modInfo?.commands) return [];
            return Object.entries(modInfo.commands).map(([cmdName, cmdInfo]) => ({
                name: cmdName,
                label: cmdInfo.label || cmdName,
                description: cmdInfo.description || '',
            }));
        },
        [moduleAliasesPerModule],
    );

    // ══════════════════════════════════════════════
    // ── 클라우드 노드 목록 로드 (연결 상태는 훅에서 관리) ──
    // ══════════════════════════════════════════════
    // biome-ignore lint/correctness/useExhaustiveDependencies: setCloudNodes/setCloudError are prop setters (stable) — biome can't track stability through props
    const loadCloudNodes = useCallback(async () => {
        if (!discordCloudHostId) return;
        setCloudError('');
        try {
            // 노드 목록 로드 (데몬 프록시 경유)
            const nodesData = await window.api.relayListHostNodes(discordCloudHostId, effectiveRelayUrl);
            setCloudNodes(Array.isArray(nodesData) ? nodesData : []);
        } catch (e) {
            setCloudError(t('errors.network_error', { defaultValue: 'Connection failed' }));
            console.error('[DiscordBotModal] Cloud nodes fetch error:', e.message);
        }
    }, [discordCloudHostId, effectiveRelayUrl]);

    // 모달 열릴 때 + 연결 확인 시 노드 로드
    useEffect(() => {
        if (isOpen && isCloud && discordCloudHostId && cloudConnected) {
            loadCloudNodes();
        }
    }, [isOpen, isCloud, discordCloudHostId, cloudConnected, loadCloudNodes]);

    // ── 노드 확장 (클릭 시 멤버도 로드 — 캐시 없을 때만) ──
    const toggleNodeExpand = useCallback(
        (guildId) => {
            if (expandedNode === guildId) {
                setExpandedNode(null);
            } else {
                setExpandedNode(guildId);
                // 클라우드: 캐시 없으면 서버에서 멤버 로드
                if (isCloud && !(cloudMembers[guildId]?.length > 0)) {
                    fetchCloudNodeMembers(guildId);
                }
                // 로컬: fetchLocalGuildMembers에서 이미 로드됨
            }
        },
        [expandedNode, cloudMembers, fetchCloudNodeMembers, isCloud],
    );

    // ── 페어링 ──
    const startPairing = useCallback(async () => {
        try {
            setPairStatus('idle');
            const data = await window.api.relayInitiatePairing({ relayUrl: effectiveRelayUrl });
            if (data.error) throw new Error(data.error);
            setPairCode(data.code);
            setPairExpiresAt(data.expiresAt);
            setPairPollSecret(data.pollSecret);  // ★ pollSecret 저장
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
        setPairPollSecret('');
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
    }, [resetPairing, setDiscordCloudHostId, setCloudNodes, setCloudMembers]);

    // ══════════════════════════════════════════════
    // ── 노드 설정 Body 렌더링 (인스턴스 + 멤버 탭) ──
    // ══════════════════════════════════════════════
    const renderNodeSettingsBody = (nodeKey, _nodeLabel) => {
        const currentTab = nodeTab[nodeKey] || 'instances';
        const cfg = getNodeConfig(nodeKey);
        const allowedInsts = cfg.allowedInstances || [];
        const memberPerms = cfg.memberPermissions || {};
        const enabledMemberIds = Object.keys(memberPerms);
        const availableMembers = cloudMembers[nodeKey] || [];

        return (
            <div className="discord-node-settings-body">
                {/* 탭 헤더 */}
                <div className="discord-node-tabs">
                    <button
                        className={clsx('discord-node-tab', { active: currentTab === 'instances' })}
                        onClick={() => setNodeTab((prev) => ({ ...prev, [nodeKey]: 'instances' }))}
                    >
                        🖥️ {t('discord_modal.tab_instances')}
                    </button>
                    <button
                        className={clsx('discord-node-tab', { active: currentTab === 'members' })}
                        onClick={() => setNodeTab((prev) => ({ ...prev, [nodeKey]: 'members' }))}
                    >
                        👥 {t('discord_modal.tab_members')} ({enabledMemberIds.length})
                    </button>
                </div>

                {/* ── 인스턴스 탭 ── */}
                {currentTab === 'instances' && (
                    <div className="discord-node-tab-content">
                        <div className="discord-instance-select-header">
                            <small className="discord-instance-select-desc">
                                {t('discord_modal.allowed_instances_desc')}
                            </small>
                            <div className="discord-instance-select-actions">
                                <button
                                    className="discord-instance-select-btn"
                                    onClick={() => setNodeAllInstances(nodeKey, true)}
                                >
                                    {t('discord_modal.select_all')}
                                </button>
                                <button
                                    className="discord-instance-select-btn"
                                    onClick={() => setNodeAllInstances(nodeKey, false)}
                                >
                                    {t('discord_modal.deselect_all')}
                                </button>
                            </div>
                        </div>
                        {!servers || servers.length === 0 ? (
                            <p className="discord-node-empty">{t('discord_modal.no_instances_available')}</p>
                        ) : (
                            <div className="discord-instance-select-list">
                                {servers.map((server) => {
                                    const isAllowed = allowedInsts.includes(server.id);
                                    const ownerNode = getInstanceOwnerNode(server.id, nodeKey);
                                    const isOtherNode = !!ownerNode;
                                    // 다른 노드에 할당된 노드 이름 찾기
                                    const ownerNodeName = isOtherNode
                                        ? (isCloud
                                            ? cloudNodes.find((n) => n.guildId === ownerNode)?.guildName
                                            : localGuilds.find((g) => g.guildId === ownerNode)?.guildName
                                          ) || ownerNode
                                        : '';
                                    return (
                                        <label
                                            key={server.id}
                                            className={clsx('discord-instance-select-item', {
                                                selected: isAllowed,
                                                disabled: isOtherNode,
                                            })}
                                            title={
                                                isOtherNode
                                                    ? t('discord_modal.instance_used_by_other', {
                                                          node: ownerNodeName,
                                                      })
                                                    : ''
                                            }
                                        >
                                            <SabaCheckbox
                                                checked={isAllowed}
                                                disabled={isOtherNode}
                                                onChange={() => toggleNodeInstance(nodeKey, server.id)}
                                            />
                                            <div className="discord-instance-select-info">
                                                <span className="discord-instance-select-name">{server.name}</span>
                                                <span className="discord-instance-select-module">
                                                    {server.module}
                                                    {isOtherNode && (
                                                        <span className="discord-instance-other-node">
                                                            {' '}
                                                            — {ownerNodeName}
                                                        </span>
                                                    )}
                                                </span>
                                            </div>
                                            <span className={clsx('discord-instance-badge', isAllowed ? 'on' : 'off')}>
                                                {isAllowed ? 'ON' : 'OFF'}
                                            </span>
                                        </label>
                                    );
                                })}
                            </div>
                        )}
                    </div>
                )}

                {/* ── 멤버 탭 ── */}
                {currentTab === 'members' && (
                    <div className="discord-node-tab-content">
                        <small className="discord-instance-select-desc">{t('discord_modal.members_desc')}</small>

                        {/* 멤버 로딩 / 봇 미실행 안내 */}
                        {membersLoading && (
                            <div className="discord-cloud-connecting" style={{ padding: '12px 0' }}>
                                <SabaSpinner size="sm" />
                                <span>{t('discord_modal.members_loading')}</span>
                            </div>
                        )}

                        {/* 로컬 모드: 봇 미실행 */}
                        {!membersLoading && !isCloud && discordBotStatus !== 'running' && (
                            <p className="discord-node-empty">{t('discord_modal.members_bot_not_running')}</p>
                        )}

                        {/* 로컬 모드: 멤버 없음 */}
                        {!membersLoading &&
                            !isCloud &&
                            discordBotStatus === 'running' &&
                            availableMembers.length === 0 && (
                                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                                    <p className="discord-node-empty" style={{ margin: 0 }}>
                                        {t('discord_modal.members_empty')}
                                    </p>
                                    <button className="discord-instance-select-btn" onClick={fetchLocalGuildMembers}>
                                        🔄 {t('discord_modal.members_refresh')}
                                    </button>
                                </div>
                            )}

                        {/* 로컬 모드: 새로고침 버튼 (멤버 있을 때) */}
                        {!membersLoading &&
                            !isCloud &&
                            discordBotStatus === 'running' &&
                            availableMembers.length > 0 && (
                                <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 6 }}>
                                    <button className="discord-instance-select-btn" onClick={fetchLocalGuildMembers}>
                                        🔄 {t('discord_modal.members_refresh')}
                                    </button>
                                </div>
                            )}

                        {/* 클라우드 모드: 멤버 없음 + 새로고침 */}
                        {!membersLoading && isCloud && availableMembers.length === 0 && (
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                                <p className="discord-node-empty" style={{ margin: 0 }}>
                                    {t('discord_modal.members_empty')}
                                </p>
                                <button
                                    className="discord-instance-select-btn"
                                    onClick={() => fetchCloudNodeMembers(nodeKey)}
                                >
                                    🔄 {t('discord_modal.members_refresh')}
                                </button>
                            </div>
                        )}

                        {/* 클라우드 모드: 새로고침 버튼 (멤버 있을 때) */}
                        {!membersLoading && isCloud && availableMembers.length > 0 && (
                            <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 6 }}>
                                <button
                                    className="discord-instance-select-btn"
                                    onClick={() => fetchCloudNodeMembers(nodeKey)}
                                >
                                    🔄 {t('discord_modal.members_refresh')}
                                </button>
                            </div>
                        )}

                        {/* 멤버 목록 (체크박스로 활성화/비활성화) */}
                        {availableMembers.length > 0 && (
                            <div className="discord-member-perm-list">
                                {availableMembers.map((member) => {
                                    const isEnabled = !(member.id in memberPerms);
                                    const isExpanded = expandedMember[`${nodeKey}:${member.id}`];

                                    return (
                                        <div
                                            key={member.id}
                                            className={clsx('discord-member-perm-card', { expanded: isExpanded })}
                                        >
                                            <div className="discord-member-perm-header">
                                                <label
                                                    className="discord-member-enable-label"
                                                    onClick={(e) => e.stopPropagation()}
                                                >
                                                    <SabaCheckbox
                                                        checked={isEnabled}
                                                        onChange={() => toggleMemberEnabled(nodeKey, member.id)}
                                                    />
                                                    <div className="discord-member-perm-id-group">
                                                        <span className="discord-member-perm-name">
                                                            {member.displayName || member.username}
                                                        </span>
                                                        <span className="discord-member-perm-id">{member.id}</span>
                                                    </div>
                                                </label>
                                                {!isEnabled && (
                                                    <button
                                                        className="discord-member-expand-btn"
                                                        onClick={() =>
                                                            setExpandedMember((prev) => ({
                                                                ...prev,
                                                                [`${nodeKey}:${member.id}`]:
                                                                    !prev[`${nodeKey}:${member.id}`],
                                                            }))
                                                        }
                                                    >
                                                        <Icon
                                                            name={isExpanded ? 'chevronDown' : 'chevronRight'}
                                                            size="sm"
                                                        />
                                                    </button>
                                                )}
                                            </div>

                                            {!isEnabled && isExpanded && (
                                                <div className="discord-member-perm-body">
                                                    {allowedInsts.length === 0 ? (
                                                        <p className="discord-node-empty">
                                                            {t('discord_modal.no_instances_for_perms')}
                                                        </p>
                                                    ) : (
                                                        allowedInsts.map((serverId) => {
                                                            const srv = servers?.find((s) => s.id === serverId);
                                                            if (!srv) return null;
                                                            const cmds = getCommandsForModule(srv.module);
                                                            const userPerms = memberPerms[member.id] || {};
                                                            const userCmds = Array.isArray(userPerms[serverId])
                                                                ? userPerms[serverId]
                                                                : []; // 차단 목록 (빈 배열 = 모두 허용)

                                                            return (
                                                                <div
                                                                    key={serverId}
                                                                    className="discord-member-instance-block"
                                                                >
                                                                    <div className="discord-member-instance-header">
                                                                        <span className="discord-member-instance-name">
                                                                            {srv.name}
                                                                        </span>
                                                                        <span className="discord-member-instance-module">
                                                                            {srv.module}
                                                                        </span>
                                                                        {cmds.length > 0 && (
                                                                            <div className="discord-instance-select-actions">
                                                                                <button
                                                                                    className="discord-instance-select-btn"
                                                                                    onClick={() =>
                                                                                        setMemberAllCommands(
                                                                                            nodeKey,
                                                                                            member.id,
                                                                                            serverId,
                                                                                            cmds.map((c) => c.name),
                                                                                            true,
                                                                                        )
                                                                                    }
                                                                                >
                                                                                    {t('discord_modal.select_all')}
                                                                                </button>
                                                                                <button
                                                                                    className="discord-instance-select-btn"
                                                                                    onClick={() =>
                                                                                        setMemberAllCommands(
                                                                                            nodeKey,
                                                                                            member.id,
                                                                                            serverId,
                                                                                            cmds.map((c) => c.name),
                                                                                            false,
                                                                                        )
                                                                                    }
                                                                                >
                                                                                    {t('discord_modal.deselect_all')}
                                                                                </button>
                                                                            </div>
                                                                        )}
                                                                    </div>
                                                                    {cmds.length === 0 ? (
                                                                        <p className="discord-cmd-empty">
                                                                            {t('discord_modal.no_commands_available')}
                                                                        </p>
                                                                    ) : (
                                                                        <div className="discord-cmd-check-grid">
                                                                            {cmds.map((cmd) => (
                                                                                <label
                                                                                    key={cmd.name}
                                                                                    className="discord-cmd-check-item"
                                                                                    title={cmd.description}
                                                                                >
                                                                                    <SabaCheckbox
                                                                                        size="sm"
                                                                                        checked={!userCmds.includes(
                                                                                            cmd.name,
                                                                                        )}
                                                                                        onChange={() =>
                                                                                            toggleMemberCommand(
                                                                                                nodeKey,
                                                                                                member.id,
                                                                                                serverId,
                                                                                                cmd.name,
                                                                                            )
                                                                                        }
                                                                                    />
                                                                                    <span className="discord-cmd-check-label">
                                                                                        {cmd.label}
                                                                                    </span>
                                                                                </label>
                                                                            ))}
                                                                        </div>
                                                                    )}
                                                                </div>
                                                            );
                                                        })
                                                    )}
                                                    {allowedInsts.length > 0 && (
                                                        <p className="discord-cmd-hint">
                                                            {t('discord_modal.no_commands_hint')}
                                                        </p>
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                    );
                                })}
                            </div>
                        )}
                    </div>
                )}
            </div>
        );
    };

    if (!isOpen) return null;

    // ── 클라우드 모드 상태 머신 ──
    // no_host → pairing → pair_success → connecting → connected
    //                                               → error
    let cloudState = null;
    if (isCloud) {
        if (pairStatus === 'success') {
            cloudState = 'pair_success';
        } else if (showPairing && !cloudConnected) {
            cloudState = 'pairing';
        } else if (!discordCloudHostId) {
            cloudState = 'no_host';
        } else if (cloudConnected) {
            cloudState = 'connected';
        } else if (cloudConnecting) {
            cloudState = 'connecting';
        } else if (cloudError) {
            cloudState = 'error';
        } else {
            cloudState = 'connecting'; // hostId 설정 직후, useEffect 실행 전
        }
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
                        <button
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
                    <button
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
                    <button
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
        <div className={clsx('discord-modal-container', { closing: isClosing, 'side-panel': isSide })} onClick={(e) => e.stopPropagation()}>
            <div className="discord-modal-header">
                <div className="discord-modal-title">
                    <span
                        className={clsx('status-indicator', {
                            'status-online': discordBotStatus === 'running',
                            'status-connecting': discordBotStatus === 'connecting',
                            'status-offline': discordBotStatus !== 'running' && discordBotStatus !== 'connecting',
                        })}
                    ></span>
                    <h2>{t('discord_modal.title')}</h2>
                </div>
                <button className="discord-modal-close" onClick={onClose}>
                    <Icon name="close" size="sm" />
                </button>
            </div>

            <div className="discord-modal-content">
                {/* ── 상태 표시 ── */}
                <div className="discord-status-section">
                    <div className="discord-status-rows">
                        {isCloud ? (
                            /* 클라우드 모드: 릴레이 서버(클라우드) 연결 상태만 표시 */
                            <div className="discord-status-row">
                                <span className="status-label">{t('discord_modal.status_cloud_label')}</span>
                                <span
                                    className={clsx('status-value', {
                                        'status-running': cloudConnected,
                                        'status-connecting': !cloudConnected && cloudConnecting,
                                        'status-needs-setup':
                                            !cloudConnected && !cloudConnecting && !discordCloudHostId,
                                        'status-stopped': !cloudConnected && !cloudConnecting && discordCloudHostId,
                                    })}
                                >
                                    {cloudConnected
                                        ? t('discord_modal.status_relay_connected')
                                        : cloudConnecting
                                          ? t('discord_modal.status_relay_connecting')
                                          : !discordCloudHostId
                                            ? t('discord_modal.status_relay_needs_setup')
                                            : t('discord_modal.status_relay_disconnected')}
                                </span>
                            </div>
                        ) : (
                            /* 로컬 모드: 봇 프로세스 상태 */
                            <div className="discord-status-row">
                                <span className="status-label">{t('discord_modal.status_bot_label')}</span>
                                <span className={clsx('status-value', `status-${discordBotStatus}`)}>
                                    {discordBotStatus === 'running'
                                        ? t('discord_modal.status_running')
                                        : discordBotStatus === 'error'
                                          ? t('discord_modal.status_error')
                                          : t('discord_modal.status_stopped')}
                                </span>
                            </div>
                        )}
                    </div>
                    {isCloud ? (
                        <span className="discord-mode-badge cloud">
                            <Icon name="cloud" size="sm" /> {t('discord_modal.mode_cloud')}
                        </span>
                    ) : (
                        <span className="discord-mode-badge local">
                            <Icon name="desktop" size="sm" /> {t('discord_modal.mode_local')}
                        </span>
                    )}
                </div>

                {/* ── 모드 전환 카드 ── */}
                <div className="discord-mode-toggle-card">
                    <div className="discord-mode-toggle-info">
                        <span className="discord-mode-toggle-icon">
                            {isCloud ? <Icon name="cloud" size="md" /> : <Icon name="desktop" size="md" />}
                        </span>
                        <div className="discord-mode-toggle-text">
                            <span className="discord-mode-toggle-label">{t('discord_modal.mode_label')}</span>
                            <span className="discord-mode-toggle-desc">
                                {isCloud ? t('discord_modal.mode_cloud_desc') : t('discord_modal.mode_local_desc')}
                            </span>
                        </div>
                    </div>
                    <SabaToggle
                        size="lg"
                        checked={isCloud}
                        onChange={(checked) => {
                            const newMode = checked ? 'cloud' : 'local';
                            // 디바운스 + 자동 재시작 (switchMode가 stop → start 처리)
                            setDiscordBotMode(newMode);
                        }}
                    />
                </div>

                {/* ══════════════════════════════════════════════ */}
                {/* ── 로컬 모드: 길드별 노드 설정 ──────────────── */}
                {/* ══════════════════════════════════════════════ */}
                {!isCloud && (
                    <div className="discord-config-section">
                        <h4>
                            <Icon name="desktop" size="sm" /> {t('discord_modal.local_node_title')}
                        </h4>
                        {discordBotStatus !== 'running' ? (
                            <p className="discord-node-empty">
                                {t('discord_modal.local_bot_not_running_for_guilds')}
                            </p>
                        ) : localGuilds.length === 0 ? (
                            <div className="discord-cloud-connecting" style={{ padding: '12px 0' }}>
                                <SabaSpinner size="sm" />
                                <span>{t('discord_modal.local_loading_guilds')}</span>
                            </div>
                        ) : (
                            <div className="discord-node-list">
                                {localGuilds.map((guild) => (
                                    <div
                                        key={guild.guildId}
                                        className={clsx('discord-node-card', {
                                            expanded: expandedNode === guild.guildId,
                                        })}
                                    >
                                        <div
                                            className="discord-node-card-header"
                                            onClick={() => toggleNodeExpand(guild.guildId)}
                                        >
                                            <div className="discord-node-card-info">
                                                <span className="discord-node-guild-name">
                                                    {guild.guildName}
                                                </span>
                                                <span className="discord-node-guild-id">{guild.guildId}</span>
                                            </div>
                                            <Icon
                                                name={expandedNode === guild.guildId ? 'chevronDown' : 'chevronRight'}
                                                size="sm"
                                            />
                                        </div>
                                        {expandedNode === guild.guildId && (
                                            <div className="discord-node-card-body">
                                                {renderNodeSettingsBody(guild.guildId, guild.guildName)}
                                            </div>
                                        )}
                                    </div>
                                ))}
                            </div>
                        )}
                    </div>
                )}

                {/* ══════════════════════════════════════════════ */}
                {/* ── 클라우드: 페어링 성공 (자동 전환 대기) ── */}
                {/* ══════════════════════════════════════════════ */}
                {cloudState === 'pair_success' && <div className="discord-cloud-section">{renderPairingBlock()}</div>}

                {/* ══════════════════════════════════════════════ */}
                {/* ── 클라우드: 호스트 미설정 → 셋업 카드 ───── */}
                {/* ══════════════════════════════════════════════ */}
                {cloudState === 'no_host' && (
                    <div className="discord-cloud-section">
                        <div className="discord-cloud-setup-card">
                            <div className="discord-cloud-setup-icon">🔗</div>
                            <h4>{t('discord_modal.cloud_setup_title')}</h4>
                            <p>{t('discord_modal.cloud_setup_desc_simple')}</p>

                            <button
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
                        <button
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
                                <p>{t('discord_modal.cloud_connection_error', { error: cloudError })}</p>
                                <small className="discord-cloud-error-hint">
                                    Host ID: {discordCloudHostId} → {effectiveRelayUrl}
                                </small>
                            </div>
                            <div className="discord-cloud-error-actions">
                                <button className="discord-pair-start-btn" onClick={loadCloudNodes}>
                                    🔄 {t('discord_modal.cloud_retry')}
                                </button>
                                <button
                                    className="discord-pair-start-btn"
                                    onClick={() => {
                                        setShowPairing(true);
                                        startPairing();
                                    }}
                                >
                                    🔗 {t('discord_modal.cloud_re_pair')}
                                </button>
                                <button className="discord-pair-start-btn discord-btn-danger" onClick={disconnectCloud}>
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
                                <button
                                    className="discord-pair-start-btn"
                                    style={{ fontSize: 11, padding: '3px 10px' }}
                                    onClick={loadCloudNodes}
                                >
                                    🔄
                                </button>
                                <button
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
                                        <div
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
                                                name={expandedNode === node.guildId ? 'chevronDown' : 'chevronRight'}
                                                size="sm"
                                            />
                                        </div>

                                        {expandedNode === node.guildId && (
                                            <div className="discord-node-card-body">
                                                {renderNodeSettingsBody(node.guildId, node.guildName || node.guildId)}
                                            </div>
                                        )}
                                    </div>
                                ))}
                            </div>
                        )}

                        {cloudNodes.length === 0 && !showPairing && (
                            <div className="discord-cloud-empty-nodes">
                                <p>{t('discord_modal.cloud_no_nodes')}</p>
                                <small>{t('discord_modal.cloud_no_nodes_hint')}</small>
                            </div>
                        )}

                        {/* 노드 추가 버튼 (항상 표시) */}
                        <button
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
                                ? '✕ ' + t('discord_modal.pair_section_title')
                                : '➕ ' + t('discord_modal.cloud_add_node')}
                        </button>
                        {showPairing && renderPairingBlock()}
                    </div>
                )}

                {/* ══════════════════════════════════════════════ */}
                {/* ── 공통 설정 섹션 ─────────────────────────── */}
                {/* ══════════════════════════════════════════════ */}
                <div className="discord-config-section">
                    {!isCloud && (
                        <div className="discord-form-group">
                            <label>
                                <Icon name="key" size="sm" /> {t('discord_modal.token_label')}
                            </label>
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
                                <SabaCheckbox
                                    checked={discordAutoStart}
                                    onChange={(checked) => setDiscordAutoStart(checked)}
                                />
                                {t('discord_modal.auto_start_label')}
                            </label>
                        </div>
                    )}
                </div>

                <div className="discord-info-box">
                    <h4>
                        <Icon name="lightbulb" size="sm" /> {t('discord_modal.usage_title')}
                    </h4>
                    <p>{t('discord_modal.usage_instruction')}</p>
                    <code>{discordPrefix || '!saba'} [module] [command]</code>
                    <p className="info-note">{t('discord_modal.usage_note')}</p>
                </div>

                {!isCloud && (
                    <>
                        <div
                            className={clsx(
                                'discord-music-toggle-card',
                                !musicExtEnabled && 'disabled',
                                musicExtEnabled && 'clickable',
                            )}
                            onClick={() => {
                                if (musicExtEnabled && !showMusicSettings) openMusicSettings();
                                else if (showMusicSettings) setShowMusicSettings(false);
                            }}
                        >
                            <div className="discord-music-toggle-info">
                                <span className="discord-music-toggle-icon">🎵</span>
                                <div className="discord-music-toggle-text">
                                    <span className="discord-music-toggle-label">
                                        {t('discord_modal.music_toggle_label')}
                                    </span>
                                    <span className="discord-music-toggle-desc">
                                        {musicExtEnabled
                                            ? t('discord_modal.music_toggle_description')
                                            : t('discord_modal.music_ext_disabled')}
                                    </span>
                                </div>
                            </div>
                            <div onClick={(e) => e.stopPropagation()}>
                                <SabaToggle
                                    checked={musicExtEnabled && discordMusicEnabled}
                                    onChange={(checked) => setDiscordMusicEnabled(checked)}
                                    disabled={!musicExtEnabled}
                                />
                            </div>
                        </div>

                        {showMusicSettings && musicExtEnabled && (
                            <div className="discord-music-settings-panel" ref={musicSettingsRef}>
                                <div className="discord-music-settings-header">
                                    <span className="discord-music-settings-title">
                                        {t('discord_modal.music_settings_title')}
                                    </span>
                                </div>
                                <div className="discord-music-settings-body">
                                    {/* M1: 모듈 별명 — 누락된 입력 필드 추가 */}
                                    <div className="discord-music-alias-section">
                                        <h4>
                                            <Icon name="tag" size="sm" />
                                            {t('discord_modal.music_module_aliases_title')}
                                        </h4>
                                        <small>{t('discord_modal.music_module_aliases_desc')}</small>
                                        <input
                                            className="discord-music-module-alias-input"
                                            type="text"
                                            placeholder={t('discord_modal.music_module_aliases_placeholder')}
                                            value={musicModuleAliases}
                                            onChange={(e) => setMusicModuleAliases(e.target.value)}
                                        />
                                    </div>

                                    {/* 명령어 별명 */}
                                    <div className="discord-music-alias-section">
                                        <h4>
                                            <Icon name="zap" size="sm" />
                                            {t('discord_modal.music_command_aliases_title')}
                                        </h4>
                                        <small>{t('discord_modal.music_command_aliases_desc')}</small>
                                        <div className="discord-music-cmd-grid">
                                            {MUSIC_COMMAND_KEYS.map((cmd) => {
                                                // i18n에서 현재 언어의 별명 로드, 범용 별명과 병합
                                                const i18nAliases = i18n.t(`bot:music_commands.${cmd}`, { returnObjects: true });
                                                const langAliases = Array.isArray(i18nAliases) ? i18nAliases : [];
                                                const universalAliases = UNIVERSAL_COMMAND_ALIASES[cmd] || [];
                                                const seen = new Set();
                                                const defaultAliases = [];
                                                for (const a of [...langAliases, ...universalAliases]) {
                                                    const lower = a.toLowerCase();
                                                    if (!seen.has(lower)) {
                                                        seen.add(lower);
                                                        defaultAliases.push(a);
                                                    }
                                                }

                                                const currentVal = musicCommandAliases[cmd] || '';
                                                const currentArr = currentVal
                                                    .split(',')
                                                    .map((a) => a.trim())
                                                    .filter((a) => a.length > 0);
                                                return (
                                                    <div key={cmd} className="discord-music-cmd-row">
                                                        <div className="discord-music-cmd-info">
                                                            <span className="discord-music-cmd-name">
                                                                {t(`discord_modal.music_cmd_${cmd}`,
                                                                    { defaultValue: cmd })}
                                                            </span>
                                                            <span className="discord-music-cmd-desc">
                                                                {t(`discord_modal.music_cmd_${cmd}_desc`,
                                                                    { defaultValue: '' })}
                                                            </span>
                                                        </div>
                                                        <div>
                                                            <input
                                                                className="discord-music-cmd-input"
                                                                type="text"
                                                                placeholder={
                                                                    defaultAliases.join(', ') +
                                                                    ' (' + t('discord_modal.music_command_aliases_placeholder') + ')'
                                                                }
                                                                value={currentVal}
                                                                onChange={(e) =>
                                                                    setMusicCommandAliases((prev) => ({
                                                                        ...prev,
                                                                        [cmd]: e.target.value,
                                                                    }))
                                                                }
                                                            />
                                                            <div className="discord-music-alias-badges">
                                                                {currentArr.length === 0
                                                                    ? defaultAliases.map((a) => (
                                                                        <span
                                                                            key={a}
                                                                            className="discord-music-alias-badge default"
                                                                        >
                                                                            {a}
                                                                        </span>
                                                                    ))
                                                                    : currentArr.map((a) => (
                                                                        <span
                                                                            key={a}
                                                                            className="discord-music-alias-badge"
                                                                        >
                                                                            {a}
                                                                        </span>
                                                                    ))}
                                                            </div>
                                                        </div>
                                                    </div>
                                                );
                                            })}
                                        </div>
                                    </div>

                                    {/* 저장 / 초기화 버튼 */}
                                    <div className="discord-music-settings-actions">
                                        <button className="btn btn-save" onClick={handleSaveMusicAliases}>
                                            <Icon name="save" size="sm" />
                                            {t('discord_modal.music_aliases_save')}
                                        </button>
                                        <button className="btn btn-reset" onClick={handleResetMusicAliases}>
                                            <Icon name="refresh" size="sm" />
                                            {t('discord_modal.music_aliases_reset')}
                                        </button>
                                    </div>

                                    {/* 전용 음악 채널 설정 */}
                                    <div className="discord-music-alias-section discord-music-channel-section">
                                        <h4>
                                            <Icon name="hash" size="sm" />
                                            {t('discord_modal.music_channel_title')}
                                        </h4>
                                        <small>{t('discord_modal.music_channel_desc')}</small>

                                        {discordBotStatus !== 'running' ? (
                                            <div className="discord-music-channel-offline">
                                                {t('discord_modal.music_channel_bot_offline')}
                                            </div>
                                        ) : channelsLoading ? (
                                            <div className="discord-music-channel-loading">
                                                <SabaSpinner size="sm" />
                                                {t('discord_modal.music_channel_loading')}
                                            </div>
                                        ) : channelsError ? (
                                            <div className="discord-music-channel-offline">
                                                ⚠️ {channelsError}
                                                <button className="btn btn-refresh-channels" onClick={loadGuildChannels} style={{ marginLeft: 8 }}>
                                                    <Icon name="refresh" size="sm" />
                                                </button>
                                            </div>
                                        ) : (
                                            <>
                                                <div className="discord-music-channel-select-row">
                                                    <select
                                                        className="discord-music-channel-select"
                                                        value={discordMusicChannelId || ''}
                                                        onChange={(e) => setDiscordMusicChannelId(e.target.value)}
                                                    >
                                                        <option value="">
                                                            {t('discord_modal.music_channel_none')}
                                                        </option>
                                                        {guildChannels &&
                                                            Object.entries(guildChannels).map(
                                                                ([guildId, guild]) => (
                                                                    <optgroup
                                                                        key={guildId}
                                                                        label={guild.guildName}
                                                                    >
                                                                        {guild.channels.map((ch) => (
                                                                            <option
                                                                                key={ch.id}
                                                                                value={ch.id}
                                                                            >
                                                                                #{ch.name}
                                                                                {ch.parentName
                                                                                    ? ` (${ch.parentName})`
                                                                                    : ''}
                                                                            </option>
                                                                        ))}
                                                                    </optgroup>
                                                                ),
                                                            )}
                                                    </select>
                                                    <button
                                                        className="btn btn-refresh-channels"
                                                        onClick={loadGuildChannels}
                                                        title={t('discord_modal.music_channel_refresh_list')}
                                                    >
                                                        <Icon name="refresh" size="sm" />
                                                    </button>
                                                </div>
                                            </>
                                        )}

                                        <div className="discord-music-channel-options">
                                            <div className="discord-music-channel-option">
                                                <label>{t('discord_modal.music_channel_queue_lines')}</label>
                                                <input
                                                    className="discord-music-channel-number-input"
                                                    type="number"
                                                    min="5"
                                                    max="10"
                                                    value={discordMusicUISettings?.queueLines || 5}
                                                    onChange={(e) => {
                                                        const val = Math.min(10, Math.max(5, parseInt(e.target.value, 10) || 5));
                                                        setDiscordMusicUISettings({
                                                            ...discordMusicUISettings,
                                                            queueLines: val,
                                                        });
                                                    }}
                                                />
                                            </div>
                                            <div className="discord-music-channel-option">
                                                <label>{t('discord_modal.music_channel_refresh')}</label>
                                                <select
                                                    className="discord-music-channel-select"
                                                    value={discordMusicUISettings?.refreshInterval || 4000}
                                                    onChange={(e) =>
                                                        setDiscordMusicUISettings({
                                                            ...discordMusicUISettings,
                                                            refreshInterval: parseInt(e.target.value, 10),
                                                        })
                                                    }
                                                >
                                                    <option value={3000}>3{t('discord_modal.music_channel_sec')}</option>
                                                    <option value={4000}>4{t('discord_modal.music_channel_sec')}</option>
                                                    <option value={5000}>5{t('discord_modal.music_channel_sec')}</option>
                                                </select>
                                            </div>
                                            <div className="discord-music-channel-option">
                                                <label>{t('discord_modal.music_normalize')}</label>
                                                <SabaToggle
                                                    checked={discordMusicUISettings?.normalize !== false}
                                                    onChange={(checked) =>
                                                        setDiscordMusicUISettings({
                                                            ...discordMusicUISettings,
                                                            normalize: checked,
                                                        })
                                                    }
                                                />
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        )}
                    </>
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
                        className={clsx(
                            'discord-btn',
                            discordBotStatus === 'running' ? 'discord-btn-stop' : 'discord-btn-start',
                        )}
                        onClick={() =>
                            discordBotStatus === 'running' ? handleStopDiscordBot() : handleStartDiscordBot()
                        }
                    >
                        {discordBotStatus === 'running'
                            ? t('discord_modal.stop_button')
                            : t('discord_modal.start_button')}
                    </button>
                )}
                {isCloud && cloudState === 'connected' && (
                    <button
                        className={clsx(
                            'discord-btn',
                            discordBotStatus === 'running' ? 'discord-btn-stop' : 'discord-btn-start',
                        )}
                        onClick={() =>
                            discordBotStatus === 'running' ? handleStopDiscordBot() : handleStartDiscordBot()
                        }
                    >
                        {discordBotStatus === 'running'
                            ? t('discord_modal.agent_stop_button')
                            : t('discord_modal.agent_start_button')}
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
