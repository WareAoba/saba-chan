/**
 * 명령 팔레트 항목 계산 (사용자 11-10 시안 v5). 화면과 떼어 둔 순수 함수다.
 *
 * - 범위(scope): 검색 막대 앞 칩. 게임 서버나 디스코드 서버 하나를 고르면 그 안에서 할 수 있는 것만 보인다.
 * - 지금 화면(context): 칩이 없을 때 빈 검색의 기준. 서버 화면이면 그 서버, 디스코드 서버 화면이면 그 서버.
 * - 묶음: 조작(시작·정지 등) → 게임 명령 → 다른 서버의 같은 명령 → 바로 가기 → 설정 칸 → 게임 서버 → 디스코드 서버.
 * - 게임 명령은 지금 보낼 수 있는 것만 (데몬 `/commands`의 `ready`).
 * - 바로 가기·설정 칸은 실행하지 않고 그 화면으로 간다 (`utils/screenAddress` 주소).
 */
import type { CommandInput } from '../../types/daemon';
import { matchScore } from '../../utils/hangul';
import { SETTINGS_TAB_LABELS, SETTINGS_TABS } from '../Modals/settingsTabs';

type Translate = (key: string, options?: Record<string, unknown>) => string;

export type Group = 'ctl' | 'cmd' | 'xcmd' | 'jump' | 'setting' | 'esrv' | 'eguild';
export type Category = 'ctl' | 'cmd' | 'jump';
export const CATEGORIES: Category[] = ['ctl', 'cmd', 'jump'];
export const GROUP_ORDER: Group[] = ['ctl', 'cmd', 'xcmd', 'jump', 'setting', 'esrv', 'eguild'];
export type Control = 'start' | 'stop' | 'restart' | 'backup';
export interface Entity {
    kind: 'server' | 'guild';
    id: string;
}

export interface PaletteCommand {
    name: string;
    label: string;
    description?: string | null;
    risk?: string;
    inputs: CommandInput[];
    /** 데몬 판정: 지금 보내면 받는가. 옛 데몬이라 없으면 켜져 있는지로 본다 */
    ready?: boolean;
}

export interface PaletteServer {
    id: string;
    name: string;
    module: string;
    status: string;
    /** 이 서버 화면에 있는 탭 (console·commands·settings·saba·maintenance·discord·info) */
    tabs: string[];
    /** 불러온 명령 (번역한 이름). 아직 없으면 명령 줄이 없다 */
    commands?: PaletteCommand[];
    /** 모듈 설정 칸 (번역한 이름). tab = 그 칸이 있는 서버 탭 */
    settings?: { name: string; label: string; tab: string; advanced?: boolean }[];
}

export interface PaletteGuild {
    id: string;
    name: string;
    /** 이 디스코드 서버 화면에 있는 탭 (instances·members·moderation·bot·chatbot·music·tts) */
    tabs: string[];
}

export interface PaletteData {
    servers: PaletteServer[];
    guilds: PaletteGuild[];
}

export interface PaletteItem {
    group: Group;
    key: string;
    title: string;
    /** 검색 대상 글 */
    hay: string;
    /** 가는 길 (미리보기·둘째 줄) */
    path?: string[];
    serverId?: string;
    guildId?: string;
    control?: Control;
    command?: PaletteCommand;
    /** 바로 가기·설정 칸: 열 화면 주소 */
    address?: string;
    /** 설정 칸: 화면을 연 뒤 찾아 반짝일 칸 이름 */
    find?: string;
    /** 서버 설정 칸 이름 (고급 묶음이면 펼친다) */
    findField?: string;
    action?: 'newServer';
    /** 빈 검색에도 보이는 앱 바로 가기 */
    featured?: boolean;
}

