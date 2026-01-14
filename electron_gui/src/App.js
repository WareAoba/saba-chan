import React, { useState, useEffect } from 'react';
import './App.css';

function App() {
    const [servers, setServers] = useState([]);
    const [modules, setModules] = useState([]);
    const [loading, setLoading] = useState(true);
    const [autoRefresh, setAutoRefresh] = useState(true);
    const [refreshInterval, setRefreshInterval] = useState(2000); // 2초마다 업데이트
    const [showModuleManager, setShowModuleManager] = useState(false);
    const [newServerName, setNewServerName] = useState('');
    const [selectedModule, setSelectedModule] = useState('');
    const [modulesPath, setModulesPath] = useState(''); // 설정에서 로드
    const [settingsPath, setSettingsPath] = useState('');

    // 설정 로드
    useEffect(() => {
        const loadSettings = async () => {
            try {
                const settings = await window.api.settingsLoad();
                if (settings) {
                    setAutoRefresh(settings.autoRefresh ?? true);
                    setRefreshInterval(settings.refreshInterval ?? 2000);
                    setModulesPath(settings.modulesPath || '');
                }
                const path = await window.api.settingsGetPath();
                setSettingsPath(path);
                console.log('Settings loaded from:', path);
            } catch (error) {
                console.error('Failed to load settings:', error);
            }
        };
        loadSettings();
    }, []);

    // 설정 저장 함수
    const saveCurrentSettings = async () => {
        try {
            await window.api.settingsSave({
                autoRefresh,
                refreshInterval,
                modulesPath
            });
            console.log('Settings saved');
        } catch (error) {
            console.error('Failed to save settings:', error);
        }
    };

    // autoRefresh 또는 refreshInterval 변경 시 저장
    useEffect(() => {
        if (settingsPath) { // 초기 로드 이후에만 저장
            saveCurrentSettings();
        }
    }, [autoRefresh, refreshInterval]);

    useEffect(() => {
        console.log('App mounted, fetching initial data...');
        fetchServers();
        fetchModules();
        
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
            const data = await window.api.moduleList();
            console.log('Module data received:', data);
            if (data && data.modules) {
                console.log('Setting modules:', data.modules.length, 'modules');
                setModules(data.modules);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                alert('Failed to load modules: ' + data.error);
            } else {
                console.warn('No modules data:', data);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            alert('Exception fetching modules: ' + error.message);
        }
    };

    const fetchServers = async () => {
        try {
            const data = await window.api.serverList();
            if (data && data.servers) {
                setServers(data.servers);
            } else {
                setServers([]);
            }
        } catch (error) {
            console.error('Failed to fetch servers:', error);
            setServers([]);
        } finally {
            setLoading(false);
        }
    };

    const handleStart = async (name, module) => {
        try {
            const result = await window.api.serverStart(name, { module });
            if (result.error) {
                alert(`Error starting server: ${result.error}`);
            } else {
                alert(`Server ${name} is starting...`);
            }
            fetchServers();
        } catch (error) {
            alert(`Failed to start server: ${error.message}`);
        }
    };

    const handleStop = async (name) => {
        if (window.confirm(`Are you sure you want to stop ${name}?`)) {
            try {
                const result = await window.api.serverStop(name, { force: false });
                if (result.error) {
                    alert(`Error stopping server: ${result.error}`);
                } else {
                    alert(`Server ${name} is stopping...`);
                }
                fetchServers();
            } catch (error) {
                alert(`Failed to stop server: ${error.message}`);
            }
        }
    };

    const handleStatus = async (name) => {
        try {
            const result = await window.api.serverStatus(name);
            const statusInfo = `Status: ${result.status}\nPID: ${result.pid || 'N/A'}\nUptime: ${result.uptime_seconds ? Math.floor(result.uptime_seconds / 60) + 'm' : 'N/A'}`;
            alert(`${name}\n${statusInfo}`);
        } catch (error) {
            alert(`Failed to get status: ${error.message}`);
        }
    };

    const handleAddServer = async () => {
        if (!newServerName.trim()) {
            alert('Please enter a server name');
            return;
        }
        if (!selectedModule) {
            alert('Please select a module');
            return;
        }

        try {
            console.log('Adding instance:', newServerName, selectedModule);
            const result = await window.api.instanceCreate({
                name: newServerName.trim(),
                module_name: selectedModule
            });
            
            if (result.error) {
                alert('Failed to add instance: ' + result.error);
            } else {
                alert(`Instance "${newServerName}" added successfully!`);
                setNewServerName('');
                setSelectedModule('');
                setShowModuleManager(false);
                fetchServers(); // 새로고침
            }
        } catch (error) {
            alert(`Failed to add instance: ${error.message}`);
        }
    };

    // 모듈 선택 시 자동으로 서버 이름 생성
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
            <header className="app-header">
                <h1>🎮 Game Server Manager</h1>
                <div className="header-controls">
                    <button 
                        className="btn btn-add"
                        onClick={() => setShowModuleManager(!showModuleManager)}
                    >
                        ➕ Add Server
                    </button>
                    <label>
                        <input 
                            type="checkbox" 
                            checked={autoRefresh}
                            onChange={(e) => setAutoRefresh(e.target.checked)}
                        />
                        Auto Refresh
                    </label>
                    <select 
                        value={refreshInterval}
                        onChange={(e) => setRefreshInterval(Number(e.target.value))}
                        disabled={!autoRefresh}
                    >
                        <option value={1000}>1s</option>
                        <option value={2000}>2s</option>
                        <option value={5000}>5s</option>
                        <option value={10000}>10s</option>
                    </select>
                    <button className="refresh-btn" onClick={fetchServers}>
                        🔄 Refresh Now
                    </button>
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
                        <input 
                            type="text"
                            placeholder="Server Name (e.g., minecraft-main)"
                            value={newServerName}
                            onChange={(e) => setNewServerName(e.target.value)}
                        />
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
                        <button className="btn btn-confirm" onClick={handleAddServer}>
                            Add Instance
                        </button>
                        <button className="btn btn-cancel" onClick={() => setShowModuleManager(false)}>
                            Cancel
                        </button>
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
                                    className="btn btn-start"
                                    onClick={() => handleStart(server.name, server.module)}
                                    disabled={server.status === 'running' || server.status === 'starting'}
                                >
                                    ▶ Start
                                </button>
                                <button 
                                    className="btn btn-stop"
                                    onClick={() => handleStop(server.name)}
                                    disabled={server.status === 'stopped' || server.status === 'stopping'}
                                >
                                    ⏹ Stop
                                </button>
                                <button 
                                    className="btn btn-status"
                                    onClick={() => handleStatus(server.name)}
                                >
                                    ℹ Info
                                </button>
                            </div>
                        </div>
                    ))
                )}
            </div>

            <footer className="app-footer">
                <p>Connected to Core Daemon at localhost:57474</p>
            </footer>
        </div>
    );
}

export default App;
