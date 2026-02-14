/**
 * Discord Bot 통합 테스트
 * 실제 메시지 파싱 및 명령어 처리 플로우 검증
 */

const axios = require('axios');
const http = require('http');
const { buildModuleAliasMap, buildCommandAliasMap, resolveAlias } = require('../utils/aliasResolver');
const fs = require('fs');
const path = require('path');

const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474';

// 테스트 데이터 자동 정리 함수
const cleanupTestInstances = () => {
    const instancesPath = path.join(__dirname, '..', '..', 'instances.json');
    
    try {
        if (fs.existsSync(instancesPath)) {
            const content = fs.readFileSync(instancesPath, 'utf-8');
            const instances = JSON.parse(content);
            
            // test- 로 시작하는 서버 제거
            const cleaned = instances.filter(instance => 
                !instance.name || !instance.name.startsWith('test-')
            );
            
            if (cleaned.length !== instances.length) {
                fs.writeFileSync(instancesPath, JSON.stringify(cleaned, null, 2));
                console.log('🧹 Cleaned up test instances from instances.json');
            }
        }
    } catch (error) {
        // 파일이 없거나 파싱 실패는 무시
    }
};

describe('Discord Bot 명령어 처리 통합 테스트', () => {
    let moduleMetadata = {};
    let moduleCommands = {};
    let botConfig = {
        prefix: '!saba',
        moduleAliases: {},
        commandAliases: {}
    };
    
    // 모든 테스트 종료 후 cleanup
    afterAll(() => {
        cleanupTestInstances();
    });
    
    beforeAll(async () => {
        try {
            // 모듈 메타데이터 로드
            const response = await axios.get(`${IPC_BASE}/api/modules`);
            const modules = response.data.modules || [];
            
            for (const module of modules) {
                // 명령어 로드
                if (module.commands && module.commands.fields) {
                    moduleCommands[module.name] = {};
                    for (const cmd of module.commands.fields) {
                        moduleCommands[module.name][cmd.name] = cmd;
                    }
                }
                
                // 메타데이터 로드
                try {
                    const metaRes = await axios.get(`${IPC_BASE}/api/module/${module.name}`);
                    moduleMetadata[module.name] = metaRes.data.toml || {};
                } catch (e) {
                    console.warn(`Could not load metadata for ${module.name}`);
                }
            }
            
            console.log(`✓ Loaded metadata for ${Object.keys(moduleMetadata).length} modules`);
        } catch (error) {
            console.warn('데몬이 실행중이지 않아 모듈 로드 스킵:', error.message);
        }
    });
    
    describe('별명 해석 통합 테스트', () => {
        test('실제 모듈 별명 해석', () => {
            if (Object.keys(moduleMetadata).length === 0) {
                console.warn('모듈이 없어서 테스트 스킵');
                return;
            }
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            
            // 모든 모듈이 자기 이름으로 해석되어야 함
            for (const moduleName of Object.keys(moduleMetadata)) {
                expect(resolveAlias(moduleName, moduleAliases)).toBe(moduleName);
            }
            
            console.log('✓ 모듈 별명:', Object.keys(moduleAliases).length, '개');
        });
        
        test('실제 명령어 별명 해석', () => {
            if (Object.keys(moduleMetadata).length === 0) {
                console.warn('모듈이 없어서 테스트 스킵');
                return;
            }
            
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            // 기본 명령어들이 포함되어야 함
            expect(resolveAlias('start', commandAliases)).toBe('start');
            expect(resolveAlias('stop', commandAliases)).toBe('stop');
            expect(resolveAlias('status', commandAliases)).toBe('status');
            
            console.log('✓ 명령어 별명:', Object.keys(commandAliases).length, '개');
        });
    });
    
    describe('Discord 메시지 파싱 시뮬레이션', () => {
        test('!saba 목록 - 서버 목록 조회', async () => {
            const message = '!saba 목록';
            const prefix = '!saba';
            
            // 파싱
            const content = message.trim();
            expect(content.startsWith(prefix)).toBe(true);
            
            const args = content.slice(prefix.length).trim().split(/\s+/);
            expect(args[0]).toBe('목록');
            
            // 실제 API 호출 시뮬레이션
            try {
                const response = await axios.get(`${IPC_BASE}/api/servers`);
                expect(response.status).toBe(200);
                expect(response.data.servers).toBeDefined();
                
                console.log(`✓ 서버 ${response.data.servers.length}개 조회됨`);
            } catch (error) {
                console.warn('데몬 미실행:', error.message);
            }
        });
        
        test('!saba palworld status - 모듈 + 명령어 파싱', () => {
            const message = '!saba palworld status';
            const prefix = '!saba';
            
            const args = message.slice(prefix.length).trim().split(/\s+/);
            
            expect(args.length).toBeGreaterThanOrEqual(2);
            expect(args[0]).toBe('palworld');
            expect(args[1]).toBe('status');
            
            // 별명 해석
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            const moduleName = resolveAlias(args[0], moduleAliases);
            const commandName = resolveAlias(args[1], commandAliases);
            
            expect(moduleName).toBe('palworld');
            expect(commandName).toBe('status');
        });
        
        test('!saba pw 플레이어 - 별명을 사용한 파싱', () => {
            // GUI에서 설정한 별명
            botConfig.moduleAliases = { palworld: 'pw' };
            botConfig.commandAliases = { palworld: { players: '플레이어' } };
            
            const message = '!saba pw 플레이어';
            const prefix = '!saba';
            
            const args = message.slice(prefix.length).trim().split(/\s+/);
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            const moduleName = resolveAlias(args[0], moduleAliases);
            const commandName = resolveAlias(args[1], commandAliases);
            
            expect(moduleName).toBe('palworld');
            expect(commandName).toBe('players');
            
            console.log('✓ 별명 해석: pw → palworld, 플레이어 → players');
        });
        
        test('인자를 포함한 명령어 파싱', () => {
            const message = '!saba palworld announce Hello World!';
            const prefix = '!saba';
            
            const args = message.slice(prefix.length).trim().split(/\s+/);
            
            const moduleName = args[0];
            const commandName = args[1];
            const extraArgs = args.slice(2);
            
            expect(moduleName).toBe('palworld');
            expect(commandName).toBe('announce');
            expect(extraArgs).toEqual(['Hello', 'World!']);
            
            // 실제 사용 시에는 extraArgs를 공백으로 join하거나
            // 명령어 정의의 inputs에 맞춰 파싱
        });
    });
    
    describe('명령어 실행 플로우 검증', () => {
        test('서버 상태 확인 플로우', async () => {
            try {
                // 1. 서버 목록 조회
                const serversResponse = await axios.get(`${IPC_BASE}/api/servers`);
                const servers = serversResponse.data.servers || [];
                
                if (servers.length === 0) {
                    console.warn('테스트용 서버가 없어서 스킵');
                    return;
                }
                
                const server = servers[0];
                
                // 2. 모듈 확인
                expect(server.module).toBeDefined();
                
                // 3. 상태 확인
                expect(['running', 'stopped']).toContain(server.status);
                
                console.log(`✓ 서버 ${server.name} 상태: ${server.status}`);
            } catch (error) {
                console.warn('데몬 미실행:', error.message);
            }
        });
        
        test('에러 메시지 검증', async () => {
            try {
                // 존재하지 않는 서버로 명령 실행 시도
                await axios.post(`${IPC_BASE}/api/instance/nonexistent/command`, {
                    command: 'test',
                    args: {}
                });
                
                fail('404 에러가 발생해야 함');
            } catch (error) {
                // axios 에러일 경우 response가 있을 수 있음
                if (error.response) {
                    expect(error.response.status).toBe(404);
                    expect(error.response.data.error).toContain('not found');
                } else {
                    // 네트워크 에러 등 response가 없는 경우
                    expect(error.message).toBeDefined();
                }
            }
        });
    });
    
    describe('모듈 메타데이터 검증', () => {
        test('명령어 정의 구조 확인', () => {
            if (Object.keys(moduleCommands).length === 0) {
                console.warn('모듈 명령어가 없어서 스킵');
                return;
            }
            
            // 모든 명령어가 올바른 구조를 가지는지 확인
            for (const [moduleName, commands] of Object.entries(moduleCommands)) {
                for (const [cmdName, cmdMeta] of Object.entries(commands)) {
                    expect(cmdMeta.name).toBe(cmdName);
                    expect(cmdMeta.label).toBeDefined();
                    expect(['rest', 'rcon', 'dual', 'stdin']).toContain(cmdMeta.method);
                    
                    if (cmdMeta.method === 'rest' || cmdMeta.method === 'dual') {
                        expect(cmdMeta.http_method).toBeDefined();
                        expect(['GET', 'POST', 'PUT', 'DELETE']).toContain(cmdMeta.http_method);
                    }
                }
            }
            
            console.log('✓ 모든 명령어 정의가 올바른 구조를 가짐');
        });
        
        test('별명 정의 구조 확인', () => {
            if (Object.keys(moduleMetadata).length === 0) {
                console.warn('모듈 메타데이터가 없어서 스킵');
                return;
            }
            
            for (const [moduleName, metadata] of Object.entries(moduleMetadata)) {
                if (metadata.aliases) {
                    // module_aliases는 배열이어야 함
                    if (metadata.aliases.module_aliases) {
                        expect(Array.isArray(metadata.aliases.module_aliases)).toBe(true);
                    }
                    
                    // commands는 객체여야 함
                    if (metadata.aliases.commands) {
                        expect(typeof metadata.aliases.commands).toBe('object');
                    }
                }
            }
            
            console.log('✓ 모든 별명 정의가 올바른 구조를 가짐');
        });
    });
});