const SERVER_TAB_LABELS: Record<string, string> = {
    console: 'shell.tab_console',
    commands: 'shell.tab_commands',
    settings: 'shell.tab_settings',
    saba: 'shell.tab_saba',
    maintenance: 'shell.tab_maintenance',
    discord: 'shell.tab_discord',
    info: 'shell.tab_info',
};

/** 디스코드 서버 탭 이름: 화면(NodeView)과 같은 키 (말하는·노래하는·읽어주는 사바쨩) */
export const GUILD_TAB_LABELS: Record<string, string> = {
    instances: 'discord.tab_instances',
    members: 'discord.tab_members',
    moderation: 'discord.tab_moderation',
    bot: 'discord.tab_bot',
    chatbot: 'chatbot.nav',
    music: 'discord.bot_settings.music_title',
    tts: 'discord.bot_settings.tts_title',
};

/** 앱 설정 칸: 설정 창 탭의 칸 이름 (`settings_modal.*_label`, 화면과 같은 키) */
const APP_SETTINGS: [string, string][] = [
    ['general', 'language_label'],
    ['general', 'auto_refresh_label'],
    ['general', 'refresh_interval_label'],
    ['general', 'close_action_label'],
    ['notify', 'os_notifications_label'],
    ['notify', 'sound_effects_label'],
    ['notify', 'toast_duration_label'],
    ['appearance', 'theme_label'],
    ['appearance', 'accent_color_label'],
    ['appearance', 'gradient_label'],
    ['appearance', 'font_scale_label'],
    ['appearance', 'transitions_label'],
    ['appearance', 'sidebar_compact_label'],
    ['console', 'console_fonts_label'],
    ['console', 'console_timestamps_label'],
    ['console', 'console_timestamp_date_label'],
    ['console', 'console_wrap_label'],
    ['console', 'console_auto_scroll_label'],
    ['console', 'console_buffer_label'],
    ['console', 'console_token_colors_label'],
    ['servers', 'auto_generate_passwords_label'],
    ['servers', 'port_conflict_check_label'],
    ['servers', 'auto_port_check_label'],
    ['servers', 'new_server_defaults_label'],
    ['servers', 'audit_keep_label'],
    ['connection', 'ipc_port_label'],
    ['connection', 'relay_url_label'],
];
const STORAGE_TABS = ['components', 'modules', 'extensions'];

/** 다른 서버 명령은 명령 카테고리, 설정 칸은 바로 가기 카테고리. 범위 줄은 카테고리가 없다 */
export const categoryOf = (g: Group): Category | null =>
    g === 'xcmd' ? 'cmd' : g === 'setting' ? 'jump' : g === 'esrv' || g === 'eguild' ? null : g;

const commandReady = (s: PaletteServer, c: PaletteCommand) => c.ready ?? s.status === 'running';

/** 지금 상태에서 할 수 있는 조작 */
function controlsOf(status: string): Control[] {
    if (status === 'running') return ['stop', 'restart', 'backup'];
    if (status === 'stopped') return ['start', 'backup'];
    return [];
}

function commandItem(group: 'cmd' | 'xcmd', s: PaletteServer, c: PaletteCommand): PaletteItem {
    return {
        group,
        key: `${group}:${s.id}:${c.name}`,
        title: c.label || c.name,
        hay: `${c.label} ${c.name}`,
        serverId: s.id,
        command: c,
        path: [s.name],
    };
}

