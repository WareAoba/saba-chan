import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';

function deferred() {
    let resolve;
    const promise = new Promise((done) => {
        resolve = done;
    });
    return { promise, resolve };
}

beforeEach(() => {
    useDiscordStore.getState()._resetForTest();
    useDiscordStore.setState({ _botConfigLoaded: true });
});

describe('디스코드 연결 설정 저장 순서', () => {
    it('앞 저장이 데몬 값을 기다리는 동안 다시 저장해도 마지막 값이 남는다', async () => {
        const firstRead = deferred();
        const secondRead = deferred();
        const saved = [];
        window.api = {
            botConfigLoad: vi.fn().mockReturnValueOnce(firstRead.promise).mockReturnValueOnce(secondRead.promise),
            botConfigSave: vi.fn(async (p) => {
                saved.push(p.cloud.hostId);
                return { success: true };
            }),
        };

        useDiscordStore.setState({ discordCloudHostId: 'h2' });
        const first = useDiscordStore.getState().saveConfig();
        await Promise.resolve();
        useDiscordStore.setState({ discordCloudHostId: 'h3' });
        const second = useDiscordStore.getState().saveConfig();
        await Promise.resolve();

        // 뒤 저장의 읽기가 먼저 끝나도 데몬에 남는 값은 화면의 최신값이다
        secondRead.resolve({ cloud: {} });
        await Promise.resolve();
        firstRead.resolve({ cloud: {} });
        await Promise.all([first, second]);

        expect(saved.at(-1)).toBe('h3');
        expect(useDiscordStore.getState().discordCloudHostId).toBe('h3');
    });
});
