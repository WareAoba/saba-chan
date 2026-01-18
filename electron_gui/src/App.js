import React, { useState, useEffect, useRef } from 'react';
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
    BackgroundModal
} from './components';

function App() {
    const [servers, setServers] = useState([]);
    const [modules, setModules] = useState([]);
    const [loading, setLoading] = useState(true);
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

    // 초기화 완료 플래그 (state로 변경)
    const [botStatusReady, setBotStatusReady] = useState(false);
    const [settingsReady, setSettingsReady] = useState(false);
    const autoStartDoneRef = useRef(false);

    // 모듈별 별명 (각 모듈의 module.toml에서 정의한 별명들)
    const [moduleAliasesPerModule, setModuleAliasesPerModule] = useState({});  // { moduleName: { moduleAliases: [...], commands: {...} } }
    const [selectedModuleForAliases, setSelectedModuleForAliases] = useState(null);
    const [editingModuleAliases, setEditingModuleAliases] = useState({});
    const [editingCommandAliases, setEditingCommandAliases] = useState({});

    // 설정 로드
    useEffect(() => {
        const loadSettings = async () => {
            try {
                // 1. GUI 설정 로드
                const settings = await window.api.settingsLoad();
                console.log('[Settings] Loaded:', settings);
                if (settings) {
                    setAutoRefresh(settings.autoRefresh ?? true);
                    setRefreshInterval(settings.refreshInterval ?? 2000);
                    setModulesPath(settings.modulesPath || '');
                    setDiscordToken(settings.discordToken || '');
                    setDiscordAutoStart(settings.discordAutoStart ?? false);
                    console.log('[Settings] discordAutoStart:', settings.discordAutoStart, 'discordToken:', settings.discordToken ? 'YES' : 'NO');
                }
                const path = await window.api.settingsGetPath();
                setSettingsPath(path);
                console.log('[Settings] GUI settings loaded from:', path);
                
                // 2. Bot 설정 로드 (별도)
                const botCfg = await window.api.botConfigLoad();
                if (botCfg) {
                    setDiscordPrefix(botCfg.prefix || '!saba');
                    setDiscordModuleAliases(botCfg.moduleAliases || {});
                    setDiscordCommandAliases(botCfg.commandAliases || {});
                    console.log('[Settings] Bot config loaded, prefix:', botCfg.prefix);
                }
                
                // 설정 로드 완료
                setSettingsReady(true);
                console.log('[Settings] Ready flag set to true');
            } catch (error) {
                console.error('[Settings] Failed to load settings:', error);
                setSettingsReady(true);
            }
        };
        loadSettings();
    }, []);

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
                console.warn(`Attempt ${i + 1} failed, retrying in ${delay}ms...`, error.message);
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

    // refreshInterval 변경 시 저장 (autoRefresh는 항상 true로 고정)
    useEffect(() => {
        if (settingsPath) { // 초기 로드 이후에만 저장
            saveCurrentSettings();
        }
    }, [autoRefresh, refreshInterval]);

    // discordPrefix 변경 시 bot config 저장
    useEffect(() => {
        // 초기 로드 완료 후에만 저장 (빈 문자열 제외)
        if (settingsReady && settingsPath && discordPrefix && discordPrefix.trim()) {
            console.log('[Settings] Prefix changed, saving bot config:', discordPrefix);
            saveBotConfig(discordPrefix);
        }
    }, [discordPrefix]);

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

    // Discord Bot 시작
    const handleStartDiscordBot = async () => {
        if (!discordToken) {
            setModal({ type: 'failure', title: '토큰 없음', message: 'Discord Bot 토큰을 입력하세요.' });
            return;
        }
        if (!discordPrefix) {
            setModal({ type: 'failure', title: 'Prefix 없음', message: '봇 별명(Prefix)을 설정하세요. 예: !pal, !mc' });
            return;
        }
        try {
            await saveCurrentSettings();
            const botConfig = {
                token: discordToken,
                prefix: discordPrefix,
                moduleAliases: discordModuleAliases,
                commandAliases: discordCommandAliases
            };
            const result = await window.api.discordBotStart(botConfig);
            if (result.error) {
                window.showToast(`❌ Discord 봇 시작 실패: ${result.error}`, 'error', 4000);
            } else {
                setDiscordBotStatus('running');
                window.showToast('✅ Discord 봇이 시작되었습니다', 'discord', 3000);
            }
        } catch (e) {
            window.showToast(`❌ Discord 봇 시작 예외: ${e.message}`, 'error', 4000);
        }
    };

    // 자동시작 (설정과 봇 상태 모두 준비되면 실행)
    useEffect(() => {
        console.log('[Auto-start] Effect triggered', {
            botStatusReady,
            settingsReady,
            autoStartDone: autoStartDoneRef.current,
            discordAutoStart,
            tokenExists: !!discordToken,
            prefixExists: !!discordPrefix,
            botStatus: discordBotStatus
        });

        if (botStatusReady && settingsReady && !autoStartDoneRef.current) {
            autoStartDoneRef.current = true;
            
            if (discordAutoStart && discordToken && discordPrefix && discordBotStatus === 'stopped') {
                console.log('[Auto-start] ✅ Starting Discord bot automatically!');
                handleStartDiscordBot();
            } else {
                console.log('[Auto-start] ❌ Skipping - conditions not met');
            }
        }
    }, [botStatusReady, settingsReady, discordAutoStart, discordToken, discordPrefix, discordBotStatus]);

    // Discord Bot 정지
    const handleStopDiscordBot = async () => {
        try {
            const result = await window.api.discordBotStop();
            if (result.error) {
                window.showToast(`❌ Discord 봇 정지 실패: ${result.error}`, 'error', 4000);
            } else {
                setDiscordBotStatus('stopped');
                window.showToast('⏹️ Discord 봇이 정지되었습니다', 'discord', 3000);
            }
        } catch (e) {
            window.showToast(`❌ Discord 봇 정지 예외: ${e.message}`, 'error', 4000);
        }
    };

    useEffect(() => {
        console.log('App mounted, fetching initial data...');
        fetchServers();
        fetchModules();
        loadBotConfig();  // bot-config.json 로드
        
        // 앱 종료 요청 리스너 등록
        if (window.api.onCloseRequest) {
            window.api.onCloseRequest(() => {
                setModal({
                    type: 'question',
                    title: '종료 확인',
                    message: '어떻게 종료하시겠습니까?',
                    detail: 'GUI만 닫기: 백그라운드에서 계속 실행 (트레이에서 다시 열기 가능)\n완전히 종료: 데몬까지 모두 종료',
                    buttons: [
                        {
                            label: 'GUI만 닫기',
                            action: () => {
                                window.api.closeResponse('hide');
                                setModal(null);
                            }
                        },
                        {
                            label: '완전히 종료',
                            action: () => {
                                window.api.closeResponse('quit');
                                setModal(null);
                            }
                        },
                        {
                            label: '취소',
                            action: () => {
                                window.api.closeResponse('cancel');
                                setModal(null);
                            }
                        }
                    ]
                });
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
        console.log('Modules state updated:', modules);
    }, [modules]);

    const fetchModules = async () => {
        try {
            console.log('Fetching modules...');
            // Daemon이 준비될 때까지 대기
            try {
                await waitForDaemon(5000);
            } catch (err) {
                console.warn('Daemon not ready, but continuing:', err.message);
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
                        if (metadata && metadata.toml && metadata.toml.aliases) {
                            aliasesMap[module.name] = metadata.toml.aliases;
                        }
                    } catch (e) {
                        console.warn(`Failed to load metadata for module ${module.name}:`, e);
                    }
                }
                setModuleAliasesPerModule(aliasesMap);
                console.log('Module aliases loaded:', aliasesMap);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                window.showToast(`❌ 모듈 로드 실패: ${data.error}`, 'error', 4000);
            } else {
                console.warn('No modules data:', data);
                window.showToast('⚠️ 모듈 목록이 비어있습니다', 'warning', 3000);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            window.showToast(`❌ 모듈 검색 실패: ${error.message}. 데몬을 확인해주세요.`, 'error', 5000);
            setModal({ type: 'failure', title: '모듈 로드 예외', message: error.message });
        }
    };

    const fetchServers = async () => {
        try {
            // 재시도 로직 적용
            const data = await retryWithBackoff(
                () => window.api.serverList(),
                3,
                800
            );
            if (data && data.servers) {
                setServers(data.servers);
            } else {
                setServers([]);
            }
        } catch (error) {
            console.error('Failed to fetch servers:', error);
            window.showToast(`⚠️ 서버 목록 업데이트 실패: ${error.message}`, 'warning', 3000);
            setServers([]);
        } finally {
            setLoading(false);
        }
    };

    const handleStart = async (name, module) => {
        try {
            const result = await window.api.serverStart(name, { module });
            if (result.error) {
                setModal({ type: 'failure', title: '서버 시작 실패', message: result.error });
            } else {
                setModal({ type: 'notification', title: '서버 시작 중', message: `${name} 서버가 시작되고 있습니다...` });
            }
            fetchServers();
        } catch (error) {
            setModal({ type: 'failure', title: '서버 시작 예외', message: error.message });
        }
    };

    const handleStop = async (name) => {
        setModal({
            type: 'question',
            title: '서버 정지',
            message: `${name} 서버를 정지하시겠습니까?`,
            onConfirm: async () => {
                setModal(null);
                try {
                    const result = await window.api.serverStop(name, { force: false });
                    if (result.error) {
                        setModal({ type: 'failure', title: '서버 정지 실패', message: result.error });
                    } else {
                        setModal({ type: 'notification', title: '서버 정지 중', message: `${name} 서버가 정지되고 있습니다...` });
                    }
                    fetchServers();
                } catch (error) {
                    setModal({ type: 'failure', title: '서버 정지 예외', message: error.message });
                }
            },
            onCancel: () => setModal(null)
        });
    };

    const handleStatus = async (name) => {
        try {
            const result = await window.api.serverStatus(name);
            const statusInfo = `Status: ${result.status}\nPID: ${result.pid || 'N/A'}\nUptime: ${result.uptime_seconds ? Math.floor(result.uptime_seconds / 60) + 'm' : 'N/A'}`;
            setModal({ type: 'notification', title: name, message: statusInfo });
        } catch (error) {
            setModal({ type: 'failure', title: '상태 조회 실패', message: error.message });
        }
    };

    const handleAddServer = async () => {
        if (!newServerName.trim()) {
            setModal({ type: 'failure', title: '입력 오류', message: '서버 이름을 입력하세요' });
            return;
        }
        if (!selectedModule) {
            setModal({ type: 'failure', title: '입력 오류', message: '모듈을 선택하세요' });
            return;
        }

        try {
            // 선택된 모듈의 기본 executable_path 가져오기
            const selectedModuleData = modules.find(m => m.name === selectedModule);
            
            const instanceData = {
                name: newServerName.trim(),
                module_name: selectedModule,
                executable_path: selectedModuleData?.executable_path || null
            };

            console.log('Adding instance:', instanceData);
            const result = await window.api.instanceCreate(instanceData);
            
            if (result.error) {
                setModal({ type: 'failure', title: '인스턴스 추가 실패', message: result.error });
            } else {
                setModal({ type: 'success', title: '성공', message: `인스턴스 "${newServerName}" 추가되었습니다` });
                // 폼 초기화
                setNewServerName('');
                setSelectedModule('');
                setShowModuleManager(false);
                fetchServers();
            }
        } catch (error) {
            setModal({ type: 'failure', title: '인스턴스 추가 예외', message: error.message });
        }
    };

    const handleDeleteServer = async (server) => {
        // Question 모달 표시
        setModal({
            type: 'question',
            title: '서버 삭제 확인',
            message: `정말로 "${server.name}" 서버를 삭제하시겠습니까?\n\n이 작업은 되돌릴 수 없습니다.`,
            onConfirm: () => performDeleteServer(server),
        });
    };

    const performDeleteServer = async (server) => {
        setModal(null); // 질문 모달 닫기

        try {
            const result = await window.api.instanceDelete(server.id);
            
            if (result.error) {
                setModal({ type: 'failure', title: '인스턴스 삭제 실패', message: result.error });
            } else {
                console.log(`Instance "${server.name}" (ID: ${server.id}) deleted`);
                setModal({ type: 'success', title: '성공', message: `"${server.name}" 서버가 삭제되었습니다` });
                fetchServers(); // 새로고침
            }
        } catch (error) {
            setModal({ type: 'failure', title: '인스턴스 삭제 예외', message: error.message });
        }
    };

    const handleOpenSettings = (server) => {
        setSettingsServer(server);
        // 선택된 모듈의 settings schema 찾기
        const module = modules.find(m => m.name === server.module);
        if (module && module.settings && module.settings.fields) {
            // 초기값 설정: instances.json에서 저장된 값 우선, 없으면 default
            const initial = {};
            module.settings.fields.forEach(field => {
                let value = '';
                
                // 1. instances.json에서 이미 저장된 값이 있는지 확인
                if (server[field.name] !== undefined && server[field.name] !== null) {
                    value = String(server[field.name]);
                    console.log(`Loaded ${field.name} from instance:`, value);
                }
                // 2. 없으면 module.toml의 default 값 사용
                else if (field.default !== undefined && field.default !== null) {
                    value = String(field.default);
                    console.log(`Using default for ${field.name}:`, value);
                }
                
                initial[field.name] = value;
            });
            console.log('Initialized settings values:', initial);
            setSettingsValues(initial);
        } else {
            setSettingsValues({});
        }
        
        // 별칭 로드 (settingsServer.module 사용)
        const moduleName = server.module;
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
                    description: (data && data.description) || ''
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
            
            console.log('Converted settings:', convertedSettings);
            console.log('Calling instanceUpdateSettings with id:', settingsServer.id);
            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            console.log('API Response:', result);
            
            if (result.error) {
                setModal({ type: 'failure', title: '설정 저장 실패', message: result.error });
                console.error('Error response:', result.error);
            } else {
                setModal({ type: 'success', title: '성공', message: `"${settingsServer.name}" 설정이 저장되었습니다` });
                setShowSettingsModal(false);
                fetchServers(); // 새로고침
            }
        } catch (error) {
            console.error('Exception in handleSaveSettings:', error);
            setModal({ type: 'failure', title: '설정 저장 예외', message: error.message });
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
                setModal({ type: 'failure', title: '별명 저장 실패', message: res.error });
            } else {
                // API에서 저장된 설정을 다시 로드
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: '저장됨', message: '별명이 저장되었습니다.' });
            }
        } catch (error) {
            console.error('Failed to save aliases:', error);
            setModal({ type: 'failure', title: '별명 저장 예외', message: error.message });
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
                    clearedCmds[cmd] = { aliases: [], description: data.description || '' };
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
                setModal({ type: 'failure', title: '초기화 실패', message: res.error });
            } else {
                // API에서 저장된 설정을 다시 로드
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: '초기화 완료', message: '별명이 기본값으로 초기화되었습니다.' });
            }
        } catch (error) {
            console.error('Failed to reset aliases:', error);
            setModal({ type: 'failure', title: '초기화 예외', message: error.message });
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
                setModal({ type: 'failure', title: '별명 저장 실패', message: res.error });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: '저장됨', message: '별명이 저장되었습니다.' });
            }
        } catch (error) {
            console.error('Failed to save aliases:', error);
            setModal({ type: 'failure', title: '별명 저장 예외', message: error.message });
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
                    clearedCmds[cmd] = { aliases: [], description: data.description || '' };
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
                setModal({ type: 'failure', title: '초기화 실패', message: res.error });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({ type: 'success', title: '초기화 완료', message: '별명이 기본값으로 초기화되었습니다.' });
            }
        } catch (error) {
            console.error('Failed to reset aliases:', error);
            setModal({ type: 'failure', title: '초기화 예외', message: error.message });
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

    if (loading && servers.length === 0) {
        return (
            <div className="App">
                <div className="loading">
                    <h2>Loading servers...</h2>
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
                        <div className="app-logo">🌌</div>
                        <h1>Saba-chan</h1>
                    </div>
                    <button 
                        className="btn btn-settings-icon-solo"
                        onClick={() => setShowGuiSettingsModal(true)}
                        title="GUI 설정"
                    >
                        ⚙️
                    </button>
                </div>
                
                {/* 두 번째 줄: 기능 버튼들 */}
                <div className="header-row header-row-controls">
                    <button 
                        className="btn btn-add"
                        onClick={() => setShowModuleManager(!showModuleManager)}
                    >
                        ➕ Add Server
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
                            className="btn btn-background btn-background-active"
                            onClick={() => setShowBackgroundSection(!showBackgroundSection)}
                        >
                            <span className="status-indicator status-online"></span>
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

            {showModuleManager && (
                <div className="module-manager">
                    <h3>Add New Server</h3>
                    
                    <div className="path-config">
                        <label>Modules Directory:</label>
                        <input 
                            type="text"
                            className="path-input"
                            value={modulesPath}
                            onChange={(e) => setModulesPath(e.target.value)}
                            placeholder="c:\Git\Bot\modules"
                        />
                        <button className="btn btn-refresh-modules" onClick={fetchModules}>
                            🔄 Reload Modules
                        </button>
                        <small className="path-hint">
                            📁 Place .zip files or folders with module.toml here
                        </small>
                        {settingsPath && (
                            <small className="settings-path">
                                💾 Settings: {settingsPath}
                            </small>
                        )}
                    </div>
                    
                    <div className="add-server-form">
                        <div className="form-row">
                            <label>Server Name *</label>
                            <input 
                                type="text"
                                placeholder="e.g., my-palworld-1"
                                value={newServerName}
                                onChange={(e) => setNewServerName(e.target.value)}
                            />
                        </div>

                        <div className="form-row">
                            <label>Game Module *</label>
                            <select 
                                value={selectedModule}
                                onChange={(e) => handleModuleSelect(e.target.value)}
                            >
                                <option value="">Select Module</option>
                                {modules.map(m => (
                                    <option key={m.name} value={m.name}>
                                        {m.name} v{m.version}
                                    </option>
                                ))}
                            </select>
                        </div>

                        <div className="form-actions">
                            <button className="btn btn-confirm" onClick={handleAddServer}>
                                ✅ Add Server
                            </button>
                            <button className="btn btn-cancel" onClick={() => setShowModuleManager(false)}>
                                ❌ Cancel
                            </button>
                        </div>
                    </div>
                    
                    <div className="module-list">
                        <h4>Available Modules:</h4>
                        {modules.map(module => (
                            <div key={module.name} className="module-item">
                                <strong>{module.name}</strong> v{module.version}
                                <p>{module.description || 'No description'}</p>
                                <small>{module.path}</small>
                            </div>
                        ))}
                    </div>
                </div>
            )}

            <div className="server-list">
                {servers.length === 0 ? (
                    <div className="no-servers">
                        <p>No servers configured</p>
                    </div>
                ) : (
                    servers.map((server) => (
                        <div key={server.name} className="server-card">
                            <div className="server-header">
                                <div className="server-info">
                                    <h2>{server.name}</h2>
                                    <p className="module-label">Module: {server.module}</p>
                                </div>
                                <div 
                                    className="status-badge"
                                    style={{ backgroundColor: getStatusColor(server.status) }}
                                    title={server.status}
                                >
                                    <span className="status-icon">{getStatusIcon(server.status)}</span>
                                    <span className="status-text">{server.status}</span>
                                </div>
                            </div>

                            <div className="server-details">
                                {server.pid && (
                                    <div className="detail-row">
                                        <span className="label">PID:</span>
                                        <span className="value">{server.pid}</span>
                                    </div>
                                )}
                                {server.resource && (
                                    <>
                                        <div className="detail-row">
                                            <span className="label">RAM:</span>
                                            <span className="value">{server.resource.ram || 'N/A'}</span>
                                        </div>
                                        <div className="detail-row">
                                            <span className="label">CPU Cores:</span>
                                            <span className="value">{server.resource.cpu || 'N/A'}</span>
                                        </div>
                                    </>
                                )}
                            </div>

                            <div className="button-group">
                                <button 
                                    className={`btn ${
                                        server.status === 'running' || server.status === 'starting'
                                            ? 'btn-stop' 
                                            : 'btn-start'
                                    }`}
                                    onClick={() => {
                                        if (server.status === 'running' || server.status === 'starting') {
                                            handleStop(server.name);
                                        } else {
                                            handleStart(server.name, server.module);
                                        }
                                    }}
                                    disabled={server.status === 'starting' || server.status === 'stopping'}
                                >
                                    {server.status === 'running' || server.status === 'starting' ? '⏹ Stop' : '▶ Start'}
                                </button>
                                <button 
                                    className="btn btn-status"
                                    onClick={() => handleStatus(server.name)}
                                >
                                    ℹ Info
                                </button>
                                <button 
                                    className="btn btn-settings"
                                    onClick={() => handleOpenSettings(server)}
                                    title="Edit server settings"
                                >
                                    ⚙️ Settings
                                </button>
                                <button 
                                    className="btn btn-command"
                                    onClick={() => {
                                        setCommandServer(server);
                                        setShowCommandModal(true);
                                    }}
                                    disabled={server.status !== 'running'}
                                    title="Execute server command (server must be running)"
                                >
                                    💻 Command
                                </button>
                                <button 
                                    className="btn btn-delete"
                                    onClick={() => handleDeleteServer(server)}
                                    disabled={server.status === 'running' || server.status === 'starting'}
                                    title="Delete this server instance"
                                >
                                    🗑️ Delete
                                </button>
                            </div>
                        </div>
                    ))
                )}
            </div>

            {showSettingsModal && settingsServer && (
                <div className="modal-overlay">
                    <div className="modal-content modal-content-large">
                        <div className="modal-header">
                            <h3>⚙️ {settingsServer.name} - Settings</h3>
                            <button className="modal-close" onClick={() => setShowSettingsModal(false)}>✕</button>
                        </div>
                        
                        {/* 탭 헤더 */}
                        <div className="settings-tabs">
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'general' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('general')}
                            >
                                🎮 일반 설정
                            </button>
                            <button 
                                className={`settings-tab ${settingsActiveTab === 'aliases' ? 'active' : ''}`}
                                onClick={() => setSettingsActiveTab('aliases')}
                            >
                                💬 Discord 별명
                            </button>
                        </div>
                        
                        <div className="modal-body">
                            {/* 일반 설정 탭 */}
                            {settingsActiveTab === 'general' && (() => {
                                const module = modules.find(m => m.name === settingsServer.module);
                                if (!module || !module.settings) {
                                    return <p className="no-settings">This module has no configurable settings.</p>;
                                }
                                return (
                                    <div className="settings-form">
                                        {module.settings.fields.map((field) => (
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
                                        ))}
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
                                                return (
                                                    <div key={cmd} className="command-alias-editor">
                                                        <div className="cmd-header">
                                                            <span className="cmd-name">{cmd}</span>
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
