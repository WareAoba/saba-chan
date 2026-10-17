/**
 * 모듈 하이라이팅 규칙 통합 테스트
 * ================================
 *
 * 각 모듈 `module.toml` 의 `[syntax_highlight]` 섹션에 정의된 규칙이
 *  1) 정상 컴파일되고
 *  2) 실제 서버 로그 샘플에 대해 의도한 토큰을 색칠하며
 *  3) 부작용(오탐, 경로 오버매치 등)이 없는지
 * 를 GUI 엔진(`syntaxHighlight.js`)으로 직접 검증한다.
 *
 * 주의: 이 파일은 TOML 파서에 의존하지 않고, 모듈에 **현재 정의된 규칙과 동일한**
 * JS 객체 배열을 테스트 케이스 안에 복제해 검증한다. TOML 수정 시 여기도 함께
 * 업데이트해야 한다. 통합 테스트의 목적은 엔진+규칙의 end-to-end 올바름 확인.
 */
import { beforeEach, describe, expect, it } from 'vitest';
import {
    compileRules,
    highlightLine,
    __resetRuleCache,
} from '../utils/syntaxHighlight';

// ───────────────────────────────────────────────────────────
// helpers
// ───────────────────────────────────────────────────────────

/**
 * `text` 안에서 `needle` 가 차지하는 구간이 특정 규칙 이름의 스타일로
 * 색칠되어 있는지 확인한다.
 */
function assertHighlighted(text, compiled, needle, expectedTokenSubstring) {
    const segments = highlightLine(text, compiled);
    // needle 전체가 단일 스타일 세그먼트(들)로 묶여 있어야 한다
    let cursor = 0;
    const hits = [];
    for (const seg of segments) {
        const segStart = cursor;
        const segEnd = cursor + seg.text.length;
        if (seg.style && text.slice(segStart, segEnd).includes(needle)) {
            hits.push(seg);
        }
        // needle 이 여러 세그먼트에 걸친 경우도 허용
        if (seg.text === needle && seg.style) hits.push(seg);
        cursor = segEnd;
    }
    expect(hits.length, `"${needle}" not highlighted in: ${text}`).toBeGreaterThan(0);
    if (expectedTokenSubstring) {
        expect(
            hits.some((h) => (h.style.color || '').includes(expectedTokenSubstring)),
            `"${needle}" not colored with ${expectedTokenSubstring}: got ${JSON.stringify(hits.map((h) => h.style))}`,
        ).toBe(true);
    }
}

function assertNotHighlighted(text, compiled, needle) {
    const segments = highlightLine(text, compiled);
    let cursor = 0;
    for (const seg of segments) {
        const segStart = cursor;
        const segEnd = cursor + seg.text.length;
        if (seg.style && text.slice(segStart, segEnd) === needle) {
            throw new Error(`"${needle}" unexpectedly highlighted: ${JSON.stringify(seg.style)}`);
        }
        cursor = segEnd;
    }
}

