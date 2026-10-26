import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useInstanceCommands } from '../../hooks/useInstanceCommands';
import { needsConfirm, withCommandAliases } from '../../utils/commandRisk';
import { createTranslateError } from '../../utils/helpers';
import { Icon } from '../Icon';
import { CliText, commandCli } from '../CliLine';
import { useUIStore } from '../../stores/useUIStore';
import { commandErrorText } from '../../utils/commandReply';
import { commandText } from '../../utils/serverView';

// 서버 상세의 "명령" 탭 (이름은 옛 모달에서 왔다)
function CommandModal({ server, modules }) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);
    const [commandInput, setCommandInput] = useState('');
    const [loading, setLoading] = useState(false);
    const history = useUIStore((s) => s.commandHistoryMap[server.id]) || [];
    const pushHistory = useUIStore((s) => s.pushCommandHistory);
    const [highlightIdx, setHighlightIdx] = useState(-1);
    const [expandedId, setExpandedId] = useState(null);
    const [confirmText, setConfirmText] = useState(null); // 위험 명령 확인 대기 중인 입력
    const inputRef = useRef(null);
    const historyEndRef = useRef(null);

    // 모듈 명령 + 켜진 익스텐션이 주는 명령
    const currentModule = modules.find((m) => m.name === server.module);
    const commands = useInstanceCommands(
        server,
        withCommandAliases(currentModule?.commands?.fields, currentModule?.command_aliases),
    );

    const cliLine = commandCli(server.name, commandInput, commands);

    // 입력값 기준 필터링된 힌트
    const filteredHints = commandInput.trim()
        ? commands.filter((cmd) => cmd.name.toLowerCase().startsWith(commandInput.trim().toLowerCase()))
        : commands;

    // 히스토리 추가 시 자동 스크롤
    useEffect(() => {
        historyEndRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'nearest' });
    }, [history]);

    // Tab 자동완성 & 방향키 힌트 탐색
    const handleKeyDown = (e) => {
        if (e.key === 'Tab') {
            e.preventDefault();
            if (filteredHints.length > 0) {
                const idx = highlightIdx >= 0 && highlightIdx < filteredHints.length ? highlightIdx : 0;
                setCommandInput(filteredHints[idx].name + ' ');
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
            pushHistory(server.id, entry);
            return;
        }

        // 되돌리기 어려운 명령은 한 번 더 확인한다 (module.toml risk = "danger")
        if (needsConfirm(commands, cmdText) && confirmText !== cmdText) {
            setConfirmText(cmdText);
            return;
        }
        setConfirmText(null);

        setLoading(true);
        try {
            // 한 줄 그대로 보낸다 — 명령 찾기·인자 나누기·검증·라우팅은 데몬이 한다
            const result = await window.api.executeCommand(server.id, { command: cmdText });

            // 응답 메시지 추출 — REST API는 data.response에 {"raw":""} 같은
            // 빈 객체를 반환할 수 있으므로 이를 감지하여 깔끔한 성공/실패 표시로 변환
            const isError = !!result.error;
            const status = isError ? 'failure' : 'success';

            // 원본 응답 데이터를 상세 내역용 문자열로 구성
            const responseObj = result?.data?.response;
            const responseText = result?.data?.response_text;

            // 응답이 의미 있는 내용인지 판별하는 헬퍼
            const isEmptyResponse = (val) => {
                if (val == null) return true;
                if (typeof val === 'string') return val.trim() === '';
                if (typeof val === 'object') {
                    // {"raw": ""} 패턴 감지
                    const keys = Object.keys(val);
                    if (keys.length === 0) return true;
                    if (keys.length === 1 && keys[0] === 'raw' && String(val.raw).trim() === '') return true;
                }
                return false;
            };

            // 사용자에게 보여줄 간결한 메시지
            let displayMsg;
            if (isError) {
                displayMsg = commandErrorText(result, t) ?? result.error;
            } else if (!isEmptyResponse(responseObj)) {
                // 의미 있는 응답이 있을 때: 문자열이면 그대로, 객체면 간략 표시
                if (typeof responseObj === 'string') {
                    displayMsg = responseObj;
                } else if (typeof responseObj === 'object' && responseObj.raw && String(responseObj.raw).trim()) {
                    displayMsg = String(responseObj.raw);
                } else {
                    displayMsg = t('command_modal.command_executed', { command: cmdText });
                }
            } else if (result.message) {
                displayMsg = result.message;
            } else {
                displayMsg = t('command_modal.command_executed', { command: cmdText });
            }

            // 상세 내역 (확장 시 표시) — 원본 응답 전체
            let detail = null;
            if (result?.data) {
                const detailParts = [];
                if (result.data.method) detailParts.push(`${result.data.method} ${result.data.url || ''}`);
                if (result.data.status != null) detailParts.push(`Status: ${result.data.status}`);
                if (!isEmptyResponse(responseObj)) {
                    const respStr =
                        typeof responseObj === 'object' ? JSON.stringify(responseObj, null, 2) : String(responseObj);
                    detailParts.push(`Response: ${respStr}`);
                }
                if (responseText && responseText.trim()) {
                    detailParts.push(`Raw: ${responseText}`);
                }
                if (detailParts.length > 0) detail = detailParts.join('\n');
            }
            if (isError && !detail) {
                detail = displayMsg;
            }

            // 서버가 돌려준 답(명령 뒤 콘솔 줄)은 펼치지 않아도 보인다
            const reply =
                !isError && displayMsg !== result.message && typeof displayMsg === 'string' ? displayMsg : null;
            const entry = {
                id: Date.now(),
                command: cmdText,
                status,
                message: displayMsg,
                reply,
                detail,
                time: new Date().toLocaleTimeString(),
            };
            pushHistory(server.id, entry);
        } catch (error) {
            const entry = {
                id: Date.now(),
                command: cmdText,
                status: 'failure',
                message: t('command_modal.execution_failed'),
                detail: translateError(error.message),
                time: new Date().toLocaleTimeString(),
            };
            pushHistory(server.id, entry);
        } finally {
            setLoading(false);
            setCommandInput('');
            inputRef.current?.focus();
        }
    };

    return (
        <div className="cmd-runner">
            {/* ── 왼쪽: 명령 목록 + 입력 ── */}
            <section className="cmd-panel cmd-main">
                <header className="cmd-panel-head">
                    <Icon name="command" size="sm" />
                    <h3>{t('command_modal.command_label')}</h3>
                </header>

                <div className="cmd-list" role="listbox" aria-label={t('command_modal.command_label')}>
                    {filteredHints.length > 0 ? (
                        filteredHints.map((cmd, idx) => {
                            const text = commandText(t, server.module, cmd);
                            return (
                                // biome-ignore lint/a11y/useKeyWithClickEvents: 키보드로는 입력 칸의 ↑↓·Tab으로 고른다
                                <div
                                    key={cmd.name}
                                    role="option"
                                    tabIndex={-1}
                                    aria-selected={idx === highlightIdx}
                                    className={clsx('cmd-item', { active: idx === highlightIdx })}
                                    onMouseDown={(e) => e.preventDefault()}
                                    onClick={() => {
                                        setCommandInput(cmd.name + ' ');
                                        inputRef.current?.focus();
                                    }}
                                    title={cmd.name}
                                >
                                    <span className="cmd-item-name">{text.label}</span>
                                    {text.description && <span className="cmd-item-desc">{text.description}</span>}
                                    {cmd.group && (
                                        <span className="cmd-source" title={cmd.group}>
                                            <Icon name="extension" size="xs" /> {cmd.group}
                                        </span>
                                    )}
                                    {cmd.source === 'extension' && (
                                        <span className="cmd-source" title={cmd.extension}>
                                            <Icon name="extension" size="xs" /> {cmd.extension}
                                        </span>
                                    )}
                                    {cmd.risk && cmd.risk !== 'safe' && (
                                        <span className={clsx('cmd-risk', `risk-${cmd.risk}`)}>
                                            {t(`shell.risk_${cmd.risk}`)}
                                        </span>
                                    )}
                                </div>
                            );
                        })
                    ) : (
                        <p className="cmd-empty">
                            {commandInput.trim()
                                ? t('command_modal.no_matching_hints')
                                : t('command_modal.type_to_filter')}
                        </p>
                    )}
                </div>

                {/* 입력줄 (항상 아래 고정) */}
                <div className="cmd-compose">
                    {confirmText && confirmText === commandInput.trim() && (
                        <div className="notice tone-stop risk-confirm" role="alert">
                            <Icon name="alertTriangle" size="sm" />
                            <span className="risk-confirm-text">
                                {t('shell.risk_confirm', { command: confirmText })}
                            </span>
                            <button type="button" className="btn btn-danger btn-sm" onClick={handleExecuteCommand}>
                                {t('shell.risk_run')}
                            </button>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirmText(null)}>
                                {t('common:cancel', { defaultValue: '취소' })}
                            </button>
                        </div>
                    )}
                    <div className="cmd-input">
                        <Icon name="chevronRight" size="sm" />
                        <input
                            ref={inputRef}
                            type="text"
                            className="cmd-field"
                            value={commandInput}
                            onChange={(e) => setCommandInput(e.target.value)}
                            onKeyDown={handleKeyDown}
                            placeholder={t('command_modal.command_placeholder')}
                            aria-label={t('command_modal.command_label')}
                            autoComplete="off"
                            spellCheck={false}
                            disabled={loading}
                        />
                        <kbd className="cmd-tab-hint">Tab {t('command_modal.autocomplete')}</kbd>
                        <button
                            type="button"
                            className="term-send"
                            onClick={handleExecuteCommand}
                            disabled={!commandInput.trim() || loading}
                            title={t('command_modal.execute')}
                            aria-label={t('command_modal.execute')}
                        >
                            <Icon name={loading ? 'loader' : 'arrowUp'} size="sm" />
                        </button>
                    </div>
                    {cliLine && <CliText line={cliLine} />}
                </div>
            </section>

            {/* ── 오른쪽: 실행 기록 ── */}
            <section className="cmd-panel cmd-history">
                <header className="cmd-panel-head">
                    <Icon name="clock" size="sm" />
                    <h3>{t('command_modal.history')}</h3>
                </header>
                <div className="cmd-history-list">
                    {history.length === 0 && <p className="cmd-empty">{t('command_modal.no_history')}</p>}
                    {history.map((entry) => {
                        const expanded = expandedId === entry.id;
                        const tone = entry.status === 'success' ? 'ok' : 'stop';
                        return (
                            <div key={entry.id} className={clsx('cmd-entry', { expanded })}>
                                <button
                                    type="button"
                                    className="cmd-entry-head"
                                    aria-expanded={expanded}
                                    onClick={() => setExpandedId((prev) => (prev === entry.id ? null : entry.id))}
                                    title={t('command_modal.click_to_expand')}
                                >
                                    <span className={clsx('status-chip-dot', `tone-${tone}`)} aria-hidden="true" />
                                    <code className="cmd-entry-cmd">{entry.command}</code>
                                    <span className={clsx('cmd-entry-status', `tone-${tone}`)}>
                                        {entry.status === 'success'
                                            ? t('command_modal.success')
                                            : t('command_modal.execution_failed')}
                                    </span>
                                    <span className="cmd-entry-time">{entry.time}</span>
                                </button>
                                {entry.reply && !expanded && <pre className="cmd-entry-out">{entry.reply}</pre>}
                                {expanded && (entry.message || entry.detail) && (
                                    <pre className="cmd-entry-out">{entry.detail || entry.message}</pre>
                                )}
                            </div>
                        );
                    })}
                    <div ref={historyEndRef} />
                </div>
            </section>
        </div>
    );
}

export default CommandModal;
