import React, { useState, useEffect } from 'react';
import './Modals.css';

function CommandModal({ server, modules, onClose, onExecute }) {
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
            onExecute({ type: 'failure', title: '입력 오류', message: '명령어를 입력하세요' });
            return;
        }

        setLoading(true);

        try {
            const result = await window.api.executeCommand(server.id, {
                command: cmdName,
                args: commandInputs
            });

            if (result.error) {
                onExecute({ type: 'failure', title: '명령어 실행 실패', message: result.error });
            } else {
                onExecute({ type: 'success', title: '성공', message: result.message || `명령어 '${cmdName}'가 실행되었습니다` });
                onClose();
            }
        } catch (error) {
            onExecute({ type: 'failure', title: '명령어 실행 오류', message: error.message });
        } finally {
            setLoading(false);
        }
    };

    const selectedCmd = commands.find(c => c.name === commandInput.trim());

    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal command-modal" onClick={e => e.stopPropagation()}>
                <h2 className="modal-title">명령어 실행 - {server.name}</h2>

                {/* CLI 입력 라인 */}
                <div className="cli-section">
                    <label className="cli-label">명령어</label>
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
                            placeholder="명령어를 입력하세요 (예: say, broadcast, save...)"
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
                        {loading ? '실행 중...' : '⏎ 실행'}
                    </button>
                    <button className="modal-button command-cancel" onClick={onClose}>
                        ✕ 닫기
                    </button>
                </div>
            </div>
        </div>
    );
}

export default CommandModal;
