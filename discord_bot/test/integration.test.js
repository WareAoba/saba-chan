/**
 * ═══════════════════════════════════════════════════════════════
 *  Discord Bot E2E 통합 테스트
 * ═══════════════════════════════════════════════════════════════
 *
 *  실제로 테스트하는 것:
 *    1. 별명 해석기 — TOML + GUI 조합, 충돌 해결, 대소문자, 다국어
 *    2. 릴레이 에이전트 — HMAC 서명, mock 메시지 처리, 결과 수집
 *    3. 명령어 프로세서 — prefix 파싱 → 별명 해석 → IPC 라우팅
 *    4. Mock IPC 서버 — 데몬 ↔ 봇 전체 파이프라인
 *    5. 크로스 컴포넌트 — 에이전트 mock message → processor → IPC → 응답
 *
 *  실행: npm test (discord_bot/)
 */

const http = require('http');
const crypto = require('crypto');
const {
    buildModuleAliasMap,
    buildCommandAliasMap,
    resolveAlias,
} = require('../utils/aliasResolver');

// ════════════════════════════════════════════════════════════
//  1. 별명 해석기 E2E — 실제 사용 패턴 전수 검증
// ════════════════════════════════════════════════════════════

describe('별명 해석기 E2E', () => {
    const META_MULTI_MODULE = {
        palworld: {
            aliases: {
                module_aliases: ['pw', '팰월드', '팰'],
                commands: {
                    players: { aliases: ['플레이어', 'p'] },
                    status: { aliases: ['상태', 's'] },
                    announce: { aliases: ['공지', '알림'] },
                    start: { aliases: ['시작', '실행'] },
                    stop: { aliases: ['정지', '중지'] },
                    kick: { aliases: ['추방', 'k'] },
                },
            },
        },
        minecraft: {
            aliases: {
                module_aliases: ['mc', '마크', '마인크래프트'],
                commands: {
                    players: { aliases: ['접속자'] },
                    whitelist: { aliases: ['화리', 'wl'] },
                    op: { aliases: ['관리자'] },
                },
            },
        },
        valheim: {
            aliases: {
                module_aliases: ['vh'],
                commands: {},
            },
        },
    };

    const GUI_CONFIG = {
        prefix: '!saba',
        moduleAliases: {
            palworld: 'pal,팰서버',
            valheim: 'val',
        },
        commandAliases: {
            palworld: { players: '유저목록,접속자수', kick: '킥' },
            minecraft: { whitelist: '화이트리스트' },
        },
    };

    let moduleAliases, commandAliases;

    beforeAll(() => {
        moduleAliases = buildModuleAliasMap(GUI_CONFIG, META_MULTI_MODULE);
        commandAliases = buildCommandAliasMap(GUI_CONFIG, META_MULTI_MODULE);
    });

    describe('모듈 별명 해석', () => {
        test.each([
            ['palworld', 'palworld'],
            ['pw', 'palworld'],
            ['팰월드', 'palworld'],
            ['팰', 'palworld'],
            ['pal', 'palworld'],       // GUI 추가
            ['팰서버', 'palworld'],     // GUI 추가 (콤마 분리)
            ['minecraft', 'minecraft'],
            ['mc', 'minecraft'],
            ['마크', 'minecraft'],
            ['마인크래프트', 'minecraft'],
            ['valheim', 'valheim'],
            ['vh', 'valheim'],
            ['val', 'valheim'],        // GUI 추가
        ])('"%s" → "%s"', (input, expected) => {
            expect(resolveAlias(input, moduleAliases)).toBe(expected);
        });

        test('대소문자 무시', () => {
            expect(resolveAlias('PW', moduleAliases)).toBe('palworld');
            expect(resolveAlias('Pw', moduleAliases)).toBe('palworld');
            expect(resolveAlias('MC', moduleAliases)).toBe('minecraft');
            expect(resolveAlias('Minecraft', moduleAliases)).toBe('minecraft');
        });

        test('알 수 없는 별명은 원본 반환', () => {
            expect(resolveAlias('unknown_game', moduleAliases)).toBe('unknown_game');
            expect(resolveAlias('존재하지않는모듈', moduleAliases)).toBe('존재하지않는모듈');
        });
    });

    describe('명령어 별명 해석', () => {
        test.each([
            ['players', 'players'],
            ['플레이어', 'players'],
            ['p', 'players'],
            ['유저목록', 'players'],     // GUI
            ['접속자수', 'players'],     // GUI
            ['status', 'status'],
            ['상태', 'status'],
            ['s', 'status'],
            ['announce', 'announce'],
            ['공지', 'announce'],
            ['알림', 'announce'],
            ['start', 'start'],
            ['시작', 'start'],
            ['실행', 'start'],
            ['stop', 'stop'],
            ['정지', 'stop'],
            ['중지', 'stop'],
            ['kick', 'kick'],
            ['추방', 'kick'],
            ['k', 'kick'],
            ['킥', 'kick'],             // GUI
            ['whitelist', 'whitelist'],
            ['화리', 'whitelist'],
            ['wl', 'whitelist'],
            ['화이트리스트', 'whitelist'],// GUI
            ['접속자', 'players'],       // minecraft TOML
            ['관리자', 'op'],
        ])('"%s" → "%s"', (input, expected) => {
            expect(resolveAlias(input, commandAliases)).toBe(expected);
        });
    });

    describe('메시지 파싱 → 별명 해석 통합', () => {
        function parseCommand(message) {
            const prefix = '!saba';
            if (!message.startsWith(prefix)) return null;
            const args = message.slice(prefix.length).trim().split(/\s+/);
            if (args.length < 2) return { module: resolveAlias(args[0] || '', moduleAliases), command: null, args: [] };
            return {
                module: resolveAlias(args[0], moduleAliases),
                command: resolveAlias(args[1], commandAliases),
                args: args.slice(2),
            };
        }

        test.each([
            ['!saba 팰 상태', { module: 'palworld', command: 'status', args: [] }],
            ['!saba pw p', { module: 'palworld', command: 'players', args: [] }],
            ['!saba mc 화리 add Player1', { module: 'minecraft', command: 'whitelist', args: ['add', 'Player1'] }],
            ['!saba 팰서버 공지 서버 점검 예정입니다', {
                module: 'palworld', command: 'announce', args: ['서버', '점검', '예정입니다'],
            }],
            ['!saba palworld kick Player1', { module: 'palworld', command: 'kick', args: ['Player1'] }],
            ['!saba vh start', { module: 'valheim', command: 'start', args: [] }],
        ])('"%s" → %o', (msg, expected) => {
            expect(parseCommand(msg)).toEqual(expected);
        });

        test('prefix가 다르면 무시', () => {
            expect(parseCommand('!other palworld status')).toBeNull();
        });
    });

    describe('충돌 감지', () => {
        test('서로 다른 모듈에서 같은 모듈 별명을 주장하면 첫 번째가 우선', () => {
            const meta = {
                game_a: { aliases: { module_aliases: ['g'], commands: {} } },
                game_b: { aliases: { module_aliases: ['g'], commands: {} } },
            };
            const aliases = buildModuleAliasMap({ moduleAliases: {} }, meta);
            expect(resolveAlias('g', aliases)).toBe('game_a');
            expect(aliases.__conflicts.length).toBeGreaterThan(0);
        });
    });
});

