import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { guildDraft, guildPatch, MEMO_MAX, relayErrorLines, SaveBar, ToolGroups, useChatbotTools } from './ChatbotView';

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * 디스코드 서버 하나의 말하는 사바쨩 (서버 화면 › 말하는 사바쨩 탭): 사용 여부, 이 서버에서 켤 도구, 서버 프롬프트, 기억 지우기.
 * 새 서버는 꺼져 있고 도구도 없다. 공통 메뉴의 전체 허용 도구에서 막은 도구는 잠근다.
 * 원본은 데몬 config.json `chatbot.guilds` (PUT /api/chatbot `guilds`)
 */
export function ChatbotGuildTab({ node }) {
    const { t, i18n } = useTranslation('gui');
    const guildId = node.guildId;
    const name = node.guildName || guildId;
    const [data, setData] = useState(null);
    const [loadError, setLoadError] = useState(null);
    const { catalog, reload } = useChatbotTools(guildId);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.chatbotGet();
            if (res?.success === false) throw new Error(res.error || t('chatbot.load_failed'));
            setData(res);
        } catch (err) {
            setLoadError(err.message || String(err));
        }
    }, [t]);
    useEffect(() => {
        load();
    }, [load]);

    const saved = data ? guildDraft(data.guilds?.[guildId], data.guildDefault) : null;
    const resetKey = JSON.stringify(saved);
    const [draft, setDraft] = useState(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 저장된 값이 바뀔 때만 초기화
    useEffect(() => setDraft(saved), [resetKey]);

    const [saving, setSaving] = useState(false);
    const [errors, setErrors] = useState([]);
    const [memArmed, setMemArmed] = useState(false);
    const [templateBusy, setTemplateBusy] = useState(false);

    if (loadError) return <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />;
    if (!data || !draft) return <SabaSpinner />;

    const set = (change) => setDraft((d) => ({ ...d, ...change }));
    const allowed = data.settings?.allowedTools;
    const blocked = (tool) => Array.isArray(allowed) && !allowed.includes(tool);
    const badMemo = draft.memo.length > MEMO_MAX;
    const dirty = !same(saved, draft);

    const save = async () => {
        setSaving(true);
        setErrors([]);
        try {
            const res = await window.api.chatbotSave({ guilds: { [guildId]: guildPatch(draft) } });
            if (res?.success === false) {
                setErrors(
                    relayErrorLines(res, () => name).concat(res.guildId || res.error ? [] : [t('chatbot.save_failed')]),
                );
                return;
            }
            setData(res);
            safeShowToast(t('chatbot.saved'), 'success', 3000);
        } catch (err) {
            setErrors([err.message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    const clearMemory = async () => {
        setMemArmed(false);
        try {
            const res = await window.api.chatbotClearMemory(guildId);
            if (res?.success === false) throw new Error(res.error || t('chatbot.save_failed'));
            safeShowToast(t('chatbot.memory_cleared', { name }), 'success', 3000);
        } catch (err) {
            safeShowToast(err.message || String(err), 'error', 6000);
        }
    };

    // 서버 프롬프트 기본 틀은 데몬이 앱 언어로 준다 (GET /api/chatbot/memo-template)
    const insertTemplate = async () => {
        setTemplateBusy(true);
        try {
            const res = await window.api.chatbotMemoTemplate(i18n.language);
            if (res?.success === false || typeof res?.template !== 'string') throw new Error(res?.error || '');
            // 기다리는 사이 무언가 썼으면 덮지 않는다
            setDraft((d) => (d.memo.trim() ? d : { ...d, memo: res.template }));
        } catch (err) {
            safeShowToast(err.message || t('chatbot.memo_template_failed'), 'error', 6000);
        } finally {
            setTemplateBusy(false);
        }
    };

    const enabledLabel = t('chatbot.guild_enabled');
    return (
        <div className="dc-tab">
            <p className="dc-card-desc">{t('chatbot.guild_tab_desc')}</p>
            {!data.hasApiKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('chatbot.key_needed')}
                </p>
            )}
            <section className="dc-card">
                <div className="cb-toggle-row">
                    <span className="dc-list-text">
                        <span className="dc-list-name">{enabledLabel}</span>
                        <span className="cb-hint">{t('chatbot.guild_enabled_hint')}</span>
                    </span>
                    <SabaToggle
                        checked={draft.enabled}
                        aria-label={enabledLabel}
                        onChange={(v) => set({ enabled: v })}
                    />
                </div>
                <p className="dc-hint">{t('chatbot.channels_moved')}</p>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.guild_tools_title')}</h2>
                </div>
                <ToolGroups
                    catalog={catalog}
                    reload={reload}
                    isOn={(tool) => draft.tools.includes(tool)}
                    blocked={blocked}
                    onToggle={(tool, on) =>
                        set({
                            tools: on ? [...new Set([...draft.tools, tool])] : draft.tools.filter((x) => x !== tool),
                        })
                    }
                />
            </section>

            <section className="dc-card">
                {/* label 이 아니라 div: 안의 기본 틀 버튼이 label 의 이름을 가져가지 않게 (글 상자는 aria-label) */}
                <div className="cb-guild-memo">
                    <span className="cb-memo-head">
                        <span>{t('chatbot.memo')}</span>
                        {!draft.memo.trim() && window.api?.chatbotMemoTemplate && (
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={templateBusy}
                                onClick={insertTemplate}
                            >
                                {t('chatbot.memo_template')}
                            </button>
                        )}
                        <span className={clsx('cb-counter', { over: badMemo })}>
                            {draft.memo.length.toLocaleString()} / {MEMO_MAX.toLocaleString()}
                        </span>
                    </span>
                    <textarea
                        className="input cb-persona"
                        rows={4}
                        value={draft.memo}
                        aria-label={t('chatbot.memo_aria', { name })}
                        placeholder={t('chatbot.memo_placeholder')}
                        onChange={(e) => set({ memo: e.target.value })}
                    />
                </div>
                {badMemo && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {t('chatbot.err_memo', { max: MEMO_MAX })}
                    </p>
                )}
                {window.api?.chatbotClearMemory && (
                    <button
                        type="button"
                        className={clsx('btn btn-sm', memArmed ? 'btn-danger' : 'btn-ghost')}
                        onClick={() => (memArmed ? clearMemory() : setMemArmed(true))}
                    >
                        {memArmed ? t('chatbot.memory_clear_confirm') : t('chatbot.memory_clear')}
                    </button>
                )}
            </section>

            {errors.length > 0 && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{errors.join('\n')}</span>
                </div>
            )}

            <SaveBar
                dirty={dirty}
                saving={saving}
                blocked={badMemo}
                onRevert={() => {
                    setDraft(saved);
                    setErrors([]);
                }}
                onSave={save}
            />
        </div>
    );
}