// ───────────────────────────────────────────────────────────
// Minecraft 규칙 (module.toml 과 동기화)
// ───────────────────────────────────────────────────────────
const MINECRAFT_RULES = [
    // 스택트레이스 우선
    { name: 'stacktrace_caused',  pattern: '^Caused by:.*',                                          color: 'error',     priority: 100 },
    { name: 'stacktrace_at',      pattern: '^\\s+at .*',                                              color: 'dim',       italic: true, priority: 100 },
    { name: 'exception_class',    pattern: '(?P<hl>[a-z][\\w.]*\\.[A-Z]\\w*(?:Exception|Error))',     color: 'error',     bold: true,  priority: 90 },
    // 구조
    { name: 'timestamp',          pattern: '^\\[\\d{2}:\\d{2}:\\d{2}\\]',                              color: 'timestamp', priority: 80 },
    { name: 'level_error',        pattern: '\\[(?P<hl>[^\\]]+/(?:ERROR|FATAL))\\]',                    color: 'error',     bold: true,  priority: 75 },
    { name: 'level_warn',         pattern: '\\[(?P<hl>[^\\]]+/WARN)\\]',                               color: 'warn',      bold: true,  priority: 75 },
    { name: 'level_debug',        pattern: '\\[(?P<hl>[^\\]]+/DEBUG)\\]',                              color: 'debug',     italic: true, priority: 75 },
    { name: 'level_info',         pattern: '\\[(?P<hl>[^\\]]+/INFO)\\]',                               color: 'dim',       priority: 70 },
    // 라이프사이클
    { name: 'done_message',       pattern: '(?P<hl>Done \\([^)]+\\)!.*)',                              color: 'success',   bold: true,  priority: 60 },
    { name: 'server_version',     pattern: '(?P<hl>Starting minecraft server version .+)',            color: 'info',      bold: true,  priority: 60 },
    { name: 'server_stopping',    pattern: '(?P<hl>Stopping (?:the )?server)',                        color: 'warn',      bold: true,  priority: 60 },
    { name: 'server_saving',      pattern: '(?P<hl>Saving (?:chunks|players|worlds).*)',              color: 'info',      priority: 55 },
    { name: 'preparing_spawn',    pattern: '(?P<hl>Preparing (?:spawn area|start region))',           color: 'info',      priority: 55 },
    // 경고
    { name: 'cant_keep_up',       pattern: "(?P<hl>Can't keep up!.*)",                                color: 'warn',      bold: true,  priority: 65 },
    { name: 'moved_incorrectly',  pattern: '(?P<hl>\\w{3,16} moved (?:too quickly|wrongly)!?)',        color: 'warn',      priority: 60 },
    { name: 'eula_warning',       pattern: '(?P<hl>You need to agree to the EULA.*)',                 color: 'error',     bold: true,  priority: 65 },
    // 플레이어
    { name: 'chat_name',          pattern: '<(?P<hl>\\w{3,16})>',                                      color: 'player',    bold: true,  priority: 50 },
    { name: 'player_join',        pattern: '(?P<hl>\\w{3,16}) joined the game',                        color: 'success',   bold: true,  priority: 50 },
    { name: 'player_leave',       pattern: '(?P<hl>\\w{3,16}) left the game',                          color: 'warning',   priority: 50 },
    { name: 'player_login',       pattern: '(?P<hl>\\w{3,16})\\[/',                                    color: 'player',    bold: true,  priority: 50 },
    { name: 'player_disconnect_reason', pattern: '\\w{3,16} lost connection: (?P<hl>.+)',             color: 'notice',    priority: 50 },
    // 사망
    { name: 'death_message',
      pattern: ': (?P<hl>\\w{3,16} (?:was (?:slain|shot|blown|pummeled|squished|killed|stung|fireballed|obliterated|skewered|impaled|struck|pricked|poked|smashed|speared|roasted|squashed|burned|frozen|doomed|sniped|spitballed)|drowned|burned|suffocated|froze|fell|hit the ground|went (?:up|off)|walked into|tried to swim|experienced kinetic|starved|withered|died|discovered|left the confines).*)',
      color: 'critical', priority: 45 },
    // 관리자
    { name: 'admin_action',       pattern: '\\[(?P<hl>\\w{3,16}: [A-Z][^\\]]+)\\]',                    color: 'notice',    italic: true, priority: 40 },
    { name: 'admin_target',       pattern: '(?P<hl>(?:Opped|De-?opped|Banned(?: player)?|Unbanned|Pardoned|Kicked|Whitelisted|Teleported) \\w{3,16})', color: 'notice', bold: true, priority: 40 },
    // 식별자
    { name: 'uuid',               pattern: '(?<![\\w/-])(?P<hl>[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})(?![\\w/-])', color: 'uuid', priority: 30 },
    { name: 'ip_address',         pattern: '/(?P<hl>\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}(?::\\d+)?)', color: 'ip', priority: 30 },
    { name: 'coordinates',        pattern: '(?P<hl>-?\\d+\\.?\\d*, -?\\d+\\.?\\d*, -?\\d+\\.?\\d*)',   color: 'number',    priority: 30 },
    // 리소스
    { name: 'resource_id',        pattern: '(?P<hl>minecraft:[\\w/]+)',                               color: 'keyword',   priority: 25 },
    { name: 'advancement',        pattern: 'the advancement \\[(?P<hl>[A-Z][A-Za-z !]+)\\]',           color: 'success',   priority: 25 },
    { name: 'item_bracket',       pattern: '(?:Gave|Summoned|Replaced) [^\\[]*\\[(?P<hl>[A-Z][\\w ]+)\\]', color: 'string', priority: 25 },
    // 숫자
    { name: 'percentage',         pattern: '(?P<hl>\\d+%)',                                            color: 'number',    bold: true, priority: 20 },
    // 시스템
    { name: 'server_broadcast',   pattern: '\\[(?P<hl>Server)\\]',                                     color: 'command',   priority: 20 },
    { name: 'rcon_label',         pattern: '\\[(?P<hl>Rcon)\\]',                                       color: 'command',   priority: 20 },
    // 명령어
    { name: 'slash_command',      pattern: '(?<=^|\\s)(?P<hl>/[a-z][a-z0-9_-]+)\\b',                      color: 'command',   bold: true, priority: 15 },
    { name: 'entity_selector',    pattern: '(?P<hl>@[paersn](?:\\[[^\\]]*\\])?)',                      color: 'selector',  bold: true, priority: 15 },
    { name: 'string_literal',     pattern: '(?P<hl>"[^"]*")',                                          color: 'string',    priority: 10 },
];