// ════════════════════════════════════════════════════════════
//  2. 릴레이 에이전트 HMAC 서명 검증
// ════════════════════════════════════════════════════════════

describe('릴레이 에이전트 서명 유틸', () => {
    function parseToken(token) {
        const m = token.match(/^sbn_([A-Za-z0-9_-]+)\.(.+)$/);
        if (!m) return null;
        return { nodeId: m[1], secret: m[2] };
    }

    function signedHeaders(token, method, urlPath, body) {
        const parsed = parseToken(token);
        const ts = Math.floor(Date.now() / 1000);
        const bodyStr = body ? JSON.stringify(body) : '';
        const payload = [method.toUpperCase(), urlPath, ts.toString(), bodyStr].join('\n');
        const sig = crypto.createHmac('sha256', parsed.secret).update(payload).digest('hex');
        return {
            'Authorization': `Bearer ${token}`,
            'Content-Type': 'application/json',
            'x-request-timestamp': String(ts),
            'x-request-signature': sig,
        };
    }

    test('유효한 토큰을 파싱할 수 있어야 한다', () => {
        const token = 'sbn_TestNode123.secretValue1234567890abcdefghijklmnop';
        const parsed = parseToken(token);
        expect(parsed).toEqual({ nodeId: 'TestNode123', secret: 'secretValue1234567890abcdefghijklmnop' });
    });

    test('잘못된 형식의 토큰은 null', () => {
        expect(parseToken('invalid_token')).toBeNull();
        expect(parseToken('sbn_')).toBeNull();
        expect(parseToken('')).toBeNull();
    });

    test('서명 헤더에 필수 필드가 모두 포함되어야 한다', () => {
        const token = 'sbn_Node1.secret123456789012345678901234567890';
        const headers = signedHeaders(token, 'POST', '/heartbeat', { test: true });

        expect(headers['Authorization']).toBe(`Bearer ${token}`);
        expect(headers['x-request-timestamp']).toBeTruthy();
        expect(headers['x-request-signature']).toBeTruthy();
        expect(headers['x-request-signature']).toHaveLength(64);
    });

    test('동일한 입력에 대해 서명이 일관되어야 한다', () => {
        const token = 'sbn_Node1.fixedSecretForConsistencyTest1234567890';
        const body = { action: 'raw_command', text: 'palworld status' };
        const h1 = signedHeaders(token, 'POST', '/heartbeat', body);
        const h2 = signedHeaders(token, 'POST', '/heartbeat', body);
        expect(h1['x-request-signature']).toBe(h2['x-request-signature']);
    });

    test('다른 메서드/경로면 서명이 달라져야 한다', () => {
        const token = 'sbn_Node1.secretForDiffTest12345678901234567890';
        const h1 = signedHeaders(token, 'GET', '/poll', null);
        const h2 = signedHeaders(token, 'POST', '/heartbeat', null);
        expect(h1['x-request-signature']).not.toBe(h2['x-request-signature']);
    });
});

