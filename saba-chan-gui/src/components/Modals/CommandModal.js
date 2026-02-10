import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';

function CommandModal({ server, modules, onClose, onExecute }) {
    const { t } = useTranslation('gui');
    const [commandInput, setCommandInput] = useState('');
    const [commandInputs, setCommandInputs] = useState({});
    const [loading, setLoading] = useState(false);
    const [suggestions, setSuggestions] = useState([]);

    // 현재 모듈의 명령어 목록 가져오기
    const currentModule = modules.find(m => m.name === server.module);
    const commands = currentModule?.commands?.fields || [];

    // 입력값 변경 시 자동완성 제안
    useEffect(() => {
        if (commandInput.trim()) {
            const matching = commands.filter(cmd => cmd.name.startsWith(commandInput.trim()));
            setSuggestions(matching);
        } else {
            setSuggestions([]);
        }
    }, [commandInput, commands]);

    // 명령어 선택 시 입력 필드 초기화
    useEffect(() => {
        const cmd = commands.find(c => c.name === commandInput.trim());
        if (cmd && cmd.inputs) {
            const initialInputs = {};
            cmd.inputs.forEach(input => {
                initialInputs[input.name] = input.default || '';
            });
            setCommandInputs(initialInputs);
        }
    }, [commandInput, commands]);

    // 입력 값 변경 처리
    const handleInputChange = (inputName, value) => {
        setCommandInputs(prev => ({
            ...prev,
            [inputName]: value
        }));
    };

    // 명령어 실행
    const handleExecuteCommand = async () => {
        const cmdName = commandInput.trim();
        if (!cmdName) {
            onExecute({ type: 'failure', title: t('command_modal.input_error'), message: t('command_modal.enter_command') });
            return;
        }

        // 서버 실행 상태 확인
        if (server.status !== 'running') {
            onExecute({ 
                type: 'failure', 
                title: t('command_modal.server_not_running_title'), 
                message: t('command_modal.server_not_running_message', { name: server.name, status: server.status })
            });
            return;
        }

        // 선택된 command 객체 찾기
        const selectedCommand = commands.find(c => c.name === cmdName);
        
        // 디버깅 로그
        console.log(`[CommandModal] cmdName: ${cmdName}`);
        console.log(`[CommandModal] Available commands:`, commands);
        console.log(`[CommandModal] Selected command:`, selectedCommand);

        // 필수 필드 검증 (selectedCommand가 있으면)
        if (selectedCommand && selectedCommand.inputs && selectedCommand.inputs.length > 0) {
            for (const field of selectedCommand.inputs) {
                const value = commandInputs[field.name];
                if (field.required && (!value || value === '')) {
                    onExecute({ 
                        type: 'failure', 
                        title: t('command_modal.input_error'), 
                        message: t('command_modal.missing_required_field', { field: field.label })
                    });
                    return;
                }
            }
        }

        setLoading(true);

        try {
            // 선택된 command 객체 전체를 전달 (http_method, inputs 포함)
            const result = await window.api.executeCommand(server.id, {
                command: cmdName,
                args: commandInputs,
                commandMetadata: selectedCommand  // 모듈에서 정의한 명령 메타데이터 (없을 수도 있음)
            });

            if (result.error) {
                onExecute({ type: 'failure', title: t('command_modal.execution_failed'), message: result.error });
            } else {
                onExecute({ type: 'success', title: t('command_modal.success'), message: result.message || t('command_modal.command_executed', { command: cmdName }) });
                onClose();
            }
        } catch (error) {
            onExecute({ type: 'failure', title: t('command_modal.execution_error'), message: error.message });
        } finally {
            setLoading(false);
        }
    };

    const selectedCmd = commands.find(c => c.name === commandInput.trim());

    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal command-modal" onClick={e => e.stopPropagation()}>
                <h2 className="modal-title">{t('command_modal.title', { name: server.name })}</h2>

                {/* CLI 입력 라인 */}
                <div className="cli-section">
                    <label className="cli-label">{t('command_modal.command_label')}</label>
                    <div className="cli-input-wrapper">
                        <span className="cli-prompt">$</span>
                        <input
                            type="text"
                            className="cli-input"
                            value={commandInput}
                            onChange={e => setCommandInput(e.target.value)}
                            onKeyPress={e => {
                                if (e.key === 'Enter') {
                                    handleExecuteCommand();
                                }
                            }}
                            placeholder={t('command_modal.command_placeholder')}
                            autoFocus
                        />
                    </div>

                    {/* 자동완성 제안 */}
                    {suggestions.length > 0 && (
                        <div className="suggestions-list">
                            {suggestions.map(cmd => (
                                <div
                                    key={cmd.name}
                                    className="suggestion-item"
                                    onClick={() => setCommandInput(cmd.name)}
                                    title={cmd.description}
                                >
                                    <span className="suggestion-name">{cmd.name}</span>
                                    <span className="suggestion-desc">{cmd.description}</span>
                                </div>
                            ))}
                        </div>
                    )}
                </div>

                {/* 명령어 설명 */}
                {selectedCmd && (
                    <div className="command-info">
                        <p className="command-description">📌 {selectedCmd.description}</p>
                    </div>
                )}

                {/* 입력 필드 */}
                {selectedCmd && selectedCmd.inputs && selectedCmd.inputs.length > 0 && (
                    <div className="command-inputs">
                        {selectedCmd.inputs.map(input => (
                            <div key={input.name} className="input-group">
                                <label className="input-label">
                                    {input.label}
                                    {input.required && <span className="required">*</span>}
                                </label>
                                {input.type === 'text' && (
                                    <input
                                        type="text"
                                        className="command-input"
                                        value={commandInputs[input.name] || ''}
                                        onChange={e => handleInputChange(input.name, e.target.value)}
                                        placeholder={input.placeholder || ''}
                                        required={input.required}
                                    />
                                )}
                                {input.type === 'number' && (
                                    <input
                                        type="number"
                                        className="command-input"
                                        value={commandInputs[input.name] || ''}
                                        onChange={e => handleInputChange(input.name, e.target.value)}
                                        min={input.min}
                                        max={input.max}
                                        required={input.required}
                                    />
                                )}
                                {input.type === 'password' && (
                                    <input
                                        type="password"
                                        className="command-input"
                                        value={commandInputs[input.name] || ''}
                                        onChange={e => handleInputChange(input.name, e.target.value)}
                                        required={input.required}
                                    />
                                )}
                                {input.type === 'select' && (
                                    <select
                                        className="command-input"
                                        value={commandInputs[input.name] || ''}
                                        onChange={e => handleInputChange(input.name, e.target.value)}
                                        required={input.required}
                                    >
                                        <option value="">-- 선택하세요 --</option>
                                        {input.options && input.options.map(opt => (
                                            <option key={opt} value={opt}>
                                                {opt}
                                            </option>
                                        ))}
                                    </select>
                                )}
                            </div>
                        ))}
                    </div>
                )}

                {/* 버튼 */}
                <div className="modal-buttons-group">
                    <button
                        className="modal-button command-execute"
                        onClick={handleExecuteCommand}
                        disabled={!commandInput.trim() || loading}
                    >
                        {loading ? '...' : `⏎ ${t('command_modal.execute')}`}
                    </button>
                    <button className="modal-button command-cancel" onClick={onClose}>
                        ✕ {t('modals.close')}
                    </button>
                </div>
            </div>
        </div>
    );
}

export default CommandModal;
