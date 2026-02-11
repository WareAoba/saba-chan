import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import Icon from '../Icon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
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
    const { t } = useTranslation('gui');
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
                    <h3 style={{ fontSize: '1.3rem' }}>{t('add_server_modal.title')}</h3>
                </div>

                <div className="modal-body">
                    <div className="path-config">
                        <label>{t('add_server_modal.modules_directory')}</label>
                        <input 
                            type="text"
                            className="path-input"
                            value={modulesPath}
                            onChange={(e) => onModulesPathChange(e.target.value)}
                            placeholder="c:\Git\Bot\modules"
                        />
                        <button className="btn btn-refresh-modules" onClick={onRefreshModules}>
                            <Icon name="refresh" size="sm" /> {t('add_server_modal.reload_modules')}
                        </button>
                        <small className="path-hint">
                            <Icon name="folder" size="sm" /> {t('add_server_modal.place_modules_hint')}
                        </small>
                        {settingsPath && (
                            <small className="settings-path">
                                <Icon name="database" size="sm" /> {t('add_server_modal.settings_path')} {settingsPath}
                            </small>
                        )}
                    </div>

                    <div className="add-server-form">
                        <div className="form-row">
                            <label>{t('add_server_modal.server_name')}</label>
                            <input 
                                type="text"
                                placeholder={t('add_server_modal.server_name_placeholder')}
                                value={newServerName}
                                onChange={(e) => setNewServerName(e.target.value)}
                            />
                        </div>

                        <div className="form-row">
                            <label>{t('add_server_modal.game_module')}</label>
                            <CustomDropdown
                                value={selectedModule}
                                onChange={(val) => handleModuleSelect(val)}
                                placeholder={t('add_server_modal.select_module')}
                                options={modules.map(m => ({ value: m.name, label: `${t(`mod_${m.name}:module.display_name`, { defaultValue: m.name })} v${m.version}` }))}
                            />
                        </div>
                    </div>

                    <div className="module-list">
                        <h4>{t('add_server_modal.available_modules')}</h4>
                        {modules.length === 0 ? (
                            <p className="no-modules">{t('add_server_modal.no_modules')}</p>
                        ) : (
                            modules.map(module => (
                                <div key={module.name} className="module-item">
                                    <strong>{t(`mod_${module.name}:module.display_name`, { defaultValue: module.name })}</strong> v{module.version}
                                    <p>{t(`mod_${module.name}:module.description`, { defaultValue: module.description || t('add_server_modal.no_description') })}</p>
                                    <small>{module.path}</small>
                                </div>
                            ))
                        )}
                    </div>
                </div>

                <div className="modal-footer">
                    <button className="btn btn-confirm" onClick={handleSubmit}>
                        <Icon name="checkCircle" size="sm" /> {t('add_server_modal.add_server')}
                    </button>
                    <button className="btn btn-cancel" onClick={onClose}>
                        <Icon name="xCircle" size="sm" /> {t('modals.cancel')}
                    </button>
                </div>
            </div>
        </div>
    );
}
