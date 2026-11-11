import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { PROVIDER_BRANDS } from './providerBrands';
import type { DiscordNode } from './types';
import { failed } from '../../utils/apiResult';

/**
 * 사바쨩 설정(공통)에 두는 값: 모델만 (데몬 `commonKeys`). 나머지는 서버마다 정한다 (ChatbotGuildTab).
 * 기본값과 같으면 null로 보내 기본을 따른다. 호출어·명령 이름 번역은 서버의 봇 탭이 맡는다
 */
export const COMMON_DEFAULTS = { model: 'gpt-6-luna' };
/** 서버마다 두는 말하는 사바쨩 동작 */
export type GuildBehavior = {
    effort: string;
    memory: boolean;
    ambient: boolean;
    ambientMinutes: number;
    ambientEffort: string;
    moderation: boolean;
    tokenLimit: number | null;
    balanceAlert: number | null;
};

/** 서버 하나의 초안 */
export interface GuildDraft extends GuildBehavior {
    enabled: boolean;
    tools: string[];
    memo: string;
}

/** 서버마다 두는 말하는 사바쨩 동작의 기본값 (데몬 relay_defaults, 데몬이 `defaults`로 주면 그쪽이 먼저) */
export const GUILD_DEFAULTS: GuildBehavior = {
    effort: 'medium',
    memory: true,
    ambient: true,
    ambientMinutes: 3,
    ambientEffort: 'low',
    moderation: false,
    // 토큰 알림 (릴레이가 PC 주인에게 DM): 이번 달 토큰 한도, 잔액 알림 금액. null 이면 알리지 않는다
    tokenLimit: null,
    balanceAlert: null,
};
export const EFFORTS = ['low', 'medium', 'high'];
export const AMBIENT_MINUTES = [1, 3, 5, 10, 30];
export const MEMO_MAX = 8000;

export const chatbotAvailable = () => !!window.api?.chatbotGet;

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * 서버 하나의 초안: 켜기·켠 도구·서버 프롬프트와 동작(답변 깊이·기억·끼어들기).
 * 켜기·도구는 값이 없으면 데몬이 준 기본(guildDefault: 새 서버는 꺼짐, 도구 없음, 옮기기 전이면 옛 공통 값).
 * 동작은 서버 값 → 아직 옮기지 않은 옛 공통 값(legacy) → 데몬 기본값 순
 */
export const guildDraft = (
    g: Partial<GuildDraft> = {},
    fallback: { enabled?: boolean; tools?: string[] } = {},
    legacy: Partial<GuildBehavior> = {},
    defaults: Partial<GuildBehavior> = {},
): GuildDraft => ({
    enabled: typeof g.enabled === 'boolean' ? g.enabled : !!fallback.enabled,
    tools: Array.isArray(g.tools) ? g.tools : Array.isArray(fallback.tools) ? fallback.tools : [],
    memo: typeof g.memo === 'string' ? g.memo : '',
    ...(Object.fromEntries(
        (Object.entries(GUILD_DEFAULTS) as [keyof GuildBehavior, unknown][]).map(([k, def]) => [
            k,
            g[k] ?? legacy[k] ?? defaults[k] ?? def,
        ]),
    ) as GuildBehavior),
});

/**
 * 서버 초안 → PUT guilds 값: 켜기·도구는 늘 그대로 적는다. 동작은 기본값과 같으면 null(기본을 따른다).
 * 옛 공통 값이 남아 있는 키는 기본값이라도 적는다 (지우면 옛 값을 따라가므로)
 */
export const guildPatch = (
    d: GuildDraft,
    legacy: Partial<GuildBehavior> = {},
    defaults: Partial<GuildBehavior> = {},
) => ({
    enabled: d.enabled,
    tools: d.tools,
    memo: d.memo.trim() ? d.memo : null,
    ...Object.fromEntries(
        (Object.entries(GUILD_DEFAULTS) as [keyof GuildBehavior, unknown][]).map(([k, def]) => [
            k,
            same(d[k], defaults[k] ?? def) && legacy[k] == null ? null : d[k],
        ]),
    ),
});

/** 언어 → 문구 */
type Localized = Record<string, string>;

interface ChatbotTool {
    name: string;
    category: string;
    available?: boolean;
    unavailable?: string;
    label?: Localized;
    hint?: Localized;
}

