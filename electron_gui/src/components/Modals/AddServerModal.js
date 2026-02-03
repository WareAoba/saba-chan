import React, { useState, useEffect } from 'react';
import Icon from '../Icon';
import './Modals.css';

export function AddServerModal({ 
    isOpen, 
    onClose, 
    modules, 
    servers,
    modulesPath,
    settingsPath,
    onModulesPathChange,
    onRefreshModules,
    onAddServer
}) {
    const [newServerName, setNewServerName] = useState('');
    const [selectedModule, setSelectedModule] = useState('');

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

    const handleSubmit = () => {
        if (!newServerName.trim()) {
            return;
        }
        if (!selectedModule) {
            return;
        }

        onAddServer(newServerName.trim(), selectedModule);
        
        // 폼 초기화
        setNewServerName('');
        setSelectedModule('');
    };

    if (!isOpen) return null;

    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal-content modal-content-large" onClick={e => e.stopPropagation()}>
                <div className="modal-header">
                    <h3><Icon name="plus" size="md" /> Add New Server</h3>
                    <button className="modal-close" onClick={onClose}>✕</button>
                </div>

                <div className="modal-body">
                    <div className="path-config">
                        <label>Modules Directory:</label>
                        <input 
                            type="text"
                            className="path-input"
                            value={modulesPath}
                            onChange={(e) => onModulesPathChange(e.target.value)}
                            placeholder="c:\Git\Bot\modules"
                        />
                        <button className="btn btn-refresh-modules" onClick={onRefreshModules}>
                            <Icon name="refresh" size="sm" /> Reload Modules
                        </button>
                        <small className="path-hint">
                            <Icon name="folder" size="sm" /> Place .zip files or folders with module.toml here
                        </small>
                        {settingsPath && (
                            <small className="settings-path">
                                <Icon name="database" size="sm" /> Settings: {settingsPath}
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
                    </div>

                    <div className="module-list">
                        <h4>Available Modules:</h4>
                        {modules.length === 0 ? (
                            <p className="no-modules">No modules available. Please check the modules directory.</p>
                        ) : (
                            modules.map(module => (
                                <div key={module.name} className="module-item">
                                    <strong>{module.name}</strong> v{module.version}
                                    <p>{module.description || 'No description'}</p>
                                    <small>{module.path}</small>
                                </div>
                            ))
                        )}
                    </div>
                </div>

                <div className="modal-footer">
                    <button className="btn btn-confirm" onClick={handleSubmit}>
                        <Icon name="checkCircle" size="sm" /> Add Server
                    </button>
                    <button className="btn btn-cancel" onClick={onClose}>
                        <Icon name="xCircle" size="sm" /> Cancel
                    </button>
                </div>
            </div>
        </div>
    );
}
