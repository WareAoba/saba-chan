/**
 * 상단 바 업데이트 알약 (시안 v8, 계약 G): 데몬 GET /api/updates/status 의 phase 네 상태를 보인다.
 * 적용 중·완료·대기 중은 누르면 버전과 패치노트, 준비됨은 다시 시작할지 묻는다
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdatePill } from '../components/shell/UpdatePill';
import { clickOutside } from './outside';

const phase = (state: string, extra: Record<string, unknown> = {}) => ({
    ok: true,
    components: [{ component: 'saba-core', current_version: '0.1.13' }],
    phase: {
        state,
        version: '0.1.14',
        since: null,
        waiting_for: [],
        reason: null,
        notes: { ko: '- 첫째\n- 둘째', en: '- first\n- second' },
        ...extra,
    },
    stable_wait: null,
});

let status: ReturnType<typeof vi.fn>;
beforeEach(() => {
    status = vi.fn();
    window.api.updaterStatus = status as never;
    window.api.updaterPhaseAck = vi.fn().mockResolvedValue({ ok: true }) as never;
    window.api.updaterRestart = vi.fn().mockResolvedValue({ ok: true, gui_restart: true }) as never;
});

afterEach(() => {
    for (const k of ['updaterStatus', 'updaterPhaseAck', 'updaterRestart'])
        delete (window.api as unknown as Record<string, unknown>)[k];
    vi.useRealTimers();
});

describe('업데이트 알약', () => {
    it('할 일이 없으면(idle) 보이지 않는다', async () => {
        status.mockResolvedValue(phase('idle', { version: null }));
        const { container } = render(<UpdatePill />);
        await waitFor(() => expect(status).toHaveBeenCalled());
        expect(container.querySelector('.appbar-pill')).toBeNull();
    });

    it.each([
        ['applying', 'Applying update...'],
        ['done', 'Update complete'],
        ['waiting', 'Update waiting...'],
        ['ready', 'Update ready'],
    ])('%s 상태는 제 문구와 색으로 보인다 (점은 고정 문자)', async (state, text) => {
        status.mockResolvedValue(phase(state));
        const { container } = render(<UpdatePill />);
        expect(await screen.findByText(text)).toBeTruthy();
        expect(container.querySelector(`.appbar-pill.tone-${state}`)).not.toBeNull();
    });

    it('적용 중을 누르면 바뀌는 버전과 패치노트를 보인다', async () => {
        status.mockResolvedValue(phase('applying'));
        render(<UpdatePill />);
        fireEvent.click(await screen.findByText('Applying update...'));
        expect(screen.getByText('0.1.13 → 0.1.14')).toBeTruthy();
        expect(screen.getByText('first')).toBeTruthy();
        expect(screen.getByText('second')).toBeTruthy();
    });

    it('대기 중을 누르면 무엇을 기다리는지 보인다', async () => {
        status.mockResolvedValue(
            phase('waiting', { waiting_for: [{ kind: 'backup', instance_id: 'mc', name: 'Wild' }] }),
        );
        render(<UpdatePill />);
        fireEvent.click(await screen.findByText('Update waiting...'));
        expect(screen.getByText("Applies after Wild's backup finishes")).toBeTruthy();
    });

    it('완료를 누르면 데몬에 확인했다고 알리고, 팝오버를 닫으면 사라진다', async () => {
        status.mockResolvedValue(phase('done'));
        const { container } = render(<UpdatePill />);
        fireEvent.click(await screen.findByText('Update complete'));
        expect(window.api.updaterPhaseAck).toHaveBeenCalledTimes(1);
        expect(screen.getByText('Updated to 0.1.14')).toBeTruthy();
        await clickOutside();
        await waitFor(() => expect(container.querySelector('.appbar-pill')).toBeNull());
    });

    // 알약을 누르지 않아도 앱을 쓰기 시작하면(완료가 뜨고 잠시 뒤 처음 누르거나 친 때) 지운다 (사용자 11-12)
    it('완료가 뜬 뒤 앱을 쓰기 시작하면 데몬에 확인했다고 알리고 사라진다', async () => {
        status.mockResolvedValue(phase('done'));
        let now = 1_000_000;
        vi.spyOn(Date, 'now').mockImplementation(() => now);
        const { container } = render(<UpdatePill />);
        await screen.findByText('Update complete');
        // 뜨자마자 누른 것은 알약을 보기 전이라 지우지 않는다
        fireEvent.pointerDown(document.body);
        expect(window.api.updaterPhaseAck).not.toHaveBeenCalled();
        now += 5_000;
        fireEvent.keyDown(document.body, { key: 'a' });
        expect(window.api.updaterPhaseAck).toHaveBeenCalledTimes(1);
        await waitFor(() => expect(container.querySelector('.appbar-pill')).toBeNull());
    });

    it('준비됨을 누르면 다시 시작할지 묻고, 나중에는 아무것도 하지 않는다', async () => {
        status.mockResolvedValue(phase('ready'));
        render(<UpdatePill />);
        fireEvent.click(await screen.findByText('Update ready'));
        expect(screen.getByRole('dialog', { name: 'Restart required to update to 0.1.14' })).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: 'Later' }));
        expect(screen.queryByRole('dialog')).toBeNull();
        expect(window.api.updaterRestart).not.toHaveBeenCalled();
    });

    it('지금 다시 시작을 누르면 데몬 재시작 길을 부르고 적용 중으로 바뀐다', async () => {
        status.mockResolvedValue(phase('ready'));
        render(<UpdatePill />);
        fireEvent.click(await screen.findByText('Update ready'));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Restart now' }));
        });
        expect(window.api.updaterRestart).toHaveBeenCalledTimes(1);
        expect(await screen.findByText('Applying update...')).toBeTruthy();
    });

    it('다시 시작한 뒤 데몬이 새로 준비됨을 주면(되돌림·미룸) 적용 중에 멈추지 않고 다시 준비됨을 보인다', async () => {
        vi.useFakeTimers();
        status.mockResolvedValue(phase('ready', { since: new Date(Date.now() - 60000).toISOString() }));
        render(<UpdatePill />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(0);
        });
        fireEvent.click(screen.getByText('Update ready'));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Restart now' }));
        });
        // 누르기 전에 정해진 준비됨은 아직 적용 중으로 둔다
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(screen.getByText('Applying update...')).toBeTruthy();
        // 되돌린 데몬이 새로 정한 준비됨
        status.mockResolvedValue(
            phase('ready', { reason: 'rolled_back', since: new Date(Date.now() + 1000).toISOString() }),
        );
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(screen.getByText('Update ready')).toBeTruthy();
    });

    it('다시 시작한 뒤 데몬이 내려갔다 돌아와 준비됨을 주면 그것을 보인다', async () => {
        vi.useFakeTimers();
        status.mockResolvedValue(phase('ready'));
        render(<UpdatePill />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(0);
        });
        fireEvent.click(screen.getByText('Update ready'));
        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Restart now' }));
        });
        status
            .mockRejectedValueOnce(new Error('ECONNREFUSED'))
            .mockResolvedValue(phase('ready', { reason: 'wait_pid_timeout' }));
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(screen.getByText('Applying update...')).toBeTruthy();
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(screen.getByText('Update ready')).toBeTruthy();
    });

    it('데몬이 꺼진 동안(적용 중)엔 마지막 상태를 그대로 보인다', async () => {
        vi.useFakeTimers();
        status.mockResolvedValueOnce(phase('applying')).mockResolvedValue({ ok: false, error_code: 'ECONNREFUSED' });
        render(<UpdatePill />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(0);
        });
        expect(screen.getByText('Applying update...')).toBeTruthy();
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(status).toHaveBeenCalledTimes(2);
        expect(screen.getByText('Applying update...')).toBeTruthy();
    });

    it('idle이면 60초, 아니면 5초마다 읽는다', async () => {
        vi.useFakeTimers();
        status.mockResolvedValue(phase('idle', { version: null }));
        render(<UpdatePill />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(0);
        });
        expect(status).toHaveBeenCalledTimes(1);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(status).toHaveBeenCalledTimes(1);
        status.mockResolvedValue(phase('waiting'));
        await act(async () => {
            await vi.advanceTimersByTimeAsync(55000);
        });
        expect(status).toHaveBeenCalledTimes(2);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(5000);
        });
        expect(status).toHaveBeenCalledTimes(3);
    });

    it('같은 판 베타를 기다리는 것(stable_wait)은 알약에 띄우지 않는다', async () => {
        status.mockResolvedValue({ ...phase('idle', { version: null }), stable_wait: '0.1.14' });
        const { container } = render(<UpdatePill />);
        await waitFor(() => expect(status).toHaveBeenCalled());
        expect(container.querySelector('.appbar-pill')).toBeNull();
    });
});
