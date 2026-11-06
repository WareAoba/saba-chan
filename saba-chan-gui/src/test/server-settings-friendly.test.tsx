/**
 * #5: 서버 설정을 알기 쉽게. 스위치 옆은 켜짐·꺼짐, 설정 이름을 눌러도 스위치가 바뀌고,
 * 다시 시작해야 적용되는 칸은 그렇다고 표시한다. 켜진 서버에서 그런 칸을 저장했으면 저장 막대 위에 다시 시작 단추를 둔다.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { RestartNotice, SettingsField } from '../components/ServerSettingsModal';
import '../i18n';
import type { SettingField } from '../types/daemon';

const flight = { name: 'allow_flight', label: 'Allow flight', field_type: 'boolean', restart_required: true } as SettingField;

describe('서버 설정 칸', () => {
    it('스위치 옆은 On·Off 대신 화면 언어의 켜짐·꺼짐이다', () => {
        const { rerender } = render(<SettingsField field={flight} value={false} modNs="game" onChange={vi.fn()} />);
        expect(screen.getByText('Off')).toBeTruthy();
        rerender(<SettingsField field={flight} value="true" modNs="game" onChange={vi.fn()} />);
        expect(screen.getByText('On')).toBeTruthy();
    });

    it('설정 이름을 눌러도 스위치가 바뀐다', () => {
        const onChange = vi.fn();
        render(<SettingsField field={flight} value={false} modNs="game" onChange={onChange} />);
        fireEvent.click(screen.getByText('Allow flight'));
        expect(onChange).toHaveBeenCalledWith('allow_flight', true);
    });

    it('다시 시작해야 적용되는 칸은 그렇다고 표시한다', () => {
        render(<SettingsField field={flight} value={false} modNs="game" onChange={vi.fn()} />);
        expect(screen.getByText('Applies after restart')).toBeTruthy();
    });
});

describe('다시 시작 안내', () => {
    const pending = { serverId: 's', fields: ['Allow flight'], startTime: 100 };

    it('켜진 서버에서 저장한 그대로면 바꾼 설정 수와 이름, 지금 다시 시작·나중에를 보인다', () => {
        const onRestart = vi.fn();
        const onDismiss = vi.fn();
        render(<RestartNotice pending={pending} server={{ id: 's', status: 'running', start_time: 100 }} onRestart={onRestart} onDismiss={onDismiss} />);
        expect(screen.getByRole('status').textContent).toContain('1 changed setting');
        expect(screen.getByRole('status').textContent).toContain('Allow flight');
        fireEvent.click(screen.getByRole('button', { name: 'Restart now' }));
        expect(onRestart).toHaveBeenCalled();
        fireEvent.click(screen.getByRole('button', { name: 'Later' }));
        expect(onDismiss).toHaveBeenCalled();
    });

    it('그 뒤 서버를 껐거나 다시 켰으면 보이지 않는다', () => {
        const props = { pending, onRestart: vi.fn(), onDismiss: vi.fn() };
        const { rerender } = render(<RestartNotice {...props} server={{ id: 's', status: 'stopped', start_time: null }} />);
        expect(screen.queryByRole('status')).toBeNull();
        rerender(<RestartNotice {...props} server={{ id: 's', status: 'running', start_time: 200 }} />);
        expect(screen.queryByRole('status')).toBeNull();
        rerender(<RestartNotice {...props} server={{ id: 'other', status: 'running', start_time: 100 }} />);
        expect(screen.queryByRole('status')).toBeNull();
    });
});
