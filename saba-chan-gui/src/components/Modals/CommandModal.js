import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useModalClose } from '../../hooks/useModalClose';
import { Icon } from '../Icon';

function CommandModal({ server, modules, onClose, onExecute }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);
    const [commandInput, setCommandInput] = useState('');
    const [loading, setLoading] = useState(false);
    const [history, setHistory] = useState([]); // { id, command, status: 'success'|'failure', message, time }
    const [highlightIdx, setHighlightIdx] = useState(-1);
    const inputRef = useRef(null);
    const historyEndRef = useRef(null);

    // 모듈 명령어 목록
    const currentModule = modules.find((m) => m.name === server.module);
    const commands = currentModule?.commands?.fields || [];

    // 입력값 기준 필터링된 힌트
    const filteredHints = commandInput.trim()
        ? commands.filter((cmd) => cmd.name.toLowerCase().startsWith(commandInput.trim().toLowerCase()))
        : commands;

    // 히스토리 추가 시 자동 스크롤
    useEffect(() => {
        historyEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [history]);

    // Tab 자동완성 & 방향키 힌트 탐색
    const handleKeyDown = (e) => {
        if (e.key === 'Tab') {
            e.preventDefault();
            if (filteredHints.length > 0) {
                const idx = highlightIdx >= 0 && highlightIdx < filteredHints.length ? highlightIdx : 0;
                setCommandInput(filteredHints[idx].name);
                setHighlightIdx(-1);
            }
        } else if (e.key === 'ArrowDown') {
            e.preventDefault();
            setHighlightIdx((prev) => (prev + 1) % (filteredHints.length || 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setHighlightIdx((prev) => (prev <= 0 ? filteredHints.length - 1 : prev - 1));
        } else if (e.key === 'Enter') {
            handleExecuteCommand();
        }
    };

    // 입력 변경 시 첫 번째 매칭 항목을 자동 하이라이트
    useEffect(() => {
        setHighlightIdx(commandInput.trim() && filteredHints.length > 0 ? 0 : -1);
    }, [commandInput, filteredHints.length]);

    // 명령어 실행
    const handleExecuteCommand = async () => {
        const cmdText = commandInput.trim();
        if (!cmdText || loading) return;

        if (server.status !== 'running') {
            const entry = {
                id: Date.now(),
                command: cmdText,
                status: 'failure',
                message: t('command_modal.server_not_running_message', { name: server.name, status: server.status }),
                time: new Date().toLocaleTimeString(),
            };
            setHistory((prev) => [...prev, entry]);
            return;
        }

        // 모듈에 정의된 명령어인지 확인
        const selectedCommand = commands.find((c) => c.name === cmdText);

        setLoading(true);
        try {
            const result = await window.api.executeCommand(server.id, {
                command: cmdText,
                args: {},
                commandMetadata: selectedCommand || { method: 'rcon' },
            });

            const entry = {
                id: Date.now(),
                command: cmdText,
                status: result.error ? 'failure' : 'success',
                message: result.error || result.message || result?.data?.response || t('command_modal.command_executed', { command: cmdText }),
                time: new Date().toLocaleTimeString(),
            };
            setHistory((prev) => [...prev, entry]);
        } catch (error) {
            const entry = {
                id: Date.now(),
                command: cmdText,
                status: 'failure',
                message: error.message,
                time: new Date().toLocaleTimeString(),
            };
            setHistory((prev) => [...prev, entry]);
        } finally {
            setLoading(false);
            setCommandInput('');
            inputRef.current?.focus();
        }
    };

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal command-modal-fixed" onClick={(e) => e.stopPropagation()}>
                {/* 닫기 버튼 (우상단) */}
                <button className="cmd-modal-close" onClick={requestClose} title={t('modals.close')}>
                    &times;
                </button>

                {/* ── 좌측: 입력 + 힌트 ── */}
                <div className="command-modal-main">
                    <h2 className="modal-title">{t('command_modal.title', { name: server.name })}</h2>

                    {/* 힌트 영역 (고정 높이) */}
                    <div className="cmd-hints-inline">
                        {filteredHints.length > 0 ? (
                            filteredHints.map((cmd, idx) => (
                                <div
                                    key={cmd.name}
                                    className={clsx('cmd-hint-chip', { 'cmd-hint-active': idx === highlightIdx })}
                                    onClick={() => {
                                        setCommandInput(cmd.name);
                                        inputRef.current?.focus();
                                    }}
                                    title={cmd.description}
                                >
                                    <span className="cmd-hint-name">{cmd.name}</span>
                                    {cmd.description && <span className="cmd-hint-desc">{cmd.description}</span>}
                                </div>
                            ))
                        ) : (
                            <span className="cmd-hints-empty">
                                {commandInput.trim()
                                    ? t('command_modal.no_matching_hints', { defaultValue: 'No matching commands — will send as raw command' })
                                    : t('command_modal.type_to_filter', { defaultValue: 'Type a command or select from the list' })}
                            </span>
                        )}
                    </div>

                    {/* 입력 라인 (항상 하단 고정) */}
                    <div className="cmd-input-fixed">
                        <div className="cli-input-wrapper">
                            <span className="cli-prompt">$</span>
                            <input
                                ref={inputRef}
                                type="text"
                                className="cli-input"
                                value={commandInput}
                                onChange={(e) => setCommandInput(e.target.value)}
                                onKeyDown={handleKeyDown}
                                placeholder={t('command_modal.command_placeholder')}
                                autoFocus
                                disabled={loading}
                            />
                            <button
                                className="console-send"
                                onClick={handleExecuteCommand}
                                disabled={!commandInput.trim() || loading}
                                title={t('command_modal.execute')}
                            >
                                {loading ? '…' : t('console.send')}
                            </button>
                        </div>
                        <div className="cmd-input-meta">
                            <span className="cmd-tab-hint">Tab ↹ {t('command_modal.autocomplete', { defaultValue: 'autocomplete' })}</span>
                        </div>
                    </div>
                </div>

                {/* ── 우측: 실행 히스토리 ── */}
                <div className="command-history-panel">
                    <div className="hints-panel-header">
                        <Icon name="list" size="sm" />
                        {t('command_modal.history', { defaultValue: 'History' })}
                    </div>
                    <div className="cmd-history-list">
                        {history.length === 0 && (
                            <div className="hints-empty">
                                {t('command_modal.no_history', { defaultValue: 'No commands executed yet' })}
                            </div>
                        )}
                        {history.map((entry) => (
                            <div
                                key={entry.id}
                                className={clsx('cmd-history-item', `cmd-history-${entry.status}`)}
                                onClick={() => {
                                    setCommandInput(entry.command);
                                    inputRef.current?.focus();
                                }}
                                title={entry.message}
                            >
                                <div className="cmd-history-top">
                                    <span className={clsx('cmd-history-dot', `dot-${entry.status}`)} />
                                    <span className="cmd-history-cmd">{entry.command}</span>
                                    <span className="cmd-history-time">{entry.time}</span>
                                </div>
                                {entry.message && (
                                    <div className="cmd-history-msg">{entry.message}</div>
                                )}
                            </div>
                        ))}
                        <div ref={historyEndRef} />
                    </div>
                </div>
            </div>
        </div>
    );
}

export default CommandModal;
