import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { hangulOutsideT } from '../../../saba-chan-gui/src/test/hangulScan';
import { initI18n, SUPPORTED_LANGUAGES } from '../i18n';

const LOCALES = join(__dirname, '../../../locales');
const SRC = join(__dirname, '..');

function flatten(obj: object, prefix = '', out: Record<string, string> = {}) {
    for (const [k, v] of Object.entries(obj)) {
        const key = prefix ? `${prefix}.${k}` : k;
        if (v && typeof v === 'object') flatten(v, key, out);
        else out[key] = String(v);
    }
    return out;
}

const catalog = (lang: string, file = 'web.json') =>
    flatten(JSON.parse(readFileSync(join(LOCALES, lang, file), 'utf8')));
const placeholders = (s: string) => (s.match(/\{\{\w+\}\}/g) ?? []).sort();

function sources(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
        const p = join(dir, e.name);
        if (e.isDirectory()) return e.name === 'test' ? [] : sources(p);
        return /\.(jsx?|tsx?)$/.test(e.name) ? [p] : [];
    });
}

describe('web.json', () => {
    it('모든 언어가 en 과 같은 키와 자리 표시자를 가진다', () => {
        const en = catalog('en');
        const problems = [];
        for (const lang of SUPPORTED_LANGUAGES.filter((l) => l !== 'en')) {
            const other = catalog(lang);
            for (const k of Object.keys(en)) {
                if (!(k in other)) problems.push(`${lang}: missing ${k}`);
                else if (placeholders(en[k]).join() !== placeholders(other[k]).join())
                    problems.push(`${lang}: placeholders ${k}`);
            }
            for (const k of Object.keys(other)) if (!(k in en)) problems.push(`${lang}: not in en ${k}`);
        }
        expect(problems).toEqual([]);
    });

    it('코드가 부르는 고정 키는 en 에 있다', () => {
        const en = catalog('en');
        const missing = [];
        for (const file of sources(SRC)) {
            for (const m of readFileSync(file, 'utf8').matchAll(/\bt\(\s*'([\w.-]+)'/g)) {
                if (!(m[1] in en)) missing.push(`${file.slice(SRC.length + 1)}: ${m[1]}`);
            }
        }
        expect(missing).toEqual([]);
    });

    it('앱 문구를 빌려 쓰는 고정 키(tg)는 gui.json en 에 있다', () => {
        const gui = catalog('en', 'gui.json');
        const missing = [];
        for (const file of sources(SRC)) {
            for (const m of readFileSync(file, 'utf8').matchAll(/\btg\(\s*'([\w.-]+)'/g)) {
                if (!(m[1] in gui)) missing.push(`${file.slice(SRC.length + 1)}: ${m[1]}`);
            }
        }
        expect(missing).toEqual([]);
    });
});

/** 언어마다 그대로 쓰는 이름·기술 낱말 (지우고 나서 남은 영어 낱말만 본다) */
const PROPER =
    /Saba-chan|Server-chan|saba-chan|Discord|GitHub|WareAoba|Zomboid|Windows|Linux|macOS|Mica|Liquid Glass|localhost|HTTP|UUID|CPU|GPU|API|URL|CLI|PC|IP|AI|ID|OK|Lv|\/saba(?: \w+)?(?: code:)?/g;

/** 모든 언어가 영어 그대로 쓰는 키 */
const SAME_EVERYWHERE = [
    // 봇이 알아듣는 명령 낱말 (릴레이 DAEMON_TOP_LEVEL_WORDS)
    'gui.json:discord.usage_list_word',
    'gui.json:discord.usage_start_word',
    'gui.json:discord.usage_module_word',
    // 가이드 카드의 딱지 (#948 부터 모든 언어가 'new!')
    'gui.json:guides.new',
];

/** 영어와 글자가 같아도 그 언어에서 맞는 문구 (언어: 파일:키) */
const SAME_AS_EN: Record<string, string[]> = {
    es: [
        'gui.json:palette.title',
        'gui.json:audit.http_other',
        'gui.json:monitor.pick_network_all',
        'web.json:app_name',
        'web.json:backups.kind.manual',
        'web.json:perm.level.2',
        'web.json:modules.plugins',
        'web.json:relay.mod.kind_chat',
        'gui.json:settings_modal.general',
        'gui.json:settings_modal.console_token_error',
        'gui.json:settings_modal.console_token_selector',
        'gui.json:settings_modal.toast_duration_normal',
        'gui.json:background_modal.status_error',
        'gui.json:shell.tab_info',
        'gui.json:shell.daemon_short',
        'gui.json:discord.level_admin',
        'gui.json:discord.bot_settings.general_title',
        'gui.json:discord.bot_settings.tts_speed_unit',
        'gui.json:maintenance.kind_manual',
        'gui.json:chatbot.provider_region_cn',
        'gui.json:audit.source_gui',
        'gui.json:audit.source_daemon',
        'gui.json:audit.source_web',
        'gui.json:daemon_switcher.label',
        'gui.json:settings_modal.backdrop_material_label',
        'gui.json:remote_daemon.prompt_no',
        'gui.json:server_settings.idle_stop_unit',
    ],
    'pt-BR': [
        'gui.json:palette.title',
        'gui.json:discord.iw_cmd_status',
        'gui.json:settings_modal.tab_console',
        'gui.json:command_modal.via_console',
        'gui.json:monitor.pick_clock',
        'gui.json:monitor.pick_network_all',
        'web.json:app_name',
        'web.json:tabs.label',
        'web.json:detail.backups',
        'web.json:backups.kind.manual',
        'web.json:perm.level.2',
        'web.json:modules.plugins',
        'web.json:relay.volume',
        'web.json:relay.mod.kind_chat',
        'gui.json:settings_modal.toast_duration_normal',
        'gui.json:shell.tab_console',
        'gui.json:shell.daemon_short',
        'gui.json:shell.bot_running',
        'gui.json:discord.back_to_menu',
        'gui.json:discord.level_admin',
        'gui.json:discord.bot_settings.tts_volume',
        'gui.json:discord.bot_settings.tts_speed_unit',
        'gui.json:maintenance.backups_title',
        'gui.json:maintenance.kind_manual',
        'gui.json:maintenance.action_backup',
        'gui.json:chatbot.provider_region_cn',
        'gui.json:audit.action_backup',
        'gui.json:audit.source_gui',
        'gui.json:audit.source_daemon',
        'gui.json:audit.source_web',
        'gui.json:daemon_switcher.label',
        'gui.json:settings_modal.backdrop_material_label',
        'gui.json:migration_modal.change_backup',
        'gui.json:console.popout_kind',
        'gui.json:palette.status',
        'gui.json:remote_daemon.online',
        'gui.json:remote_daemon.offline',
        'gui.json:server_settings.idle_stop_unit',
        'gui.json:players.online',
    ],
    de: [
        'gui.json:palette.title',
        'gui.json:audit.filter',
        'gui.json:audit.detail_server',
        'gui.json:audit.detail_args',
        'gui.json:discord.iw_cmd_status',
        'gui.json:shell.info_more',
        'gui.json:monitor.ports_server',
        'gui.json:monitor.ports_port',
        'web.json:app_name',
        'web.json:servers.name',
        'web.json:detail.backups',
        'web.json:perm.level.2',
        'web.json:modules.plugins',
        'web.json:relay.mod.kind_chat',
        'gui.json:settings_modal.accent_orange',
        'gui.json:settings_modal.accent_cyan',
        'gui.json:settings_modal.console_token_info',
        'gui.json:settings_modal.console_token_debug',
        'gui.json:settings_modal.toast_duration_normal',
        'gui.json:add_server_modal.port',
        'gui.json:shell.tab_info',
        'gui.json:shell.info_version',
        'gui.json:shell.info_port',
        'gui.json:shell.daemon_short',
        'gui.json:shell.bot_running',
        'gui.json:discord.level_admin',
        'gui.json:discord.bot_settings.tts_speed_unit',
        'gui.json:discord.mod.name',
        'gui.json:maintenance.backups_title',
        'gui.json:maintenance.action_backup',
        'gui.json:chatbot.provider_region_cn',
        'gui.json:audit.source_gui',
        'gui.json:audit.source_daemon',
        'gui.json:diagnostics.version',
        'gui.json:remote_daemon.port',
        'gui.json:daemon_switcher.label',
        'gui.json:picker.minute',
        'gui.json:settings_modal.backdrop_material_label',
        'gui.json:palette.status',
        'gui.json:palette.optional',
        'gui.json:onboarding.optional',
        'gui.json:remote_daemon.online',
        'gui.json:remote_daemon.offline',
        'gui.json:shell.build',
        'gui.json:branch.name',
        'gui.json:branch.count',
        'gui.json:players.optional',
        'gui.json:players.online',
    ],
    fr: [
        'gui.json:palette.title',
        'gui.json:saba_storage.version_kind_module',
        'gui.json:saba_storage.version_kind_ext',
        'gui.json:audit.detail_action',
        'gui.json:audit.detail_args',
        'gui.json:monitor.ports_port',
        'gui.json:settings_modal.tab_console',
        'gui.json:command_modal.via_console',
        'gui.json:monitor.pick_network_all',
        'web.json:app_name',
        'web.json:tabs.label',
        'web.json:tabs.modules',
        'web.json:gate_word.relay_closing',
        'web.json:perm.level.2',
        'web.json:perm.instances',
        'web.json:perm.back_instances',
        'web.json:perm.preset_strict',
        'web.json:modules.plugins',
        'web.json:relay.volume',
        'web.json:relay.mod.kind',
        'web.json:relay.mod.kind_chat',
        'gui.json:settings_modal.about_intro.motif_label',
        'gui.json:settings_modal.accent_orange',
        'gui.json:settings_modal.accent_cyan',
        'gui.json:settings_modal.transitions_label',
        'gui.json:settings_modal.console_token_info',
        'gui.json:add_server_modal.port',
        'gui.json:notice_modal.title',
        'gui.json:notice_modal.tooltip',
        'gui.json:updates.channel_stable',
        'gui.json:saba_storage.tab_modules',
        'gui.json:saba_storage.tab_extensions',
        'gui.json:shell.tab_console',
        'gui.json:shell.info_version',
        'gui.json:shell.info_port',
        'gui.json:shell.risk_danger',
        'gui.json:shell.exit_crash',
        'gui.json:discord.back_to_menu',
        'gui.json:discord.tab_instances',
        'gui.json:discord.level_admin',
        'gui.json:discord.preset_strict',
        'gui.json:discord.bot_settings.tts_volume',
        'gui.json:discord.bot_settings.tts_speed_unit',
        'gui.json:discord.back_to_instances',
        'gui.json:maintenance.minutes',
        'gui.json:maintenance.message',
        'gui.json:maintenance.on_date',
        'gui.json:maintenance.command_danger_chip',
        'gui.json:audit.source_gui',
        'gui.json:audit.source_web',
        'gui.json:diagnostics.version',
        'gui.json:diagnostics.modules',
        'gui.json:folder_paths.modules',
        'gui.json:folder_paths.extensions',
        'gui.json:remote_daemon.port',
        'gui.json:picker.minute',
        'gui.json:console.popout_kind',
        'gui.json:palette.risk.danger',
        'gui.json:remote_daemon.install_prompt',
        'gui.json:guides.title',
        'gui.json:server_settings.idle_stop_unit',
        'gui.json:shell.build',
        'gui.json:branch.count',
    ],
    ru: ['gui.json:discord.bot_settings.tts_speed_unit'],
};

/** 그 언어 값이 en 과 똑같고 이름이 아닌 영어 낱말이 남으면 번역하지 않은 것이다 */
function untranslated(file: string) {
    const en = catalog('en', file);
    const out: string[] = [];
    for (const lang of SUPPORTED_LANGUAGES.filter((l) => l !== 'en')) {
        const other = catalog(lang, file);
        const allowed = new Set([...SAME_EVERYWHERE, ...(SAME_AS_EN[lang] ?? [])]);
        for (const [k, v] of Object.entries(en)) {
            if (other[k] !== v || allowed.has(`${file}:${k}`)) continue;
            if (/[A-Za-z]/.test(v.replace(/\{\{\w+\}\}/g, '').replace(PROPER, ''))) out.push(`${lang}: ${k} = ${v}`);
        }
    }
    return out;
}

describe('번역 누락', () => {
    it('web.json 의 모든 언어가 영어를 그대로 두지 않는다', () => {
        expect(untranslated('web.json')).toEqual([]);
    });

    it('웹이 빌려 쓰는 gui.json 의 모든 언어가 영어를 그대로 두지 않는다', () => {
        expect(untranslated('gui.json')).toEqual([]);
    });
});

describe('탭 제목', () => {
    it('브라우저 탭 제목은 고른 언어의 app_name 이다', async () => {
        await initI18n('ja');
        expect(document.title).toBe(catalog('ja').app_name);
        expect(document.documentElement.lang).toBe('ja');
    });

    it('index.html 의 처음 제목·언어는 기본 언어(en)다', () => {
        const html = readFileSync(join(__dirname, '../../index.html'), 'utf8');
        expect(html).toContain(`<title>${catalog('en').app_name}</title>`);
        expect(html).toContain('<html lang="en">');
    });
});

describe('한글 하드코딩', () => {
    it('화면 코드에 번역 키를 거치지 않은 한글이 없다', () => {
        const found = [];
        for (const file of sources(SRC)) {
            for (const h of hangulOutsideT(readFileSync(file, 'utf8')))
                found.push(`${file.slice(SRC.length + 1)}:${h.line}: ${h.text}`);
        }
        expect(found).toEqual([]);
    });
});