export interface ToolCatalog {
    error?: string;
    categories?: string[];
    categoryLabels?: Record<string, Localized>;
    tools?: ChatbotTool[];
}

/**
 * 고를 수 있는 도구 목록 — 릴레이가 원본 (GET /api/relay/node/:guild/chatbot/tools).
 * {categories, categoryLabels, tools: [{name, category, available, unavailable?, label, hint}]}
 * 표시 이름·설명(label·hint, 언어 → 문구)도 릴레이가 준다 — 새 도구가 앱 업데이트 없이 제 이름으로 보인다
 */
export function useChatbotTools(guildId: string | undefined, keyVersion = 0) {
    const [res, setRes] = useState<ToolCatalog | null>(null);
    const canAsk = !!(guildId && window.api?.chatbotTools);
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyVersion 은 다시 묻는 신호 (제공사가 바뀐다)
    const load = useCallback(async () => {
        if (!canAsk) return;
        setRes(null);
        try {
            const r = await window.api.chatbotTools(guildId as string);
            setRes(Array.isArray(r?.tools) ? r : { error: r?.error || 'FAILED' });
        } catch (err) {
            setRes({ error: (err as Error).message || 'FAILED' });
        }
    }, [canAsk, guildId, keyVersion]);
    useEffect(() => {
        load();
    }, [load]);
    return { catalog: canAsk ? res : { error: 'NO_GUILD' }, reload: load };
}

interface ToolGroupsProps {
    catalog: ToolCatalog | null;
    reload: () => void;
    isOn: (name: string) => boolean;
    onToggle: (name: string, on: boolean) => void;
}

/**
 * 카테고리별 도구 스위치. 쓸 수 없는 도구(운영 키·제공사)는 이유를 덧붙인다
 */
export function ToolGroups({ catalog, reload, isOn, onToggle }: ToolGroupsProps) {
    const { t, i18n } = useTranslation('gui');
    const local = (texts: Localized | undefined, fallback: string) => texts?.[i18n.language] || texts?.en || fallback;
    if (!catalog) return <SabaSpinner size="sm" />;
    if (catalog.error)
        return (
            <output className={clsx('notice', catalog.error === 'NO_GUILD' ? 'tone-info' : 'tone-warn')}>
                <Icon name={catalog.error === 'NO_GUILD' ? 'info' : 'alertTriangle'} size="sm" />
                <span>{catalog.error === 'NO_GUILD' ? t('chatbot.tools_need_guild') : t('chatbot.tools_failed')}</span>
                {catalog.error !== 'NO_GUILD' && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={reload}>
                        {t('maintenance.retry')}
                    </button>
                )}
            </output>
        );
    return (catalog.categories || []).map((category) => {
        const tools = (catalog.tools as ChatbotTool[]).filter((tool) => tool.category === category);
        if (!tools.length) return null;
        return (
            <div className="cb-tool-group" key={category}>
                <h3 className="cb-tool-category">{local(catalog.categoryLabels?.[category], category)}</h3>
                <div className="cb-toggles">
                    {tools.map((tool) => {
                        const label = local(tool.label, tool.name);
                        const notes = [
                            local(tool.hint, ''),
                            !tool.available ? t(`chatbot.tool_unavailable_${tool.unavailable || 'operator_key'}`) : '',
                        ].filter(Boolean);
                        return (
                            <div className="cb-toggle-row" key={tool.name}>
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{label}</span>
                                    {notes.map((n) => (
                                        <span className="cb-hint" key={n}>
                                            {n}
                                        </span>
                                    ))}
                                </span>
                                <SabaToggle
                                    checked={isOn(tool.name)}
                                    aria-label={label}
                                    onChange={(on) => onToggle(tool.name, on)}
                                />
                            </div>
                        );
                    })}
                </div>
            </div>
        );
    });
}

/** 릴레이 400 {guildId, status, error, details} → 읽을 수 있는 줄 */
export function relayErrorLines(res: ApiBody | null | undefined, nameOf: (id: string) => string) {
    const lines: string[] = [];
    const head = [res?.guildId && nameOf(res.guildId), res?.error].filter(Boolean).join(': ');
    if (head) lines.push(head);
    const details = res?.details;
    if (Array.isArray(details)) {
        for (const d of details) {
            if (typeof d === 'string') lines.push(d);
            else if (d)
                lines.push([d.path || d.field, d.message || d.error].filter(Boolean).join(': ') || JSON.stringify(d));
        }
    } else if (details && typeof details === 'object') {
        lines.push(JSON.stringify(details));
    } else if (typeof details === 'string') {
        lines.push(details);
    }
    return lines;
}