// ───────────────────────────────────────────────────────────
// Zomboid 규칙 (module.toml 과 동기화)
// ───────────────────────────────────────────────────────────
const ZOMBOID_RULES = [
    { name: 'timestamp',          pattern: '^\\d{2}-\\d{2}-\\d{4}\\s+\\d{2}:\\d{2}:\\d{2}\\.\\d+',     color: 'timestamp', priority: 90 },
    { name: 'epoch_ms',           pattern: ',\\s+(?P<hl>\\d{13})>',                                    color: 'dim',       priority: 80 },
    { name: 'server_started',     pattern: '(?P<hl>SERVER STARTED|Server Started|Server is now listening)', color: 'success', bold: true, priority: 75 },
    { name: 'server_quitting',    pattern: '(?P<hl>Quitting!|Server is shutting down|Shutting down)',  color: 'warn',      bold: true,  priority: 75 },
    { name: 'level_error',        pattern: '\\b(?P<hl>ERROR|FATAL)\\b\\s*:',                           color: 'error',     bold: true,  priority: 70 },
    { name: 'level_warning',      pattern: '\\b(?P<hl>WARN(?:ING)?)\\b\\s*:',                          color: 'warn',      bold: true,  priority: 70 },
    { name: 'level_info',         pattern: '\\b(?P<hl>LOG|INFO)\\b\\s*:',                              color: 'info',      priority: 70 },
    { name: 'level_debug',        pattern: '\\b(?P<hl>DEBUG)\\b\\s*:',                                 color: 'dim',       italic: true, priority: 70 },
    { name: 'log_category',       pattern: ':\\s+(?P<hl>[A-Z][\\w.]+)\\s*,',                          color: 'keyword',   priority: 65 },
    { name: 'player_connected_quoted', pattern: '"(?P<hl>[^"]+)"\\s+(?:fully )?connected',             color: 'player',    bold: true, priority: 60 },
    { name: 'player_connected',   pattern: '\\b(?P<hl>\\w{3,32})\\s+(?:fully )?connected\\b',          color: 'player',    bold: true, priority: 55 },
    { name: 'player_disconnected', pattern: '\\b(?P<hl>\\w{3,32})\\s+disconnected\\b',                 color: 'warning',   priority: 55 },
    { name: 'player_kicked',      pattern: '\\b(?P<hl>\\w{3,32})\\s+(?:has been |was )?kicked\\b',     color: 'warning',   bold: true, priority: 55 },
    { name: 'chat_bracket',       pattern: '\\[(?P<hl>General Chat|Chat|Shout|Admin Chat|Faction|Safehouse)\\]', color: 'command', priority: 50 },
    { name: 'save_complete',      pattern: '(?P<hl>(?:World|Save|Map) saved(?: successfully)?)',      color: 'success',   priority: 50 },
    { name: 'zombie_count',       pattern: '(?P<hl>Zombies\\s*:\\s*\\d+)',                            color: 'number',    priority: 45 },
    { name: 'ip_address',         pattern: '(?P<hl>\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}(?::\\d+)?)', color: 'ip', priority: 40 },
    { name: 'steam_id',           pattern: '(?P<hl>7656119\\d{10})',                                   color: 'uuid',      priority: 40 },
    { name: 'file_path_win',      pattern: '(?P<hl>[A-Z]:\\\\[\\w\\\\./\\-]+)',                        color: 'path',      priority: 35 },
    { name: 'file_path_unix',     pattern: '(?<![\\w.])(?P<hl>/[\\w./\\-]*/[\\w.\\-]+)',              color: 'path',      priority: 35 },
    { name: 'duration',           pattern: '(?P<hl>\\d+\\s*ms)\\b',                                    color: 'duration',  priority: 20 },
    { name: 'slash_command',      pattern: '(?P<hl>/[a-z][a-zA-Z0-9_]+)',                             color: 'command',   bold: true, priority: 15 },
];