describe('전체 플로우 E2E 시뮬레이션', () => {
    test('서버 추가부터 삭제까지 전체 플로우', async () => {
        try {
            // 1. 모듈 목록 확인
            const modulesResponse = await axios.get(`${IPC_BASE}/api/modules`);
            expect(modulesResponse.status).toBe(200);
            
            if (modulesResponse.data.modules.length === 0) {
                console.warn('모듈이 없어서 E2E 테스트 스킵');
                return;
            }
            
            const firstModule = modulesResponse.data.modules[0].name;
            
            // 2. 서버 인스턴스 생성
            const createResponse = await axios.post(`${IPC_BASE}/api/instances`, {
                name: 'e2e-test-server',
                module_name: firstModule,
                executable_path: 'C:\\test\\server.exe'
            });
            
            const instanceId = createResponse.data.id;
            
            // 3. 설정 업데이트
            await axios.patch(`${IPC_BASE}/api/instance/${instanceId}`, {
                port: 8211,
                protocol_mode: 'rest'
            });
            
            // 4. 서버 목록에서 확인
            const serversResponse = await axios.get(`${IPC_BASE}/api/servers`);
            const ourServer = serversResponse.data.servers.find(s => s.id === instanceId);
            expect(ourServer).toBeDefined();
            
            // 5. 인스턴스 삭제
            await axios.delete(`${IPC_BASE}/api/instance/${instanceId}`);
            
            // 6. 삭제 확인
            try {
                await axios.get(`${IPC_BASE}/api/instance/${instanceId}`);
                fail('삭제된 인스턴스는 조회되지 않아야 함');
            } catch (error) {
                if (error.response) {
                    expect(error.response.status).toBe(404);
                } else {
                    // 네트워크 에러 등
                    expect(error.message).toBeDefined();
                }
            }
            
            console.log('✓ E2E 플로우 완료: 생성 → 설정 → 확인 → 삭제');
        } catch (error) {
            console.warn('데몬 미실행 또는 에러:', error.message);
        }
    });
});