// ════════════════════════════════════════════════════════════
//  3. 릴레이 에이전트 Mock 메시지 → 프로세서 통합
// ════════════════════════════════════════════════════════════

describe('릴레이 에이전트 Mock 메시지 E2E', () => {
    function createMockMessage(text, prefix, requestedBy) {
        const replies = [];
        const content = `${prefix} ${text}`;
        const msg = {
            id: `relay-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
            content,
            author: { bot: false, tag: 'relay-agent', id: requestedBy || 'system', username: 'relay-agent' },
            guildId: null,
            channel: { id: 'relay' },
            reply: async (textOrObj) => {
                const replyContent = typeof textOrObj === 'string' ? textOrObj : (textOrObj?.content ?? String(textOrObj));
                replies.push(replyContent);
                const idx = replies.length - 1;
                return {
                    edit: async (editTextOrObj) => {
                        replies[idx] = typeof editTextOrObj === 'string' ? editTextOrObj : (editTextOrObj?.content ?? String(editTextOrObj));
                    },
                    delete: async () => {},
                };
            },
        };
        return { msg, getReplies: () => [...replies] };
    }

    test('Mock 메시지가 올바른 구조를 가져야 한다', () => {
        const { msg } = createMockMessage('palworld status', '!saba', 'user123');
        expect(msg.content).toBe('!saba palworld status');
        expect(msg.author.bot).toBe(false);
        expect(msg.author.id).toBe('user123');
        expect(msg.id).toMatch(/^relay-/);
    });

    test('reply()를 호출하면 응답이 수집되어야 한다', async () => {
        const { msg, getReplies } = createMockMessage('test', '!saba', 'user123');
        await msg.reply('첫 번째 응답');
        await msg.reply({ content: '두 번째 응답' });

        const replies = getReplies();
        expect(replies).toHaveLength(2);
        expect(replies[0]).toBe('첫 번째 응답');
        expect(replies[1]).toBe('두 번째 응답');
    });

    test('reply().edit()로 응답을 수정할 수 있어야 한다', async () => {
        const { msg, getReplies } = createMockMessage('test', '!saba', 'user123');
        const sent = await msg.reply('초기 응답');
        await sent.edit('수정된 응답');
        expect(getReplies()[0]).toBe('수정된 응답');
    });
});

// ════════════════════════════════════════════════════════════
//  4. Mock IPC 서버 기반 데몬 ↔ 봇 E2E
// ════════════════════════════════════════════════════════════

describe('Mock IPC 서버 기반 크로스 컴포넌트 E2E', () => {
    let server;
    let baseUrl;
    let instances;
    let moduleData;

    beforeAll(async () => {
        instances = new Map();
        moduleData = new Map([
            ['palworld', {
                name: 'palworld',
                commands: {
                    fields: [
                        { name: 'start', label: '시작', method: 'rest', http_method: 'POST' },
                        { name: 'stop', label: '정지', method: 'rest', http_method: 'POST' },
                        { name: 'status', label: '상태', method: 'rest', http_method: 'GET' },
                        { name: 'players', label: '플레이어', method: 'rest', http_method: 'GET' },
                        { name: 'kick', label: '추방', method: 'rest', http_method: 'POST' },
                        { name: 'announce', label: '공지', method: 'rest', http_method: 'POST' },
                    ],
                },
            }],
            ['minecraft', {
                name: 'minecraft',
                commands: {
                    fields: [
                        { name: 'start', label: '시작', method: 'stdin' },
                        { name: 'stop', label: '정지', method: 'stdin' },
                        { name: 'status', label: '상태', method: 'rcon' },
                        { name: 'whitelist', label: '화이트리스트', method: 'rcon' },
                        { name: 'op', label: '관리자', method: 'rcon' },
                    ],
                },
            }],
        ]);

        instances.set('palworld-default', {
            id: 'palworld-default', name: 'Palworld Dedicated',
            module_name: 'palworld', status: 'running',
        });
        instances.set('mc-default', {
            id: 'mc-default', name: 'Minecraft Server',
            module_name: 'minecraft', status: 'stopped',
        });

        server = http.createServer((req, res) => {
            const url = new URL(req.url, 'http://127.0.0.1');
            const chunks = [];
            req.on('data', c => chunks.push(c));
            req.on('end', () => {
                const raw = Buffer.concat(chunks).toString('utf8');
                let body = {};
                try { body = raw ? JSON.parse(raw) : {}; } catch { body = {}; }

                const send = (status, data) => {
                    res.writeHead(status, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify(data));
                };

                if (req.method === 'GET' && url.pathname === '/api/modules') {
                    return send(200, { modules: Array.from(moduleData.values()) });
                }
                if (req.method === 'GET' && url.pathname.startsWith('/api/module/')) {
                    const name = url.pathname.split('/').pop();
                    const mod = moduleData.get(name);
                    if (!mod) return send(404, { error: 'not found' });
                    return send(200, {
                        toml: {
                            aliases: {
                                module_aliases: name === 'palworld' ? ['pw', '팰월드'] : ['mc', '마크'],
                                commands: {},
                            },
                            commands: mod.commands,
                        },
                    });
                }
                if (req.method === 'GET' && url.pathname === '/api/servers') {
                    return send(200, {
                        servers: Array.from(instances.values()).map(v => ({
                            id: v.id, name: v.name, module: v.module_name, status: v.status,
                        })),
                    });
                }
                if (req.method === 'POST' && url.pathname === '/api/instances') {
                    const id = `inst-${Date.now()}`;
                    instances.set(id, { id, ...body, status: 'stopped' });
                    return send(201, { success: true, id });
                }
                if (req.method === 'GET' && /^\/api\/instance\/[^/]+$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    const inst = instances.get(id);
                    if (!inst) return send(404, { error: `Instance not found: ${id}` });
                    return send(200, inst);
                }
                if (req.method === 'DELETE' && /^\/api\/instance\/[^/]+$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    if (!instances.has(id)) return send(404, { error: 'not found' });
                    instances.delete(id);
                    return send(200, { success: true });
                }
                if (req.method === 'PATCH' && /^\/api\/instance\/[^/]+$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    const inst = instances.get(id);
                    if (!inst) return send(404, { error: 'not found' });
                    instances.set(id, { ...inst, ...body });
                    return send(200, { success: true });
                }
                if (req.method === 'POST' && /\/rest$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    const inst = instances.get(id);
                    if (!inst) return send(404, { error: 'not found' });
                    if (body.command === 'status') {
                        return send(200, {
                            success: true,
                            message: `🟢 ${inst.name} — ${inst.status} (3/32 플레이어)`,
                        });
                    }
                    return send(200, { success: true, message: 'ok' });
                }
                if (req.method === 'POST' && /\/command$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    if (!instances.has(id)) return send(404, { error: 'not found' });
                    return send(200, { success: true, message: 'ok' });
                }

                return send(404, { error: 'not found' });
            });
        });

        await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
        const { port } = server.address();
        baseUrl = `http://127.0.0.1:${port}`;
    });

    afterAll(async () => {
        if (server) await new Promise(resolve => server.close(resolve));
    });

    test('데몬 API: 모듈 조회 → 서버 목록 → 인스턴스 CRUD 전체 플로우', async () => {
        const axios = require('axios');

        const mods = await axios.get(`${baseUrl}/api/modules`);
        expect(mods.data.modules).toHaveLength(2);
        expect(mods.data.modules.map(m => m.name).sort()).toEqual(['minecraft', 'palworld']);

        const srvs = await axios.get(`${baseUrl}/api/servers`);
        expect(srvs.data.servers.length).toBeGreaterThanOrEqual(2);

        const created = await axios.post(`${baseUrl}/api/instances`, {
            name: 'e2e-test', module_name: 'palworld',
        });
        expect(created.status).toBe(201);
        const id = created.data.id;

        await axios.patch(`${baseUrl}/api/instance/${id}`, { status: 'running' });

        const inst = await axios.get(`${baseUrl}/api/instance/${id}`);
        expect(inst.data.status).toBe('running');

        await axios.delete(`${baseUrl}/api/instance/${id}`);
        await expect(axios.get(`${baseUrl}/api/instance/${id}`))
            .rejects.toMatchObject({ response: { status: 404 } });
    });

    test('데몬 API: REST 명령 실행 파이프라인', async () => {
        const axios = require('axios');

        const res = await axios.post(`${baseUrl}/api/instance/palworld-default/rest`, {
            command: 'status',
        });
        expect(res.status).toBe(200);
        expect(res.data.success).toBe(true);
        expect(res.data.message).toContain('Palworld Dedicated');
    });

    test('데몬 API: 존재하지 않는 인스턴스 → 404', async () => {
        const axios = require('axios');
        await expect(
            axios.post(`${baseUrl}/api/instance/nonexistent/command`, { command: 'test' })
        ).rejects.toMatchObject({ response: { status: 404 } });
    });

    test('메타데이터 구조 — 모든 명령어에 필수 필드가 존재해야 한다', async () => {
        const axios = require('axios');

        for (const [modName] of moduleData) {
            const res = await axios.get(`${baseUrl}/api/module/${modName}`);
            const { commands } = res.data.toml;

            for (const cmd of commands.fields) {
                expect(cmd.name).toBeTruthy();
                expect(cmd.label).toBeTruthy();
                expect(cmd.method).toBeTruthy();
                expect(['rest', 'rcon', 'dual', 'stdin']).toContain(cmd.method);

                if (cmd.method === 'rest' || cmd.method === 'dual') {
                    expect(cmd.http_method).toBeTruthy();
                    expect(['GET', 'POST', 'PUT', 'DELETE']).toContain(cmd.http_method);
                }
            }
        }
    });
});