// ───────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────

describe('Minecraft 규칙 통합 검증', () => {
    beforeEach(() => __resetRuleCache());

    const { compiled, diagnostics } = compileRules(MINECRAFT_RULES);

    it('모든 규칙이 진단 오류 없이 컴파일된다', () => {
        const errors = diagnostics.filter((d) => d.level === 'error');
        expect(errors, JSON.stringify(errors)).toEqual([]);
    });

    it('타임스탬프 + [Server thread/INFO] + 채팅 <Player> 조합', () => {
        const line = '[17:23:45] [Server thread/INFO]: <Steve> Hello world!';
        assertHighlighted(line, compiled, '[17:23:45]');
        assertHighlighted(line, compiled, 'Server thread/INFO');
        assertHighlighted(line, compiled, 'Steve');
    });

    it('ERROR 레벨이 WARN/INFO 보다 우선한다', () => {
        const line = '[17:24:00] [Server thread/ERROR]: java.lang.NullPointerException: bad';
        assertHighlighted(line, compiled, 'Server thread/ERROR', 'error');
        assertHighlighted(line, compiled, 'java.lang.NullPointerException', 'error');
    });

    it('Done 메시지', () => {
        assertHighlighted('[17:23:11] [Server thread/INFO]: Done (8.234s)! For help, type "help"', compiled, 'Done (8.234s)!', 'success');
    });

    it('플레이어 접속/퇴장', () => {
        assertHighlighted('[17:23:30] [Server thread/INFO]: Steve joined the game', compiled, 'Steve', 'success');
        assertHighlighted('[17:25:00] [Server thread/INFO]: Alex left the game', compiled, 'Alex', 'warning');
    });

    it('로그인 라인: Player[/IP:port] + IP', () => {
        const line = '[17:23:29] [Server thread/INFO]: Steve[/192.168.1.100:54321] logged in with entity id 42 at (-12.5, 64.0, 283.7)';
        assertHighlighted(line, compiled, 'Steve', 'player');
        assertHighlighted(line, compiled, '192.168.1.100:54321', 'ip');
        assertHighlighted(line, compiled, '-12.5, 64.0, 283.7', 'number');
    });

    it('사망 메시지 (현대 1.21 verb 포함)', () => {
        assertHighlighted('[17:30:00] [Server thread/INFO]: Steve was slain by Zombie', compiled, 'Steve was slain by Zombie', 'critical');
        assertHighlighted('[17:30:05] [Server thread/INFO]: Alex froze to death', compiled, 'Alex froze to death', 'critical');
        assertHighlighted('[17:30:10] [Server thread/INFO]: Bob was obliterated by a sonically-charged shriek', compiled, 'Bob was obliterated by a sonically-charged shriek', 'critical');
        assertHighlighted('[17:30:15] [Server thread/INFO]: Charlie left the confines of this world', compiled, 'Charlie left the confines of this world', 'critical');
    });

    it('관리자 명령 피드백 — Opped / Banned / Kicked', () => {
        assertHighlighted('[17:40:00] [Server thread/INFO]: Opped Steve', compiled, 'Opped Steve', 'notice');
        assertHighlighted('[17:40:05] [Server thread/INFO]: Banned player Alex', compiled, 'Banned player Alex', 'notice');
        assertHighlighted('[17:40:10] [Server thread/INFO]: Kicked Bob', compiled, 'Kicked Bob', 'notice');
    });

    it('관리자 액션 대괄호 [Player: Action]', () => {
        assertHighlighted('[17:41:00] [Server thread/INFO]: [Steve: Set own game mode to Creative Mode]', compiled, 'Steve: Set own game mode to Creative Mode', 'notice');
    });

    it('아이템 지급: Gave N [Item] to Player', () => {
        assertHighlighted('[17:27:30] [Server thread/INFO]: Gave 64 [Diamond] to Steve', compiled, 'Diamond', 'string');
    });

    it('Caused by + at 스택트레이스가 error/dim 으로 강조됨', () => {
        assertHighlighted('Caused by: java.io.IOException: disk full', compiled, 'Caused by: java.io.IOException: disk full', 'error');
        assertHighlighted('\tat net.minecraft.server.MinecraftServer.run(MinecraftServer.java:123)', compiled, 'at net.minecraft.server.MinecraftServer.run(MinecraftServer.java:123)', 'dim');
    });

    it('UUID(소문자/대문자) + coordinates 동시 매치', () => {
        const line = '[17:23:29] [Server thread/INFO]: UUID of player Steve is 12345678-ABCD-1234-5678-9abcdef01234';
        assertHighlighted(line, compiled, '12345678-ABCD-1234-5678-9abcdef01234', 'uuid');
    });

    it('[Rcon] / [Server] 브로드캐스트 구분', () => {
        assertHighlighted('[17:50:00] [Server thread/INFO]: [Server] Welcome!', compiled, 'Server', 'command');
        assertHighlighted('[17:50:05] [Server thread/INFO]: [Rcon] Connected', compiled, 'Rcon', 'command');
    });

    it("Can't keep up + moved too quickly 경고", () => {
        assertHighlighted("[17:45:00] [Server thread/WARN]: Can't keep up! Is the server overloaded?", compiled, "Can't keep up!", 'warn');
        assertHighlighted('[17:45:05] [Server thread/WARN]: Steve moved too quickly!', compiled, 'Steve moved too quickly!', 'warn');
    });

    it('플레이어 이름 3자 미만은 매칭하지 않음 (오탐 방지)', () => {
        const line = '[17:23:30] [Server thread/INFO]: Hi joined the game';
        // "Hi" 는 2자이므로 player_join 이 매칭되면 안 됨
        const segments = highlightLine(line, compiled);
        const hiSeg = segments.find((s) => s.text === 'Hi');
        expect(hiSeg?.style).toBeFalsy();
    });

    it('file:/ URL 안 세그먼트는 슬래시 명령으로 오인되지 않는다', () => {
        const line = 'WARNING: java.lang.System::load has been called by com.sun.jna.Native in an unnamed module (file:/C:/Users/jace5/AppData/Roaming/saba-chan/jna-5.17.0.jar)';
        assertNotHighlighted(line, compiled, '/jace5');
        assertNotHighlighted(line, compiled, '/AppData');
        assertNotHighlighted(line, compiled, '/Roaming');
        assertNotHighlighted(line, compiled, '/saba-chan');
    });

    it('https:// URL 의 호스트 경로가 슬래시 명령으로 오인되지 않는다', () => {
        const line = '[17:23:01] [ServerMain/INFO]: Environment: Environment[sessionHost=https://sessionserver.mojang.com, servicesHost=https://api.minecraftservices.com, name=PROD]';
        assertNotHighlighted(line, compiled, '/sessionserver');
        assertNotHighlighted(line, compiled, '/api');
        assertNotHighlighted(line, compiled, '/minecraftservices');
    });

    it('파일 경로 안 UUID 는 강조되지 않는다', () => {
        const line = 'WARNING: file:/C:/Users/x/instances/3604730f-ef0e-4f7c-8560-4e64427cddc5/server/lib.jar';
        assertNotHighlighted(line, compiled, '3604730f-ef0e-4f7c-8560-4e64427cddc5');
    });

    it('공백/줄 시작 뒤의 /command 는 여전히 매칭된다 (회귀 방지)', () => {
        assertHighlighted('Unknown command. Try /help', compiled, '/help', 'command');
        assertHighlighted('/stop', compiled, '/stop', 'command');
    });

    it('독립된 UUID 는 여전히 강조된다 (회귀 방지)', () => {
        const line = '[17:24:15] [User Authenticator #1/INFO]: UUID of player Steve is 069a79f4-44e9-4726-a5be-fca90e38aaf5';
        assertHighlighted(line, compiled, '069a79f4-44e9-4726-a5be-fca90e38aaf5', 'uuid');
    });
});