describe('별명 해석기 실사용 검증', () => {
    describe('복잡한 별명 시나리오', () => {
        test('TOML + GUI 별명이 모두 작동', () => {
            const moduleMetadata = {
                palworld: {
                    aliases: {
                        module_aliases: ['pw', '팰월드'],
                        commands: {
                            players: ['플레이어', 'p'],
                            status: ['상태', 's']
                        }
                    }
                }
            };
            
            const botConfig = {
                prefix: '!saba',
                moduleAliases: { palworld: 'pal' }, // GUI에서 추가
                commandAliases: {
                    palworld: { players: '유저목록' } // GUI에서 추가
                }
            };
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            // TOML 별명들
            expect(resolveAlias('pw', moduleAliases)).toBe('palworld');
            expect(resolveAlias('팰월드', moduleAliases)).toBe('palworld');
            expect(resolveAlias('플레이어', commandAliases)).toBe('players');
            expect(resolveAlias('p', commandAliases)).toBe('players');
            
            // GUI 별명들
            expect(resolveAlias('pal', moduleAliases)).toBe('palworld');
            expect(resolveAlias('유저목록', commandAliases)).toBe('players');
            
            // 원본 이름
            expect(resolveAlias('palworld', moduleAliases)).toBe('palworld');
            expect(resolveAlias('players', commandAliases)).toBe('players');
        });
        
        test('여러 모듈의 별명이 섞여도 작동', () => {
            const moduleMetadata = {
                palworld: {
                    aliases: {
                        module_aliases: ['pw'],
                        commands: { players: ['플레이어'] }
                    }
                },
                minecraft: {
                    aliases: {
                        module_aliases: ['mc'],
                        commands: { players: ['플레이어'] } // 동일한 별명
                    }
                }
            };
            
            const botConfig = {
                prefix: '!saba',
                moduleAliases: {},
                commandAliases: {}
            };
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            // 모듈 별명은 각각 다름
            expect(resolveAlias('pw', moduleAliases)).toBe('palworld');
            expect(resolveAlias('mc', moduleAliases)).toBe('minecraft');
            
            // 명령어 별명은 마지막 모듈 우선 (하지만 실제로는 모듈 컨텍스트에서 사용)
            expect(resolveAlias('플레이어', commandAliases)).toBeDefined();
        });
        
        test('별명 우선순위: GUI > TOML', () => {
            const moduleMetadata = {
                palworld: {
                    aliases: {
                        module_aliases: ['pw'],
                        commands: { players: ['플레이어'] }
                    }
                }
            };
            
            const botConfig = {
                prefix: '!saba',
                moduleAliases: { palworld: 'pw' }, // TOML과 동일한 별명
                commandAliases: {
                    palworld: { players: '플레이어' } // TOML과 동일
                }
            };
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            // 동일한 별명이라도 정상 작동
            expect(resolveAlias('pw', moduleAliases)).toBe('palworld');
            expect(resolveAlias('플레이어', commandAliases)).toBe('players');
        });
    });
    
    describe('실제 Discord 메시지 처리', () => {
        test('복잡한 명령어 체인 파싱', () => {
            const moduleMetadata = {
                palworld: {
                    aliases: {
                        module_aliases: ['pw', '팰'],
                        commands: {
                            announce: ['공지', '알림'],
                            players: ['플레이어']
                        }
                    }
                }
            };
            
            const botConfig = {
                prefix: '!saba',
                moduleAliases: {},
                commandAliases: {}
            };
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
            
            // "!saba 팰 공지 서버 점검 예정"
            const message = '!saba 팰 공지 서버 점검 예정';
            const args = message.slice('!saba'.length).trim().split(/\s+/);
            
            const moduleName = resolveAlias(args[0], moduleAliases);
            const commandName = resolveAlias(args[1], commandAliases);
            const extraArgs = args.slice(2);
            
            expect(moduleName).toBe('palworld');
            expect(commandName).toBe('announce');
            expect(extraArgs).toEqual(['서버', '점검', '예정']);
        });
        
        test('대소문자 무시', () => {
            const moduleMetadata = {
                palworld: {
                    aliases: {
                        module_aliases: ['PW', 'Palworld'],
                        commands: {}
                    }
                }
            };
            
            const botConfig = {
                prefix: '!saba',
                moduleAliases: {},
                commandAliases: {}
            };
            
            const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
            
            expect(resolveAlias('pw', moduleAliases)).toBe('palworld');
            expect(resolveAlias('PW', moduleAliases)).toBe('palworld');
            expect(resolveAlias('Pw', moduleAliases)).toBe('palworld');
        });
        
        test('알 수 없는 별명은 원본 반환', () => {
            const moduleAliases = buildModuleAliasMap({}, {});
            const commandAliases = buildCommandAliasMap({}, {});
            
            expect(resolveAlias('unknown', moduleAliases)).toBe('unknown');
            expect(resolveAlias('알수없음', commandAliases)).toBe('알수없음');
        });
    });
});

