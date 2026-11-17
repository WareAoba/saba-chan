/**
 * 서버 설정의 포트 충돌 경고: 고치는 중인 값으로 데몬에 묻고(`instancePortConflicts`), 데몬이 답한 충돌만 그린다.
 * GUI가 다른 서버의 포트와 직접 비교하지 않는다.
 */
import { render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import i18n from '../i18n';

const t = (k: string, o?: Record<string, unknown>) => i18n.t(`gui:${k}`, o ?? {}) as string;
const modules = [{ name: 'game', settings: { fields: [{ name: 'port', field_type: 'number' }] } }];

beforeEach(() => {
    window.api = {
        instancePortConflicts: vi.fn().mockResolvedValue({
            success: true,
            port_conflicts: [
                {
                    port: 25565,
                    port_type: 'port',
                    conflict_name: 'Other',
                    conflict_id: 'o',
                    conflict_port_type: 'port',
                },
            ],
        }),
    } as unknown as typeof window.api;
});

describe('포트 충돌 경고', () => {
    it('고치는 중인 포트를 데몬에 묻고 답한 충돌을 보인다', async () => {
        const props = {
            settingsServer: { id: 's', name: 'S', module: 'game', status: 'stopped', port: 25570 },
            settingsValues: { port: '25565' },
            settingsActiveTab: 'general',
            setSettingsActiveTab: vi.fn(),
            modules,
            advancedExpanded: false,
            setAdvancedExpanded: vi.fn(),
            availableVersions: [],
            handleSettingChange: vi.fn(),
            handleSaveSettings: vi.fn(),
            section: 'server',
            // 다른 서버가 같은 포트를 써도 GUI는 비교하지 않는다 (데몬 답만 본다)
            servers: [],
        };
        render(<ServerSettingsModal {...(props as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
        await waitFor(() =>
            expect(screen.getByText(t('errors.port_conflict_detail', { port: 25565, name: 'Other' }))).toBeTruthy(),
        );
        expect(window.api.instancePortConflicts).toHaveBeenCalledWith('s', { port: 25565 });
    });
});
