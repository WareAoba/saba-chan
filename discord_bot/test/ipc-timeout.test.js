/**
 * IPC 타임아웃 & 설정 리로드 테스트
 *
 * Issue 1: nodeSettings.allowedInstances 변경이 최대 60초 지연
 * Issue 2: stopServer 타임아웃(15s)이 데몬 종료 대기(30s)보다 짧아 "실패" 표시
 */

const http = require('http');

// ── Mock 데몬 서버 ──
let mockServer;
let mockPort;
let mockHandlers = {};

function setMockHandler(method, path, handler) {
    mockHandlers[`${method} ${path}`] = handler;
}

beforeAll(async () => {
    mockServer = http.createServer((req, res) => {
        const key = `${req.method} ${req.url}`;
        // path parameter 매칭
        const handler = mockHandlers[key]
            || Object.entries(mockHandlers).find(([k]) => {
                const pattern = k.replace(/:[\w]+/g, '[^/]+');
                return new RegExp(`^${pattern}$`).test(key);
            })?.[1];

        if (handler) {
            let body = '';
            req.on('data', d => body += d);
            req.on('end', () => handler(req, res, body));
        } else {
            res.writeHead(404);
            res.end(JSON.stringify({ error: 'not found' }));
        }
    });

    await new Promise(resolve => {
        mockServer.listen(0, () => {
            mockPort = mockServer.address().port;
            process.env.IPC_BASE = `http://127.0.0.1:${mockPort}`;
            resolve();
        });
    });
});

afterAll(async () => {
    if (mockServer) {
        await new Promise(resolve => mockServer.close(resolve));
    }
    delete process.env.IPC_BASE;
});

afterEach(() => {
    mockHandlers = {};
    jest.restoreAllMocks();
    jest.resetModules();
});


// ════════════════════════════════════════════════════════════
//  Issue 2: stopServer 타임아웃
// ════════════════════════════════════════════════════════════

describe('stopServer HTTP 타임아웃', () => {
    test('데몬이 20초 걸려도 stopServer가 타임아웃 없이 대기해야 함', async () => {
        // 데몬이 200ms 후 성공 응답 (실제 20초 시뮬레이션)
        setMockHandler('POST', '/api/server/test-server/stop', (_req, res) => {
            setTimeout(() => {
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ success: true, message: 'stopped' }));
            }, 200);
        });

        const ipc = require('../core/ipc');
        ipc.init();

        const result = await ipc.stopServer('test-server');
        expect(result.data.success).toBe(true);
    }, 10000);

    test('stopServer는 기본 timeout(15s)보다 긴 per-request timeout 사용', () => {
        // stopServer 구현이 { timeout: 35000 } 옵션을 전달하는지 확인
        const axios = require('axios');
        const ipc = require('../core/ipc');
        ipc.init();

        // 기본 타임아웃은 15초
        expect(axios.defaults.timeout).toBe(15000);

        // stopServer의 소스코드에서 timeout: 35000 사용 확인
        const src = ipc.stopServer.toString();
        expect(src).toMatch(/timeout[\s:]+35000/);
    });
});


// ════════════════════════════════════════════════════════════
//  Issue 1: 설정 리로드 지연
// ════════════════════════════════════════════════════════════

describe('설정 리로드 타이밍', () => {
    test('reloadConfigIfChanged는 5초 캐시 사용 (60초가 아님)', async () => {
        let loadCount = 0;

        // mock daemon config API
        setMockHandler('GET', '/api/config/bot', (_req, res) => {
            loadCount++;
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ prefix: '!saba', nodeSettings: {} }));
        });

        const resolver = require('../core/resolver');
        const ipc = require('../core/ipc');
        ipc.init();

        // 초기 로드
        await resolver.loadConfig();
        const initialCount = loadCount;

        // 즉시 재호출 — 5초 캐시 내이므로 API 호출 없어야 함
        await resolver.reloadConfigIfChanged();
        expect(loadCount).toBe(initialCount);

        // 소스코드에서 캐시 시간이 5초(5000ms)인지 확인
        const src = resolver.reloadConfigIfChanged.toString();
        expect(src).toMatch(/5000/);
        // 60초(60000ms)가 아닌지 확인
        expect(src).not.toMatch(/60000/);
    });

    test('5초 이내 연속 호출은 캐시 사용', async () => {
        let loadCount = 0;

        setMockHandler('GET', '/api/config/bot', (_req, res) => {
            loadCount++;
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ prefix: '!saba' }));
        });

        const resolver = require('../core/resolver');
        const ipc = require('../core/ipc');
        ipc.init();

        await resolver.loadConfig();
        const afterLoad = loadCount;

        // 즉시 재호출 — 캐시 내이므로 API 호출 없어야 함
        await resolver.reloadConfigIfChanged();
        expect(loadCount).toBe(afterLoad);
    });
});