function serverItems(s: PaletteServer, t: Translate): PaletteItem[] {
    const out: PaletteItem[] = [];
    for (const c of controlsOf(s.status)) {
        const title = t(`palette.control.${c}`);
        out.push({
            group: 'ctl',
            key: `ctl:${s.id}:${c}`,
            title,
            hay: `${title} ${c}`,
            serverId: s.id,
            control: c,
            path: [s.name],
        });
    }
    for (const c of s.commands || []) if (commandReady(s, c)) out.push(commandItem('cmd', s, c));
    for (const tab of s.tabs) {
        const title = t(SERVER_TAB_LABELS[tab] || tab);
        out.push({
            group: 'jump',
            key: `jump:${s.id}:${tab}`,
            title,
            hay: `${title} ${tab}`,
            serverId: s.id,
            address: `server/${s.id}/${tab}`,
            path: [s.name, title],
        });
    }
    for (const f of s.settings || []) {
        if (!s.tabs.includes(f.tab)) continue;
        const tabName = t(SERVER_TAB_LABELS[f.tab] || f.tab);
        out.push({
            group: 'setting',
            key: `setting:${s.id}:${f.name}`,
            title: f.label,
            hay: `${f.label} ${f.name}`,
            serverId: s.id,
            address: `server/${s.id}/${f.tab}`,
            find: f.label,
            findField: f.name,
            path: [s.name, tabName],
        });
    }
    return out;
}

function guildItems(g: PaletteGuild, t: Translate): PaletteItem[] {
    return g.tabs.map((tab) => {
        const title = t(GUILD_TAB_LABELS[tab] || tab);
        return {
            group: 'jump' as const,
            key: `jump:${g.id}:${tab}`,
            title,
            hay: `${title} ${tab}`,
            guildId: g.id,
            address: `discord/${g.id}/${tab}`,
            path: [g.name, title],
        };
    });
}

function appItems(t: Translate): PaletteItem[] {
    const settings = t('shell.nav_settings');
    const out: PaletteItem[] = [
        {
            group: 'jump',
            key: 'app:new',
            title: t('shell.add_server'),
            hay: t('shell.add_server'),
            action: 'newServer',
            featured: true,
        },
        {
            group: 'jump',
            key: 'app:discord',
            title: t('shell.nav_discord'),
            hay: `${t('shell.nav_discord')} discord`,
            address: 'discord',
            featured: true,
        },
        {
            group: 'jump',
            key: 'app:activity',
            title: t('shell.nav_activity'),
            hay: t('shell.nav_activity'),
            address: 'activity',
            featured: true,
        },
    ];
    for (const tab of SETTINGS_TABS) {
        const label = t(SETTINGS_TAB_LABELS[tab]);
        out.push({
            group: 'jump',
            key: `app:settings:${tab}`,
            title: label,
            hay: `${settings} ${label}`,
            address: `settings/${tab}`,
            path: [settings, label],
        });
    }
    const storage = t(SETTINGS_TAB_LABELS.sabaStorage);
    for (const sub of STORAGE_TABS) {
        const label = t(`saba_storage.tab_${sub}`);
        out.push({
            group: 'jump',
            key: `app:storage:${sub}`,
            title: label,
            hay: `${storage} ${label}`,
            address: `settings/sabaStorage/${sub}`,
            path: [settings, storage, label],
        });
    }
    for (const [tab, key] of APP_SETTINGS) {
        const label = t(`settings_modal.${key}`);
        const tabName = t(SETTINGS_TAB_LABELS[tab as keyof typeof SETTINGS_TAB_LABELS]);
        out.push({
            group: 'setting',
            key: `app:field:${key}`,
            title: label,
            hay: `${label} ${tabName}`,
            address: `settings/${tab}`,
            find: label,
            path: [settings, tabName],
        });
    }
    return out;
}

/** 범위로 고를 수 있는 것: 게임 서버, 디스코드 서버 */
export function entityRows(data: PaletteData, t: Translate): PaletteItem[] {
    return [
        ...data.servers.map((s) => ({
            group: 'esrv' as const,
            key: `esrv:${s.id}`,
            title: s.name,
            hay: `${s.name} ${s.module} ${t('palette.game_server')}`,
            serverId: s.id,
        })),
        ...data.guilds.map((g) => ({
            group: 'eguild' as const,
            key: `eguild:${g.id}`,
            title: g.name,
            hay: `${g.name} ${t('palette.discord_server')}`,
            guildId: g.id,
        })),
    ];
}

