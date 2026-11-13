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
import { startGuide, useGuideStore } from '../guides/guides';
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
    type Translate,
    visibleItems,
} from './paletteModel';
import { inputChoices, type Mode, optional } from './paletteView';

/*
 * 명령 팔레트의 상태와 동작 (CommandPalette는 이 훅이 주는 값으로 그리기만 한다).
 * 서버마다 명령 목록 받기, 항목 계산(paletteModel), 단계(mode) 전환, 칸 입력, 명령 보내기·조작 실행, 키 처리.
 */

const CONTROL_ACTION: Record<Control, string> = {
    start: 'server.start',
    stop: 'server.stop',
    restart: 'server.restart',
    backup: 'backup.create',
};

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
            ready: c.ready === true,
        };
    });
}

interface Picked {
    serverId: string;
    command: PaletteCommand;
}
interface ArgValue {
    value: string;
    label: string;
}

export function usePalette(onClose: () => void, busyRef: MutableRefObject<boolean>) {
    const { t } = useTranslation('gui');
    // i18next 27의 TFunction은 키를 리소스 타입으로 좁힌다: 명령 이름 같은 동적 키를 받으려고 넓힌다
    const tr = t as unknown as Translate;
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

    const guides = useGuideStore((s) => s.guides);
    const data: PaletteData = useMemo(
        () => ({
            servers: servers.map((s) => {
                const module = modules.find((m) => m.name === s.module);
                const list = commandLists[s.id];
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
        [servers, modules, commandLists, cloudNodes, tr, guides],
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

    const typeQuery = (value: string) => {
        setQuery(value);
        setCursor(0);
        if (mode === 'entities') setMode('list');
    };
    const expandGroup = (group: Group) => setExpanded((prev) => ({ ...prev, [group]: true }));

    return {
        tr,
        mode,
        scope,
        leaving,
        context,
        query,
        q,
        cur,
        rowCount,
        current,
        cats,
        activeCategory,
        groups,
        flat,
        input,
        choices,
        shownChoices,
        picked,
        pickServer,
        args,
        armed,
        control,
        error,
        busyText,
        done,
        shake,
        inputRef,
        listRef,
        hlRef,
        serverOf,
        entityName,
        close,
        backToList,
        clearScope,
        acceptArg,
        editLastArg,
        commandLine,
        sendCommand,
        runControl,
        choose,
        toggleCategory,
        onKeyDown,
        typeQuery,
        expandGroup,
        setCursor,
    };
}
