import clsx from 'clsx';
import { type MutableRefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import type { ApiBody, CommandInput, ModuleInfo } from '../../types/daemon';
import { failed } from '../../utils/apiResult';
import { commandErrorText } from '../../utils/commandReply';
import { commandArgs } from '../../utils/commandSyntax';
import { flashSetting } from '../../utils/findOnScreen';
import { choseong } from '../../utils/hangul';
import { safeShowToast } from '../../utils/helpers';
import { isImeKey } from '../../utils/ime';
import { openScreen } from '../../utils/openScreen';
import { commandInputLabel, commandText, interactionOf } from '../../utils/serverView';
import { botSettingsAvailable } from '../discord/BotSettingsTab';
import { chatbotAvailable } from '../discord/ChatbotView';
import { instanceWordsAvailable } from '../discord/InstanceWordsTab';
import { moderationAvailable } from '../discord/ModerationTab';
import { OPEN_SONAR_EVENT, startGuide, useGuideStore } from '../guides/guides';
import { Icon } from '../Icon';
import { maintenanceAvailable } from '../servers/MaintenanceTab';
import {
    CATEGORIES,
    type Category,
    type Control,
    categoriesOf,
    type Entity,
    entityRows,
    type Group,
    groupItems,
    type PaletteCommand,
    type PaletteData,
    type PaletteItem,
    visibleItems,
} from './paletteModel';

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** plan이 돌려주는 요약 한 줄 */
interface PlanSummary {
    label?: string;
    labelKey?: string;
    value?: unknown;
    before?: unknown;
}

/**
 * 명령 팔레트 (Ctrl+K, 사용자 11-10 시안 v5): Spotlight 모양의 검색 막대 + 카테고리 버튼(조작·게임 명령·바로 가기).
 * 항목 계산은 paletteModel. 바로 가기는 실행하지 않고 그 화면을 열고, 조작은 데몬 액션(plan → run),
 * 게임 명령은 지금 보낼 수 있는 것만(데몬 `/commands`의 ready) 칸을 하나씩 받아 보낸다.
 */
export const paletteAvailable = () => !!window.api?.actionsPlan && !!window.api?.actionsRun;

/** 계획 요약 값. 설정 변경은 `before`가 붙어 "이전 → 새 값"으로 보인다 (null이면 없음) */
export const summaryValue = (s: PlanSummary, t: Translate) =>
    'before' in s
        ? `${s.before == null ? t('palette.none') : String(s.before)} → ${String(s.value ?? '')}`
        : String(s.value ?? '');

const CONTROL_ACTION: Record<Control, string> = {
    start: 'server.start',
    stop: 'server.stop',
    restart: 'server.restart',
    backup: 'backup.create',
};
const CONTROL_ICON: Record<Control, string> = { start: 'play', stop: 'stop', restart: 'refresh', backup: 'save' };

function errorText(res: ApiBody | null | undefined, t: Translate) {
    if (res?.error_code === 'invalid_input') {
        return t('palette.invalid_input', { field: res.field || '?', reason: res.reason || '' });
    }
    if (res?.error_code === 'plan_expired') return t('palette.plan_expired');
    return res?.error_message || res?.error || res?.message || t('palette.failed');
}

/** 명령이 돌려준 답 중 사람이 읽을 한 줄 (없으면 null) */
function replyText(res: ApiBody | null | undefined) {
    const r = res?.data?.response;
    if (typeof r === 'string' && r.trim()) return r.trim();
    if (r && typeof r === 'object' && typeof r.raw === 'string' && r.raw.trim()) return r.raw.trim();
    return null;
}

/** 서버 화면에 있는 탭 (ServerDetail과 같은 조건) */
function serverTabs(server: Parameters<typeof interactionOf>[0], module?: ModuleInfo) {
    const interaction = interactionOf(server, module);
    return [
        interaction.console && 'console',
        interaction.commands && 'commands',
        'settings',
        'saba',
        maintenanceAvailable() && 'maintenance',
        instanceWordsAvailable() && 'discord',
        'info',
    ].filter(Boolean) as string[];
}

/** 디스코드 서버 화면에 있는 탭 (NodeView와 같은 조건) */
function guildTabs() {
    return [
        'instances',
        'members',
        moderationAvailable() && 'moderation',
        botSettingsAvailable() && 'bot',
        chatbotAvailable() && 'chatbot',
        botSettingsAvailable() && 'music',
        botSettingsAvailable() && 'tts',
    ].filter(Boolean) as string[];
}

/** 데몬 명령 목록 → 팔레트 명령 (모듈 locales 이름, module.toml 위험도 우선) */
function paletteCommands(list: ApiBody[], module: ModuleInfo | undefined, t: Translate): PaletteCommand[] {
    const fields = module?.commands?.fields || [];
    const moduleName = module?.name || '';
    return list.map((c) => {
        const own = c.source !== 'extension' ? fields.find((f) => f.name === c.name) : undefined;
        const text = commandText(t as never, moduleName, c as { name: string; label?: string; description?: string });
        const inputs: CommandInput[] = (c.inputs || own?.inputs || []).map((i: CommandInput) => ({
            ...i,
            label: commandInputLabel(t as never, moduleName, c.name, i),
        }));
        return {
            name: c.name,
            label: text.label,
            description: text.description,
            risk: own ? own.risk : c.risk === 'danger' ? 'danger' : undefined,
            inputs,
            ready: typeof c.ready === 'boolean' ? c.ready : undefined,
        };
    });
}

const optional = (i: CommandInput) => !i.required || i.default != null;

/** 입력 칸 후보 (고르는 칸만) */
function inputChoices(i: CommandInput, t: Translate) {
    if (i.type === 'boolean') {
        return [
            { value: 'true', label: t('palette.bool_on') },
            { value: 'false', label: t('palette.bool_off') },
        ];
    }
    if (Array.isArray(i.options) && i.options.length) return i.options.map((o) => ({ value: o, label: o }));
    return null;
}

interface Picked {
    serverId: string;
    command: PaletteCommand;
}
interface ArgValue {
    value: string;
    label: string;
}
type Mode = 'list' | 'entities' | 'args' | 'confirm' | 'controlConfirm' | 'busy' | 'done';

function Magnifier() {
    return (
        <svg
            className="palette-mag"
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
            strokeLinecap="round"
            aria-hidden="true"
        >
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" />
        </svg>
    );
}

function Palette({ onClose, busyRef }: { onClose: () => void; busyRef: MutableRefObject<boolean> }) {
    const { t } = useTranslation('gui');
    const tr = t as Translate;
    const activeTab = useUIStore((s) => s.activeTab);
    const activeServerId = useUIStore((s) => s.activeServerId);
    const openNodeId = useUIStore((s) => s.openDiscordNodeId);
    const servers = useServerStore((s) => s.servers);
    const modules = useServerStore((s) => s.modules);
    const cloudNodes = useDiscordStore((s) => s.cloudNodes);

    const [scope, setScopeState] = useState<Entity | null>(null);
    const [leaving, setLeaving] = useState(false);
    const [mode, setMode] = useState<Mode>('list');
    const [query, setQuery] = useState('');
    const [cursor, setCursor] = useState(0);
    const [category, setCategory] = useState<Category | null>(null);
    const [expanded, setExpanded] = useState<Partial<Record<Group, boolean>>>({});
    const [picked, setPicked] = useState<Picked | null>(null);
    const [args, setArgs] = useState<ArgValue[]>([]);
    const [armed, setArmed] = useState(false);
    const [control, setControl] = useState<{ item: PaletteItem; plan: ApiBody } | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [busyText, setBusyText] = useState('');
    const [done, setDone] = useState<{ title: string; line?: string | null } | null>(null);
    const [shake, setShake] = useState(0);
    const [commandLists, setCommandLists] = useState<Record<string, ApiBody[]>>({});
    const inputRef = useRef<HTMLInputElement>(null);
    const listRef = useRef<HTMLDivElement>(null);
    const hlRef = useRef<HTMLDivElement>(null);

    const busy = mode === 'busy';
    busyRef.current = busy;

    // 열 때 서버마다 명령 목록을 한 번 받는다 (지금 보낼 수 있는지는 데몬이 ready로 알려 준다)
    const serverIds = servers.map((s) => s.id).join('|');
    // biome-ignore lint/correctness/useExhaustiveDependencies: 서버 목록이 바뀔 때만
    useEffect(() => {
        if (!window.api?.instanceCommands) return;
        let alive = true;
        for (const s of servers) {
            Promise.resolve(window.api.instanceCommands(s.id))
                .then((r: ApiBody) => {
                    if (alive && Array.isArray(r?.commands))
                        setCommandLists((prev) => ({ ...prev, [s.id]: r.commands }));
                })
                .catch(() => {});
        }
        return () => {
            alive = false;
        };
    }, [serverIds]);

    // 옛 데몬·브리지라 명령 목록 API가 없으면 모듈 명령만 쓴다 (켜져 있으면 보낼 수 있다고 본다)
    const hasCatalog = !!window.api?.instanceCommands;
    const guides = useGuideStore((s) => s.guides);
    const data: PaletteData = useMemo(
        () => ({
            servers: servers.map((s) => {
                const module = modules.find((m) => m.name === s.module);
                const list = commandLists[s.id] ?? (hasCatalog ? undefined : module?.commands?.fields);
                return {
                    id: s.id,
                    name: s.name,
                    module: s.module,
                    status: s.status,
                    tabs: serverTabs(s, module),
                    commands: list ? paletteCommands(list as ApiBody[], module, tr) : undefined,
                    settings: (module?.settings?.fields || []).map((f) => ({
                        name: f.name,
                        label: tr(`mod_${s.module}:settings.${f.name}.label`, { defaultValue: f.label || f.name }),
                        tab: f.group === 'saba-chan' ? 'saba' : 'settings',
                        advanced: f.group === 'advanced',
                    })),
                };
            }),
            guilds: cloudNodes.map((n) => ({ id: n.guildId, name: n.guildName || n.guildId, tabs: guildTabs() })),
            guides: (guides ?? []).map((g) => ({ id: g.id, title: g.title, icon: g.icon })),
        }),
        [servers, modules, commandLists, cloudNodes, tr, hasCatalog, guides],
    );

    // 지금 열린 화면 (칩이 없을 때 빈 검색의 기준)
    const context: Entity | null =
        activeTab === 'servers' && activeServerId && data.servers.some((s) => s.id === activeServerId)
            ? { kind: 'server', id: activeServerId }
            : activeTab === 'discord' && openNodeId && data.guilds.some((g) => g.id === openNodeId)
              ? { kind: 'guild', id: openNodeId }
              : null;
    const contextKey = context ? `${context.kind}:${context.id}` : '';
    const serverOf = (id?: string) => data.servers.find((s) => s.id === id);
    const guildOf = (id?: string) => data.guilds.find((g) => g.id === id);
    const entityName = (e: Entity) => (e.kind === 'server' ? serverOf(e.id)?.name : guildOf(e.id)?.name) || e.id;

    const cats = categoriesOf({ scope, context }, data, tr);
    const activeCategory = category && cats.includes(category) ? category : null;
    // biome-ignore lint/correctness/useExhaustiveDependencies: context는 contextKey로 본다
    const items = useMemo(
        () =>
            mode === 'entities'
                ? entityRows(data, tr)
                : visibleItems({ scope, context, query, category: activeCategory }, data, tr),
        [mode, data, scope, contextKey, query, activeCategory, tr],
    );
    const groups = useMemo(
        () =>
            groupItems(items, {
                query,
                category: activeCategory,
                expanded: mode === 'entities' ? { esrv: true, eguild: true } : expanded,
            }),
        [items, query, activeCategory, expanded, mode],
    );
    const flat = groups.flatMap((g) => g.items);

    // 칸 입력 단계
    const pickServer = picked ? serverOf(picked.serverId) : undefined;
    const input = picked && mode === 'args' ? picked.command.inputs[args.length] : undefined;
    const choices = input ? inputChoices(input, tr) : null;
    const q = query.trim().toLowerCase();
    const shownChoices = (choices || []).filter(
        (c) =>
            !q ||
            c.value.toLowerCase().includes(q) ||
            c.label.toLowerCase().includes(q) ||
            choseong(c.label).includes(q),
    );
    const rowCount = mode === 'args' ? shownChoices.length : flat.length;
    const cur = Math.min(cursor, Math.max(0, rowCount - 1));
    const current = mode === 'args' ? null : flat[cur];

    // 선택 띠: 지금 줄 위치로 미끄러진다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 줄이 바뀔 때마다
    useLayoutEffect(() => {
        const list = listRef.current;
        const hl = hlRef.current;
        if (!list || !hl) return;
        const row = list.querySelector<HTMLElement>(`[data-row="${cur}"]`);
        if (!row) {
            hl.style.opacity = '0';
            return;
        }
        hl.style.opacity = '1';
        hl.style.height = `${row.offsetHeight}px`;
        hl.style.transform = `translateY(${row.offsetTop}px)`;
        if (row.offsetTop < list.scrollTop) list.scrollTop = row.offsetTop - 30;
        else if (row.offsetTop + row.offsetHeight > list.scrollTop + list.clientHeight)
            list.scrollTop = row.offsetTop + row.offsetHeight - list.clientHeight + 8;
    }, [cur, groups, shownChoices.length, mode]);

    const focusInput = () => setTimeout(() => inputRef.current?.focus({ preventScroll: true }), 0);
    const resetQuery = () => {
        setQuery('');
        setCursor(0);
    };
    const close = () => {
        if (!busy) onClose();
    };

    const setScope = (e: Entity | null) => {
        setScopeState(e);
        setLeaving(false);
        setMode('list');
        setExpanded({});
        setError(null);
        resetQuery();
        focusInput();
    };
    const clearScope = () => {
        if (!scope || leaving) return;
        setLeaving(true);
        setTimeout(() => setScope(null), 160);
    };

    const backToList = () => {
        if (busy) return;
        setMode('list');
        setPicked(null);
        setArgs([]);
        setArmed(false);
        setControl(null);
        setError(null);
        resetQuery();
        focusInput();
    };

    const startCommand = (serverId: string, command: PaletteCommand) => {
        setPicked({ serverId, command });
        setArgs([]);
        setArmed(false);
        setError(null);
        resetQuery();
        setMode(command.inputs.length ? 'args' : 'confirm');
        focusInput();
    };

    const acceptArg = (index = cur) => {
        if (!picked || !input) return;
        const typed = query.trim();
        let v: ArgValue | null = null;
        if (choices) v = shownChoices[index] ?? null;
        else if (typed) v = { value: typed, label: typed };
        if (!v && !typed && optional(input)) v = { value: '', label: tr('palette.arg_skipped') };
        if (input.type === 'number' && v?.value && !Number.isFinite(Number(v.value))) v = null;
        if (!v) {
            setShake((n) => n + 1);
            return;
        }
        const next = [...args, v];
        setArgs(next);
        resetQuery();
        if (next.length >= picked.command.inputs.length) setMode('confirm');
    };
    const editLastArg = () => {
        setArgs(args.slice(0, -1));
        setArmed(false);
        setError(null);
        setMode('args');
        focusInput();
    };

    const argValues = () => {
        const values: Record<string, unknown> = {};
        picked?.command.inputs.forEach((i, n) => {
            const v = args[n]?.value;
            if (v !== undefined && v !== '') values[i.name] = i.type === 'boolean' ? v === 'true' : v;
        });
        return values;
    };
    const commandLine = () => [picked?.command.name, ...args.map((a) => a.value).filter(Boolean)].join(' ');

    const sendCommand = async () => {
        if (!picked || busy) return;
        // 위험한 명령은 한 번 더 누른다
        if (picked.command.risk === 'danger' && !armed) {
            setArmed(true);
            return;
        }
        const name = pickServer?.name || picked.serverId;
        const line = commandLine();
        setBusyText(tr('palette.sending', { name }));
        setMode('busy');
        setError(null);
        try {
            const res = await window.api.executeCommand(picked.serverId, {
                command: picked.command.name,
                args: commandArgs(picked.command.inputs, argValues()),
            });
            if (failed(res) || res?.error) {
                setError(commandErrorText(res, t as never) ?? errorText(res, tr));
                setArmed(false);
                setMode('confirm');
                return;
            }
            setDone({ title: tr('palette.sent'), line: replyText(res) || line });
            setMode('done');
            setTimeout(() => {
                onClose();
                safeShowToast(tr('palette.sent_toast', { name, command: picked.command.label }), 'success', 3000);
            }, 900);
        } catch (e) {
            setError((e as Error).message);
            setArmed(false);
            setMode('confirm');
        }
    };

    const runControl = async (item: PaletteItem, plan?: ApiBody) => {
        const name = serverOf(item.serverId)?.name || item.serverId || '';
        const action = CONTROL_ACTION[item.control as Control];
        setBusyText(tr('palette.running', { name, action: item.title }));
        setMode('busy');
        setError(null);
        try {
            let planned = plan;
            if (!planned) {
                const res = await window.api.actionsPlan(action, { input: {}, instance: item.serverId });
                if (failed(res)) throw new Error(errorText(res, tr));
                // 정지·재시작처럼 데몬이 되묻는 조작은 한 번 더 확인한다
                if (res.needsConfirm) {
                    setControl({ item, plan: res });
                    setMode('controlConfirm');
                    return;
                }
                planned = res;
            }
            const res = await window.api.actionsRun(action, { planId: planned?.planId });
            if (failed(res)) throw new Error(errorText(res, tr));
            onClose();
            safeShowToast(tr('palette.control_done', { name, action: item.title }), 'success', 3000);
        } catch (e) {
            setError((e as Error).message);
            setMode(plan ? 'controlConfirm' : 'list');
        }
    };

    /** 바로 가기·설정 칸: 실행하지 않고 그 화면을 연다 (설정 칸이면 찾아서 반짝인다) */
    const jump = (item: PaletteItem) => {
        onClose();
        if (item.action === 'newServer') {
            useUIStore.getState().setShowModuleManager(true);
            return;
        }
        if (item.action === 'guide' && item.guide) {
            startGuide(item.guide);
            return;
        }
        if (!item.address) return;
        openScreen(item.address);
        if (item.findField) useUIStore.getState().requestFindSettingField(item.findField);
        if (item.find) flashSetting(item.find);
    };

    const choose = (item?: PaletteItem) => {
        if (!item || busy) return;
        switch (item.group) {
            case 'esrv':
                return setScope({ kind: 'server', id: item.serverId as string });
            case 'eguild':
                return setScope({ kind: 'guild', id: item.guildId as string });
            case 'xcmd':
                // 다른 서버의 같은 명령: 그 서버로 범위를 바꾸고 이어서 입력한다
                setScope({ kind: 'server', id: item.serverId as string });
                return startCommand(item.serverId as string, item.command as PaletteCommand);
            case 'cmd':
                return startCommand(item.serverId as string, item.command as PaletteCommand);
            case 'ctl':
                return runControl(item);
            default:
                return jump(item);
        }
    };

    const toggleCategory = (c: Category) => {
        if (mode !== 'list' || !cats.includes(c)) return;
        setCategory((prev) => (prev === c ? null : c));
        setCursor(0);
    };

    const onKeyDown = (e: React.KeyboardEvent) => {
        if (isImeKey(e)) return;
        if (busy || mode === 'done') {
            if (e.key === 'Escape') e.preventDefault();
            return;
        }
        if ((e.ctrlKey || e.metaKey) && ['1', '2', '3'].includes(e.key)) {
            e.preventDefault();
            toggleCategory(CATEGORIES[Number(e.key) - 1]);
            return;
        }
        if (e.key === 'Escape') {
            e.preventDefault();
            if (mode === 'list') close();
            else backToList();
            return;
        }
        if (e.key === 'Tab' && mode === 'list') {
            e.preventDefault();
            setMode('entities');
            resetQuery();
            return;
        }
        if (mode === 'confirm' || mode === 'controlConfirm') {
            if (e.key === 'Enter') {
                e.preventDefault();
                if (mode === 'confirm') sendCommand();
                else if (control) runControl(control.item, control.plan);
            } else if (e.key === 'Backspace' && mode === 'confirm' && args.length) {
                e.preventDefault();
                editLastArg();
            }
            return;
        }
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setCursor(Math.min(cur + 1, rowCount - 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setCursor(Math.max(cur - 1, 0));
        } else if (e.key === 'Enter') {
            e.preventDefault();
            if (mode === 'args') acceptArg();
            else choose(flat[cur]);
        } else if (e.key === 'Backspace' && !query) {
            if (mode === 'args' && args.length) {
                e.preventDefault();
                setArgs(args.slice(0, -1));
            } else if (mode === 'list' && scope) {
                e.preventDefault();
                clearScope();
            }
        }
    };

    const statusText = (status?: string) =>
        tr(
            status === 'running'
                ? 'palette.status_running'
                : status === 'stopped'
                  ? 'palette.status_stopped'
                  : 'palette.status_busy',
        );
    const placeholder =
        mode === 'entities'
            ? tr('palette.placeholder_entities')
            : input
              ? optional(input)
                  ? tr('palette.arg_optional', { label: input.label || input.name })
                  : input.label || input.name
              : mode === 'list'
                ? scope
                    ? tr('palette.placeholder_scope', { name: entityName(scope) })
                    : tr('palette.placeholder')
                : '';
    const ctxName = !scope && !q && context && mode === 'list' ? ` · ${entityName(context)}` : '';

    const subOf = (it: PaletteItem) => {
        const s = serverOf(it.serverId);
        switch (it.group) {
            case 'eguild':
                return tr('palette.discord_server');
            case 'esrv':
                return `${tr('palette.game_server')} · ${statusText(s?.status)}`;
            case 'xcmd':
                return `${s?.name ?? ''} · ${statusText(s?.status)}`;
            case 'ctl':
                return statusText(s?.status);
            case 'cmd':
                return it.command?.inputs.length
                    ? it.command.inputs
                          .map((i) => (optional(i) ? `${i.label} (${tr('palette.optional')})` : i.label))
                          .join(', ')
                    : tr('palette.no_inputs');
            default:
                // 가는 길에서 제목과 같은 조각은 뺀다 (서버 탭이면 서버 이름, 설정 칸이면 서버 › 탭)
                return (it.path || []).filter((p) => p !== it.title).join(' › ');
        }
    };
    const ACT: Record<Group, string> = {
        ctl: 'palette.act.run',
        cmd: 'palette.act.input',
        xcmd: 'palette.act.switch',
        jump: 'palette.act.open',
        setting: 'palette.act.open',
        esrv: 'palette.act.scope',
        eguild: 'palette.act.scope',
    };
    const iconOf = (it: PaletteItem, big = false) => {
        const cls = (...c: (string | false | undefined)[]) => clsx('palette-ico', { big }, ...c);
        switch (it.group) {
            case 'ctl':
                return (
                    <span className={cls('ctl', it.control === 'stop' && 'stop')}>
                        <Icon name={CONTROL_ICON[it.control as Control]} size="sm" />
                    </span>
                );
            case 'cmd':
            case 'xcmd':
                return (
                    <span className={cls('cmd')}>
                        <Icon name="terminal" size="sm" />
                    </span>
                );
            case 'setting':
                return (
                    <span className={cls('set')}>
                        <Icon name="settings" size="sm" />
                    </span>
                );
            case 'esrv':
                return (
                    <span className={cls('srv')}>
                        <span className={clsx('palette-dot', serverOf(it.serverId)?.status)} />
                    </span>
                );
            case 'eguild':
                return <span className={cls('gld')}>{Array.from(it.title)[0]}</span>;
            default:
                return (
                    <span className={cls(it.serverId || it.guildId ? 'jump' : 'app')}>
                        <Icon name={it.icon ?? (it.action === 'newServer' ? 'plus' : 'externalLink')} size="sm" />
                    </span>
                );
        }
    };

    const commandPreview = (c: PaletteCommand, serverName?: string) => (
        <>
            <span className="palette-ico big cmd">
                <Icon name="terminal" size="sm" />
            </span>
            <h3>{c.label}</h3>
            {c.description && <p>{c.description}</p>}
            <div className="palette-cmdline">
                {c.name}
                {c.inputs.map((i) => (
                    <span key={i.name} className="ph">
                        {` <${i.label}>`}
                    </span>
                ))}
            </div>
            {c.inputs.length ? (
                <dl>
                    {c.inputs.map((i) => (
                        <div key={i.name} className="palette-dl-row">
                            <dt>{i.label}</dt>
                            <dd>
                                {inputChoices(i, tr)
                                    ?.map((o) => o.label)
                                    .join(' · ') ||
                                    (i.placeholder
                                        ? tr('palette.example', { value: i.placeholder })
                                        : tr('palette.free_input'))}
                                {optional(i) ? ` · ${tr('palette.optional')}` : ''}
                            </dd>
                        </div>
                    ))}
                </dl>
            ) : (
                <p>{tr('palette.no_inputs_hint')}</p>
            )}
            {serverName && (
                <div className="palette-why">
                    <Icon name="check" size="xs" /> {tr('palette.pv_ready', { name: serverName })}
                </div>
            )}
        </>
    );

    const preview = (it?: PaletteItem | null) => {
        if (picked && mode === 'args') return commandPreview(picked.command, pickServer?.name);
        if (!it) return null;
        const s = serverOf(it.serverId);
        if (it.group === 'eguild' || it.group === 'esrv') {
            const cmds = s ? (s.commands || []).filter((c) => c.ready ?? s.status === 'running').length : 0;
            return (
                <>
                    {iconOf(it, true)}
                    <h3>{it.title}</h3>
                    {s && (
                        <dl>
                            <div className="palette-dl-row">
                                <dt>{tr('palette.status')}</dt>
                                <dd>
                                    <span className={clsx('palette-pill', { ok: s.status === 'running' })}>
                                        {statusText(s.status)}
                                    </span>
                                </dd>
                            </div>
                            <div className="palette-dl-row">
                                <dt>{tr('palette.group.cmd')}</dt>
                                <dd>
                                    {cmds ? tr('palette.commands_count', { count: cmds }) : tr('palette.commands_none')}
                                </dd>
                            </div>
                        </dl>
                    )}
                    <p>{tr('palette.pv_scope')}</p>
                </>
            );
        }
        if (it.group === 'xcmd' && it.command && s) {
            return (
                <>
                    {iconOf(it, true)}
                    <h3>{it.title}</h3>
                    {it.command.description && <p>{it.command.description}</p>}
                    <p>{tr('palette.pv_switch', { name: s.name })}</p>
                    <div className="palette-why">
                        <Icon name="check" size="xs" /> {tr('palette.pv_ready', { name: s.name })}
                    </div>
                </>
            );
        }
        if (it.group === 'cmd' && it.command) return commandPreview(it.command, s?.name);
        if (it.group === 'ctl') {
            return (
                <>
                    {iconOf(it, true)}
                    <h3>
                        {s?.name} {it.title}
                    </h3>
                    <p>{tr(`palette.control_desc.${it.control}`)}</p>
                    <dl>
                        <div className="palette-dl-row">
                            <dt>{tr('palette.status')}</dt>
                            <dd>
                                <span className={clsx('palette-pill', { ok: s?.status === 'running' })}>
                                    {statusText(s?.status)}
                                </span>
                            </dd>
                        </div>
                    </dl>
                </>
            );
        }
        return (
            <>
                {iconOf(it, true)}
                <h3>{it.title}</h3>
                {it.path && (
                    <div className="palette-crumb">
                        {it.path.map((p, i) => (
                            // biome-ignore lint/suspicious/noArrayIndexKey: 고정된 경로 조각
                            <span key={i}>{p}</span>
                        ))}
                    </div>
                )}
                <p>{tr(it.group === 'setting' ? 'palette.pv_setting' : 'palette.pv_jump')}</p>
            </>
        );
    };

    const foot = () => {
        const k = (key: string, label: string) => (
            <span key={key + label}>
                <kbd>{key}</kbd>
                {label}
            </span>
        );
        if (mode === 'args')
            return [
                k('↵', tr('palette.foot.next_field')),
                k('⌫', tr('palette.foot.prev_field')),
                k('Esc', tr('palette.foot.to_list')),
            ];
        if (mode === 'confirm' || mode === 'controlConfirm')
            return [k('↵', tr('palette.foot.run')), k('Esc', tr('palette.foot.back'))];
        if (mode === 'busy' || mode === 'done') return [];
        if (mode === 'entities')
            return [
                k('↑↓', tr('palette.foot.move')),
                k('↵', tr('palette.foot.to_scope')),
                k('Esc', tr('palette.foot.keep')),
            ];
        return [
            k('↑↓', tr('palette.foot.move')),
            k('↵', tr('palette.foot.open')),
            k('Tab', tr('palette.foot.pick_scope')),
            ...(scope ? [k('⌫', tr('palette.foot.clear_scope'))] : []),
            k('Ctrl 1·2·3', tr('palette.foot.categories')),
            k('Esc', tr('palette.foot.close')),
            <span key="cho" className="r">
                {tr('palette.foot.choseong')}
            </span>,
        ];
    };

    const stage = mode === 'confirm' || mode === 'controlConfirm' || mode === 'busy' || mode === 'done';
    const danger = picked?.command.risk === 'danger';
    const showGo = mode === 'args' || mode === 'confirm';
    let rowIndex = 0;

    const row = (
        i: number,
        key: string,
        content: React.ReactNode,
        onPick: () => void,
        delay = Math.min(i, 14) * 18,
    ) => (
        <div
            key={key}
            id={`palette-opt-${i}`}
            data-row={i}
            role="option"
            aria-selected={i === cur}
            tabIndex={-1}
            className={clsx('palette-row', { on: i === cur })}
            style={{ animationDelay: `${delay}ms` }}
            onMouseMove={() => i !== cur && setCursor(i)}
            onClick={onPick}
            onKeyDown={() => {}}
        >
            {content}
        </div>
    );

    return (
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 누르면 닫는다
        <div className="palette-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
            <div
                className="palette"
                role="dialog"
                aria-modal="true"
                aria-label={tr('palette.title')}
                onKeyDown={onKeyDown}
            >
                <div className="palette-top">
                    <div key={shake} className={clsx('palette-bar palette-glass', { shake: shake > 0 })}>
                        <Magnifier />
                        <div className="palette-tokens">
                            {scope && (
                                <button
                                    type="button"
                                    className={clsx('palette-tok scope', { leaving })}
                                    aria-label={`${entityName(scope)} · ${tr('palette.chip_clear')}`}
                                    onClick={() => {
                                        if (mode !== 'list') backToList();
                                        clearScope();
                                    }}
                                >
                                    {scope.kind === 'server' ? (
                                        <span className={clsx('palette-dot', serverOf(scope.id)?.status)} />
                                    ) : (
                                        <span className="palette-gmark">{Array.from(entityName(scope))[0]}</span>
                                    )}
                                    {entityName(scope)}
                                    <span className="x" aria-hidden="true">
                                        ×
                                    </span>
                                </button>
                            )}
                            {picked && mode !== 'list' && (
                                <span className="palette-tok">
                                    {scope ? '' : `${pickServer?.name ?? ''} · `}
                                    {picked.command.label}
                                </span>
                            )}
                            {picked &&
                                mode !== 'list' &&
                                args.map((a, i) => (
                                    // biome-ignore lint/suspicious/noArrayIndexKey: 칸 순서대로 쌓인다
                                    <span key={i} className="palette-tok arg">
                                        <em>{picked.command.inputs[i]?.label}</em>
                                        {a.label}
                                    </span>
                                ))}
                        </div>
                        <input
                            ref={inputRef}
                            // biome-ignore lint/a11y/noAutofocus: 팔레트를 열면 바로 친다
                            autoFocus
                            value={query}
                            readOnly={stage}
                            onChange={(e) => {
                                setQuery(e.target.value);
                                setCursor(0);
                                if (mode === 'entities') setMode('list');
                            }}
                            placeholder={placeholder}
                            aria-label={tr('palette.search_label')}
                            role="combobox"
                            aria-expanded="true"
                            aria-controls="palette-list"
                            aria-activedescendant={rowCount && !stage ? `palette-opt-${cur}` : undefined}
                            spellCheck={false}
                            autoComplete="off"
                            type={input?.type === 'password' ? 'password' : 'text'}
                        />
                        {showGo && (
                            <button
                                type="button"
                                className={clsx('palette-go', { danger: mode === 'confirm' && danger })}
                                onClick={() => (mode === 'confirm' ? sendCommand() : acceptArg())}
                            >
                                {mode === 'args'
                                    ? `${tr('palette.next')} ↵`
                                    : armed
                                      ? `${tr('palette.again')} ↵`
                                      : danger
                                        ? `${picked?.command.label} ↵`
                                        : `${tr('palette.send')} ↵`}
                            </button>
                        )}
                    </div>
                    <div className="palette-cats">
                        {CATEGORIES.filter((c) => cats.includes(c)).map((c) => (
                            <button
                                key={c}
                                type="button"
                                className={clsx('palette-cat palette-glass', { on: activeCategory === c })}
                                aria-label={tr(`palette.category.${c}`)}
                                aria-pressed={activeCategory === c}
                                onClick={() => toggleCategory(c)}
                            >
                                <Icon
                                    name={c === 'ctl' ? 'power' : c === 'cmd' ? 'terminal' : 'externalLink'}
                                    size="md"
                                />
                                <span className="tip">{tr(`palette.category.${c}`)}</span>
                            </button>
                        ))}
                    </div>
                </div>

                <div className={clsx('palette-res palette-glass', { single: stage })}>
                    {stage ? (
                        <div className="palette-stage">
                            {mode === 'busy' && (
                                <div className="palette-done">
                                    <div className="palette-spin" />
                                    <span className="muted">{busyText}</span>
                                </div>
                            )}
                            {mode === 'done' && done && (
                                <div className="palette-done ok">
                                    <svg
                                        width="64"
                                        height="64"
                                        viewBox="0 0 64 64"
                                        fill="none"
                                        stroke="currentColor"
                                        strokeWidth="4"
                                        strokeLinecap="round"
                                        strokeLinejoin="round"
                                        aria-hidden="true"
                                    >
                                        <circle cx="32" cy="32" r="25" />
                                        <path d="m21 33 8 8 15-17" />
                                    </svg>
                                    <b>{done.title}</b>
                                    {done.line && <span className="palette-cmdline">{done.line}</span>}
                                </div>
                            )}
                            {mode === 'confirm' && picked && (
                                <>
                                    <h3>{tr('palette.confirm_send', { name: pickServer?.name ?? '' })}</h3>
                                    <div className="palette-cmdline">{commandLine()}</div>
                                    {danger && (
                                        <div className="palette-warn">
                                            <Icon name="alertTriangle" size="sm" /> {tr('palette.danger_warn')}
                                        </div>
                                    )}
                                    {error && <p className="palette-error">{error}</p>}
                                    <div className="palette-btns">
                                        <button
                                            type="button"
                                            className="palette-btn"
                                            onClick={() => (args.length ? editLastArg() : backToList())}
                                        >
                                            {tr('palette.edit')}
                                        </button>
                                        <button
                                            type="button"
                                            className={clsx('palette-btn', danger ? 'dan' : 'pri', { armed })}
                                            onClick={sendCommand}
                                        >
                                            {armed
                                                ? tr('palette.send_again')
                                                : danger
                                                  ? picked.command.label
                                                  : tr('palette.send')}
                                        </button>
                                    </div>
                                </>
                            )}
                            {mode === 'controlConfirm' && control && (
                                <>
                                    <h3>
                                        {tr('palette.confirm_title', {
                                            action: `${serverOf(control.item.serverId)?.name ?? ''} ${control.item.title}`,
                                        })}
                                    </h3>
                                    {Array.isArray(control.plan.summary) && control.plan.summary.length > 0 && (
                                        <dl className="palette-summary">
                                            {control.plan.summary.map((s: PlanSummary, i: number) => (
                                                // biome-ignore lint/suspicious/noArrayIndexKey: 읽기 전용 목록
                                                <div key={i} className="palette-dl-row">
                                                    <dt>{s.label || s.labelKey}</dt>
                                                    <dd>{summaryValue(s, tr)}</dd>
                                                </div>
                                            ))}
                                        </dl>
                                    )}
                                    {error && <p className="palette-error">{error}</p>}
                                    <div className="palette-btns">
                                        <button type="button" className="palette-btn" onClick={backToList}>
                                            {tr('modals.cancel')}
                                        </button>
                                        <button
                                            type="button"
                                            className={clsx(
                                                'palette-btn',
                                                control.item.control === 'stop' || control.item.control === 'restart'
                                                    ? 'dan'
                                                    : 'pri',
                                            )}
                                            onClick={() => runControl(control.item, control.plan)}
                                        >
                                            {control.item.title}
                                        </button>
                                    </div>
                                </>
                            )}
                        </div>
                    ) : (
                        <>
                            <div
                                className="palette-list"
                                id="palette-list"
                                role="listbox"
                                aria-label={tr('palette.title')}
                                ref={listRef}
                            >
                                <div className="palette-hl" ref={hlRef} />
                                {mode === 'args' && input ? (
                                    <>
                                        <div className="palette-grp">
                                            {input.label}
                                            {optional(input) && <span className="n">{tr('palette.optional')}</span>}
                                        </div>
                                        {shownChoices.map((c, i) =>
                                            row(
                                                i,
                                                c.value,
                                                <>
                                                    <span className="palette-ico">{Array.from(c.label)[0]}</span>
                                                    <span className="palette-txt">
                                                        <b>{c.label}</b>
                                                        {c.label !== c.value && <small>{c.value}</small>}
                                                    </span>
                                                    <span className="palette-meta">
                                                        <span className="ret">↵ {tr('palette.act.pick')}</span>
                                                    </span>
                                                </>,
                                                () => acceptArg(i),
                                                i * 22,
                                            ),
                                        )}
                                        {!choices && (
                                            <div className="palette-hint">
                                                {tr('palette.arg_hint', { label: input.label })}
                                                {input.placeholder &&
                                                    ` ${tr('palette.example', { value: input.placeholder })}`}
                                                {input.type === 'number' && ` ${tr('palette.arg_number')}`}
                                                {optional(input) && (
                                                    <>
                                                        <br />
                                                        {tr('palette.arg_skip')}
                                                    </>
                                                )}
                                            </div>
                                        )}
                                        {choices && !shownChoices.length && (
                                            <div className="palette-none">{tr('palette.empty')}</div>
                                        )}
                                    </>
                                ) : (
                                    <>
                                        {groups.map((g) => (
                                            <div key={g.group} className="palette-section">
                                                <div className="palette-grp">
                                                    {tr(`palette.group.${g.group}`)}
                                                    {['ctl', 'cmd', 'jump'].includes(g.group) ? ctxName : ''}
                                                    <span className="n">{g.total}</span>
                                                </div>
                                                {g.items.map((it) => {
                                                    const i = rowIndex++;
                                                    const risk = it.command?.risk;
                                                    return row(
                                                        i,
                                                        it.key,
                                                        <>
                                                            {iconOf(it)}
                                                            <span className="palette-txt">
                                                                <b>{it.title}</b>
                                                                <small>{subOf(it)}</small>
                                                            </span>
                                                            <span className="palette-meta">
                                                                {(risk === 'danger' || risk === 'caution') && (
                                                                    <span
                                                                        className={clsx(
                                                                            'palette-pill',
                                                                            risk === 'danger' ? 'danger' : 'warn',
                                                                        )}
                                                                    >
                                                                        {tr(`palette.risk.${risk}`)}
                                                                    </span>
                                                                )}
                                                                <span className="ret">↵ {tr(ACT[it.group])}</span>
                                                            </span>
                                                        </>,
                                                        () => {
                                                            setCursor(i);
                                                            choose(it);
                                                        },
                                                    );
                                                })}
                                                {g.total > g.items.length && (
                                                    <button
                                                        type="button"
                                                        className="palette-more"
                                                        onClick={() =>
                                                            setExpanded((prev) => ({ ...prev, [g.group]: true }))
                                                        }
                                                    >
                                                        {tr('palette.more', {
                                                            group: tr(`palette.group.${g.group}`),
                                                            count: g.total - g.items.length,
                                                        })}
                                                    </button>
                                                )}
                                            </div>
                                        ))}
                                        {!flat.length && (
                                            <div className="palette-none">
                                                {tr('palette.empty')}
                                                <br />
                                                <small>{tr('palette.empty_hint')}</small>
                                            </div>
                                        )}
                                        {error && <p className="palette-error">{error}</p>}
                                    </>
                                )}
                            </div>
                            <aside
                                className="palette-pv"
                                key={mode === 'args' ? `args:${picked?.command.name}` : (current?.key ?? '')}
                            >
                                {preview(current)}
                            </aside>
                        </>
                    )}
                    <footer className="palette-foot">{foot()}</footer>
                </div>
            </div>
        </div>
    );
}

/** Ctrl+K (macOS ⌘K)로 여닫는다. 액션 IPC가 없으면 아무것도 하지 않는다 */
export function CommandPalette() {
    const [open, setOpen] = useState(false);
    const busyRef = useRef(false);

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
                e.preventDefault();
                setOpen((o) => (o && busyRef.current ? o : !o));
            }
        };
        // 가이드가 소나를 연다
        const onOpen = () => setOpen(true);
        window.addEventListener('keydown', onKey);
        window.addEventListener(OPEN_SONAR_EVENT, onOpen);
        return () => {
            window.removeEventListener('keydown', onKey);
            window.removeEventListener(OPEN_SONAR_EVENT, onOpen);
        };
    }, []);

    return open ? (
        <Palette
            busyRef={busyRef}
            onClose={() => {
                busyRef.current = false;
                setOpen(false);
            }}
        />
    ) : null;
}