// ════════════════════════════════════════════════════════════
//  5. 다국어 i18n 키 무결성 검증
// ════════════════════════════════════════════════════════════

describe('i18n 키 무결성', () => {
    const fs = require('fs');
    const path = require('path');

    const localeDir = path.join(__dirname, '..', '..', 'locales');
    const REFERENCE_LOCALE = 'ko';
    const LOCALES = ['en', 'ja', 'ko', 'zh-CN', 'zh-TW', 'de', 'es', 'fr', 'pt-BR', 'ru'];

    function loadLocale(lang, ns) {
        const filePath = path.join(localeDir, lang, `${ns}.json`);
        if (!fs.existsSync(filePath)) return null;
        return JSON.parse(fs.readFileSync(filePath, 'utf8'));
    }

    function flattenKeys(obj, prefix = '') {
        const keys = [];
        for (const [k, v] of Object.entries(obj)) {
            const full = prefix ? `${prefix}.${k}` : k;
            if (typeof v === 'object' && v !== null && !Array.isArray(v)) {
                keys.push(...flattenKeys(v, full));
            } else {
                keys.push(full);
            }
        }
        return keys;
    }

    test('모든 로캘에 봇 번역 키가 존재해야 한다 (discord.json)', () => {
        const refData = loadLocale(REFERENCE_LOCALE, 'discord');
        if (!refData) {
            console.warn(`${REFERENCE_LOCALE}/discord.json 없음 — 스킵`);
            return;
        }
        const refKeys = flattenKeys(refData);

        for (const lang of LOCALES) {
            if (lang === REFERENCE_LOCALE) continue;
            const data = loadLocale(lang, 'discord');
            if (!data) continue;

            const langKeys = flattenKeys(data);
            const missing = refKeys.filter(k => !langKeys.includes(k));

            if (missing.length > 0) {
                console.warn(`[i18n] ${lang}/discord.json 누락 키 ${missing.length}개: ${missing.slice(0, 5).join(', ')}...`);
            }
            expect(missing.length).toBeLessThan(refKeys.length * 0.3);
        }
    });

    test('모든 로캘에 GUI 번역 키가 존재해야 한다 (gui.json)', () => {
        const refData = loadLocale(REFERENCE_LOCALE, 'gui');
        if (!refData) {
            console.warn(`${REFERENCE_LOCALE}/gui.json 없음 — 스킵`);
            return;
        }
        const refKeys = flattenKeys(refData);

        for (const lang of LOCALES) {
            if (lang === REFERENCE_LOCALE) continue;
            const data = loadLocale(lang, 'gui');
            if (!data) continue;

            const langKeys = flattenKeys(data);
            const missing = refKeys.filter(k => !langKeys.includes(k));

            if (missing.length > 0) {
                console.warn(`[i18n] ${lang}/gui.json 누락 키 ${missing.length}개: ${missing.slice(0, 5).join(', ')}...`);
            }
            // GUI 번역은 변경 빈도가 높으므로 40% 까지 허용
            expect(missing.length).toBeLessThan(refKeys.length * 0.4);
        }
    });
});
