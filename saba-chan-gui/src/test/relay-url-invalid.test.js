/**
 * 데몬은 https(루프백만 http)가 아닌 릴레이 주소를 400 invalid_relay_url로 거절한다 (릴레이 2차 점검 R31).
 * GUI는 "저장 실패"나 "인터넷 연결 확인" 대신 주소가 잘못됐다고 알린다.
 */
import { act, render, renderHook, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { PairingCard } from '../components/discord/ConnectionView';
import { usePairing } from '../components/discord/usePairing';
import { setDiscordI18n, useDiscordStore } from '../stores/useDiscordStore';

const refused = { status: 400, data: { success: false, error: 'bad relay url', error_code: 'invalid_relay_url' } };

describe('릴레이 주소 거절 (R31)', () => {
    afterEach(() => {
        delete window.showToast;
        setDiscordI18n((key, fallback) => fallback || key);
    });

    it('봇 설정 저장이 거절되면 error_code를 그대로 넘긴다', async () => {
        const invoke = vi.fn().mockResolvedValue(refused);
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        const res = await api.botConfigSave({ cloud: { relayUrl: 'http://example.com' } });
        expect(res.error_code).toBe('invalid_relay_url');
        expect(res.error).toBeTruthy();
    });

    describe('저장', () => {
        beforeEach(() => {
            useDiscordStore.getState()._resetForTest();
            useDiscordStore.setState({ _botConfigLoaded: true });
        });

        it('주소가 거절되면 저장 실패 대신 주소 형식을 알린다', async () => {
            setDiscordI18n((key) => key);
            window.showToast = vi.fn();
            window.api = {
                botConfigLoad: vi.fn().mockResolvedValue({ cloud: {} }),
                botConfigSave: vi.fn().mockResolvedValue({ error: 'bad', error_code: 'invalid_relay_url' }),
            };
            useDiscordStore.setState({ discordCloudRelayUrl: 'http://example.com' });
            expect(await useDiscordStore.getState().saveConfig()).toBe(false);
            expect(window.showToast.mock.calls.at(-1)[0]).toBe('discord.invalid_relay_url');
        });
    });

    describe('페어링', () => {
        afterEach(() => {
            delete window.api.relayInitiatePairing;
        });

        it('코드 발급이 주소 때문에 거절되면 그 이유를 남긴다', async () => {
            window.api.relayInitiatePairing = vi
                .fn()
                .mockResolvedValue({ error: 'bad', error_code: 'invalid_relay_url' });
            const { result } = renderHook(() => usePairing({ relayUrl: 'http://example.com' }));
            await act(async () => result.current.start());
            expect(result.current.status).toBe('error');
            expect(result.current.errorCode).toBe('invalid_relay_url');
        });

        it('주소를 저장하지 못해 시작하지 못해도 그 이유를 남긴다', async () => {
            window.api.relayInitiatePairing = vi.fn();
            const beforeStart = async () => {
                throw Object.assign(new Error('relay url not saved'), { code: 'invalid_relay_url' });
            };
            const { result } = renderHook(() => usePairing({ relayUrl: 'http://example.com', beforeStart }));
            await act(async () => result.current.start());
            expect(window.api.relayInitiatePairing).not.toHaveBeenCalled();
            expect(result.current.errorCode).toBe('invalid_relay_url');
        });

        it('주소 때문에 실패했으면 인터넷 확인 대신 주소 형식을 안내한다', () => {
            render(
                <PairingCard
                    pairing={{ status: 'error', errorCode: 'invalid_relay_url', code: '', start: () => {} }}
                    onCancel={() => {}}
                />,
            );
            expect(screen.getByText(/https:\/\//)).toBeInTheDocument();
            expect(screen.queryByText(/internet connection/i)).not.toBeInTheDocument();
        });
    });
});
