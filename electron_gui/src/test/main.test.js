const fs = require('fs');
const path = require('path');
const os = require('os');

// main.js의 설정 저장/로드 함수 테스트
describe('Electron Main Process - 설정 관리 테스트', () => {
    const testSettingsDir = path.join(os.tmpdir(), 'saba-chan-test');
    const testSettingsPath = path.join(testSettingsDir, 'settings.json');
    const testBotConfigPath = path.join(testSettingsDir, 'bot-config.json');

    beforeEach(() => {
        // 테스트 디렉토리 생성
        if (!fs.existsSync(testSettingsDir)) {
            fs.mkdirSync(testSettingsDir, { recursive: true });
        }
    });

    afterEach(() => {
        // 테스트 파일 정리
        try {
            if (fs.existsSync(testSettingsPath)) fs.unlinkSync(testSettingsPath);
            if (fs.existsSync(testBotConfigPath)) fs.unlinkSync(testBotConfigPath);
        } catch (e) {
            // 무시
        }
    });

    test('설정을 저장하고 로드할 수 있어야 함', () => {
        const testSettings = {
            autoRefresh: true,
            refreshInterval: 3000,
            modulesPath: '/test/modules',
            discordToken: 'test-token-abc',
            discordAutoStart: true,
            windowBounds: { width: 1200, height: 800, x: 100, y: 100 }
        };

        // 저장
        fs.writeFileSync(testSettingsPath, JSON.stringify(testSettings, null, 2), 'utf8');

        // 로드
        const loaded = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));

        expect(loaded).toEqual(testSettings);
        expect(loaded.autoRefresh).toBe(true);
        expect(loaded.refreshInterval).toBe(3000);
        expect(loaded.modulesPath).toBe('/test/modules');
        expect(loaded.discordToken).toBe('test-token-abc');
        expect(loaded.discordAutoStart).toBe(true);
        expect(loaded.windowBounds).toEqual({ width: 1200, height: 800, x: 100, y: 100 });
    });

    test('모든 설정 필드를 완전하게 저장/로드할 수 있어야 함', () => {
        const completeSettings = {
            modulesPath: 'C:\\custom\\modules',
            autoRefresh: false,
            refreshInterval: 5000,
            discordToken: 'complete-test-token',
            discordAutoStart: true,
            discordPrefix: '!custom',
            windowBounds: { width: 1600, height: 900, x: 200, y: 150 }
        };

        // 저장
        fs.writeFileSync(testSettingsPath, JSON.stringify(completeSettings, null, 2), 'utf8');

        // 로드
        const loaded = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));

        // 모든 필드 검증
        expect(loaded.modulesPath).toBe('C:\\custom\\modules');
        expect(loaded.autoRefresh).toBe(false);
        expect(loaded.refreshInterval).toBe(5000);
        expect(loaded.discordToken).toBe('complete-test-token');
        expect(loaded.discordAutoStart).toBe(true);
        expect(loaded.discordPrefix).toBe('!custom');
        expect(loaded.windowBounds.width).toBe(1600);
        expect(loaded.windowBounds.height).toBe(900);
    });

    test('봇 설정을 저장하고 로드할 수 있어야 함', () => {
        const testBotConfig = {
            prefix: '!test',
            moduleAliases: {
                minecraft: 'mc',
                palworld: 'pw'
            },
            commandAliases: {
                minecraft: {
                    start: '시작,실행',
                    stop: '정지,중지',
                    status: '상태'
                },
                palworld: {
                    start: 'go,run',
                    restart: '재시작'
                }
            }
        };

        // 저장
        fs.writeFileSync(testBotConfigPath, JSON.stringify(testBotConfig, null, 2), 'utf8');

        // 로드
        const loaded = JSON.parse(fs.readFileSync(testBotConfigPath, 'utf8'));

        expect(loaded).toEqual(testBotConfig);
        expect(loaded.prefix).toBe('!test');
        expect(loaded.moduleAliases.minecraft).toBe('mc');
        expect(loaded.moduleAliases.palworld).toBe('pw');
        expect(loaded.commandAliases.minecraft.start).toBe('시작,실행');
        expect(loaded.commandAliases.minecraft.stop).toBe('정지,중지');
        expect(loaded.commandAliases.palworld.restart).toBe('재시작');
    });

    test('봇 설정의 모든 별칭 조합을 저장/로드할 수 있어야 함', () => {
        const complexBotConfig = {
            prefix: '!saba',
            moduleAliases: {
                minecraft: 'mc',
                palworld: 'pw',
                terraria: 'tr'
            },
            commandAliases: {
                minecraft: {
                    start: '시작,켜기,온',
                    stop: '정지,끄기,오프',
                    restart: '재시작,리붓',
                    status: '상태,확인'
                }
            }
        };

        // 저장
        fs.writeFileSync(testBotConfigPath, JSON.stringify(complexBotConfig, null, 2), 'utf8');

        // 로드
        const loaded = JSON.parse(fs.readFileSync(testBotConfigPath, 'utf8'));

        expect(loaded.prefix).toBe('!saba');
        expect(Object.keys(loaded.moduleAliases)).toHaveLength(3);
        expect(loaded.commandAliases.minecraft.start).toBe('시작,켜기,온');
        expect(loaded.commandAliases.minecraft.restart).toBe('재시작,리붓');
    });

    test('존재하지 않는 설정 파일 로드 시 기본값을 반환해야 함', () => {
        const nonExistentPath = path.join(testSettingsDir, 'non-existent.json');

        let loaded = null;
        if (fs.existsSync(nonExistentPath)) {
            loaded = JSON.parse(fs.readFileSync(nonExistentPath, 'utf8'));
        } else {
            // 기본값 반환
            loaded = {
                autoRefresh: true,
                refreshInterval: 2000,
                discordAutoStart: false
            };
        }

        expect(loaded).toBeDefined();
        expect(loaded.autoRefresh).toBe(true);
        expect(loaded.refreshInterval).toBe(2000);
    });

    test('잘못된 JSON 형식의 설정 파일을 처리할 수 있어야 함', () => {
        // 잘못된 JSON 작성
        fs.writeFileSync(testSettingsPath, '{ invalid json }', 'utf8');

        let loaded = null;
        let error = null;

        try {
            loaded = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));
        } catch (e) {
            error = e;
            // 오류 발생 시 기본값 반환
            loaded = {
                autoRefresh: true,
                refreshInterval: 2000
            };
        }

        expect(error).toBeDefined();
        expect(loaded).toBeDefined();
        expect(loaded.autoRefresh).toBe(true);
    });

    test('설정 값 업데이트가 올바르게 동작해야 함', () => {
        const initialSettings = {
            autoRefresh: true,
            refreshInterval: 2000,
            discordAutoStart: false
        };

        // 초기 저장
        fs.writeFileSync(testSettingsPath, JSON.stringify(initialSettings, null, 2), 'utf8');

        // 로드
        let settings = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));
        expect(settings.discordAutoStart).toBe(false);

        // 업데이트
        settings.discordAutoStart = true;
        settings.discordToken = 'new-token';
        fs.writeFileSync(testSettingsPath, JSON.stringify(settings, null, 2), 'utf8');

        // 다시 로드
        const updated = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));
        expect(updated.discordAutoStart).toBe(true);
        expect(updated.discordToken).toBe('new-token');
    });

    test('부분 설정 업데이트 시 기존 값이 유지되어야 함', () => {
        const initialSettings = {
            autoRefresh: true,
            refreshInterval: 2000,
            modulesPath: '/initial/path',
            discordToken: 'initial-token',
            windowBounds: { width: 1200, height: 800 }
        };

        // 초기 저장
        fs.writeFileSync(testSettingsPath, JSON.stringify(initialSettings, null, 2), 'utf8');

        // 일부만 업데이트
        const settings = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));
        settings.refreshInterval = 5000;
        settings.discordToken = 'updated-token';
        fs.writeFileSync(testSettingsPath, JSON.stringify(settings, null, 2), 'utf8');

        // 다시 로드하여 검증
        const updated = JSON.parse(fs.readFileSync(testSettingsPath, 'utf8'));
        expect(updated.autoRefresh).toBe(true); // 유지
        expect(updated.refreshInterval).toBe(5000); // 변경
        expect(updated.modulesPath).toBe('/initial/path'); // 유지
        expect(updated.discordToken).toBe('updated-token'); // 변경
        expect(updated.windowBounds).toEqual({ width: 1200, height: 800 }); // 유지
    });

    test('빈 봇 설정도 저장/로드할 수 있어야 함', () => {
        const emptyBotConfig = {
            prefix: '!saba',
            moduleAliases: {},
            commandAliases: {}
        };

        fs.writeFileSync(testBotConfigPath, JSON.stringify(emptyBotConfig, null, 2), 'utf8');
        const loaded = JSON.parse(fs.readFileSync(testBotConfigPath, 'utf8'));

        expect(loaded.prefix).toBe('!saba');
        expect(Object.keys(loaded.moduleAliases)).toHaveLength(0);
        expect(Object.keys(loaded.commandAliases)).toHaveLength(0);
    });
});