interface ChatbotViewProps {
    nodes?: DiscordNode[];
}

/**
 * 사바쨩 설정 (디스코드 › 사바쨩 설정): 모든 서버에 같은 API 키·모델·크레딧만 둔다.
 * 원본은 데몬 config.json `chatbot`, 데몬이 길드마다 릴레이에 맞춘다. API 키는 쓰기만: 화면에는 등록 여부만 보인다.
 */
export function ChatbotView({ nodes = [] }: ChatbotViewProps) {
    const { t } = useTranslation('gui');
    const [data, setData] = useState<ApiBody | null>(null);
    const [loadError, setLoadError] = useState<string | null>(null);
    // 키를 바꿀 때마다 올린다 — 모델 목록과 연결된 제공사를 다시 묻는다
    const [keyVersion, setKeyVersion] = useState(0);

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

    const nameOf = (id: string) => nodes.find((n) => n.guildId === id)?.guildName || id;
    const guildIds = data ? [...new Set([...nodes.map((n) => n.guildId), ...Object.keys(data.guilds || {})])] : [];

    return (
        <div className="dc-view">
            <header className="dc-view-head">
                <h1>
                    <Icon name="sabaFace" size="lg" /> {t('chatbot.saba_title')}
                </h1>
                <p className="muted">{t('chatbot.saba_desc')}</p>
            </header>
            {loadError && <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />}
            {!data && !loadError && <SabaSpinner />}
            {data && (
                <>
                    {data.pendingRelayDelete && (
                        <div className="notice tone-info" role="status">
                            <Icon name="cloudOff" size="sm" />
                            <span>{t('chatbot.relay_delete_pending')}</span>
                        </div>
                    )}
                    <ApiKeyCard
                        hasApiKey={!!data.hasApiKey}
                        onChanged={(res) => {
                            setData(res);
                            setKeyVersion((v) => v + 1);
                        }}
                        nameOf={nameOf}
                    />
                    <ModelCard
                        data={data}
                        guildIds={guildIds}
                        nameOf={nameOf}
                        onSaved={(res) => setData(res)}
                        keyVersion={keyVersion}
                    />
                    <CreditCard
                        guildIds={guildIds}
                        nameOf={nameOf}
                        hasApiKey={!!data.hasApiKey}
                        keyVersion={keyVersion}
                    />
                </>
            )}
        </div>
    );
}

interface ApiKeyCardProps {
    hasApiKey: boolean;
    onChanged: (res: ApiBody) => void;
    nameOf: (id: string) => string;
}

function ApiKeyCard({ hasApiKey, onChanged, nameOf }: ApiKeyCardProps) {
    const { t } = useTranslation('gui');
    const [value, setValue] = useState('');
    const [busy, setBusy] = useState(false);
    const [armed, setArmed] = useState(false);

    const send = async (apiKey: string | null) => {
        setBusy(true);
        setArmed(false);
        try {
            const res = await window.api.chatbotSave({ apiKey });
            if (failed(res)) {
                safeShowToast(relayErrorLines(res, nameOf).join('\n') || t('chatbot.save_failed'), 'error', 6000);
                return;
            }
            setValue('');
            onChanged(res);
            safeShowToast(apiKey ? t('chatbot.key_saved') : t('chatbot.key_removed'), 'success', 3000);
        } catch (err) {
            safeShowToast((err as Error).message || String(err), 'error', 6000);
        } finally {
            setBusy(false);
        }
    };

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('chatbot.key_title')}</h2>
                <StatusChip
                    tone={hasApiKey ? 'ok' : 'idle'}
                    label={hasApiKey ? t('chatbot.key_set') : t('chatbot.key_none')}
                />
                {hasApiKey && (
                    <button
                        type="button"
                        className={clsx('btn btn-sm', armed ? 'btn-danger' : 'btn-ghost btn-danger-text')}
                        disabled={busy}
                        onClick={() => (armed ? send(null) : setArmed(true))}
                    >
                        <Icon name="trash" size="sm" /> {armed ? t('chatbot.key_remove_confirm') : t('chatbot.key_remove')}
                    </button>
                )}
            </div>
            <p className="dc-card-desc">{t('chatbot.key_desc')}</p>
            <p className="dc-hint">{t('chatbot.key_providers')}</p>
            <form
                className="cb-key-row"
                onSubmit={(e) => {
                    e.preventDefault();
                    if (value.trim()) send(value.trim());
                }}
            >
                <input
                    className="input"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    value={value}
                    aria-label={t('chatbot.key_title')}
                    placeholder={hasApiKey ? t('chatbot.key_replace_placeholder') : t('chatbot.key_placeholder')}
                    onChange={(e) => setValue(e.target.value)}
                />
                <button type="submit" className="btn btn-primary" disabled={busy || !value.trim()}>
                    <Icon name="key" size="sm" /> {hasApiKey ? t('chatbot.key_replace') : t('chatbot.key_save')}
                </button>
            </form>
            {!hasApiKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('chatbot.key_needed')}
                </p>
            )}
        </section>
    );
}