describe('Zomboid 규칙 통합 검증', () => {
    beforeEach(() => __resetRuleCache());

    const { compiled, diagnostics } = compileRules(ZOMBOID_RULES);

    it('모든 규칙이 진단 오류 없이 컴파일된다', () => {
        const errors = diagnostics.filter((d) => d.level === 'error');
        expect(errors, JSON.stringify(errors)).toEqual([]);
    });

    it('레벨 토큰은 `:` 앞에서만 매칭된다 (오탐 방지)', () => {
        // LOG 가 포함된 자유 텍스트는 하이라이트되면 안 됨
        const benign = '12-30-2024 10:15:23.456 LOG: message contains the word LOG casually';
        const segments = highlightLine(benign, compiled);
        // 두 번째 "LOG" (casually 앞) 는 색칠되면 안 됨. 첫 "LOG:" 만 highlight.
        const logHits = segments.filter((s) => s.text === 'LOG' && s.style);
        expect(logHits.length).toBe(1);
    });

    it('LOG  : General     , epoch> 포맷 완전 분해', () => {
        const line = 'LOG  : General     , 1703936123456> Loading completed';
        assertHighlighted(line, compiled, 'LOG', 'info');
        assertHighlighted(line, compiled, 'General', 'keyword');
        assertHighlighted(line, compiled, '1703936123456', 'dim');
    });

    it('log_category 는 단일 단어도 매치한다 (기존 결함 수정)', () => {
        const line = 'LOG  : Network     , 1703936123456> client connected';
        assertHighlighted(line, compiled, 'Network', 'keyword');
    });

    it('서버 시작/종료 메시지', () => {
        assertHighlighted('12-30-2024 10:15:23.456 SERVER STARTED', compiled, 'SERVER STARTED', 'success');
        assertHighlighted('12-30-2024 10:15:23.456 Quitting!', compiled, 'Quitting!', 'warn');
    });

    it('플레이어 접속 — "quoted" 및 plain 모두 처리', () => {
        assertHighlighted('LOG  : General     , 1703936123456> User "SurvivorSteve" fully connected', compiled, 'SurvivorSteve', 'player');
        assertHighlighted('LOG  : General     , 1703936123456> Alice fully connected', compiled, 'Alice', 'player');
    });

    it('Steam ID 는 7656119 prefix + 10자리 뒷자리만 매치', () => {
        assertHighlighted('Player steamID=76561198012345678', compiled, '76561198012345678', 'uuid');
        // 틀린 prefix
        assertNotHighlighted('Number 12345611980123456', compiled, '12345611980123456');
    });

    it('Unix 경로는 최소 2 세그먼트 요구 (URL/단순 슬래시 오탐 방지)', () => {
        assertHighlighted('Loading /opt/zomboid/server/config.ini', compiled, '/opt/zomboid/server/config.ini', 'path');
        // 단일 슬래시 + 단어는 매치되지 않아야 한다
        const segments = highlightLine('Format: key/value', compiled);
        const pathSeg = segments.find((s) => s.text.includes('/value') && s.style?.color?.includes('path'));
        expect(pathSeg).toBeFalsy();
    });

    it('Windows 경로', () => {
        assertHighlighted('Save path: C:\\Zomboid\\Saves\\world.zip', compiled, 'C:\\Zomboid\\Saves\\world.zip', 'path');
    });

    it('IP:port + duration(ms)', () => {
        assertHighlighted('Ping from 192.168.1.50:16261 was 45 ms', compiled, '192.168.1.50:16261', 'ip');
        assertHighlighted('Ping from 192.168.1.50:16261 was 45 ms', compiled, '45 ms', 'duration');
    });
});
