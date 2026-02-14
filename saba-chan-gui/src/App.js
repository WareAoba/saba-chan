import React, { useState, useEffect, useRef, useMemo, useCallback } from 'react';
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
    NoticeModal,
    Icon,
    CustomDropdown
} from './components';
import { useModalClose } from './hooks/useModalClose';

function App() {
    const { t, i18n } = useTranslation('gui');

    // 언어별 로고 이미지 선택
    const logoSrc = useMemo(() => {
        const lang = (i18n.language || 'en').toLowerCase();
        if (lang.startsWith('ko')) return './logo-kr.png';
        if (lang.startsWith('ja')) return './logo-jp.png';
        return './logo-en.png';
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
    const [ipcPort, setIpcPort] = useState(57474);
    const [consoleBufferSize, setConsoleBufferSize] = useState(2000);
    const consoleBufferRef = useRef(2000);
    const [showModuleManager, setShowModuleManager] = useState(false);
    const [settingsInitialView, setSettingsInitialView] = useState(null);
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
    const [advancedExpanded, setAdvancedExpanded] = useState(false); // 고급 설정 접힘/펼침
    const [availableVersions, setAvailableVersions] = useState([]); // 서버 버전 목록
    const [versionsLoading, setVersionsLoading] = useState(false); // 버전 로딩 중
    
    // Command 모달 상태
    const [showCommandModal, setShowCommandModal] = useState(false);
    const [commandServer, setCommandServer] = useState(null);
    
    // GUI 설정 모달 상태
    const [showGuiSettingsModal, setShowGuiSettingsModal] = useState(false);
    
    // 모달 상태 (Success/Failure/Notification)
    const [modal, setModal] = useState(null);

    // 글로벌 프로그레스바 상태
    const [progressBar, setProgressBar] = useState(null); // { message, percent?, indeterminate? }

    // waiting.png 표시 상태 (느린 진행/타임아웃 감지)
    const [showWaitingImage, setShowWaitingImage] = useState(false);
    const waitingTimerRef = useRef(null);
    const progressSnapshotRef = useRef(null);

    // waiting.png: 프로그레스바가 5초 이상 느리면 표시
    useEffect(() => {
        if (!progressBar) {
            // 프로그레스바 사라지면 초기화
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }

        // 완료 상태면 무시
        if (progressBar.percent === 100) {
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }

        // 스냅샷 초기화
        if (!progressSnapshotRef.current) {
            progressSnapshotRef.current = { percent: progressBar.percent || 0, timestamp: Date.now() };
        }

        // 1초마다 진행 속도 체크
        if (!waitingTimerRef.current) {
            waitingTimerRef.current = setInterval(() => {
                const snap = progressSnapshotRef.current;
                if (!snap) return;
                const elapsed = (Date.now() - snap.timestamp) / 1000;
                if (elapsed >= 5) {
                    // 5초 이상 경과 시 waiting.png 표시
                    setShowWaitingImage(true);
                }
            }, 1000);
        }

        // percent 변화 감지 → 빠르게 진행되면 스냅샷 리셋
        const currentPercent = progressBar.percent || 0;
        const snap = progressSnapshotRef.current;
        if (snap && currentPercent - snap.percent > 5) {
            // 5% 이상 진행됨 → 리셋
            progressSnapshotRef.current = { percent: currentPercent, timestamp: Date.now() };
            setShowWaitingImage(false);
        }

        return () => {
            if (waitingTimerRef.current) {
                clearInterval(waitingTimerRef.current);
                waitingTimerRef.current = null;
            }
        };
    }, [progressBar]);

    // waiting.png: 타임아웃 토스트 감지
    useEffect(() => {
        const origUpdateToast = window.updateToast;
        const wrappedUpdateToast = (id, message, type, duration) => {
            // "시간이 걸릴 수 있습니다" 메시지 감지
            if (message && message.includes('시간이 걸릴')) {
                setShowWaitingImage(true);
                setTimeout(() => setShowWaitingImage(false), duration || 5000);
            }
            if (origUpdateToast) origUpdateToast(id, message, type, duration);
        };
        window.updateToast = wrappedUpdateToast;
        return () => { window.updateToast = origUpdateToast; };
    }, []);

    // 콘솔 패널 상태
    const [consoleServer, setConsoleServer] = useState(null); // { id, name } — 현재 콘솔이 열린 서버
    const [consoleLines, setConsoleLines] = useState([]);
    const [consoleSinceId, setConsoleSinceId] = useState(0);
    const [consoleInput, setConsoleInput] = useState('');
    const consoleEndRef = useRef(null);
    const consolePollingRef = useRef(null);

    // Discord Bot 상태
    const [discordBotStatus, setDiscordBotStatus] = useState('stopped'); // stopped | running | error
    const [discordToken, setDiscordToken] = useState('');
    const [showDiscordSection, setShowDiscordSection] = useState(false);
    const [showBackgroundSection, setShowBackgroundSection] = useState(false);
    const [showNoticeSection, setShowNoticeSection] = useState(false);
    const [unreadNoticeCount, setUnreadNoticeCount] = useState(0);
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
    const discordTokenRef = useRef('');

    // 모듈별 별명 (각 모듈의 module.toml에서 정의한 별명들)
    const [moduleAliasesPerModule, setModuleAliasesPerModule] = useState({});  // { moduleName: { moduleAliases: [...], commands: {...} } }
    const [selectedModuleForAliases, setSelectedModuleForAliases] = useState(null);
    const [editingModuleAliases, setEditingModuleAliases] = useState({});
    const [editingCommandAliases, setEditingCommandAliases] = useState({});

    // 서버 설정 모달 닫기 트랜지션
    const closeSettingsModal = useCallback(() => setShowSettingsModal(false), []);
    const { isClosing: isSettingsClosing, requestClose: requestSettingsClose } = useModalClose(closeSettingsModal);

    // Discord / Background 모달 닫기 트랜지션
    const closeDiscordSection = useCallback(() => setShowDiscordSection(false), []);
    const { isClosing: isDiscordClosing, requestClose: requestDiscordClose } = useModalClose(closeDiscordSection);
    const closeBackgroundSection = useCallback(() => setShowBackgroundSection(false), []);
    const { isClosing: isBackgroundClosing, requestClose: requestBackgroundClose } = useModalClose(closeBackgroundSection);
    const closeNoticeSection = useCallback(() => setShowNoticeSection(false), []);
    const { isClosing: isNoticeClosing, requestClose: requestNoticeClose } = useModalClose(closeNoticeSection);

    // 읽지 않은 알림 수 추적
    useEffect(() => {
        const updateCount = () => {
            if (window.__sabaNotice) {
                setUnreadNoticeCount(window.__sabaNotice.getUnreadCount());
            }
        };
        updateCount();
        window.addEventListener('saba-notice-update', updateCount);
        return () => window.removeEventListener('saba-notice-update', updateCount);
    }, []);

    // 초기화 상태 모니터링
    useEffect(() => {
        // HMR 재렌더링 시: 데몬이 이미 준비된 상태라면 로딩 화면을 건너뜀
        if (window.api && window.api.daemonStatus) {
            window.api.daemonStatus().then((status) => {
                if (status && status.running) {
                    console.log('[HMR] Daemon already running, skipping loading screen');
                    setInitStatus('Ready!');
                    setInitProgress(100);
                    setDaemonReady(true);
                    setServersInitializing(false);
                }
            }).catch(() => {});
        }

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

        // 업데이트 발견 알림 → 알림 모달에 추가
        if (window.api && window.api.onUpdatesAvailable) {
            window.api.onUpdatesAvailable((data) => {
                console.log('[Updater] Updates available notification:', data);
                const count = data.count || data.updates_available || 0;
                const names = data.names || data.update_names || [];
                if (count > 0 && window.__sabaNotice) {
                    window.__sabaNotice.addNotice({
                        message: `📦 ${count}개 업데이트 발견: ${names.join(', ') || '확인 필요'}`,
                        type: 'info',
                        source: 'Updater',
                        action: 'openUpdateModal',
                        dedup: true,
                    });
                }
            });
        }

        // --after-update로 재기동된 경우 완료 알림 표시
        if (window.api && window.api.onUpdateCompleted) {
            window.api.onUpdateCompleted((data) => {
                console.log('[Updater] Update completed notification:', data);
                setTimeout(() => {
                    if (typeof window.showToast === 'function') {
                        window.showToast(data.message || '업데이트가 완료되었습니다!', 'success', 5000, { isNotice: true, source: 'saba-chan' });
                    }
                    // 알림 모달에도 추가
                    if (window.__sabaNotice) {
                        window.__sabaNotice.addNotice({
                            message: data.message || '업데이트가 완료되었습니다!',
                            type: 'success',
                            source: 'Updater',
                        });
                    }
                }, 1500); // UI가 완전히 렌더링될 때까지 약간 대기
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
                    setIpcPort(settings.ipcPort ?? 57474);
                    setConsoleBufferSize(settings.consoleBufferSize ?? 2000);
                    consoleBufferRef.current = settings.consoleBufferSize ?? 2000;
                    setModulesPath(settings.modulesPath || '');
                    setDiscordToken(settings.discordToken || '');
                    discordTokenRef.current = settings.discordToken || '';
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
                ipcPort,
                consoleBufferSize,
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

    // refreshInterval / ipcPort / consoleBufferSize 변경 시 저장
    useEffect(() => {
        // 초기 로드 완료 전에는 저장하지 않음
        if (!settingsReady || !settingsPath) return;
        
        const currentSettings = { autoRefresh, refreshInterval, ipcPort, consoleBufferSize };
        
        // 첫 번째 호출 시 초기값 저장만 하고 저장하지 않음
        if (prevSettingsRef.current === null) {
            prevSettingsRef.current = currentSettings;
            return;
        }
        
        // 실제로 값이 변경되었을 때만 저장
        if (prevSettingsRef.current.autoRefresh !== autoRefresh ||
            prevSettingsRef.current.refreshInterval !== refreshInterval ||
            prevSettingsRef.current.ipcPort !== ipcPort ||
            prevSettingsRef.current.consoleBufferSize !== consoleBufferSize) {
            console.log('[Settings] Settings changed, saving...');
            saveCurrentSettings();
            prevSettingsRef.current = currentSettings;
        }
    }, [settingsReady, autoRefresh, refreshInterval, ipcPort, consoleBufferSize]);

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
    const safeShowToast = (message, type, duration, options) => {
        if (typeof window.showToast === 'function') {
            return window.showToast(message, type, duration, options);
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
                safeShowToast(t('discord_bot.started_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
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
                safeShowToast(t('discord_bot.stopped_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
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
                    // 봇을 재시작 (bot-config.json에는 token이 없으므로 현재 토큰을 주입)
                    const configWithToken = { ...botConfig, token: discordTokenRef.current };
                    const result = await window.api.discordBotStart(configWithToken);
                    if (result.error) {
                        console.error('[Bot Relaunch] Failed to relaunch bot:', result.error);
                    } else {
                        console.log('[Bot Relaunch] Bot relaunched successfully');
                        setDiscordBotStatus('running');
                        safeShowToast(t('discord_bot.relaunched_toast'), 'discord', 3000, { isNotice: true, source: 'Discord Bot' });
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
        
        return () => {
            clearInterval(interval);
            // IPC 리스너 정리 (중복 등록 방지)
            if (window.api.offCloseRequest) window.api.offCloseRequest();
            if (window.api.offBotRelaunch) window.api.offBotRelaunch();
        };
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
                
                // 각 모듈의 locale 파일을 로드하여 i18next에 동적 등록
                for (const module of data.modules) {
                    try {
                        if (window.api.moduleGetLocales) {
                            const locales = await window.api.moduleGetLocales(module.name);
                            if (locales && typeof locales === 'object') {
                                for (const [lang, localeData] of Object.entries(locales)) {
                                    i18n.addResourceBundle(lang, `mod_${module.name}`, localeData, true, true);
                                }
                                console.log(`Module locales registered for ${module.name}:`, Object.keys(locales));
                            }
                        }
                    } catch (e) {
                        console.warn(`Failed to load locales for module ${module.name}:`, e);
                    }
                }
                
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
    // GUI에서 시작/종료를 요청한 서버 이름 (외부 변경 vs GUI 조작 구분용)
    const guiInitiatedOpsRef = useRef(new Set());
    // 최초 fetchServers 완료 여부 (초기화 중 외부 변경 오감지 방지)
    const firstFetchDoneRef = useRef(false);
    
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
                    // 최초 fetch일 때는 상태 변경 감지 스킵 (기존 서버가 이미 running일 수 있음)
                    if (!firstFetchDoneRef.current) {
                        firstFetchDoneRef.current = true;
                        return data.servers.map(newServer => {
                            const existing = prev.find(s => s.name === newServer.name);
                            return { ...newServer, expanded: existing?.expanded || false };
                        });
                    }

                    // 상태 변경 감지 (크래시 / 외부 시작·종료)
                    for (const newServer of data.servers) {
                        const existing = prev.find(s => s.name === newServer.name);
                        if (!existing) continue;

                        const wasRunning = existing.status === 'running';
                        const nowStopped = newServer.status === 'stopped';
                        const nowRunning = newServer.status === 'running';
                        const wasStopped = existing.status === 'stopped';
                        const isGuiOp = guiInitiatedOpsRef.current.has(newServer.name);

                        if (wasRunning && nowStopped && !isGuiOp) {
                            // 서버가 예상치 못하게 종료됨 (크래시 또는 디스코드 봇 명령)
                            safeShowToast(
                                t('servers.unexpected_stop_toast', { name: newServer.name }),
                                'error', 5000,
                                { isNotice: true, source: newServer.name }
                            );
                        } else if (wasStopped && nowRunning && !isGuiOp) {
                            // 외부에서 서버가 시작됨 (디스코드 봇 명령 등)
                            safeShowToast(
                                t('servers.external_start_toast', { name: newServer.name }),
                                'info', 3000,
                                { isNotice: true, source: newServer.name }
                            );
                        }

                        // GUI 조작 플래그 해제 (상태 전환 완료)
                        if (isGuiOp && (nowStopped || nowRunning) && existing.status !== newServer.status) {
                            guiInitiatedOpsRef.current.delete(newServer.name);
                        }
                    }

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
        try {
            // 인스턴스 ID 찾기
            const srv = servers.find(s => s.name === name);
            if (!srv) {
                safeShowToast(t('servers.start_failed_toast', { error: 'Instance not found' }), 'error', 4000);
                return;
            }

            // 시작 방식 결정: 인스턴스별 managed_start 설정 우선, 없으면 모듈 interaction_mode
            const mod = modules.find(m => m.name === module);
            const instanceManagedStart = srv.module_settings?.managed_start;
            let interactionMode;
            if (instanceManagedStart === true) {
                interactionMode = 'console';
            } else if (instanceManagedStart === false) {
                interactionMode = 'commands';
            } else {
                interactionMode = mod?.interaction_mode || 'console';
            }
            let result;
            if (interactionMode === 'console') {
                // Managed 모드로 시작 (stdin/stdout capture) — console 모드 전용
                result = await window.api.managedStart(srv.id);
            } else {
                // 일반 모드로 시작 — commands 모드 (프로세스만 실행, 콘솔 미사용)
                result = await window.api.serverStart(name, { module });
            }

            // ── action_required: 서버 jar 미발견 → 사용자에게 선택지 제시 ──
            if (result.action_required === 'server_jar_not_found') {
                setModal({
                    type: 'question',
                    title: t('servers.jar_not_found_title'),
                    message: result.configured_path
                        ? t('servers.jar_not_found_message_with_path', { path: result.configured_path })
                        : t('servers.jar_not_found_message'),
                    buttons: [
                        {
                            label: t('servers.jar_action_update_path'),
                            action: async () => {
                                setModal(null);
                                try {
                                    const filePath = await window.api.openFileDialog({
                                        filters: [{ name: 'JAR', extensions: ['jar'] }],
                                        title: t('servers.select_server_jar'),
                                    });
                                    if (filePath) {
                                        // 서버 인스턴스에서 해당 이름 찾아 ID 가져오기
                                        const srv = servers.find(s => s.name === name);
                                        if (srv) {
                                            await window.api.instanceUpdateSettings(srv.id, { executable_path: filePath });
                                            safeShowToast(t('servers.jar_path_updated'), 'success', 3000);
                                            await fetchServers();
                                            // 경로 업데이트 후 자동 시작
                                            handleStart(name, module);
                                        }
                                    }
                                } catch (err) {
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            }
                        },
                        {
                            label: t('servers.jar_action_install_new'),
                            action: async () => {
                                setModal(null);
                                try {
                                    // 설치 디렉토리 선택
                                    const installDir = await window.api.openFolderDialog();
                                    if (!installDir) return;

                                    setProgressBar({ message: t('servers.progress_fetching_versions'), indeterminate: true });

                                    // 최신 릴리즈 버전으로 설치
                                    const versions = await window.api.moduleListVersions(module, { per_page: 1 });
                                    const latestVersion = versions?.latest?.release;
                                    if (!latestVersion) {
                                        setProgressBar(null);
                                        safeShowToast(t('servers.version_fetch_failed'), 'error', 4000);
                                        return;
                                    }

                                    setProgressBar({ message: t('servers.progress_downloading', { version: latestVersion }), percent: 0 });

                                    const installResult = await window.api.moduleInstallServer(module, {
                                        version: latestVersion,
                                        install_dir: installDir,
                                        accept_eula: true,
                                    });

                                    if (installResult.error || installResult.success === false) {
                                        setProgressBar(null);
                                        safeShowToast(installResult.error || installResult.message, 'error', 4000);
                                        return;
                                    }

                                    setProgressBar({ message: t('servers.progress_configuring'), percent: 90 });

                                    // 인스턴스의 executable_path를 설치된 jar로 업데이트
                                    const srv = servers.find(s => s.name === name);
                                    if (srv && installResult.jar_path) {
                                        await window.api.instanceUpdateSettings(srv.id, {
                                            executable_path: installResult.jar_path,
                                            working_dir: installResult.install_path,
                                        });
                                    }

                                    setProgressBar({ message: t('servers.progress_complete'), percent: 100 });
                                    setTimeout(() => setProgressBar(null), 2000);

                                    const msg = installResult.java_warning
                                        ? `${t('servers.install_completed', { version: latestVersion })}\n⚠️ ${installResult.java_warning}`
                                        : t('servers.install_completed', { version: latestVersion });
                                    safeShowToast(msg, 'success', 5000);
                                    await fetchServers();

                                    // Java 버전 경고가 없으면 자동 시작
                                    if (!installResult.java_warning) {
                                        handleStart(name, module);
                                    }
                                } catch (err) {
                                    setProgressBar(null);
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            }
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null)
                        }
                    ]
                });
                return;
            }

            if (result.error) {
                const errorMsg = translateError(result.error);
                safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
            } else {
                // GUI에서 시작한 것으로 표시 (외부 시작 감지 방지)
                guiInitiatedOpsRef.current.add(name);
                // 시작 명령 성공 — indeterminate 프로그레스바 표시
                setProgressBar({ message: t('servers.starting_toast', { name }), indeterminate: true });
                // console 모드일 때만 콘솔 자동 오픈
                if (interactionMode === 'console') {
                    openConsole(srv.id, name);
                }
                
                // 서버 상태가 running이 될 때까지 대기 (최대 30초)
                // setTimeout 순차 실행으로 async 경쟁 조건 방지
                let attempts = 0;
                const maxAttempts = 60;
                const delay = 500;
                let resolved = false;
                
                const checkStatus = async () => {
                    if (resolved) return;
                    attempts++;
                    try {
                        const statusResult = await window.api.serverStatus(name);
                        if (statusResult.status === 'running') {
                            resolved = true;
                            setProgressBar(null);
                            safeShowToast(t('servers.start_completed_toast', { name }), 'success', 3000, { isNotice: true, source: name });
                            fetchServers();
                            return;
                        }
                    } catch (error) { /* ignore */ }
                    if (attempts >= maxAttempts) {
                        resolved = true;
                        setProgressBar(null);
                        safeShowToast(t('servers.start_timeout_toast', { name }), 'warning', 3000);
                        fetchServers();
                        return;
                    }
                    if (!resolved) setTimeout(checkStatus, delay);
                };
                setTimeout(checkStatus, delay);
            }
        } catch (error) {
            setProgressBar(null);
            const errorMsg = translateError(error.message);
            safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
        }
    };

    // ── Console Panel Management ──────────────────────────────

    const openConsole = (instanceId, serverName) => {
        setConsoleServer({ id: instanceId, name: serverName });
        setConsoleLines([]);
        setConsoleSinceId(0);
        setConsoleInput('');

        // Start polling
        if (consolePollingRef.current) clearInterval(consolePollingRef.current);
        let sinceId = 0;
        consolePollingRef.current = setInterval(async () => {
            try {
                const data = await window.api.managedConsole(instanceId, sinceId, 200);
                if (data?.lines?.length > 0) {
                    setConsoleLines(prev => {
                        const newLines = [...prev, ...data.lines];
                        // Keep last N lines (from settings)
                        const maxLines = consoleBufferRef.current || 2000;
                        return newLines.length > maxLines ? newLines.slice(-maxLines) : newLines;
                    });
                    sinceId = data.lines[data.lines.length - 1].id + 1;
                    setConsoleSinceId(sinceId);
                }
            } catch (err) {
                // silent — server might not be ready yet
            }
        }, 500);
    };

    const closeConsole = () => {
        if (consolePollingRef.current) {
            clearInterval(consolePollingRef.current);
            consolePollingRef.current = null;
        }
        setConsoleServer(null);
        setConsoleLines([]);
        setConsoleSinceId(0);
    };

    const sendConsoleCommand = async () => {
        if (!consoleInput.trim() || !consoleServer) return;
        const cmd = consoleInput.trim();
        try {
            // managed 프로세스 stdin으로 먼저 시도
            const result = await window.api.managedStdin(consoleServer.id, cmd);
            if (result?.error) {
                // stdin 실패 시 → RCON 직접 호출 (Python lifecycle 우회, 빠른 경로)
                console.log('[Console] stdin failed, trying RCON direct:', result.error);
                const rconResult = await window.api.executeCommand(consoleServer.id, {
                    command: cmd,
                    args: {},
                    commandMetadata: { method: 'rcon' },
                });
                if (rconResult?.error) {
                    safeShowToast(translateError(rconResult.error), 'error', 3000);
                } else {
                    // RCON 응답을 콘솔에 표시 (콘솔 렌더링은 content/source/level 필드 사용)
                    const responseText = rconResult?.data?.response || rconResult?.message || '';
                    const lines = [
                        { id: Date.now(), content: `> ${cmd}`, source: 'STDIN', level: 'INFO' },
                    ];
                    if (responseText) {
                        lines.push({ id: Date.now() + 1, content: responseText, source: 'STDOUT', level: 'INFO' });
                    }
                    setConsoleLines(prev => [...prev, ...lines]);
                }
            }
            setConsoleInput('');
        } catch (err) {
            safeShowToast(translateError(err.message), 'error', 3000);
        }
    };

    // Auto-scroll console
    useEffect(() => {
        if (consoleEndRef.current) {
            consoleEndRef.current.scrollIntoView({ behavior: 'smooth' });
        }
    }, [consoleLines]);

    // Cleanup polling on unmount
    useEffect(() => {
        return () => {
            if (consolePollingRef.current) clearInterval(consolePollingRef.current);
        };
    }, []);

    const handleStop = async (name) => {
        setModal({
            type: 'question',
            title: t('servers.stop_confirm_title'),
            message: t('servers.stop_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                try {
                    // graceful_stop 설정 확인 (인스턴스 module_settings에서)
                    const srv = servers.find(s => s.name === name);
                    const useGraceful = srv?.module_settings?.graceful_stop;
                    const forceStop = useGraceful === false; // graceful_stop이 명시적으로 false면 force
                    
                    const result = await window.api.serverStop(name, { force: forceStop });
                    if (result.error) {
                        const errorMsg = translateError(result.error);
                        safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                    } else {
                        // GUI에서 정지한 것으로 표시 (외부 정지 감지 방지)
                        guiInitiatedOpsRef.current.add(name);
                        // 정지 명령 성공 - 콘솔 열려있으면 닫기
                        if (srv && consoleServer?.id === srv.id) {
                            closeConsole();
                        }
                        // indeterminate 프로그레스바 표시
                        setProgressBar({ message: t('servers.stopping_toast', { name }), indeterminate: true });
                        
                        // 서버 상태가 stopped가 될 때까지 대기 (최대 10초)
                        // setTimeout 순차 실행으로 async 경쟁 조건 방지
                        let attempts = 0;
                        const maxAttempts = 20;
                        const delay = 500;
                        let resolved = false;
                        
                        const checkStatus = async () => {
                            if (resolved) return;
                            attempts++;
                            try {
                                const statusResult = await window.api.serverStatus(name);
                                if (statusResult.status === 'stopped') {
                                    resolved = true;
                                    setProgressBar(null);
                                    safeShowToast(t('servers.stop_completed_toast', { name }), 'success', 3000, { isNotice: true, source: name });
                                    fetchServers();
                                    return;
                                }
                            } catch (error) { /* ignore */ }
                            if (attempts >= maxAttempts) {
                                resolved = true;
                                setProgressBar(null);
                                safeShowToast(t('servers.stop_timeout_toast', { name }), 'warning', 3000);
                                fetchServers();
                                return;
                            }
                            if (!resolved) setTimeout(checkStatus, delay);
                        };
                        setTimeout(checkStatus, delay);
                    }
                } catch (error) {
                    setProgressBar(null);
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
                
                // 1. instances.json에서 이미 저장된 값이 있는지 확인 (기본 필드)
                if (latestServer[field.name] !== undefined && latestServer[field.name] !== null) {
                    value = String(latestServer[field.name]);
                    console.log(`Loaded ${field.name} from instance:`, value);
                }
                // 2. module_settings에서 동적 설정 값 확인
                else if (latestServer.module_settings && latestServer.module_settings[field.name] !== undefined && latestServer.module_settings[field.name] !== null) {
                    value = String(latestServer.module_settings[field.name]);
                    console.log(`Loaded ${field.name} from module_settings:`, value);
                }
                // 3. 없으면 module.toml의 default 값 사용
                else if (field.default !== undefined && field.default !== null) {
                    value = String(field.default);
                    console.log(`Using default for ${field.name}:`, value);
                }
                
                initial[field.name] = value;
            });
            
            // protocol_mode 초기화 (별도 처리)
            // 모듈의 지원 프로토콜 확인하여 올바른 기본값 사용
            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];
            if (latestServer.protocol_mode && latestServer.protocol_mode !== 'auto' && latestServer.protocol_mode !== 'rest' || (latestServer.protocol_mode === 'rest' && supportedProtocols.includes('rest'))) {
                initial.protocol_mode = latestServer.protocol_mode;
            } else if (protocols.default) {
                initial.protocol_mode = protocols.default;
            } else if (supportedProtocols.length > 0) {
                initial.protocol_mode = supportedProtocols[0];
            } else {
                initial.protocol_mode = latestServer.protocol_mode || 'auto';
            }
            console.log('Loaded protocol_mode:', initial.protocol_mode);
            
            console.log('Initialized settings values:', initial);
            setSettingsValues(initial);
        } else {
            // 모듈 설정이 없어도 protocol_mode는 설정
            const protocols = module?.protocols || {};
            const defaultProto = protocols.default || (protocols.supported?.length > 0 ? protocols.supported[0] : null);
            setSettingsValues({
                protocol_mode: (latestServer.protocol_mode && latestServer.protocol_mode !== 'auto' && latestServer.protocol_mode !== 'rest') ? latestServer.protocol_mode : (defaultProto || latestServer.protocol_mode || 'auto')
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
        setAdvancedExpanded(false); // 고급 설정 접힘
        setShowSettingsModal(true);
        
        // 비동기로 서버 버전 목록 로드
        setAvailableVersions([]);
        setVersionsLoading(true);
        try {
            const versions = await window.api.moduleListVersions(latestServer.module, { per_page: 30 });
            if (versions && versions.versions) {
                setAvailableVersions(versions.versions);
            }
        } catch (err) {
            console.warn('Failed to load versions:', err);
        } finally {
            setVersionsLoading(false);
        }
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
                    
                    if (field.field_type === 'boolean') {
                        convertedSettings[field.name] = value === true || value === 'true';
                        return;
                    }
                    
                    if (value === '' || value === null || value === undefined) {
                        return; // 빈 값은 전송하지 않음
                    }
                    
                    if (field.field_type === 'number') {
                        convertedSettings[field.name] = Number(value);
                    } else if (field.field_type === 'boolean') {
                        convertedSettings[field.name] = value === true || value === 'true';
                    } else {
                        convertedSettings[field.name] = value;
                    }
                });
            }
            
            // server_version 수동 추가 (module.toml fields에 없는 하드코딩 필드)
            if (settingsValues.server_version) {
                convertedSettings.server_version = settingsValues.server_version;
            }
            
            // 프로토콜 지원 여부 확인
            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];
            
            // 프로토콜이 지원되는 경우 protocol_mode 전송
            if (supportedProtocols.length > 0) {
                // 모듈이 둘 다 지원하면 사용자 선택값, 하나만 지원하면 기본값 사용
                if (supportedProtocols.includes('rest') && supportedProtocols.includes('rcon')) {
                    convertedSettings.protocol_mode = settingsValues.protocol_mode || protocols.default || supportedProtocols[0];
                } else {
                    convertedSettings.protocol_mode = protocols.default || supportedProtocols[0];
                }
            } else {
                // 프로토콜 정보가 없으면 auto
                convertedSettings.protocol_mode = settingsValues.protocol_mode || 'auto';
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
                return <Icon name="play" size="xs" />;
            case 'stopped':
                return <Icon name="stop" size="xs" />;
            case 'starting':
                return <Icon name="loader" size="xs" />;
            case 'stopping':
                return <Icon name="pause" size="xs" />;
            default:
                return <Icon name="alertCircle" size="xs" />;
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
                        <img src="./title.png" alt="" className="loading-logo-img" />
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
                    onClick={requestDiscordClose}
                />
            )}
            {/* Background overlay backdrop */}
            {showBackgroundSection && (
                <div 
                    className="discord-backdrop" 
                    onClick={requestBackgroundClose}
                />
            )}
            {/* Notice overlay backdrop */}
            {showNoticeSection && (
                <div 
                    className="discord-backdrop" 
                    onClick={requestNoticeClose}
                />
            )}
            <TitleBar />
            <Toast />
            <header className="app-header">
                {/* 첫 번째 줄: 타이틀과 설정 */}
                <div className="header-row header-row-title">
                    <div className="app-title-section">
                        <img src="./icon.png" alt="" className="app-logo-icon" />
                        <img src={logoSrc} alt={t('common:app_name')} className="app-logo-text" />
                    </div>
                    <div className="header-actions">
                        <div className="notice-button-wrapper">
                            <button 
                                className="btn-settings-icon-solo"
                                onClick={() => showNoticeSection ? requestNoticeClose() : setShowNoticeSection(true)}
                                title={t('notice_modal.tooltip')}
                            >
                                <Icon name="bell" size="lg" />
                            </button>
                            {unreadNoticeCount > 0 && (
                                <span className="notice-badge-dot">{unreadNoticeCount > 9 ? '9+' : unreadNoticeCount}</span>
                            )}
                            <NoticeModal
                                isOpen={showNoticeSection}
                                onClose={requestNoticeClose}
                                isClosing={isNoticeClosing}
                                onOpenUpdateModal={() => {
                                    setSettingsInitialView('update');
                                    setShowGuiSettingsModal(true);
                                }}
                            />
                        </div>
                        <button 
                            className="btn-settings-icon-solo"
                            onClick={() => setShowGuiSettingsModal(true)}
                            title={t('settings.gui_settings_tooltip')}
                        >
                            <Icon name="settings" size="lg" />
                        </button>
                    </div>
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
                            onClick={() => showDiscordSection ? requestDiscordClose() : setShowDiscordSection(true)}
                        >
                            <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : 'status-offline'}`}></span>
                            Discord Bot
                        </button>
                        {/* Discord Bot Modal */}
                        <DiscordBotModal
                            isOpen={showDiscordSection}
                            onClose={requestDiscordClose}
                            isClosing={isDiscordClosing}
                            discordBotStatus={discordBotStatus}
                            discordToken={discordToken}
                            setDiscordToken={(val) => { setDiscordToken(val); discordTokenRef.current = val; }}
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
                            onClick={() => showBackgroundSection ? requestBackgroundClose() : setShowBackgroundSection(true)}
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
                            onClose={requestBackgroundClose}
                            isClosing={isBackgroundClosing}
                            ipcPort={ipcPort}
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
                        const gameName = t(`mod_${server.module}:module.display_name`, { defaultValue: moduleData?.game_name || server.module });
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
                                        <p className="game-name">
                                            {gameName}
                                            {server.server_version && (
                                                <span className="server-version-badge">{server.server_version}</span>
                                            )}
                                        </p>
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
                                        <span className="value">{(() => {
                                            const mod = modules.find(m => m.name === server.module);
                                            const proto = server.protocol_mode;
                                            // auto 또는 모듈이 지원하지 않는 프로토콜이면 모듈 기본값 표시
                                            if (proto === 'auto' || proto === 'rest') {
                                                const moduleDefault = mod?.protocols?.default;
                                                const supported = mod?.protocols?.supported || [];
                                                if (proto === 'rest' && supported.includes('rest')) {
                                                    return 'REST';
                                                }
                                                if (moduleDefault) return moduleDefault.toUpperCase();
                                                if (supported.length > 0) return supported[0].toUpperCase();
                                            }
                                            return proto?.toUpperCase() || 'AUTO';
                                        })()}</span>
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
                                    {server.status === 'running' ? (
                                        <>
                                            {/* interaction_mode에 따라 콘솔 또는 커맨드 버튼 표시 */}
                                            {(() => {
                                                const mod = modules.find(m => m.name === server.module);
                                                const mode = mod?.interaction_mode || 'console';
                                                if (mode === 'console') {
                                                    return (
                                                        <button 
                                                            className={`action-icon ${consoleServer?.id === server.id ? 'action-active' : ''}`}
                                                            onClick={() => {
                                                                if (consoleServer?.id === server.id) {
                                                                    closeConsole();
                                                                } else {
                                                                    openConsole(server.id, server.name);
                                                                }
                                                            }}
                                                            title="Console"
                                                        >
                                                            <Icon name="terminal" size="md" />
                                                        </button>
                                                    );
                                                } else {
                                                    return (
                                                        <button 
                                                            className="action-icon"
                                                            onClick={() => {
                                                                setCommandServer(server);
                                                                setShowCommandModal(true);
                                                            }}
                                                            title="Command"
                                                        >
                                                            <Icon name="command" size="md" />
                                                        </button>
                                                    );
                                                }
                                            })()}
                                        </>
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

                {/* 콘솔 패널 */}
                {consoleServer && (
                    <div className="console-panel">
                        <div className="console-header">
                            <span className="console-title">
                                <span className="console-icon">{'>'}_</span>
                                {consoleServer.name}
                            </span>
                            <button className="console-close" onClick={closeConsole} title="Close">&times;</button>
                        </div>
                        <div className="console-output">
                            {consoleLines.length === 0 && (
                                <div className="console-empty">{t('console.waiting')}</div>
                            )}
                            {consoleLines.map((line) => (
                                <div key={line.id} className={`console-line console-${line.source?.toLowerCase() || 'stdout'} console-level-${line.level?.toLowerCase() || 'info'}`}>
                                    <span className="console-content">{line.content}</span>
                                </div>
                            ))}
                            <div ref={consoleEndRef} />
                        </div>
                        <div className="console-input-row">
                            <span className="console-prompt">{'>'}</span>
                            <input
                                type="text"
                                className="console-input"
                                value={consoleInput}
                                onChange={(e) => setConsoleInput(e.target.value)}
                                onKeyDown={(e) => { if (e.key === 'Enter') sendConsoleCommand(); }}
                                placeholder={t('console.input_placeholder')}
                                autoFocus
                            />
                            <button className="console-send" onClick={sendConsoleCommand}>{t('console.send')}</button>
                        </div>
                    </div>
                )}

            </main>

            {showSettingsModal && settingsServer && (
                <div className={`modal-overlay ${isSettingsClosing ? 'closing' : ''}`} onClick={requestSettingsClose}>
                    <div className="modal-content modal-content-large" onClick={e => e.stopPropagation()}>
                        <div className="modal-header">
                            <h3 style={{ fontSize: '1.3rem' }}>{settingsServer.name} - {t('server_settings.title')}</h3>
                        </div>
                        
                        {/* 탭 헤더 */}
                        <div className="settings-tabs" data-tab={settingsActiveTab}>
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'general' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('general')}
                            >
                                <Icon name="gamepad" size="sm" /> {t('server_settings.general_tab')}
                            </button>
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'aliases' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('aliases')}
                            >
                                <Icon name="discord" size="sm" /> {t('server_settings.aliases_tab') }
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
                                                    <span className="protocol-mode-title"><Icon name="plug" size="sm" /> {t('server_settings.protocol_title')}</span>
                                                </div>
                                                <p className="protocol-mode-description">
                                                    {t('server_settings.protocol_description')}
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
                                                    <span className="hint-icon"><Icon name="lightbulb" size="sm" /></span>
                                                    {settingsValues.protocol_mode === 'rest' 
                                                        ? t('server_settings.protocol_rest_hint')
                                                        : t('server_settings.protocol_rcon_hint')}
                                                </p>
                                            </div>
                                        )}
                                        
                                        {/* 프로토콜이 하나만 지원될 때 정보 표시 */}
                                        {!showProtocolToggle && supportedProtocols.length > 0 && (
                                            <div className="protocol-mode-section protocol-mode-info">
                                                <div className="protocol-mode-header">
                                                    <span className="protocol-mode-title"><Icon name="plug" size="sm" /> {t('server_settings.protocol_title')}</span>
                                                </div>
                                                <p className="protocol-mode-description" dangerouslySetInnerHTML={{ __html: t('server_settings.protocol_single_only', { protocol: supportedProtocols[0].toUpperCase() }) }} />
                                            </div>
                                        )}

                                        {/* 모듈 설정 필드 - 그룹별 렌더링 */}
                                        {hasModuleSettings ? (() => {
                                            const modNs = `mod_${settingsServer.module}`;
                                            
                                            // 필드를 그룹별로 분류
                                            const sabaFields = module.settings.fields.filter(f => f.group === 'saba-chan');
                                            const basicFields = module.settings.fields.filter(f => !f.group || f.group === 'basic');
                                            const advancedFields = module.settings.fields.filter(f => f.group === 'advanced');
                                            
                                            const renderField = (field) => {
                                                const fieldLabel = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label });
                                                const fieldDesc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
                                                return (
                                                <div key={field.name} className="settings-field">
                                                    <label>{fieldLabel} {field.required ? '*' : ''}</label>
                                                    {field.field_type === 'text' && (
                                                        <input 
                                                            type="text"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={fieldDesc}
                                                        />
                                                    )}
                                                    {field.field_type === 'password' && (
                                                        <input 
                                                            type="password"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={fieldDesc}
                                                        />
                                                    )}
                                                    {field.field_type === 'number' && (
                                                        <input 
                                                            type="number"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            min={field.min}
                                                            max={field.max}
                                                            placeholder={fieldDesc}
                                                        />
                                                    )}
                                                    {field.field_type === 'file' && (
                                                        <input 
                                                            type="text"
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(e) => handleSettingChange(field.name, e.target.value)}
                                                            placeholder={fieldDesc}
                                                        />
                                                    )}
                                                    {field.field_type === 'select' && (
                                                        <CustomDropdown
                                                            value={String(settingsValues[field.name] || '')}
                                                            onChange={(val) => handleSettingChange(field.name, val)}
                                                            placeholder={fieldLabel}
                                                            options={(field.options || []).map(opt => ({ value: opt, label: opt }))}
                                                        />
                                                    )}
                                                    {field.field_type === 'boolean' && (
                                                        <div className="toggle-row">
                                                            <label className="toggle-switch">
                                                                <input 
                                                                    type="checkbox"
                                                                    checked={settingsValues[field.name] === true || settingsValues[field.name] === 'true'}
                                                                    onChange={(e) => handleSettingChange(field.name, e.target.checked)}
                                                                />
                                                                <span className="toggle-slider"></span>
                                                            </label>
                                                            <span className="toggle-label-text">
                                                                {settingsValues[field.name] === true || settingsValues[field.name] === 'true' ? 'ON' : 'OFF'}
                                                            </span>
                                                        </div>
                                                    )}
                                                    {fieldDesc && (
                                                        <small className="field-description">{fieldDesc}</small>
                                                    )}
                                                </div>
                                                );
                                            };
                                            
                                            return (
                                                <>
                                                    {/* saba-chan 전용 설정 */}
                                                    {sabaFields.length > 0 && (
                                                        <div className="settings-group">
                                                            <h4 className="settings-group-title">
                                                                <Icon name="settings" size="sm" /> {t('server_settings.saba_chan_group', { defaultValue: 'saba-chan Settings' })}
                                                            </h4>
                                                            
                                                            {/* 서버 버전 선택 */}
                                                            <div className="settings-field">
                                                                <label>{t('server_settings.server_version', { defaultValue: 'Server Version' })}</label>
                                                                {versionsLoading ? (
                                                                    <div className="version-loading">
                                                                        <Icon name="loader" size="sm" /> {t('server_settings.loading_versions', { defaultValue: 'Loading versions...' })}
                                                                    </div>
                                                                ) : (
                                                                    <CustomDropdown
                                                                        value={settingsValues.server_version || ''}
                                                                        onChange={(val) => handleSettingChange('server_version', val)}
                                                                        placeholder={t('server_settings.select_version', { defaultValue: 'Select version' })}
                                                                        options={availableVersions.map(v => ({
                                                                            value: v.id || v.version || v,
                                                                            label: `${v.id || v.version || v}${v.type ? ` (${v.type})` : ''}`
                                                                        }))}
                                                                    />
                                                                )}
                                                                <small className="field-description">
                                                                    {t('server_settings.version_description', { defaultValue: 'Server version to track (for display purposes)' })}
                                                                </small>
                                                            </div>
                                                            
                                                            {sabaFields.map(renderField)}
                                                        </div>
                                                    )}
                                                    
                                                    {/* 기본 서버 설정 */}
                                                    {basicFields.length > 0 && (
                                                        <div className="settings-group">
                                                            <h4 className="settings-group-title">
                                                                <Icon name="gamepad" size="sm" /> {t('server_settings.basic_group', { defaultValue: 'Server Settings' })}
                                                            </h4>
                                                            {basicFields.map(renderField)}
                                                        </div>
                                                    )}
                                                    
                                                    {/* 고급 설정 (접이식) */}
                                                    {advancedFields.length > 0 && (
                                                        <div className="settings-group settings-group-advanced">
                                                            <h4 
                                                                className="settings-group-title settings-group-collapsible"
                                                                onClick={() => setAdvancedExpanded(!advancedExpanded)}
                                                            >
                                                                <Icon name={advancedExpanded ? 'chevron-down' : 'chevron-right'} size="sm" />
                                                                {' '}{t('server_settings.advanced_group', { defaultValue: 'Advanced Settings' })}
                                                                <span className="settings-group-count">({advancedFields.length})</span>
                                                            </h4>
                                                            {advancedExpanded && advancedFields.map(renderField)}
                                                        </div>
                                                    )}
                                                </>
                                            );
                                        })() : (
                                            <p className="no-settings" style={{marginTop: '16px'}}>{t('server_settings.no_settings')}</p>
                                        )}
                                    </div>
                                );
                            })()}
                            
                            {/* Discord 별명 탭 */}
                            {settingsActiveTab === 'aliases' && (
                                <div className="aliases-tab-content">
                                    <div className="module-aliases-detail">
                                        <h4><Icon name="edit" size="sm" /> {t('server_settings.module_aliases_title')}</h4>
                                        <small>{t('server_settings.module_aliases_hint', { module: settingsServer.module })}</small>
                                        <div className="module-aliases-input">
                                            <input
                                                type="text"
                                                placeholder={t('server_settings.module_aliases_placeholder', { module: settingsServer.module })}
                                                value={editingModuleAliases.join(' ')}
                                                onChange={(e) => {
                                                    const aliases = e.target.value.split(/\s+/).filter(a => a.length > 0);
                                                    setEditingModuleAliases(aliases);
                                                }}
                                            />
                                            {editingModuleAliases.length === 0 && (
                                                <div className="placeholder-hint">
                                                    <small><Icon name="lightbulb" size="xs" /> {t('server_settings.module_aliases_empty_hint')} <code>{settingsServer.module}</code></small>
                                                </div>
                                            )}
                                        </div>
                                        <div className="aliases-display">
                                            {editingModuleAliases.map((alias, idx) => (
                                                <span key={idx} className="alias-badge">{alias}</span>
                                            ))}
                                        </div>

                                        <h4><Icon name="zap" size="sm" /> {t('server_settings.command_aliases_title')}</h4>
                                        <small>{t('server_settings.command_aliases_hint')}</small>
                                        <div className="command-aliases-input">
                                            {Object.entries(editingCommandAliases).map(([cmd, cmdData]) => {
                                                const aliases = cmdData.aliases || [];
                                                const modNs = `mod_${settingsServer.module}`;
                                                const description = t(`${modNs}:commands.${cmd}.description`, { defaultValue: cmdData.description || '' });
                                                const label = t(`${modNs}:commands.${cmd}.label`, { defaultValue: cmdData.label || cmd });
                                                return (
                                                    <div key={cmd} className="command-alias-editor">
                                                        <div className="cmd-header">
                                                            <span className="cmd-name">{cmd}</span>
                                                            {label !== cmd && <span className="cmd-label">({label})</span>}
                                                            {description && <span className="cmd-help" title={description}>?</span>}
                                                        </div>
                                                        <input
                                                            type="text"
                                                            placeholder={t('server_settings.command_aliases_placeholder', { cmd })}
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
                                                <Icon name="save" size="sm" /> {t('server_settings.save_aliases')}
                                            </button>
                                            <button className="btn btn-reset" onClick={() => {
                                                const moduleName = settingsServer.module;
                                                handleResetAliasesForModule(moduleName);
                                            }}>
                                                <Icon name="refresh" size="sm" /> {t('server_settings.reset_aliases')}
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                        
                        <div className="modal-footer">
                            {settingsActiveTab === 'general' && (
                                <button className="btn btn-confirm" onClick={handleSaveSettings}>
                                    <Icon name="save" size="sm" /> {t('server_settings.save_settings')}
                                </button>
                            )}
                            <button className="btn btn-cancel" onClick={requestSettingsClose}>
                                <Icon name="close" size="sm" /> {t('server_settings.close')}
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
                onClose={() => { setShowGuiSettingsModal(false); setSettingsInitialView(null); }}
                refreshInterval={refreshInterval}
                onRefreshIntervalChange={setRefreshInterval}
                ipcPort={ipcPort}
                onIpcPortChange={setIpcPort}
                consoleBufferSize={consoleBufferSize}
                onConsoleBufferSizeChange={(val) => { setConsoleBufferSize(val); consoleBufferRef.current = val; }}
                onTestModal={setModal}
                onTestProgressBar={setProgressBar}
                initialView={settingsInitialView}
                onTestWaitingImage={() => {
                    setShowWaitingImage(true);
                    setTimeout(() => setShowWaitingImage(false), 4000);
                }}
                onTestLoadingScreen={() => {
                    setShowGuiSettingsModal(false);
                    setDaemonReady(false);
                    setInitStatus('Loading test...');
                    setInitProgress(0);
                    let p = 0;
                    const iv = setInterval(() => {
                        p += Math.random() * 20 + 10;
                        if (p >= 100) {
                            p = 100;
                            setInitStatus('Ready!');
                            setInitProgress(100);
                            clearInterval(iv);
                            setTimeout(() => setDaemonReady(true), 600);
                        } else {
                            setInitStatus(`Loading test... ${Math.round(p)}%`);
                            setInitProgress(p);
                        }
                    }, 500);
                }}
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



            {/* waiting.png (느린 진행 감지) */}
            {showWaitingImage && (
                <div className="waiting-image-overlay" onClick={() => setShowWaitingImage(false)}>
                    <img src="./waiting.png" alt="waiting" className="waiting-image" />
                </div>
            )}

            {/* 글로벌 프로그레스바 */}
            {progressBar && (
                <div className="global-progress-bar">
                    <div className="global-progress-content">
                        <span className="global-progress-message">{progressBar.message}</span>
                        {progressBar.percent != null && !progressBar.indeterminate && (
                            <span className="global-progress-percent">{Math.round(progressBar.percent)}%</span>
                        )}
                    </div>
                    <div className="global-progress-track">
                        <div
                            className={`global-progress-fill ${progressBar.indeterminate ? 'indeterminate' : ''} ${progressBar.percent === 100 ? 'complete' : ''}`}
                            style={progressBar.indeterminate ? {} : { width: `${progressBar.percent || 0}%` }}
                        />
                    </div>
                </div>
            )}
        </div>
    );
}

export default App;