interface ModelCardProps {
    data: ApiBody;
    guildIds: string[];
    nameOf: (id: string) => string;
    onSaved: (res: ApiBody) => void;
    keyVersion: number;
}

/** 모델: 모든 서버가 같은 모델을 쓴다 (서버마다 다른 값은 받지 않는다) */
function ModelCard({ data, guildIds, nameOf, onSaved, keyVersion }: ModelCardProps) {
    const { t } = useTranslation('gui');
    const def = data.defaults?.model ?? COMMON_DEFAULTS.model;
    const savedModel = data.settings?.model ?? def;
    const [draft, setDraft] = useState(savedModel);
    useEffect(() => setDraft(savedModel), [savedModel]);
    const [saving, setSaving] = useState(false);
    const [errors, setErrors] = useState<string[]>([]);
    const [syncWarn, setSyncWarn] = useState(false);

    // 고르면 바로 저장한다 (저장 막대는 즉시 저장할 수 없는 칸에만 쓴다)
    const save = async (picked: string) => {
        setDraft(picked);
        setSaving(true);
        setErrors([]);
        const model = String(picked ?? '').trim() || def;
        try {
            const res = await window.api.chatbotSave({ settings: { model: model === def ? null : model } });
            if (failed(res)) {
                setDraft(savedModel);
                setErrors(
                    relayErrorLines(res, nameOf).concat(res.guildId || res.error ? [] : [t('chatbot.save_failed')]),
                );
                return;
            }
            setSyncWarn(Array.isArray(res?.sync) && res.sync.some((r: ApiBody) => !r.ok));
            onSaved(res);
            safeShowToast(t('chatbot.saved'), 'success', 3000);
        } catch (err) {
            setDraft(savedModel);
            setErrors([(err as Error).message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('chatbot.model_title')}</h2>
                {saving && <SabaSpinner size="sm" />}
            </div>
            <p className="dc-card-desc">{t('chatbot.model_desc')}</p>
            <ModelField
                value={draft}
                saved={savedModel}
                onChange={(m) => m !== draft && save(m)}
                guildId={guildIds[0]}
                hasApiKey={!!data.hasApiKey}
                keyVersion={keyVersion}
            />
            {errors.length > 0 && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{errors.join('\n')}</span>
                </div>
            )}
            {syncWarn && (
                <div className="notice tone-info">
                    <Icon name="cloudOff" size="sm" />
                    <span>{t('chatbot.sync_pending')}</span>
                </div>
            )}
        </section>
    );
}

const CREDIT_ERRORS = ['NO_API_KEY', 'INVALID_API_KEY', 'KEY_PERMISSION', 'QUOTA_EXCEEDED', 'PROVIDER_UNAVAILABLE'];

/** 길드 하나의 크레딧 답 (또는 그 실패) */
type CreditRow = ApiBody & { id: string };

interface CreditCardProps {
    guildIds: string[];
    nameOf: (id: string) => string;
    hasApiKey: boolean;
    keyVersion?: number;
}

/**
 * 남은 크레딧: 잔액은 키로 잔액을 알려 주는 제공사만(릴레이가 묻는다), 사용량은 이번 달 길드마다 (릴레이 집계).
 * 키는 길드마다 같으니 잔액은 첫 길드 답의 것을 보인다
 */
export function CreditCard({ guildIds, nameOf, hasApiKey, keyVersion = 0 }: CreditCardProps) {
    const { t, i18n } = useTranslation('gui');
    const [rows, setRows] = useState<CreditRow[] | null>(null);
    // 늦게 온 앞 요청의 답이 새 답을 덮지 않게
    const asked = useRef(0);
    const canAsk = !!(guildIds.length && hasApiKey && window.api?.chatbotCredit);
    const idsKey = guildIds.join(',');

    // biome-ignore lint/correctness/useExhaustiveDependencies: idsKey·keyVersion 이 다시 묻는 신호
    const load = useCallback(async () => {
        const seq = ++asked.current;
        setRows(null);
        if (!canAsk) return;
        const ids = idsKey.split(',');
        const answers = await Promise.all(
            ids.map((id) =>
                window.api
                    .chatbotCredit(id)
                    .then((r) =>
                        r?.error || r?.success === false || !r?.usage
                            ? { id, error: r?.error || 'FAILED' }
                            : { id, ...r },
                    )
                    .catch(() => ({ id, error: 'FAILED' })),
            ),
        );
        if (seq === asked.current) setRows(answers);
    }, [canAsk, idsKey, keyVersion]);

    useEffect(() => {
        load();
    }, [load]);

    if (!window.api?.chatbotCredit) return null;
    const first = rows?.find((r) => !r.error);
    const number = (n: unknown) => Number(n || 0).toLocaleString(i18n.language);
    const money = (b: { currency: string; amount: number }) => {
        try {
            return new Intl.NumberFormat(i18n.language, { style: 'currency', currency: b.currency }).format(b.amount);
        } catch {
            return `${b.amount} ${b.currency}`;
        }
    };
    const providerName = (id: string) =>
        (PROVIDER_BRANDS[id] ?? PROVIDER_BRANDS[String(id).replace(/-cn$/, '')])?.name ?? id;
    const exhausted = rows?.map((r) => r.exhaustedAt).find(Boolean);
    const errorKey = !hasApiKey
        ? 'chatbot.models_need_key'
        : !guildIds.length
          ? 'chatbot.models_need_guild'
          : rows && !first
            ? `chatbot.models_err_${CREDIT_ERRORS.includes(rows[0]?.error as string) ? rows[0].error : 'FAILED'}`
            : null;

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('chatbot.credit_title')}</h2>
                {canAsk && (
                    <button type="button" className="btn btn-ghost btn-sm" disabled={!rows} onClick={load}>
                        {t('chatbot.credit_refresh')}
                    </button>
                )}
            </div>
            <p className="dc-card-desc">{t('chatbot.credit_desc')}</p>
            {canAsk && !rows && <SabaSpinner size="sm" />}
            {errorKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t(errorKey)}
                </p>
            )}
            {exhausted && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('chatbot.credit_exhausted', { time: new Date(exhausted).toLocaleString(i18n.language) })}
                    </span>
                </div>
            )}
            {first && (
                <>
                    <div className="mt-field">
                        <span>{t('chatbot.credit_balance')}</span>
                        {first.balance ? (
                            <strong>{money(first.balance)}</strong>
                        ) : (
                            <p className="dc-hint">
                                {first.balanceSupported
                                    ? t('chatbot.credit_balance_none')
                                    : t('chatbot.credit_balance_unsupported', {
                                          provider: providerName(first.provider),
                                      })}
                            </p>
                        )}
                    </div>
                    <div className="mt-field">
                        <span>{t('chatbot.credit_usage', { month: first.usage.month })}</span>
                        <ul className="dc-list">
                            {(rows as CreditRow[]).map((r) => (
                                <li key={r.id} className="dc-list-row">
                                    <span className="dc-list-text">
                                        <span className="dc-list-name">{nameOf(r.id)}</span>
                                        <span className="cb-hint">
                                            {r.error
                                                ? t('chatbot.credit_guild_failed')
                                                : t('chatbot.credit_usage_line', {
                                                      calls: number(r.usage.calls),
                                                      input: number(r.usage.input),
                                                      cached: number(r.usage.cached),
                                                      output: number(r.usage.output),
                                                  })}
                                        </span>
                                    </span>
                                </li>
                            ))}
                        </ul>
                    </div>
                </>
            )}
        </section>
    );
}

