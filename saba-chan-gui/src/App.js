import React, { useState, useEffect, useRef, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import './App.css';
import { 
    SuccessModal, 
    FailureModal, 
    NotificationModal, 
    QuestionModal,
    CommandModal,
    Toast,
    TitleBar,
    SettingsModal,
    DiscordBotModal,
    BackgroundModal,
    AddServerModal,
    Icon
} from './components';

function App() {
    const { t, i18n } = useTranslation('gui');

    // 언어별 로고 이미지 선택
    const logoSrc = useMemo(() => {
        const lang = (i18n.language || 'en').toLowerCase();
        if (lang.startsWith('ko')) return '/logo-kr.png';
        if (lang.startsWith('ja')) return '/logo-jp.png';
        return '/logo-en.png';
    }, [i18n.language]);
    
    // 테스트 환경 감지 (Jest 실행 중인지 확인)
    const isTestEnv = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
    
    // 테스트 환경에서만 로그 억제
    const debugLog = (...args) => {
        if (!isTestEnv) console.log(...args);
    };
    const debugWarn = (...args) => {
        if (!isTestEnv) console.warn(...args);
    };
    
    // 에러 메시지 변환 함수 (사용자 친화적으로)
    const translateError = (errorMessage) => {
        if (!errorMessage) return t('errors.unknown_error');
        
        const msg = String(errorMessage);
        
        // 파일 경로 관련 에러
        if (msg.includes('Executable not found') || msg.includes('executable not found')) {
            return t('errors.executable_not_found');
        }
        if (msg.includes('No such file or directory')) {
            return t('errors.file_not_found');
        }
        if (msg.includes('Permission denied')) {
            return t('errors.permission_denied');
        }
        
        // 네트워크 연결 에러
        if (msg.includes('ECONNREFUSED')) {
            return t('errors.daemon_connection_refused');
        }
        if (msg.includes('ETIMEDOUT')) {
            return t('errors.request_timeout');
        }
        if (msg.includes('ENOTFOUND')) {
            return t('errors.server_not_found');
        }
        if (msg.includes('Network Error') || msg.includes('network error')) {
            return t('errors.network_error');
        }
        
        // 서버 시작/정지 에러
        if (msg.includes('Module failed to start')) {
            return t('errors.module_failed_to_start');
        }
        if (msg.includes('Failed to stop')) {
            return t('errors.failed_to_stop');
        }
        if (msg.includes('Already running')) {
            return t('errors.already_running');
        }
        if (msg.includes('Not running')) {
            return t('errors.not_running');
        }
        
        // 프로세스 관련 에러
        if (msg.includes('Process not found')) {
            return t('errors.process_not_found');
        }
        if (msg.includes('Process crashed')) {
            return t('errors.process_crashed');
        }
        
        // 설정 관련 에러
        if (msg.includes('Invalid configuration') || msg.includes('invalid config')) {
            return t('errors.invalid_configuration');
        }
        if (msg.includes('Missing required field')) {
            return t('errors.missing_required_field');
        }
        
        // 모듈 관련 에러
        if (msg.includes('Module not found')) {
            return t('errors.module_not_found');
        }
        if (msg.includes('Failed to load module')) {
            return t('errors.failed_to_load_module');
        }
        
        // Discord 봇 관련 에러
        if (msg.includes('Invalid token') || msg.includes('invalid token')) {
            return t('errors.invalid_token');
        }
        if (msg.includes('Bot connection failed')) {
            return t('errors.network_error');
        }
        
        // 일반적인 에러 (원본 메시지 반환)
        return msg;
    };
    
    // 로딩 화면 상태
    const [daemonReady, setDaemonReady] = useState(false);
    const [initStatus, setInitStatus] = useState('Initialize...');
    const [initProgress, setInitProgress] = useState(0);
    const [serversInitializing, setServersInitializing] = useState(true); // 서버 상태 안정화 대기
    
    const [servers, setServers] = useState([]);
    const [modules, setModules] = useState([]);
    const [loading, setLoading] = useState(true);

    // 업타임 실시간 계산용 (1초마다 갱신)
    const [nowEpoch, setNowEpoch] = useState(() => Math.floor(Date.now() / 1000));
    useEffect(() => {
        const timer = setInterval(() => setNowEpoch(Math.floor(Date.now() / 1000)), 1000);
        return () => clearInterval(timer);
    }, []);

    const formatUptime = (startTime) => {
        if (!startTime) return null;
        const elapsed = Math.max(0, nowEpoch - startTime);
        const h = String(Math.floor(elapsed / 3600)).padStart(2, '0');
        const m = String(Math.floor((elapsed % 3600) / 60)).padStart(2, '0');
        const s = String(elapsed % 60).padStart(2, '0');
        return `${h}:${m}:${s}`;
    };
    const [autoRefresh, setAutoRefresh] = useState(true);
    const [refreshInterval, setRefreshInterval] = useState(2000); // 2초마다 업데이트
    const [showModuleManager, setShowModuleManager] = useState(false);
    const [newServerName, setNewServerName] = useState('');
    const [selectedModule, setSelectedModule] = useState('');
    const [executablePath, setExecutablePath] = useState('');
    const [modulesPath, setModulesPath] = useState(''); // 설정에서 로드
    const [settingsPath, setSettingsPath] = useState('');
    
    // Settings 모달 상태
    const [showSettingsModal, setShowSettingsModal] = useState(false);
    const [settingsServer, setSettingsServer] = useState(null);
    const [settingsValues, setSettingsValues] = useState({});
    const [settingsActiveTab, setSettingsActiveTab] = useState('general'); // 'general' | 'aliases'
    
    // Command 모달 상태
    const [showCommandModal, setShowCommandModal] = useState(false);
    const [commandServer, setCommandServer] = useState(null);
    
    // GUI 설정 모달 상태
    const [showGuiSettingsModal, setShowGuiSettingsModal] = useState(false);
    
    // 모달 상태 (Success/Failure/Notification)
    const [modal, setModal] = useState(null);

    // Discord Bot 상태
    const [discordBotStatus, setDiscordBotStatus] = useState('stopped'); // stopped | running | error
    const [discordToken, setDiscordToken] = useState('');
    const [showDiscordSection, setShowDiscordSection] = useState(false);
    const [showBackgroundSection, setShowBackgroundSection] = useState(false);
    const [discordPrefix, setDiscordPrefix] = useState('!saba');  // 기본값: !saba
    const [discordAutoStart, setDiscordAutoStart] = useState(false);
    const [discordModuleAliases, setDiscordModuleAliases] = useState({});  // 저장된 사용자 커스텀 모듈 별명
    const [discordCommandAliases, setDiscordCommandAliases] = useState({});  // 저장된 사용자 커스텀 명령어 별명

    // Background Daemon 상태
    const [backgroundDaemonStatus, setBackgroundDaemonStatus] = useState('checking'); // checking | running | stopped | error

    // 초기화 완료 플래그 (state로 변경)
    const [botStatusReady, setBotStatusReady] = useState(false);
    const [settingsReady, setSettingsReady] = useState(false);
    const autoStartDoneRef = useRef(false);

    // 모듈별 별명 (각 모듈의 module.toml에서 정의한 별명들)
    const [moduleAliasesPerModule, setModuleAliasesPerModule] = useState({});  // { moduleName: { moduleAliases: [...], commands: {...} } }
    const [selectedModuleForAliases, setSelectedModuleForAliases] = useState(null);
    const [editingModuleAliases, setEditingModuleAliases] = useState({});
    const [editingCommandAliases, setEditingCommandAliases] = useState({});

    // 초기화 상태 모니터링
    useEffect(() => {
        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Init Status]', data.step, ':', data.message);
                
                const statusMessages = {
                    init: 'Initialize...',
                    ui: 'UI loaded',
                    daemon: 'Daemon preparing...',
                    modules: 'Loading modules...',
                    instances: 'Loading instances...',
                    ready: 'Ready!'
                };
                
                const progressValues = {
                    init: 10,
                    ui: 20,
                    daemon: 50,
                    modules: 70,
                    instances: 90,
                    ready: 100
                };
                
                setInitStatus(statusMessages[data.step] || data.message);
                setInitProgress(progressValues[data.step] || initProgress);
                
                // 'ready' 상태에 도달하면 UI 활성화
                if (data.step === 'ready') {
                    setTimeout(() => setDaemonReady(true), 600);
                    // 서버 상태 안정화 대기 (3초 후 초기화 완료)
                    setTimeout(() => setServersInitializing(false), 3500);
                }
            });
        }
    }, []);

    // 설정 로드
    useEffect(() => {
        const loadSettings = async () => {
            try {
                const isTestEnv = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
                
                // 1. GUI 설정 로드
                const settings = await window.api.settingsLoad();
                if (!isTestEnv) console.log('[Settings] Loaded:', settings);
                if (settings) {
                    setAutoRefresh(settings.autoRefresh ?? true);
                    setRefreshInterval(settings.refreshInterval ?? 2000);
                    setModulesPath(settings.modulesPath || '');
                    setDiscordToken(settings.discordToken || '');
                    setDiscordAutoStart(settings.discordAutoStart ?? false);
                    if (!isTestEnv) console.log('[Settings] discordAutoStart:', settings.discordAutoStart, 'discordToken:', settings.discordToken ? 'YES' : 'NO');
                }
                const path = await window.api.settingsGetPath();
                setSettingsPath(path);
                if (!isTestEnv) console.log('[Settings] GUI settings loaded from:', path);
                
                // 2. Bot 설정 로드 (별도)
                const botCfg = await window.api.botConfigLoad();
                if (botCfg) {
                    setDiscordPrefix(botCfg.prefix || '!saba');
                    setDiscordModuleAliases(botCfg.moduleAliases || {});
                    setDiscordCommandAliases(botCfg.commandAliases || {});
                    if (!isTestEnv) console.log('[Settings] Bot config loaded, prefix:', botCfg.prefix);
                }
                
                // 설정 로드 완료
                setSettingsReady(true);
                if (!isTestEnv) console.log('[Settings] Ready flag set to true');
            } catch (error) {
                console.error('[Settings] Failed to load settings:', error);
                setSettingsReady(true);
            }
        };
        loadSettings();
    }, []);

    // Background Daemon 상태 주기적 확인
    useEffect(() => {
        if (!daemonReady) return;

        const checkDaemonStatus = async () => {
            try {
                if (window.api && window.api.daemonStatus) {
                    const status = await window.api.daemonStatus();
                    setBackgroundDaemonStatus(status.running ? 'running' : 'stopped');
                } else {
                    setBackgroundDaemonStatus('error');
                }
            } catch (error) {
                console.error('Failed to check daemon status:', error);
                setBackgroundDaemonStatus('error');
            }
        };

        // 초기 상태 확인
        checkDaemonStatus();

        // 5초마다 상태 확인
        const interval = setInterval(checkDaemonStatus, 5000);

        return () => clearInterval(interval);
    }, [daemonReady]);

    // bot-config.json 로드
    const loadBotConfig = async () => {
        try {
            const botCfg = await window.api.botConfigLoad();
            if (botCfg) {
                setDiscordPrefix(botCfg.prefix || '!saba');
                setDiscordModuleAliases(botCfg.moduleAliases || {});
                setDiscordCommandAliases(botCfg.commandAliases || {});
            }
        } catch (err) {
            console.error('Failed to load bot config:', err);
        }
    };

    // 설정 저장 함수 (settings.json - Discord 별칭 제외)
    const saveCurrentSettings = async () => {
        if (!settingsPath) {
            console.warn('[Settings] Settings path not initialized, skipping save');
            return;
        }
        try {
            await window.api.settingsSave({
                autoRefresh,
                refreshInterval,
                modulesPath,
                discordToken,
                discordAutoStart
            });
            console.log('[Settings] GUI settings saved');
        } catch (error) {
            console.error('[Settings] Failed to save GUI settings:', error);
        }
    };

    // Bot Config 저장 함수 (prefix, moduleAliases, commandAliases)
    const saveBotConfig = async (newPrefix = discordPrefix) => {
        try {
            const payload = {
                prefix: newPrefix || '!saba',
                moduleAliases: discordModuleAliases,
                commandAliases: discordCommandAliases
            };
            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
            } else {
                console.log('[Settings] Bot config saved, prefix:', newPrefix);
            }
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
        }
    };

    // API 호출 재시도 헬퍼 (exponential backoff)
    const retryWithBackoff = async (fn, maxRetries = 3, initialDelay = 500) => {
        for (let i = 0; i < maxRetries; i++) {
            try {
                return await fn();
            } catch (error) {
                if (i === maxRetries - 1) {
                    throw error;
                }
                const delay = initialDelay * Math.pow(2, i);
                debugWarn(`Attempt ${i + 1} failed, retrying in ${delay}ms...`, error.message);
                await new Promise((resolve) => setTimeout(resolve, delay));
            }
        }
    };

    // Daemon 준비 확인
    const waitForDaemon = async (timeout = 10000) => {
        const start = Date.now();
        while (Date.now() - start < timeout) {
            try {
                const status = await window.api.daemonStatus();
                if (status.running) {
                    console.log('✓ Daemon is ready');
                    return true;
                }
            } catch (err) {
                // 무시
            }
            await new Promise((resolve) => setTimeout(resolve, 500));
        }
        throw new Error('Daemon startup timeout');
    };

    // ======== 드래그 앤 드롭 순서 변경 (Pointer Events 기반) ========
    const cardRefs = useRef({});
    const dragRef = useRef({ active: false, draggedName: null });
    const [draggedName, setDraggedName] = useState(null);
    const skipNextClick = useRef(false);

    const handleCardPointerDown = (e, index) => {
        if (e.button !== 0) return;
        if (e.target.closest('button') || e.target.closest('.action-icon')) return;

        const name = servers[index].name;
        const card = cardRefs.current[name];
        if (!card) return;

        const rect = card.getBoundingClientRect();

        // 모든 카드의 슬롯 위치 스냅샷 (드래그 시작 시점의 레이아웃)
        const slotPositions = servers.map(s => {
            const el = cardRefs.current[s.name];
            if (!el) return null;
            const r = el.getBoundingClientRect();
            return { x: r.left, y: r.top, w: r.width, h: r.height };
        });

        dragRef.current = {
            active: false,
            draggedName: name,
            fromSlot: index,
            targetSlot: index,
            startX: e.clientX,
            startY: e.clientY,
            offsetX: e.clientX - rect.left,
            offsetY: e.clientY - rect.top,
            slotPositions,
            originalOrder: servers.map(s => s.name),
            nameToId: Object.fromEntries(servers.map(s => [s.name, s.id])),
        };

        const onMove = (me) => {
            const d = dragRef.current;
            if (!d.draggedName) return;

            const dx = me.clientX - d.startX;
            const dy = me.clientY - d.startY;

            // 활성화 임계값 (6px 이상 이동 시 드래그 시작)
            if (!d.active) {
                if (Math.abs(dx) < 6 && Math.abs(dy) < 6) return;
                d.active = true;
                setDraggedName(d.draggedName);
                const dragCard = cardRefs.current[d.draggedName];
                if (dragCard) {
                    dragCard.style.transition = 'box-shadow 0.2s ease, opacity 0.2s ease';
                }
            }

            // 드래그 중인 카드를 커서 따라 이동
            const dragCard = cardRefs.current[d.draggedName];
            if (dragCard) {
                dragCard.style.transform = `translate(${dx}px, ${dy}px)`;
            }

            // 가장 가까운 슬롯 찾기
            let targetSlot = d.targetSlot;
            let minDist = Infinity;
            for (let i = 0; i < d.slotPositions.length; i++) {
                const slot = d.slotPositions[i];
                if (!slot) continue;
                const cx = slot.x + slot.w / 2;
                const cy = slot.y + slot.h / 2;
                const dist = Math.hypot(me.clientX - cx, me.clientY - cy);
                if (dist < minDist) {
                    minDist = dist;
                    targetSlot = i;
                }
            }

            if (targetSlot !== d.targetSlot) {
                d.targetSlot = targetSlot;

                // 새로운 시각적 순서 계산
                const order = [...d.originalOrder];
                const draggedIdx = order.indexOf(d.draggedName);
                const [item] = order.splice(draggedIdx, 1);
                order.splice(targetSlot, 0, item);

                // 다른 카드들을 목표 슬롯 위치로 CSS transform 이동
                order.forEach((cardName, newSlotIdx) => {
                    if (cardName === d.draggedName) return;
                    const el = cardRefs.current[cardName];
                    if (!el) return;

                    const origSlotIdx = d.originalOrder.indexOf(cardName);
                    const origPos = d.slotPositions[origSlotIdx];
                    const targetPos = d.slotPositions[newSlotIdx];
                    if (!origPos || !targetPos) return;

                    const tx = targetPos.x - origPos.x;
                    const ty = targetPos.y - origPos.y;

                    if (Math.abs(tx) < 1 && Math.abs(ty) < 1) {
                        el.style.transform = '';
                    } else {
                        el.style.transform = `translate(${tx}px, ${ty}px)`;
                    }
                });
            }
        };

        const onUp = async () => {
            document.removeEventListener('pointermove', onMove);
            document.removeEventListener('pointerup', onUp);

            const d = dragRef.current;

            // 모든 카드 인라인 스타일 정리
            Object.values(cardRefs.current).forEach(el => {
                if (el) {
                    el.style.transform = '';
                    el.style.transition = '';
                }
            });

            const wasActive = d.active;
            const { targetSlot, fromSlot, originalOrder, nameToId } = d;

            dragRef.current = { active: false, draggedName: null };
            setDraggedName(null);

            // 드래그 후 클릭 방지
            if (wasActive) {
                skipNextClick.current = true;
                requestAnimationFrame(() => { skipNextClick.current = false; });
            }

            if (!wasActive || targetSlot === fromSlot) return;

            // 최종 순서 계산 및 적용
            const order = [...originalOrder];
            const draggedIdx = order.indexOf(d.draggedName);
            const [item] = order.splice(draggedIdx, 1);
            order.splice(targetSlot, 0, item);

            setServers(prev => {
                const byName = {};
                prev.forEach(s => { byName[s.name] = s; });
                return order.map(n => byName[n]);
            });

            // 백엔드에 순서 저장
            try {
                const orderedIds = order.map(n => nameToId[n]);
                await window.api.instanceReorder(orderedIds);
                debugLog('Server order saved:', orderedIds);
            } catch (err) {
                debugWarn('Failed to save server order:', err);
            }
        };

        document.addEventListener('pointermove', onMove);
        document.addEventListener('pointerup', onUp);
    };

    // 이전 설정값 추적 (초기 로드와 사용자 변경 구분)
    const prevSettingsRef = useRef(null);
    const prevPrefixRef = useRef(null);

    // refreshInterval 변경 시 저장 (autoRefresh는 항상 true로 고정)
    useEffect(() => {
        // 초기 로드 완료 전에는 저장하지 않음
        if (!settingsReady || !settingsPath) return;
        
        const currentSettings = { autoRefresh, refreshInterval };
        
        // 첫 번째 호출 시 초기값 저장만 하고 저장하지 않음
        if (prevSettingsRef.current === null) {
            prevSettingsRef.current = currentSettings;
            return;
        }
        
        // 실제로 값이 변경되었을 때만 저장
        if (prevSettingsRef.current.autoRefresh !== autoRefresh ||
            prevSettingsRef.current.refreshInterval !== refreshInterval) {
            console.log('[Settings] Settings changed, saving...');
            saveCurrentSettings();
            prevSettingsRef.current = currentSettings;
        }
    }, [settingsReady, autoRefresh, refreshInterval]);

    // modulesPath 변경 시 저장
    useEffect(() => {
        // 초기 로드 완료 전에는 저장하지 않음
        if (!settingsReady || !settingsPath || !modulesPath) return;
        
        console.log('[Settings] Modules path changed, saving...', modulesPath);
        saveCurrentSettings();
    }, [modulesPath]);

    // discordPrefix 변경 시 bot config 저장
    useEffect(() => {
        // 초기 로드 완료 전에는 저장하지 않음
        if (!settingsReady || !settingsPath) return;
        if (!discordPrefix || !discordPrefix.trim()) return;
        
        // 첫 번째 호출 시 초기값 저장만 하고 저장하지 않음
        if (prevPrefixRef.current === null) {
            prevPrefixRef.current = discordPrefix;
            return;
        }
        
        // 실제로 값이 변경되었을 때만 저장
        if (prevPrefixRef.current !== discordPrefix) {
            console.log('[Settings] Prefix changed, saving bot config:', discordPrefix);
            saveBotConfig(discordPrefix);
            prevPrefixRef.current = discordPrefix;
        }
    }, [settingsReady, discordPrefix]);

    // Discord Bot 상태 폴링
    useEffect(() => {
        let mounted = true;
        
        // 초기 상태 확인 (약간의 지연을 두고)
        const checkBotStatusInitially = async () => {
            try {
                // Electron IPC 준비 시간 확보
                await new Promise(resolve => setTimeout(resolve, 200));
                const status = await window.api.discordBotStatus();
                
                if (mounted) {
                    const botRunning = status === 'running';
                    setDiscordBotStatus(botRunning ? 'running' : 'stopped');
                    setBotStatusReady(true);
                    console.log('[Init] Discord bot initial status:', botRunning ? 'running' : 'stopped');
                    console.log('[Init] BotStatusReady flag set to true');
                }
            } catch (e) {
                if (mounted) {
                    setDiscordBotStatus('stopped');
                    setBotStatusReady(true);
                    console.log('[Init] Discord bot status check failed, assuming stopped');
                }
            }
        };
        
        checkBotStatusInitially();
        
        // 5초마다 폴링
        const interval = setInterval(async () => {
            if (!mounted) return;
            try {
                const status = await window.api.discordBotStatus();
                setDiscordBotStatus(status || 'stopped');
            } catch (e) {
                setDiscordBotStatus('stopped');
            }
        }, 5000);
        
        return () => {
            mounted = false;
            clearInterval(interval);
        };
    }, []);

    // 안전한 토스트 호출 헬퍼
    const safeShowToast = (message, type, duration) => {
        if (typeof window.showToast === 'function') {
            return window.showToast(message, type, duration);
        } else {
            console.warn('[Toast] window.showToast not ready, message:', message);
            return null;
        }
    };

    // Discord Bot 시작
    const handleStartDiscordBot = async () => {
        if (!discordToken) {
            setModal({ type: 'failure', title: t('discord_bot.token_missing_title'), message: t('discord_bot.token_missing_message') });
            return;
        }
        if (!discordPrefix) {
            setModal({ type: 'failure', title: t('discord_bot.prefix_missing_title'), message: t('discord_bot.prefix_missing_message') });
            return;
        }
        try {
            // Note: 봇 시작 시 설정 저장은 사용자가 명시적으로 저장 버튼을 눌렀을 때만 수행
            // 자동시작 시에는 이미 저장된 설정을 사용하므로 저장 불필요
            const botConfig = {
                token: discordToken,
                prefix: discordPrefix,
                moduleAliases: discordModuleAliases,
                commandAliases: discordCommandAliases
            };
            const result = await window.api.discordBotStart(botConfig);
            if (result.error) {
                safeShowToast(t('discord_bot.start_failed_toast', { error: translateError(result.error) }), 'error', 4000);
            } else {
                setDiscordBotStatus('running');
                safeShowToast(t('discord_bot.started_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(t('discord_bot.start_error_toast', { error: translateError(e.message) }), 'error', 4000);
        }
    };

    // 자동시작 (설정과 봇 상태 모두 준비되면 실행)
    useEffect(() => {
        const isTestEnv = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        if (!isTestEnv) {
            console.log('[Auto-start] Effect triggered', {
                botStatusReady,
                settingsReady,
                autoStartDone: autoStartDoneRef.current,
                discordAutoStart,
                tokenExists: !!discordToken,
                prefixExists: !!discordPrefix,
                botStatus: discordBotStatus
            });
        }

        if (botStatusReady && settingsReady && !autoStartDoneRef.current) {
            autoStartDoneRef.current = true;
            
            if (discordAutoStart && discordToken && discordPrefix && discordBotStatus === 'stopped') {
                const isTestEnv = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
                if (!isTestEnv) console.log('[Auto-start] Starting Discord bot automatically!');
                handleStartDiscordBot();
            }
            // else: 조건 미충족 시 조용히 스킵
        }
    }, [botStatusReady, settingsReady, discordAutoStart, discordToken, discordPrefix, discordBotStatus]);

    // Discord Bot 정지
    const handleStopDiscordBot = async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result.error) {
                safeShowToast(t('discord_bot.stop_failed_toast', { error: translateError(result.error) }), 'error', 4000);
            } else {
                setDiscordBotStatus('stopped');
                safeShowToast(t('discord_bot.stopped_toast'), 'discord', 3000);
            }
        } catch (e) {
            safeShowToast(t('discord_bot.stop_error_toast', { error: translateError(e.message) }), 'error', 4000);
        }
    };

    useEffect(() => {
        const isTestEnv = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        if (!isTestEnv) console.log('App mounted, fetching initial data...');
        fetchServers();
        fetchModules();
        loadBotConfig();  // bot-config.json 로드
        
        // 앱 종료 요청 리스너 등록
        if (window.api.onCloseRequest) {
            window.api.onCloseRequest(() => {
                setModal({
                    type: 'question',
                    title: t('app_exit.confirm_title'),
                    message: t('app_exit.confirm_message'),
                    detail: t('app_exit.confirm_detail'),
                    buttons: [
                        {
                            label: t('app_exit.hide_only_label'),
                            action: () => {
                                window.api.closeResponse('hide');
                                setModal(null);
                            }
                        },
                        {
                            label: t('app_exit.quit_all_label'),
                            action: () => {
                                window.api.closeResponse('quit');
                                setModal(null);
                            }
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => {
                                window.api.closeResponse('cancel');
                                setModal(null);
                            }
                        }
                    ]
                });
            });
        }
        
        // Discord 봇 언어 변경 시 재시작 신호 리스너
        if (window.api.onBotRelaunch) {
            window.api.onBotRelaunch((botConfig) => {
                console.log('[Bot Relaunch] Received signal to relaunch bot with new language settings');
                // Discord 봇 프로세스가 재시작될 때까지 대기
                setTimeout(async () => {
                    // 봇을 재시작
                    const result = await window.api.discordBotStart(botConfig);
                    if (result.error) {
                        console.error('[Bot Relaunch] Failed to relaunch bot:', result.error);
                    } else {
                        console.log('[Bot Relaunch] Bot relaunched successfully');
                        setDiscordBotStatus('running');
                    }
                }, 1000);
            });
        }
        
        // 자동 새로고침
        const interval = setInterval(() => {
            if (autoRefresh) {
                fetchServers();
            }
        }, refreshInterval);
        
        return () => clearInterval(interval);
    }, [autoRefresh, refreshInterval]);

    useEffect(() => {
        if (modules.length > 0) {
            // Modules loaded successfully
        }
    }, [modules]);

    const fetchModules = async () => {
        try {
            console.log('Fetching modules...');
            // Daemon이 준비될 때까지 대기
            try {
                await waitForDaemon(5000);
            } catch (err) {
                debugWarn('Daemon not ready, but continuing:', err.message);
            }
            
            // 재시도 로직 적용
            const data = await retryWithBackoff(
                () => window.api.moduleList(),
                3,
                800
            );
            
            console.log('Module data received:', data);
            if (data && data.modules) {
                console.log('Setting modules:', data.modules.length, 'modules');
                setModules(data.modules);
                
                // 각 모듈의 메타데이터 로드 (별명 포함)
                const aliasesMap = {};
                for (const module of data.modules) {
                    try {
                        const metadata = await window.api.moduleGetMetadata(module.name);
                        if (metadata && metadata.toml) {
                            // [aliases] 섹션 기본값
                            const aliases = metadata.toml.aliases || {};
                            const aliasCommands = aliases.commands || {};
                            
                            // [commands.fields]에서 정의된 명령어들 추출
                            const commandFields = metadata.toml.commands?.fields || [];
                            
                            // commands.fields의 명령어들을 aliases.commands 형식으로 병합
                            const mergedCommands = {};
                            
                            // 먼저 aliases.commands에서 정의된 것들 복사
                            for (const [cmdName, cmdData] of Object.entries(aliasCommands)) {
                                mergedCommands[cmdName] = {
                                    aliases: cmdData.aliases || [],
                                    description: cmdData.description || '',
                                    label: cmdName  // 기본적으로 영문 이름 사용
                                };
                            }
                            
                            // commands.fields의 명령어들 추가/보완
                            for (const cmdField of commandFields) {
                                const cmdName = cmdField.name;
                                if (!mergedCommands[cmdName]) {
                                    // aliases에 없으면 기본 구조 생성
                                    mergedCommands[cmdName] = {
                                        aliases: [],
                                        description: cmdField.description || '',
                                        label: cmdField.label || cmdName
                                    };
                                } else {
                                    // 이미 있으면 label과 description 보완
                                    if (!mergedCommands[cmdName].description && cmdField.description) {
                                        mergedCommands[cmdName].description = cmdField.description;
                                    }
                                    if (cmdField.label) {
                                        mergedCommands[cmdName].label = cmdField.label;
                                    }
                                }
                            }
                            
                            aliasesMap[module.name] = {
                                ...aliases,
                                commands: mergedCommands
                            };
                        }
                    } catch (e) {
                        console.warn(`Failed to load metadata for module ${module.name}:`, e);
                    }
                }
                setModuleAliasesPerModule(aliasesMap);
                console.log('Module aliases loaded:', aliasesMap);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                safeShowToast(t('modules.load_failed_toast', { error: translateError(data.error) }), 'error', 4000);
            } else {
                debugWarn('No modules data:', data);
                safeShowToast(t('modules.list_empty'), 'warning', 3000);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            safeShowToast(t('modules.fetch_failed_toast', { error: translateError(error.message) }), 'error', 5000);
            setModal({ type: 'failure', title: t('modules.load_error_title'), message: translateError(error.message) });
        }
    };

    // 마지막 에러 토스트 표시 시간 추적 (중복 방지)
    const lastErrorToastRef = useRef(0);
    
    const fetchServers = async () => {
        try {
            // 재시도 로직 적용
            const data = await retryWithBackoff(
                () => window.api.serverList(),
                3,
                800
            );
            if (data && data.servers) {
                // 기존 expanded 상태 보존하면서 서버 목록 업데이트
                setServers(prev => {
                    return data.servers.map(newServer => {
                        const existing = prev.find(s => s.name === newServer.name);
                        return {
                            ...newServer,
                            expanded: existing?.expanded || false
                        };
                    });
                });
            } else if (data && data.error) {
                console.error('Server list error:', data.error);
                // 초기 로딩이 아니고, 최근 5초 이내에 에러 토스트를 표시하지 않았을 때만 표시
                const now = Date.now();
                if (!loading && (now - lastErrorToastRef.current) > 5000) {
                    safeShowToast(t('servers.fetch_failed_toast', { error: translateError(data.error) }), 'warning', 3000);
                    lastErrorToastRef.current = now;
                }
                // 에러 발생 시 서버 목록을 비우지 않고 기존 상태 유지
            } else {
                // 데이터가 없을 때만 빈 배열로 설정
                if (loading) {
                    setServers([]);
                }
            }
        } catch (error) {
            console.error('Failed to fetch servers:', error);
            
            const errorMsg = translateError(error.message);
            
            // 초기 로딩이 아니고, 최근 5초 이내에 에러 토스트를 표시하지 않았을 때만 표시
            const now = Date.now();
            if (!loading && (now - lastErrorToastRef.current) > 5000) {
                safeShowToast(t('servers.fetch_update_failed_toast', { error: errorMsg }), 'warning', 3000);
                lastErrorToastRef.current = now;
            }
            // 에러 발생 시 서버 목록을 비우지 않고 기존 상태 유지
        } finally {
            setLoading(false);
        }
    };

    const handleStart = async (name, module) => {
        let toastId = null;
        try {
            const result = await window.api.serverStart(name, { module });
            if (result.error) {
                const errorMsg = translateError(result.error);
                safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
            } else {
                // 시작 명령 성공 - 상태 확인 시작
                toastId = safeShowToast(t('servers.starting_toast', { name }), 'info', 0);
                
                // 서버 상태가 running이 될 때까지 대기 (최대 10초)
                let attempts = 0;
                const maxAttempts = 20; // 10초 (500ms * 20)
                const checkInterval = 500;
                
                const checkStatus = setInterval(async () => {
                    attempts++;
                    try {
                        const statusResult = await window.api.serverStatus(name);
                        if (statusResult.status === 'running') {
                            clearInterval(checkStatus);
                            if (toastId && window.updateToast) {
                                window.updateToast(toastId, t('servers.start_completed_toast', { name }), 'success', 3000);
                            }
                            fetchServers();
                        } else if (attempts >= maxAttempts) {
                            clearInterval(checkStatus);
                            if (toastId && window.updateToast) {
                                window.updateToast(toastId, t('servers.start_timeout_toast', { name }), 'warning', 3000);
                            }
                            fetchServers();
                        }
                    } catch (error) {
                        if (attempts >= maxAttempts) {
                            clearInterval(checkStatus);
                            fetchServers();
                        }
                    }
                }, checkInterval);
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
        }
    };

    const handleStop = async (name) => {
        setModal({
            type: 'question',
            title: t('servers.stop_confirm_title'),
            message: t('servers.stop_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                let toastId = null;
                try {
                    const result = await window.api.serverStop(name, { force: false });
                    if (result.error) {
                        const errorMsg = translateError(result.error);
                        safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                    } else {
                        // 정지 명령 성공 - 상태 확인 시작
                        toastId = safeShowToast(t('servers.stopping_toast', { name }), 'info', 0);
                        
                        // 서버 상태가 stopped가 될 때까지 대기 (최대 10초)
                        let attempts = 0;
                        const maxAttempts = 20; // 10초 (500ms * 20)
                        const checkInterval = 500;
                        
                        const checkStatus = setInterval(async () => {
                            attempts++;
                            try {
                                const statusResult = await window.api.serverStatus(name);
                                if (statusResult.status === 'stopped') {
                                    clearInterval(checkStatus);
                                    if (toastId && window.updateToast) {
                                        window.updateToast(toastId, t('servers.stop_completed_toast', { name }), 'success', 3000);
                                    }
                                    fetchServers();
                                } else if (attempts >= maxAttempts) {
                                    clearInterval(checkStatus);
                                    if (toastId && window.updateToast) {
                                        window.updateToast(toastId, t('servers.stop_timeout_toast', { name }), 'warning', 3000);
                                    }
                                    fetchServers();
                                }
                            } catch (error) {
                                if (attempts >= maxAttempts) {
                                    clearInterval(checkStatus);
                                    fetchServers();
                                }
                            }
                        }, checkInterval);
                    }
                } catch (error) {
                    const errorMsg = translateError(error.message);
                    safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                }
            },
            onCancel: () => setModal(null)
        });
    };

    const handleStatus = async (name) => {
        try {
            const result = await window.api.serverStatus(name);
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.status_check_failed_title'), message: errorMsg });
            } else {
                const uptime = result.start_time ? formatUptime(result.start_time) : 'N/A';
                const statusInfo = `Status: ${result.status}\nPID: ${result.pid || 'N/A'}\nUptime: ${uptime}`;
                setModal({ type: 'notification', title: name, message: statusInfo });
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.status_check_failed_title'), message: errorMsg });
        }
    };

    const handleAddServer = async (serverName, moduleName) => {
        if (!serverName || !serverName.trim()) {
            setModal({ type: 'failure', title: t('servers.add_server_name_empty_title'), message: t('servers.add_server_name_empty_message') });
            return;
        }
        if (!moduleName) {
            setModal({ type: 'failure', title: t('servers.add_module_empty_title'), message: t('servers.add_module_empty_message') });
            return;
        }

        try {
            // 선택된 모듈의 기본 executable_path 가져오기
            const selectedModuleData = modules.find(m => m.name === moduleName);
            
            const instanceData = {
                name: serverName.trim(),
                module_name: moduleName,
                executable_path: selectedModuleData?.executable_path || null
            };

            console.log('Adding instance:', instanceData);
            const result = await window.api.instanceCreate(instanceData);
            
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: errorMsg });
            } else {
                setModal({ type: 'success', title: t('command_modal.success'), message: t('server_actions.server_added', { name: serverName }) });
                setShowModuleManager(false);
                fetchServers();
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.add_error_title'), message: errorMsg });
        }
    };

    const handleDeleteServer = async (server) => {
        // Question 모달 표시
        setModal({
            type: 'question',
            title: t('server_actions.delete_confirm_title'),
            message: t('server_actions.delete_confirm_message', { name: server.name }),
            onConfirm: () => performDeleteServer(server),
        });
    };

    const performDeleteServer = async (server) => {
        setModal(null); // 질문 모달 닫기

        try {
            const result = await window.api.instanceDelete(server.id);
            
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.delete_failed_title'), message: errorMsg });
            } else {
                console.log(`Instance "${server.name}" (ID: ${server.id}) deleted`);
                setModal({ type: 'success', title: t('command_modal.success'), message: t('server_actions.server_deleted', { name: server.name }) });
                fetchServers(); // 새로고침
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.delete_error_title'), message: errorMsg });
        }
    };

    const handleOpenSettings = async (server) => {
        // 최신 서버 데이터를 API에서 직접 가져옴
        let latestServer = server;
        try {
            const data = await window.api.serverList();
            if (data && data.servers) {
                const found = data.servers.find(s => s.id === server.id);
                if (found) {
                    latestServer = found;
                    console.log('Loaded latest server data:', latestServer);
                }
            }
        } catch (error) {
            console.warn('Failed to fetch latest server data:', error);
        }
        
        setSettingsServer(latestServer);
        // 선택된 모듈의 settings schema 찾기
        const module = modules.find(m => m.name === latestServer.module);
        if (module && module.settings && module.settings.fields) {
            // 초기값 설정: instances.json에서 저장된 값 우선, 없으면 default
            const initial = {};
            module.settings.fields.forEach(field => {
                let value = '';
                
                // 1. instances.json에서 이미 저장된 값이 있는지 확인
                if (latestServer[field.name] !== undefined && latestServer[field.name] !== null) {
                    value = String(latestServer[field.name]);
                    console.log(`Loaded ${field.name} from instance:`, value);
                }
                // 2. 없으면 module.toml의 default 값 사용
                else if (field.default !== undefined && field.default !== null) {
                    value = String(field.default);
                    console.log(`Using default for ${field.name}:`, value);
                }
                
                initial[field.name] = value;
            });
            
            // protocol_mode 초기화 (별도 처리)
            initial.protocol_mode = latestServer.protocol_mode || 'rest';
            console.log('Loaded protocol_mode:', initial.protocol_mode);
            
            console.log('Initialized settings values:', initial);
            setSettingsValues(initial);
        } else {
            // 모듈 설정이 없어도 protocol_mode는 설정
            setSettingsValues({
                protocol_mode: latestServer.protocol_mode || 'rest'
            });
        }
        
        // 별칭 로드 (settingsServer.module 사용)
        const moduleName = latestServer.module;
        if (moduleAliasesPerModule[moduleName]) {
            const aliases = moduleAliasesPerModule[moduleName];
            
            // 저장된 모듈 별명 로드
            if (moduleName in discordModuleAliases) {
                const saved = discordModuleAliases[moduleName] || '';
                const parsed = saved.split(',').map(a => a.trim()).filter(a => a.length > 0);
                setEditingModuleAliases(parsed);
            } else {
                setEditingModuleAliases(aliases.module_aliases || []);
            }
            
            // 명령어 별명 로드
            const cmdAliases = aliases.commands || {};
            const normalized = {};
            for (const [cmd, data] of Object.entries(cmdAliases)) {
                let baseAliases = [];
                if (Array.isArray(data)) {
                    baseAliases = data;
                } else if (data.aliases) {
                    baseAliases = data.aliases;
                }

                const hasSavedCmd = discordCommandAliases[moduleName] && 
                    (cmd in discordCommandAliases[moduleName]);
                const merged = hasSavedCmd
                    ? (discordCommandAliases[moduleName][cmd] || '').split(',').map(a => a.trim()).filter(a => a.length > 0)
                    : baseAliases;

                normalized[cmd] = {
                    aliases: merged,
                    description: (data && data.description) || '',
                    label: (data && data.label) || cmd
                };
            }
            setEditingCommandAliases(normalized);
        }
        
        setSettingsActiveTab('general'); // 탭 초기화
        setShowSettingsModal(true);
    };

    const handleSettingChange = (fieldName, value) => {
        console.log(`Setting ${fieldName} changed to:`, value);
        setSettingsValues(prev => {
            const updated = {
                ...prev,
                [fieldName]: String(value)
            };
            console.log('Updated settings values:', updated);
            return updated;
        });
    };

    const handleSaveSettings = async () => {
        if (!settingsServer) return;
        
        try {
            console.log('Saving settings for', settingsServer.name, settingsValues);
            
            // 설정값 타입 변환 (number 필드는 숫자로 변환)
            const module = modules.find(m => m.name === settingsServer.module);
            const convertedSettings = {};
            
            if (module && module.settings && module.settings.fields) {
                module.settings.fields.forEach(field => {
                    const value = settingsValues[field.name];
                    
                    if (value === '' || value === null || value === undefined) {
                        return; // 빈 값은 전송하지 않음
                    }
                    
                    if (field.field_type === 'number') {
                        convertedSettings[field.name] = Number(value);
                    } else {
                        convertedSettings[field.name] = value;
                    }
                });
            }
            
            // 프로토콜 지원 여부 확인
            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];
            
            // 프로토콜이 지원되는 경우에만 protocol_mode 전송
            if (supportedProtocols.length > 0) {
                // 모듈이 둘 다 지원하면 사용자 선택값, 하나만 지원하면 기본값 사용
                if (supportedProtocols.includes('rest') && supportedProtocols.includes('rcon')) {
                    convertedSettings.protocol_mode = settingsValues.protocol_mode || protocols.default || 'rest';
                } else {
                    convertedSettings.protocol_mode = supportedProtocols[0];
                }
            }
            
            console.log('Converted settings:', convertedSettings);
            console.log('protocol_mode being sent:', convertedSettings.protocol_mode);
            console.log('Calling instanceUpdateSettings with id:', settingsServer.id);
            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            console.log('API Response:', result);
            
            if (result.error) {
                setModal({ type: 'failure', title: t('settings.save_failed_title'), message: translateError(result.error) });
                console.error('Error response:', result.error);
            } else {
                setModal({ type: 'success', title: t('command_modal.success'), message: t('server_actions.settings_saved', { name: settingsServer.name }) });
                setShowSettingsModal(false);
                fetchServers(); // 새로고침
            }
        } catch (error) {
            console.error('Exception in handleSaveSettings:', error);
            setModal({ type: 'failure', title: t('settings.save_error_title'), message: translateError(error.message) });
        }
    };

    // 모듈/명령어 별명 저장 (bot-config.json)
    const handleSaveAliases = async () => {
        if (!selectedModuleForAliases) return;
        try {
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };

            // 모듈 별명 저장 (콤마 구분 문자열)
            moduleAliases[selectedModuleForAliases] = (editingModuleAliases || []).join(',');

            // 명령어 별명 저장 (모듈별 객체)
            const cmdMap = {};
            Object.entries(editingCommandAliases || {}).forEach(([cmd, data]) => {
                const list = (data.aliases || []).join(',');
                cmdMap[cmd] = list;
            });
            commandAliases[selectedModuleForAliases] = cmdMap;

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({ type: 'failure', title: t('settings.aliases_save_failed_title'), message: translateError(res.error) });
            } else {
                // API에서 저장된 설정을 다시 로드
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: t('server_actions.aliases_saved'), message: t('server_actions.aliases_saved') });
            }
        } catch (error) {
            console.error('Failed to save aliases:', error);
            setModal({ type: 'failure', title: t('settings.aliases_save_error_title'), message: translateError(error.message) });
        }
    };

    // 모듈/명령어 별명 초기화 (기본값으로)
    const handleResetAliases = async () => {
        if (!selectedModuleForAliases) return;
        try {
            // UI 입력을 모두 비우기 (런타임 기본값은 모듈명/명령어명으로 처리됨)
            setEditingModuleAliases([]);
            const clearedCmds = {};
            const defaults = moduleAliasesPerModule[selectedModuleForAliases];
            if (defaults && defaults.commands) {
                for (const [cmd, data] of Object.entries(defaults.commands)) {
                    clearedCmds[cmd] = { aliases: [], description: data.description || '', label: data.label || cmd };
                }
            }
            setEditingCommandAliases(clearedCmds);

            // 저장된 사용자 별명 제거 후 저장
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };
            delete moduleAliases[selectedModuleForAliases];
            delete commandAliases[selectedModuleForAliases];

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({ type: 'failure', title: t('settings.aliases_reset_failed_title'), message: translateError(res.error) });
            } else {
                // API에서 저장된 설정을 다시 로드
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: t('settings.aliases_reset_completed_title'), message: t('settings.aliases_reset_message') });
            }
        } catch (error) {
            console.error('Failed to reset aliases:', error);
            setModal({ type: 'failure', title: t('settings.aliases_reset_failed_title'), message: translateError(error.message) });
        }
    };

    // Settings 모달에서 사용할 모듈별 별명 저장 함수
    const handleSaveAliasesForModule = async (moduleName) => {
        try {
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };

            // 모듈 별명 저장
            moduleAliases[moduleName] = (editingModuleAliases || []).join(',');

            // 명령어 별명 저장
            const cmdMap = {};
            Object.entries(editingCommandAliases || {}).forEach(([cmd, data]) => {
                cmdMap[cmd] = (data.aliases || []).join(',');
            });
            commandAliases[moduleName] = cmdMap;

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({ type: 'failure', title: t('settings.aliases_save_failed_title'), message: translateError(res.error) });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: t('server_actions.aliases_saved'), message: t('server_actions.aliases_saved') });
            }
        } catch (error) {
            console.error('Failed to save aliases:', error);
            setModal({ type: 'failure', title: t('settings.aliases_save_error_title'), message: translateError(error.message) });
        }
    };

    // Settings 모달에서 사용할 모듈별 별명 초기화 함수
    const handleResetAliasesForModule = async (moduleName) => {
        try {
            // UI 초기화
            setEditingModuleAliases([]);
            const clearedCmds = {};
            const defaults = moduleAliasesPerModule[moduleName];
            if (defaults && defaults.commands) {
                for (const [cmd, data] of Object.entries(defaults.commands)) {
                    clearedCmds[cmd] = { aliases: [], description: data.description || '', label: data.label || cmd };
                }
            }
            setEditingCommandAliases(clearedCmds);

            // 저장된 별명 제거
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };
            delete moduleAliases[moduleName];
            delete commandAliases[moduleName];

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({ type: 'failure', title: t('settings.aliases_reset_failed_title'), message: translateError(res.error) });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: t('settings.aliases_reset_completed_title'), message: t('settings.aliases_reset_message') });
            }
        } catch (error) {
            console.error('Failed to reset aliases:', error);
            setModal({ type: 'failure', title: t('settings.aliases_reset_failed_title'), message: translateError(error.message) });
        }
    };

    // Handle module selection and auto-generate server name
    const handleModuleSelect = (moduleName) => {
        setSelectedModule(moduleName);
        
        // 이름이 비어있거나 자동 생성된 이름인 경우에만 자동완성
        if (!newServerName || newServerName.startsWith('my-')) {
            const existingCount = servers.filter(s => s.module === moduleName).length;
            const suggestedName = `my-${moduleName}-${existingCount + 1}`;
            setNewServerName(suggestedName);
        }
    };

    const getStatusColor = (status) => {
        switch (status) {
            case 'running':
                return '#4CAF50';
            case 'stopped':
                return '#f44336';
            case 'starting':
                return '#2196F3';
            case 'stopping':
                return '#FF9800';
            default:
                return '#999';
        }
    };

    const getStatusIcon = (status) => {
        switch (status) {
            case 'running':
                return '▶';
            case 'stopped':
                return '■';
            case 'starting':
                return '⟳';
            case 'stopping':
                return '⏹';
            default:
                return '?';
        }
    };

    // 로딩 화면 (Daemon 준비 전)
    if (!daemonReady) {
        return (
            <div className="loading-screen">
                <TitleBar />
                <div className="loading-content">
                    <div className="loading-logo-container">
                        <i className="glow-blur"></i>
                        <i className="glow-ring"></i>
                        <i className="glow-mask"></i>
                        <img src="/title.png" alt="" className="loading-logo-img" />
                    </div>
                    <img src={logoSrc} alt={t('common:app_name')} className="loading-logo-text" />
                    <div className="loading-status">
                        <Icon name="loader" size="sm" /> {initStatus}
                    </div>
                    <div className="loading-progress-bar">
                        <div 
                            className="loading-progress-fill" 
                            style={{ width: `${initProgress}%` }}
                        ></div>
                    </div>
                    <div className="loading-tips">
                        <Icon name="info" size="sm" /> {t('buttons.loading_tips')}
                    </div>
                </div>
            </div>
        );
    }

    if (loading) {
        return (
            <div className="App">
                <div className="loading">
                    <h2>{t('buttons.loading')}</h2>
                </div>
            </div>
        );
    }

    return (
        <div className="App">
            {/* Discord overlay backdrop */}
            {showDiscordSection && (
                <div 
                    className="discord-backdrop" 
                    onClick={() => setShowDiscordSection(false)}
                />
            )}
            {/* Background overlay backdrop */}
            {showBackgroundSection && (
                <div 
                    className="discord-backdrop" 
                    onClick={() => setShowBackgroundSection(false)}
                />
            )}
            <TitleBar />
            <Toast />
            <header className="app-header">
                {/* 첫 번째 줄: 타이틀과 설정 */}
                <div className="header-row header-row-title">
                    <div className="app-title-section">
                        <img src="/icon.png" alt="" className="app-logo-icon" />
                        <img src={logoSrc} alt={t('common:app_name')} className="app-logo-text" />
                    </div>
                    <button 
                        className="btn-settings-icon-solo"
                        onClick={() => setShowGuiSettingsModal(true)}
                        title={t('settings.gui_settings_tooltip')}
                    >
                        <Icon name="settings" size="lg" />
                    </button>
                </div>
                
                {/* 두 번째 줄: 기능 버튼들 */}
                <div className="header-row header-row-controls">
                    <button 
                        className="btn btn-add"
                        onClick={() => setShowModuleManager(!showModuleManager)}
                    >
                        <Icon name="plus" size="sm" /> Add Server
                    </button>
                    <div className="header-spacer"></div>
                    <div className="discord-button-wrapper">
                        <button 
                            className={`btn btn-discord ${discordBotStatus === 'running' ? 'btn-discord-active' : ''}`}
                            onClick={() => setShowDiscordSection(!showDiscordSection)}
                        >
                            <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : 'status-offline'}`}></span>
                            Discord Bot
                        </button>
                        {/* Discord Bot Modal */}
                        <DiscordBotModal
                            isOpen={showDiscordSection}
                            onClose={() => setShowDiscordSection(false)}
                            discordBotStatus={discordBotStatus}
                            discordToken={discordToken}
                            setDiscordToken={setDiscordToken}
                            discordPrefix={discordPrefix}
                            setDiscordPrefix={setDiscordPrefix}
                            discordAutoStart={discordAutoStart}
                            setDiscordAutoStart={setDiscordAutoStart}
                            handleStartDiscordBot={handleStartDiscordBot}
                            handleStopDiscordBot={handleStopDiscordBot}
                            saveCurrentSettings={saveCurrentSettings}
                        />
                    </div>
                    <div className="background-button-wrapper">
                        <button 
                            className={`btn btn-background ${backgroundDaemonStatus === 'running' ? 'btn-background-active' : ''}`}
                            onClick={() => setShowBackgroundSection(!showBackgroundSection)}
                        >
                            <span className={`status-indicator ${
                                backgroundDaemonStatus === 'running' ? 'status-online' : 
                                backgroundDaemonStatus === 'checking' ? 'status-checking' : 
                                'status-offline'
                            }`}></span>
                            Background
                        </button>
                        {/* Background Modal */}
                        <BackgroundModal
                            isOpen={showBackgroundSection}
                            onClose={() => setShowBackgroundSection(false)}
                        />
                    </div>
                </div>
            </header>

            {/* AddServerModal */}
            <AddServerModal
                isOpen={showModuleManager}
                onClose={() => setShowModuleManager(false)}
                modules={modules}
                servers={servers}
                modulesPath={modulesPath}
                settingsPath={settingsPath}
                onModulesPathChange={setModulesPath}
                onRefreshModules={fetchModules}
                onAddServer={handleAddServer}
            />

            <main className="app-main">
                <div className="server-list">
                {/* 서버 상태 초기화 중 오버레이 */}
                {serversInitializing && servers.length > 0 && (
                    <div className="servers-initializing-overlay">
                        <div className="servers-initializing-content">
                            <div className="servers-initializing-spinner"></div>
                            <span>{t('gui:servers.initializing_overlay')}</span>
                        </div>
                    </div>
                )}
                
                {servers.length === 0 ? (
                    <div className="no-servers">
                        <p>{t('servers.no_servers_configured', { defaultValue: 'No servers configured' })}</p>
                    </div>
                ) : (
                    servers.map((server, index) => {
                        // 모듈 메타데이터에서 게임 이름 가져오기
                        const moduleData = modules.find(m => m.name === server.module);
                        const gameName = moduleData?.game_name || server.module;
                        const gameIcon = moduleData?.icon || null; // 모듈에서 base64 인코딩된 아이콘 가져오기
                        
                        return (
                            <div 
                                key={server.name}
                                ref={el => { cardRefs.current[server.name] = el; }}
                                className={`server-card ${server.expanded ? 'expanded' : ''} ${draggedName === server.name ? 'dragging' : ''}`}
                                onPointerDown={(e) => handleCardPointerDown(e, index)}
                            >
                                <div 
                                    className="server-card-header"
                                    onClick={(e) => {
                                        if (skipNextClick.current) return;
                                        // 버튼 클릭은 무시
                                        if (e.target.closest('button')) return;
                                        // expanded 상태 토글
                                        setServers(prev => prev.map(s => 
                                            s.name === server.name ? { ...s, expanded: !s.expanded } : s
                                        ));
                                    }}
                                    style={{ cursor: 'pointer' }}
                                >
                                    {/* 게임 아이콘 영역 */}
                                    <div className="game-icon-container">
                                        {gameIcon ? (
                                            <img src={gameIcon} alt={gameName} className="game-icon" />
                                        ) : (
                                            <div className="game-icon-placeholder">
                                                <Icon name="gamepad" size="lg" />
                                            </div>
                                        )}
                                    </div>
                                    
                                    {/* 서버 정보 */}
                                    <div className="server-card-info">
                                        <h2>{server.name}</h2>
                                        <p className="game-name">{gameName}</p>
                                    </div>
                                    
                                    {/* 상태 버튼 (인디케이터 + 텍스트) */}
                                    <button 
                                        className={`status-button status-${server.status}`}
                                        onClick={() => {
                                            if (server.status === 'starting' || server.status === 'stopping') {
                                                return; // 전환 중에는 클릭 불가
                                            }
                                            if (server.status === 'running' || server.status === 'starting') {
                                                handleStop(server.name);
                                            } else {
                                                handleStart(server.name, server.module);
                                            }
                                        }}
                                        disabled={server.status === 'starting' || server.status === 'stopping'}
                                        title={server.status === 'running' || server.status === 'starting' ? 'Click to stop' : 'Click to start'}
                                    >
                                        <span className="status-label status-label-default">
                                            {server.status === 'running' ? t('server_status.running') : 
                                             server.status === 'starting' ? t('server_status.stopping') :
                                             server.status === 'stopping' ? t('server_status.stopping') : t('server_status.stopped')}
                                        </span>
                                        <span className="status-label status-label-hover">
                                            {server.status === 'running' ? t('server_status.stop') : 
                                             server.status === 'starting' ? t('server_status.stopping') :
                                             server.status === 'stopping' ? t('server_status.stopping') : t('server_status.start')}
                                        </span>
                                        <span className="status-dot"></span>
                                    </button>
                                </div>

                                <div className="server-card-collapsible">
                                    <div className="server-details">
                                    {server.status === 'running' && server.pid && (
                                        <div className="detail-row">
                                            <span className="label">PID:</span>
                                            <span className="value">{server.pid}</span>
                                        </div>
                                    )}
                                    {server.status === 'running' && server.start_time && (
                                        <div className="detail-row">
                                            <span className="label">{t('servers.uptime', 'Uptime')}:</span>
                                            <span className="value">{formatUptime(server.start_time)}</span>
                                        </div>
                                    )}
                                    {server.port && (
                                        <div className="detail-row">
                                            <span className="label">{t('servers.port', 'Port')}:</span>
                                            <span className="value">{server.port}</span>
                                        </div>
                                    )}
                                    {server.rcon_port && (
                                        <div className="detail-row">
                                            <span className="label">RCON:</span>
                                            <span className="value">{server.rcon_port}</span>
                                        </div>
                                    )}
                                    {server.rest_port && (
                                        <div className="detail-row">
                                            <span className="label">REST:</span>
                                            <span className="value">{server.rest_host || '127.0.0.1'}:{server.rest_port}</span>
                                        </div>
                                    )}
                                    <div className="detail-row">
                                        <span className="label">{t('servers.protocol', 'Protocol')}:</span>
                                        <span className="value">{server.protocol_mode?.toUpperCase() || 'AUTO'}</span>
                                    </div>
                                </div>

                                {/* 아이콘 버튼들 (좌하단) */}
                                <div className="server-actions">
                                    <button 
                                        className="action-icon"
                                        onClick={() => handleOpenSettings(server)}
                                        title="Settings"
                                    >
                                        <Icon name="settings" size="md" />
                                    </button>
                                    <button 
                                        className="action-icon"
                                        onClick={() => handleStatus(server.name)}
                                        title="Info"
                                    >
                                        <Icon name="info" size="md" />
                                    </button>
                                    {server.status === 'running' ? (
                                        <button 
                                            className="action-icon"
                                            onClick={() => {
                                                setCommandServer(server);
                                                setShowCommandModal(true);
                                            }}
                                            title="Command"
                                        >
                                            <Icon name="terminal" size="md" />
                                        </button>
                                    ) : (
                                        <button 
                                            className="action-icon action-delete"
                                            onClick={() => handleDeleteServer(server)}
                                            disabled={server.status === 'starting' || server.status === 'stopping'}
                                            title="Delete"
                                        >
                                            <Icon name="trash" size="md" />
                                        </button>
                                    )}
                                </div>
                                </div>
                            </div>
                        );
                    })
                )}
                </div>
            </main>

            {showSettingsModal && settingsServer && (
                <div className="modal-overlay">
                    <div className="modal-content modal-content-large">
                        <div className="modal-header">
                            <h3><Icon name="settings" size="md" /> {settingsServer.name} - Settings</h3>
                            <button className="modal-close" onClick={() => setShowSettingsModal(false)}>✕</button>
                        </div>
                        
                        {/* 탭 헤더 */}
                        <div className="settings-tabs">
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'general' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('general')}
                            >
                                <Icon name="gamepad" size="sm" /> 일반 설정
                            </button>
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'aliases' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('aliases')}
                            >
                                <Icon name="messageSquare" size="sm" /> Discord 별명
                            </button>
                        </div>
                        
                        <div className="modal-body">
                            {/* 일반 설정 탭 */}
                            {settingsActiveTab === 'general' && (() => {
                                const module = modules.find(m => m.name === settingsServer.module);
                                const hasModuleSettings = module && module.settings && module.settings.fields && module.settings.fields.length > 0;
                                
                                // 프로토콜 지원 여부 확인
                                const protocols = module?.protocols || {};
                                const supportedProtocols = protocols.supported || [];
                                const showProtocolToggle = supportedProtocols.includes('rest') && supportedProtocols.includes('rcon');
                                
                                return (
                                    <div className="settings-form">
                                        {/* 프로토콜 모드 토글 - 모듈이 REST와 RCON을 모두 지원할 때만 표시 */}
                                        {showProtocolToggle && (
                                            <div className="protocol-mode-section">
                                                <div className="protocol-mode-header">
                                                    <span className="protocol-mode-title">🔌 서버 조작 방식</span>
                                                </div>
                                                <p className="protocol-mode-description">
                                                    서버 명령어를 실행할 때 사용할 프로토콜을 선택합니다.
                                                </p>
                                                <div className="protocol-toggle-container">
                                                    <span className={`protocol-label ${settingsValues.protocol_mode === 'rest' ? 'active' : ''}`}>
                                                        REST
                                                    </span>
                                                    <label className="toggle-switch">
                                                        <input 
                                                            type="checkbox"
                                                            checked={settingsValues.protocol_mode === 'rcon'}
                                                            onChange={(e) => handleSettingChange('protocol_mode', e.target.checked ? 'rcon' : 'rest')}
                                                        />
                                                        <span className="toggle-slider"></span>
                                                    </label>
                                                    <span className={`protocol-label ${settingsValues.protocol_mode === 'rcon' ? 'active' : ''}`}>
                                                        RCON
                                                    </span>
                                                </div>
                                                <p className="protocol-mode-hint">
                                                    <span className="hint-icon">💡</span>
                                                    {settingsValues.protocol_mode === 'rest' 
                                                        ? 'REST API는 HTTP 기반으로 안정적이며 인증이 용이합니다.'
                                                        : 'RCON은 실시간 콘솔 명령어를 직접 전송합니다.'}
                                                </p>
                                            </div>
                                        )}
                                        
                                        {/* 프로토콜이 하나만 지원될 때 정보 표시 */}
                                        {!showProtocolToggle && supportedProtocols.length > 0 && (
                                            <div className="protocol-mode-section protocol-mode-info">
                                                <div className="protocol-mode-header">
                                                    <span className="protocol-mode-title">🔌 서버 조작 방식</span>
                                                </div>
                                                <p className="protocol-mode-description">
                                                    이 모듈은 <strong>{supportedProtocols[0].toUpperCase()}</strong> 프로토콜만 지원합니다.
                                                </p>
                                            </div>
                                        )}

                                        {/* 모듈 설정 필드 */}
                                        {hasModuleSettings ? (
                                            module.settings.fields.map((field) => (
                                                <div key={field.name} className="settings-field">
                                                    <label>{field.label} {field.required ? '*' : ''}</label>
                                                    {field.field_type === 'text' && (
                                                        <input 
                                                            type="text"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={field.description || ''}
                                                        />
                                                    )}
                                                    {field.field_type === 'password' && (
                                                        <input 
                                                            type="password"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={field.description || ''}
                                                        />
                                                    )}
                                                    {field.field_type === 'number' && (
                                                        <input 
                                                            type="number"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            min={field.min}
                                                            max={field.max}
                                                            placeholder={field.description || ''}
                                                        />
                                                    )}
                                                    {field.field_type === 'file' && (
                                                        <input 
                                                            type="text"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={field.description || ''}
                                                        />
                                                    )}
                                                    {field.field_type === 'select' && (
                                                        <select 
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                        >
                                                            <option value="">Select {field.label}</option>
                                                            {field.options && field.options.map(opt => (
                                                                <option key={opt} value={opt}>{opt}</option>
                                                            ))}
                                                        </select>
                                                    )}
                                                    {field.description && (
                                                        <small className="field-description">{field.description}</small>
                                                    )}
                                                </div>
                                            ))
                                        ) : (
                                            <p className="no-settings" style={{marginTop: '16px'}}>이 모듈에는 추가 설정 항목이 없습니다.</p>
                                        )}
                                    </div>
                                );
                            })()}
                            
                            {/* Discord 별명 탭 */}
                            {settingsActiveTab === 'aliases' && (
                                <div className="aliases-tab-content">
                                    <div className="module-aliases-detail">
                                        <h4>📝 모듈 별명 (Discord에서 이 서버를 부를 이름)</h4>
                                        <small>공백으로 구분하여 여러 개 입력 가능. 예: {settingsServer.module} pw palworld</small>
                                        <div className="module-aliases-input">
                                            <input
                                                type="text"
                                                placeholder={`예: ${settingsServer.module}`}
                                                value={editingModuleAliases.join(' ')}
                                                onChange={(e) => {
                                                    const aliases = e.target.value.split(/\s+/).filter(a => a.length > 0);
                                                    setEditingModuleAliases(aliases);
                                                }}
                                            />
                                            {editingModuleAliases.length === 0 && (
                                                <div className="placeholder-hint">
                                                    <small>💡 공백 시 기본값: <code>{settingsServer.module}</code></small>
                                                </div>
                                            )}
                                        </div>
                                        <div className="aliases-display">
                                            {editingModuleAliases.map((alias, idx) => (
                                                <span key={idx} className="alias-badge">{alias}</span>
                                            ))}
                                        </div>

                                        <h4>⚡ 명령어 별명 (커스텀 명령어)</h4>
                                        <small>콤마로 구분하여 여러 별명 입력. 예: 시작, start, 실행</small>
                                        <div className="command-aliases-input">
                                            {Object.entries(editingCommandAliases).map(([cmd, cmdData]) => {
                                                const aliases = cmdData.aliases || [];
                                                const description = cmdData.description || '';
                                                const label = cmdData.label || cmd;
                                                return (
                                                    <div key={cmd} className="command-alias-editor">
                                                        <div className="cmd-header">
                                                            <span className="cmd-name">{cmd}</span>
                                                            {label !== cmd && <span className="cmd-label">({label})</span>}
                                                            {description && <span className="cmd-help" title={description}>?</span>}
                                                        </div>
                                                        <input
                                                            type="text"
                                                            placeholder={`예: ${cmd}`}
                                                            value={aliases.join(', ')}
                                                            onChange={(e) => {
                                                                const newAliases = e.target.value.split(',').map(a => a.trim()).filter(a => a.length > 0);
                                                                setEditingCommandAliases({
                                                                    ...editingCommandAliases,
                                                                    [cmd]: { ...cmdData, aliases: newAliases }
                                                                });
                                                            }}
                                                        />
                                                        <div className="aliases-display">
                                                            {aliases.length === 0 ? (
                                                                <span className="alias-badge-default">{cmd}</span>
                                                            ) : (
                                                                aliases.map((alias, idx) => (
                                                                    <span key={idx} className="alias-badge-sm">{alias}</span>
                                                                ))
                                                            )}
                                                        </div>
                                                    </div>
                                                );
                                            })}
                                        </div>
                                        
                                        <div className="module-aliases-actions">
                                            <button className="btn btn-save" onClick={() => {
                                                // settingsServer.module을 사용하여 저장
                                                const moduleName = settingsServer.module;
                                                handleSaveAliasesForModule(moduleName);
                                            }}>
                                                💾 별명 저장
                                            </button>
                                            <button className="btn btn-reset" onClick={() => {
                                                const moduleName = settingsServer.module;
                                                handleResetAliasesForModule(moduleName);
                                            }}>
                                                🔄 초기화
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                        
                        <div className="modal-footer">
                            {settingsActiveTab === 'general' && (
                                <button className="btn btn-confirm" onClick={handleSaveSettings}>
                                    💾 설정 저장
                                </button>
                            )}
                            <button className="btn btn-cancel" onClick={() => setShowSettingsModal(false)}>
                                ✕ 닫기
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* 모달 렌더링 */}
            {modal && modal.type === 'success' && (
                <SuccessModal
                    title={modal.title}
                    message={modal.message}
                    onClose={() => setModal(null)}
                />
            )}
            {modal && modal.type === 'failure' && (
                <FailureModal
                    title={modal.title}
                    message={modal.message}
                    onClose={() => setModal(null)}
                />
            )}
            {modal && modal.type === 'notification' && (
                <NotificationModal
                    title={modal.title}
                    message={modal.message}
                    onClose={() => setModal(null)}
                />
            )}
            {modal && modal.type === 'question' && (
                <QuestionModal
                    title={modal.title}
                    message={modal.message}
                    detail={modal.detail}
                    buttons={modal.buttons}
                    onConfirm={modal.onConfirm}
                    onCancel={() => setModal(null)}
                />
            )}

            {/* SettingsModal 렌더링 */}
            <SettingsModal 
                isOpen={showGuiSettingsModal} 
                onClose={() => setShowGuiSettingsModal(false)}
                refreshInterval={refreshInterval}
                onRefreshIntervalChange={setRefreshInterval}
            />

            {/* CommandModal 렌더링 */}
            {showCommandModal && commandServer && (
                <CommandModal
                    server={commandServer}
                    modules={modules}
                    onClose={() => setShowCommandModal(false)}
                    onExecute={setModal}
                />
            )}
        </div>
    );
}

export default App;
