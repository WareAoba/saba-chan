import clsx from 'clsx';
import type { ReactNode } from 'react';
import { useCallback, useEffect, useState } from 'react';
import { PROVIDER_BRANDS } from './providerBrands';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { SabaNumberInput, SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { LinkedText } from '../ui/ScreenLink';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import {
    AMBIENT_MINUTES,
    EFFORTS,
    type GuildDraft,
    guildDraft,
    guildPatch,
    MEMO_MAX,
    relayErrorLines,
    SaveBar,
    ToolGroups,
    useChatbotTools,
} from './ChatbotView';
import type { DiscordNode } from './types';
import { failed } from '../../utils/apiResult';

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

interface ChatbotGuildTabProps {
    node: DiscordNode;
}

/**
 * 디스코드 서버 하나의 말하는 사바쨩 (서버 화면 › 말하는 사바쨩 탭): 사용 여부, 답변 깊이·기억·끼어들기,
 * 이 서버에서 켤 도구, 서버 프롬프트, 기억 지우기. 새 서버는 꺼져 있고 도구도 없다.
 * 원본은 데몬 config.json `chatbot.guilds` (PUT /api/chatbot `guilds`)
 */
export function ChatbotGuildTab({ node }: ChatbotGuildTabProps) {
    const { t, i18n } = useTranslation('gui');
    const guildId = node.guildId;
    const name = node.guildName || guildId;
    const [data, setData] = useState<ApiBody | null>(null);
    const [loadError, setLoadError] = useState<string | null>(null);
    const { catalog, reload } = useChatbotTools(guildId);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.chatbotGet();
            if (failed(res)) throw new Error(res.error || t('chatbot.load_failed'));
            setData(res);
        } catch (err) {
            setLoadError((err as Error).message || String(err));
        }
    }, [t]);
    useEffect(() => {
        load();
    }, [load]);
    // 잔액을 알려 주는 제공사인지 (OpenAI 등은 모른다: 그때는 내가 정한 토큰 한도로 알린다)
    const hasKey = !!data?.hasApiKey;
    const [credit, setCredit] = useState<{ provider: string; balanceSupported: boolean } | null>(null);
    useEffect(() => {
        if (!hasKey || !window.api?.chatbotCredit) return;
        let alive = true;
        window.api
            .chatbotCredit(guildId)
            .then((r) => {
                if (alive && r?.provider && typeof r.balanceSupported === 'boolean')
                    setCredit({ provider: r.provider, balanceSupported: r.balanceSupported });
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [guildId, hasKey]);

    // 아직 서버마다 옮기지 않은 옛 공통 값 (데몬이 서버 목록을 받으면 옮긴다)
    const legacy = data?.settings || {};
    const saved = data ? guildDraft(data.guilds?.[guildId], data.guildDefault, legacy, data.defaults) : null;
    const resetKey = JSON.stringify(saved);
    const [draft, setDraft] = useState<GuildDraft | null>(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 저장된 값이 바뀔 때만 초기화
    useEffect(() => setDraft(saved), [resetKey]);

    const [saving, setSaving] = useState(false);
    const [errors, setErrors] = useState<string[]>([]);
    const [syncWarn, setSyncWarn] = useState(false);
    const [memArmed, setMemArmed] = useState(false);
    const [templateBusy, setTemplateBusy] = useState(false);

    if (loadError) return <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />;
    if (!data || !draft) return <SabaSpinner />;

    const set = (change: Record<string, unknown>) => setDraft((d) => ({ ...d, ...change }) as GuildDraft);
    const badMemo = draft.memo.length > MEMO_MAX;
    // 토큰 한도는 1 이상 정수, 잔액 금액은 0 보다 큰 수 (비우면 끔)
    const badLimit = draft.tokenLimit != null && !(Number.isInteger(draft.tokenLimit) && draft.tokenLimit >= 1);
    const noBalance = credit?.balanceSupported === false;
    const providerName = credit
        ? ((PROVIDER_BRANDS[credit.provider] ?? PROVIDER_BRANDS[credit.provider.replace(/-cn$/, '')])?.name ??
          credit.provider)
        : '';
    const badBalance = draft.balanceAlert != null && !(draft.balanceAlert > 0 && draft.balanceAlert <= 1e9);
    const dirty = !same(saved, draft);

    const save = async () => {
        setSaving(true);
        setErrors([]);
        try {
            const res = await window.api.chatbotSave({
                guilds: { [guildId]: guildPatch(draft, legacy, data.defaults) },
            });
            if (failed(res)) {
                setErrors(
                    relayErrorLines(res, () => name).concat(res.guildId || res.error ? [] : [t('chatbot.save_failed')]),
                );
                return;
            }
            // 데몬에는 저장됐지만 릴레이에 못 보낸 서버는 다음 연결 때 맞춘다
            setSyncWarn(Array.isArray(res?.sync) && res.sync.some((r: ApiBody) => !r.ok));
            setData(res);
            safeShowToast(t('chatbot.saved'), 'success', 3000);
        } catch (err) {
            setErrors([(err as Error).message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    const clearMemory = async () => {
        setMemArmed(false);
        try {
            const res = await window.api.chatbotClearMemory(guildId);
            if (failed(res)) throw new Error(res.error || t('chatbot.save_failed'));
            safeShowToast(t('chatbot.memory_cleared', { name }), 'success', 3000);
        } catch (err) {
            safeShowToast((err as Error).message || String(err), 'error', 6000);
        }
    };

    // 서버 프롬프트 기본 틀은 데몬이 앱 언어로 준다 (GET /api/chatbot/memo-template)
    const insertTemplate = async () => {
        setTemplateBusy(true);
        try {
            const res = await window.api.chatbotMemoTemplate(i18n.language);
            if (failed(res) || typeof res?.template !== 'string') throw new Error(res?.error || '');
            // 기다리는 사이 무언가 썼으면 덮지 않는다
            setDraft((d) => (d?.memo.trim() ? d : ({ ...d, memo: res.template } as GuildDraft)));
        } catch (err) {
            safeShowToast((err as Error).message || t('chatbot.memo_template_failed'), 'error', 6000);
        } finally {
            setTemplateBusy(false);
        }
    };

    const toggleRow = (key: keyof GuildDraft, label: string, hint?: ReactNode) => (
        <div className="cb-toggle-row">
            <span className="dc-list-text">
                <span className="dc-list-name">{label}</span>
                {hint && <span className="cb-hint">{hint}</span>}
            </span>
            <SabaToggle checked={!!draft[key]} aria-label={label} onChange={(v) => set({ [key]: v })} />
        </div>
    );
    const selectField = (
        key: keyof GuildDraft,
        label: string,
        hint: ReactNode,
        options: { value: string | number; label: string }[],
        disabled = false,
    ) => (
        <div className="mt-field">
            <span>{label}</span>
            <CustomDropdown
                className="cb-select"
                value={draft[key] as string | number}
                ariaLabel={label}
                disabled={disabled}
                onChange={(v) => set({ [key]: v })}
                options={options}
            />
            {hint && <p className="dc-hint">{hint}</p>}
        </div>
    );
    const numberField = (key: keyof GuildDraft, label: string, hint: ReactNode, step: number, bad: boolean) => (
        <label className="mt-field">
            <span>{label}</span>
            <SabaNumberInput
                className="input mt-num"
                min={step}
                step={step}
                value={(draft[key] ?? '') as string | number}
                aria-label={label}
                placeholder={t('chatbot.alert_off')}
                onChange={(e) => set({ [key]: e.target.value === '' ? null : Number(e.target.value) })}
            />
            {hint && <span className="cb-hint">{hint}</span>}
            {bad && <span className="dc-field-error">{t('chatbot.alert_invalid')}</span>}
        </label>
    );
    const effortOptions = EFFORTS.map((e) => ({ value: e, label: t(`chatbot.effort_${e}`) }));

    const enabledLabel = t('chatbot.guild_enabled');
    return (
        <div className="dc-tab">
            <p className="dc-card-desc">
                <LinkedText i18nKey="chatbot.guild_desc" to="discord/saba" />
            </p>
            {!data.hasApiKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('chatbot.key_needed')}
                </p>
            )}
            <section data-spot="chatbot.guild" className="dc-card">
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
                <p className="dc-hint">
                    <LinkedText i18nKey="chatbot.channels_moved" to={`discord/${node.guildId}/moderation/channels`} />
                </p>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.basic_title')}</h2>
                </div>
                {selectField('effort', t('chatbot.effort'), t('chatbot.effort_hint'), effortOptions)}
                <p className="dc-hint">
                    <LinkedText i18nKey="chatbot.prefix_calls_guild" to={`discord/${node.guildId}/bot`} />
                </p>
                <div className="cb-toggles">{toggleRow('memory', t('chatbot.memory'), t('chatbot.memory_hint'))}</div>
            </section>

            <section data-spot="chatbot.moderation" className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.moderation_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.moderation_desc')}</p>
                <div className="cb-toggles">
                    {toggleRow('moderation', t('chatbot.moderation'), t('chatbot.moderation_hint'))}
                </div>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.token_alert_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.token_alert_desc')}</p>
                <MillionTokensField
                    label={t('chatbot.token_limit')}
                    unit={t('chatbot.token_limit_unit')}
                    hint={
                        noBalance
                            ? `${t('chatbot.token_limit_hint')} ${t('chatbot.token_limit_no_balance', { provider: providerName })}`
                            : t('chatbot.token_limit_hint')
                    }
                    placeholder={t('chatbot.alert_off')}
                    error={badLimit ? t('chatbot.token_limit_invalid') : null}
                    tokens={draft.tokenLimit}
                    onChange={(tokenLimit) => set({ tokenLimit })}
                />
                {numberField(
                    'balanceAlert',
                    t('chatbot.balance_alert'),
                    noBalance
                        ? t('chatbot.balance_alert_unsupported', { provider: providerName })
                        : t('chatbot.balance_alert_hint'),
                    0.01,
                    badBalance,
                )}
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.ambient_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.ambient_desc')}</p>
                <div className="cb-toggles">
                    {toggleRow('ambient', t('chatbot.ambient'), t('chatbot.ambient_hint'))}
                </div>
                {selectField(
                    'ambientMinutes',
                    t('chatbot.ambient_minutes'),
                    null,
                    // CLI·디스코드로 목록 밖 값(1-60)을 저장했으면 그 값도 보인다
                    [...new Set([...AMBIENT_MINUTES, draft.ambientMinutes].filter(Number.isInteger))]
                        .sort((a, b) => a - b)
                        .map((m) => ({ value: m, label: t('chatbot.ambient_minutes_option', { count: m }) })),
                    !draft.ambient,
                )}
                {selectField(
                    'ambientEffort',
                    t('chatbot.ambient_effort'),
                    t('chatbot.ambient_effort_hint'),
                    effortOptions,
                    !draft.ambient,
                )}
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.guild_tools_title')}</h2>
                </div>
                <ToolGroups
                    catalog={catalog}
                    reload={reload}
                    isOn={(tool) => draft.tools.includes(tool)}
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
                        {!draft.memo.trim() && typeof window.api?.chatbotMemoTemplate === 'function' && (
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
                {typeof window.api?.chatbotClearMemory === 'function' && (
                    <button
                        type="button"
                        className={clsx('btn btn-sm', memArmed ? 'btn-danger' : 'btn-ghost')}
                        onClick={() => (memArmed ? clearMemory() : setMemArmed(true))}
                    >
                        {memArmed ? t('chatbot.memory_clear_confirm') : t('chatbot.memory_clear')}
                    </button>
                )}
            </section>

            {syncWarn && (
                <div className="notice tone-info">
                    <Icon name="cloudOff" size="sm" />
                    <span>{t('chatbot.sync_pending')}</span>
                </div>
            )}
            {errors.length > 0 && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{errors.join('\n')}</span>
                </div>
            )}

            <SaveBar
                dirty={dirty}
                saving={saving}
                blocked={badMemo || badLimit || badBalance}
                onRevert={() => {
                    setDraft(saved);
                    setErrors([]);
                }}
                onSave={save}
            />
        </div>
    );
}

interface MillionTokensFieldProps {
    label: string;
    unit: string;
    hint: ReactNode;
    placeholder: string;
    error: string | null;
    tokens: number | null;
    onChange: (tokens: number | null) => void;
}

const MILLION = 1_000_000;

/** 토큰 한도는 백만 단위로 적는다 (저장은 토큰 수 정수 `tokenLimit`). 쓰는 동안의 글자는 그대로 둔다 ("1." 등) */
function MillionTokensField({ label, unit, hint, placeholder, error, tokens, onChange }: MillionTokensFieldProps) {
    const shown = tokens == null ? '' : String(tokens / MILLION);
    const [text, setText] = useState(shown);
    // 저장된 값이 밖에서 바뀌면(불러오기·저장 뒤) 따라간다
    useEffect(() => {
        setText((cur) => (cur !== '' && Math.round(Number(cur) * MILLION) === tokens ? cur : shown));
    }, [shown, tokens]);
    return (
        <label className="mt-field">
            <span>{label}</span>
            <span className="mt-inline">
                <SabaNumberInput
                    className="input mt-num"
                    min={0.1}
                    step={0.1}
                    value={text}
                    aria-label={label}
                    placeholder={placeholder}
                    onChange={(e) => {
                        const v = e.target.value;
                        setText(v);
                        // 0 이하·숫자 아님은 0으로 두어 검사가 잡게 한다
                        onChange(v === '' ? null : Math.max(0, Math.round(Number(v) * MILLION) || 0));
                    }}
                />
                <span className="muted">{unit}</span>
            </span>
            {hint && <span className="cb-hint">{hint}</span>}
            {error && <span className="dc-field-error">{error}</span>}
        </label>
    );
}
