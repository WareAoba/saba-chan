import { describe, expect, it } from 'vitest';
import {
    categoriesOf,
    groupItems,
    type PaletteData,
    type PaletteItem,
    visibleItems,
} from '../components/shell/paletteModel';

// 번역은 키 그대로 (무슨 문구를 쓰는지 키로 확인한다)
const t = (key: string) => key;

const say = { name: 'say', label: 'Broadcast', description: null, risk: 'safe', inputs: [], ready: true };
const kick = { name: 'kick', label: 'Kick player', description: null, risk: 'danger', inputs: [], ready: true };
const data = (): PaletteData => ({
    servers: [
        {
            id: 'mc',
            name: 'Survival',
            module: 'minecraft',
            status: 'running',
            tabs: ['console', 'settings', 'info'],
            commands: [say, { ...kick, ready: false }],
            settings: [{ name: 'difficulty', label: 'Difficulty', tab: 'settings' }],
        },
        {
            id: 'tr',
            name: 'Terraria',
            module: 'terraria',
            status: 'running',
            tabs: ['console', 'info'],
            commands: [{ ...say, label: 'Broadcast' }],
        },
        {
            id: 'pw',
            name: 'Palworld',
            module: 'palworld',
            status: 'stopped',
            tabs: ['settings', 'info'],
            commands: [{ ...say, ready: false }],
        },
    ],
    guilds: [{ id: 'g1', name: 'Mayo guild', tabs: ['instances', 'chatbot', 'tts'] }],
});
const keys = (items: PaletteItem[]) => items.map((i) => i.key);
const view = (state: Partial<Parameters<typeof visibleItems>[0]>) =>
    visibleItems({ scope: null, context: null, query: '', category: null, ...state }, data(), t);