/** 게임 서버 또는 디스코드 서버 하나에서 할 수 있는 것 */
export function entityItems(e: Entity, data: PaletteData, t: Translate): PaletteItem[] {
    if (e.kind === 'server') {
        const s = data.servers.find((x) => x.id === e.id);
        return s ? serverItems(s, t) : [];
    }
    const g = data.guilds.find((x) => x.id === e.id);
    return g ? guildItems(g, t) : [];
}

export interface PaletteState {
    scope: Entity | null;
    context: Entity | null;
    query: string;
    category: Category | null;
}

/** 범위(없으면 지금 화면 + 앱)에서 고를 수 있는 기능 */
function baseItems(state: Pick<PaletteState, 'scope' | 'context'>, data: PaletteData, t: Translate) {
    if (state.scope) return entityItems(state.scope, data, t);
    return [...(state.context ? entityItems(state.context, data, t) : []), ...appItems(t)];
}

/** 보일 카테고리 버튼: 지금 범위(없으면 지금 화면)에 그 카테고리 기능이 있는 것만 (꺼진 서버면 명령이 빠진다) */
export function categoriesOf(state: Pick<PaletteState, 'scope' | 'context'>, data: PaletteData, t: Translate) {
    const has = new Set(baseItems(state, data, t).map((i) => categoryOf(i.group)));
    return CATEGORIES.filter((c) => has.has(c));
}

function ranked(items: PaletteItem[], query: string) {
    return items
        .map((item, i) => ({ item, i, score: matchScore(item.hay, query) }))
        .filter((x) => x.score > 0)
        .sort((a, b) => b.score - a.score || a.i - b.i)
        .map((x) => x.item);
}

/** 지금 보일 항목 (점수 순. 묶음은 groupItems가 나눈다) */
export function visibleItems(state: PaletteState, data: PaletteData, t: Translate): PaletteItem[] {
    const q = state.query.trim();
    const inCategory = (i: PaletteItem) => !state.category || categoryOf(i.group) === state.category;
    if (state.scope) {
        const list = entityItems(state.scope, data, t).filter(inCategory);
        if (!q) return list.filter((i) => state.category || i.group !== 'setting');
        return ranked(list, q);
    }
    const { context } = state;
    if (!q) {
        // 빈 검색: 지금 열린 화면 기준 추천 (설정 칸은 검색할 때만)
        const own = context ? entityItems(context, data, t).filter((i) => i.group !== 'setting') : [];
        const app = appItems(t).filter((i) => i.group === 'jump' && (state.category || i.featured));
        return [...own, ...app].filter(inCategory);
    }
    // 검색: 1) 쓸 수 있는 기능 (+ 같은 명령을 지금 보낼 수 있는 다른 서버) 2) 범위로 고를 게임 서버·디스코드 서버
    const features = baseItems(state, data, t);
    for (const s of data.servers) {
        if (context?.kind === 'server' && context.id === s.id) continue;
        for (const c of s.commands || []) if (commandReady(s, c)) features.push(commandItem('xcmd', s, c));
    }
    const rows = state.category ? [] : entityRows(data, t);
    return ranked([...features.filter(inCategory), ...rows], q);
}

const LIMIT = 5;

/** 묶음으로 나눈다. 카테고리를 골랐거나 "더 보기"를 누른 묶음은 다 보인다 */
export function groupItems(
    items: PaletteItem[],
    opts: { query: string; category: Category | null; expanded: Partial<Record<Group, boolean>> },
) {
    const out: { group: Group; items: PaletteItem[]; total: number }[] = [];
    for (const group of GROUP_ORDER) {
        const all = items.filter((i) => i.group === group);
        if (!all.length) continue;
        const limit =
            opts.category || opts.expanded[group] ? Infinity : !opts.query.trim() && group === 'ctl' ? 8 : LIMIT;
        out.push({ group, items: all.slice(0, limit), total: all.length });
    }
    return out;
}