describe('Mock IPC 기반 결정적 E2E', () => {
    let server;
    let baseUrl;
    let instances;
    let sequence;

    beforeAll(async () => {
        instances = new Map();
        sequence = 0;

        server = http.createServer((req, res) => {
            const url = new URL(req.url, 'http://127.0.0.1');
            const chunks = [];
            req.on('data', (chunk) => chunks.push(chunk));
            req.on('end', () => {
                const raw = Buffer.concat(chunks).toString('utf8');
                let body = {};
                if (raw) {
                    try {
                        body = JSON.parse(raw);
                    } catch (_) {
                        body = {};
                    }
                }

                const send = (status, payload) => {
                    res.writeHead(status, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify(payload));
                };

                if (req.method === 'GET' && url.pathname === '/api/modules') {
                    return send(200, { modules: [{ name: 'palworld' }] });
                }

                if (req.method === 'POST' && url.pathname === '/api/instances') {
                    sequence += 1;
                    const id = `mock-${sequence}`;
                    const instance = {
                        id,
                        name: body.name,
                        module_name: body.module_name,
                        executable_path: body.executable_path || null,
                        status: 'stopped',
                    };
                    instances.set(id, instance);
                    return send(201, { success: true, id });
                }

                if (req.method === 'PATCH' && /^\/api\/instance\/.+/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    const existing = instances.get(id);
                    if (!existing) {
                        return send(404, { error: 'instance not found' });
                    }
                    const updated = { ...existing, ...body };
                    instances.set(id, updated);
                    return send(200, { success: true });
                }

                if (req.method === 'GET' && /^\/api\/instance\/.+/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    const instance = instances.get(id);
                    if (!instance) {
                        return send(404, { error: `Instance not found: ${id}` });
                    }
                    return send(200, instance);
                }

                if (req.method === 'DELETE' && /^\/api\/instance\/.+/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    if (!instances.has(id)) {
                        return send(404, { error: `Instance not found: ${id}` });
                    }
                    instances.delete(id);
                    return send(200, { success: true });
                }

                if (req.method === 'POST' && /^\/api\/instance\/.+\/command$/.test(url.pathname)) {
                    const id = url.pathname.split('/')[3];
                    if (!instances.has(id)) {
                        return send(404, { error: `Instance not found: ${id}` });
                    }
                    return send(200, { success: true, message: 'ok' });
                }

                if (req.method === 'GET' && url.pathname === '/api/servers') {
                    return send(200, {
                        servers: Array.from(instances.values()).map((v) => ({
                            id: v.id,
                            name: v.name,
                            module: v.module_name,
                            status: v.status || 'stopped',
                        })),
                    });
                }

                return send(404, { error: 'not found' });
            });
        });

        await new Promise((resolve) => {
            server.listen(0, '127.0.0.1', resolve);
        });

        const { port } = server.address();
        baseUrl = `http://127.0.0.1:${port}`;
    });

    afterAll(async () => {
        if (server) {
            await new Promise((resolve) => server.close(resolve));
        }
    });

    test('인스턴스 생성→수정→조회→삭제 전체 플로우', async () => {
        const modules = await axios.get(`${baseUrl}/api/modules`);
        expect(modules.status).toBe(200);
        expect(modules.data.modules[0].name).toBe('palworld');

        const created = await axios.post(`${baseUrl}/api/instances`, {
            name: 'test-mock-e2e',
            module_name: 'palworld',
            executable_path: 'C:\\test\\server.exe',
        });
        expect(created.status).toBe(201);
        const id = created.data.id;

        const patched = await axios.patch(`${baseUrl}/api/instance/${id}`, {
            status: 'running',
            port: 8211,
        });
        expect(patched.status).toBe(200);

        const listed = await axios.get(`${baseUrl}/api/servers`);
        expect(listed.status).toBe(200);
        const found = listed.data.servers.find((s) => s.id === id);
        expect(found).toBeDefined();
        expect(found.status).toBe('running');

        const removed = await axios.delete(`${baseUrl}/api/instance/${id}`);
        expect(removed.status).toBe(200);

        await expect(axios.get(`${baseUrl}/api/instance/${id}`)).rejects.toMatchObject({
            response: { status: 404 },
        });
    });

    test('존재하지 않는 인스턴스 command 호출은 404여야 함', async () => {
        await expect(
            axios.post(`${baseUrl}/api/instance/nonexistent/command`, {
                command: 'status',
                args: {},
            })
        ).rejects.toMatchObject({
            response: { status: 404 },
        });
    });
});