describe('명령 팔레트 항목', () => {
    it('빈 검색: 지금 화면의 서버로 조작·보낼 수 있는 명령·바로 가기를 추천하고, 설정 칸과 범위 줄은 숨긴다', () => {
        const items = view({ context: { kind: 'server', id: 'mc' } });
        const groups = new Set(items.map((i) => i.group));
        expect(groups).toEqual(new Set(['ctl', 'cmd', 'jump']));
        expect(keys(items.filter((i) => i.group === 'ctl'))).toEqual([
            'ctl:mc:stop',
            'ctl:mc:restart',
            'ctl:mc:backup',
        ]);
        // 지금 보낼 수 없는 명령(ready=false)은 없다
        expect(keys(items.filter((i) => i.group === 'cmd'))).toEqual(['cmd:mc:say']);
        // 서버 탭으로 가는 바로 가기 + 앱의 추천 바로 가기
        expect(items.find((i) => i.key === 'jump:mc:settings')).toMatchObject({
            title: 'shell.tab_settings',
            address: 'server/mc/settings',
        });
        expect(items.some((i) => i.action === 'newServer')).toBe(true);
    });

    it('꺼진 서버는 시작·백업만 있고 명령 카테고리가 없다', () => {
        const items = view({ context: { kind: 'server', id: 'pw' } });
        expect(keys(items.filter((i) => i.group === 'ctl'))).toEqual(['ctl:pw:start', 'ctl:pw:backup']);
        expect(items.some((i) => i.group === 'cmd')).toBe(false);
        expect(categoriesOf({ scope: null, context: { kind: 'server', id: 'pw' } }, data(), t)).toEqual([
            'ctl',
            'jump',
        ]);
        expect(categoriesOf({ scope: null, context: { kind: 'server', id: 'mc' } }, data(), t)).toEqual([
            'ctl',
            'cmd',
            'jump',
        ]);
    });

    it('검색: 쓸 수 있는 기능 다음에 같은 명령을 보낼 수 있는 다른 서버, 이름이 맞는 서버·디스코드 서버가 나온다', () => {
        const items = view({ context: { kind: 'server', id: 'mc' }, query: 'broad' });
        expect(keys(items)).toEqual(['cmd:mc:say', 'xcmd:tr:say']);
        const ents = view({ context: { kind: 'server', id: 'mc' }, query: 'a' }).filter((i) =>
            ['esrv', 'eguild'].includes(i.group),
        );
        expect(keys(ents)).toEqual(expect.arrayContaining(['esrv:tr', 'esrv:pw', 'eguild:g1']));
    });

    it('검색에는 설정 칸도 나오고, 고르면 그 탭으로 가서 그 칸을 찾는다', () => {
        const items = view({ context: { kind: 'server', id: 'mc' }, query: 'diffic' });
        expect(items[0]).toMatchObject({
            group: 'setting',
            address: 'server/mc/settings',
            find: 'Difficulty',
            findField: 'difficulty',
        });
        // 앱 설정 칸
        const app = view({ query: 'settings_modal.theme' });
        expect(app[0]).toMatchObject({ group: 'setting', address: 'settings/appearance' });
    });

    it('범위 칩: 그 디스코드 서버 탭만 보이고, 이름은 실제 메뉴 이름(말하는 사바쨩 등)이다', () => {
        const items = view({ scope: { kind: 'guild', id: 'g1' } });
        expect(items.map((i) => [i.title, i.address])).toEqual([
            ['discord.tab_instances', 'discord/g1/instances'],
            ['chatbot.nav', 'discord/g1/chatbot'],
            ['discord.bot_settings.tts_title', 'discord/g1/tts'],
        ]);
        // 범위 안 검색은 그 안에서만 (다른 서버·앱 항목·범위 줄 없음)
        expect(keys(view({ scope: { kind: 'server', id: 'tr' }, query: 'broad' }))).toEqual(['cmd:tr:say']);
    });

    it('카테고리: 명령을 고르면 다른 서버 명령까지, 바로 가기를 고르면 설정 칸까지 보이고 범위 줄은 빠진다', () => {
        const cmd = view({ context: { kind: 'server', id: 'mc' }, query: 'a', category: 'cmd' });
        expect(new Set(cmd.map((i) => i.group))).toEqual(new Set(['cmd', 'xcmd']));
        const jump = view({ context: { kind: 'server', id: 'mc' }, query: 'diff', category: 'jump' });
        expect(jump.map((i) => i.group)).toEqual(['setting']);
    });

    it('가이드 다시 열기도 앱 바로 가기로 찾는다 (빈 검색에는 없다)', () => {
        const ko = (key: string, o?: Record<string, unknown>) =>
            key === 'guides.palette' ? `가이드: ${o?.name}` : key;
        const items = visibleItems({ scope: null, context: null, query: '가이드', category: null }, data(), ko);
        expect(items.filter((i) => i.action === 'guide').map((i) => [i.group, i.guide, i.title])).toEqual([
            ['jump', 'firstRun', '가이드: guides.entry_firstRun'],
            ['jump', 'import', '가이드: guides.entry_import'],
            ['jump', 'modules', '가이드: guides.entry_modules'],
        ]);
        expect(view({}).some((i) => i.action === 'guide')).toBe(false);
    });

    it('초성으로도 찾는다', () => {
        const ko = (key: string) => ({ 'palette.control.backup': '백업 만들기' })[key] || key;
        const items = visibleItems(
            { scope: { kind: 'server', id: 'mc' }, context: null, query: 'ㅂㅇ', category: null },
            data(),
            ko,
        );
        expect(keys(items)).toEqual(['ctl:mc:backup']);
    });

    it('묶음 순서는 조작 → 명령 → 다른 서버 명령 → 바로 가기 → 설정 → 서버 → 디스코드 서버, 검색 중엔 묶음마다 5개까지', () => {
        const items = view({ context: { kind: 'server', id: 'mc' }, query: 'a' });
        const groups = groupItems(items, { query: 'a', category: null, expanded: {} });
        const order = ['ctl', 'cmd', 'xcmd', 'jump', 'setting', 'esrv', 'eguild'];
        const seen = groups.map((g) => g.group);
        expect(seen).toEqual(order.filter((g) => seen.includes(g as never)));
        for (const g of groups) expect(g.items.length).toBeLessThanOrEqual(5);
        const all = groupItems(items, { query: 'a', category: null, expanded: { jump: true } });
        const jumps = all.find((g) => g.group === 'jump');
        expect(jumps?.items.length).toBe(jumps?.total);
    });
});