interface ModelFieldProps {
    value: string;
    saved: string;
    onChange: (value: string) => void;
    guildId?: string;
    hasApiKey: boolean;
    keyVersion?: number;
}

/**
 * 모델 고르기: 릴레이가 사용자 키로 물은 목록에서 고른다 (chatbot-settings-v2 §1).
 * 고른 모델을 쓸 수 없어 릴레이가 다른 모델로 대신 쓰고 있으면 알리고, 그 모델을 고른 상태로 둔다
 * (저장하면 그 모델이 새 설정이 된다).
 */
export function ModelField({ value, saved, onChange, guildId, hasApiKey, keyVersion = 0 }: ModelFieldProps) {
    const { t } = useTranslation('gui');
    const [res, setRes] = useState<ApiBody | null>(null);
    const canAsk = !!(guildId && hasApiKey && window.api?.chatbotModels);

    // keyVersion: 키를 바꾸면 같은 길드라도 다시 묻는다 (제공사·목록이 바뀐다)
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyVersion 은 다시 묻는 신호
    const load = useCallback(async () => {
        if (!canAsk) return;
        setRes({ loading: true });
        try {
            const r = await window.api.chatbotModels(guildId as string);
            setRes(r?.error || r?.success === false || !Array.isArray(r?.models) ? { error: r?.error || 'FAILED' } : r);
        } catch (err) {
            setRes({ error: (err as Error).message || 'FAILED' });
        }
    }, [canAsk, guildId, keyVersion]);

    useEffect(() => {
        setRes(null);
        load();
    }, [load]);

    const fallback = res?.effectiveModel && res.effectiveModel !== saved ? res.effectiveModel : null;
    // 대신 쓰는 모델이 있으면 처음 한 번 그쪽을 고른다 (저장 전까지는 되돌릴 수 있다)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 목록을 새로 받았을 때만
    useEffect(() => {
        if (fallback && value === saved) onChange(fallback);
    }, [fallback]);

    const models = res?.models || [];
    const options = value && !models.includes(value) ? [value, ...models] : models;
    const errorKey = !hasApiKey
        ? 'chatbot.models_need_key'
        : !guildId
          ? 'chatbot.models_need_guild'
          : res?.error
            ? `chatbot.models_err_${['NO_API_KEY', 'INVALID_API_KEY', 'KEY_PERMISSION', 'QUOTA_EXCEEDED', 'PROVIDER_UNAVAILABLE'].includes(res.error as string) ? res.error : 'FAILED'}`
            : null;

    return (
        <div className="mt-field">
            <span>{t('chatbot.model')}</span>
            <span className="mt-inline">
                <CustomDropdown
                    className="cb-select"
                    value={value}
                    ariaLabel={t('chatbot.model')}
                    disabled={!res || res.loading || !!res.error}
                    onChange={onChange}
                    options={(options.length ? options : [value]).map((m: string) => ({ value: m, label: m }))}
                />
                {res?.loading && <SabaSpinner size="sm" />}
                {res?.error && canAsk && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={load}>
                        {t('maintenance.retry')}
                    </button>
                )}
            </span>
            {errorKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t(errorKey)}
                </p>
            )}
            {res?.provider && !res.error && <ProviderBadge provider={res.provider} />}
            {fallback && <p className="dc-hint">{t('chatbot.model_fallback', { model: saved, fallback })}</p>}
        </div>
    );
}

/** 릴레이가 키로 알아낸 제공사 — 로고와 이름 */
export function ProviderBadge({ provider }: { provider: string }) {
    const { t } = useTranslation('gui');
    // 중국판이 따로 이름이 있으면(zhipu-cn) 그대로, 없으면 국제판 이름에 지역을 붙인다
    const own = PROVIDER_BRANDS[provider];
    const brand = own ?? PROVIDER_BRANDS[String(provider).replace(/-cn$/, '')];
    if (!brand) return null;
    const name =
        !own && String(provider).endsWith('-cn') ? t('chatbot.provider_region_cn', { name: brand.name }) : brand.name;
    return (
        <p className="cb-provider">
            <span
                className="cb-provider-logo"
                // biome-ignore lint/security/noDangerouslySetInnerHtml: 저장소에 넣은 고정 SVG (providerBrands.js)
                dangerouslySetInnerHTML={{ __html: brand.svg }}
            />
            {t('chatbot.provider_connected', { provider: name })}
        </p>
    );
}
