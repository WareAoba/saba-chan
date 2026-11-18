import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StorageTab } from '../components/Modals/settings/StorageTab';
import { pickOption } from './dropdown';

const GB = 1024 ** 3;

const storage = (measuring = false) => ({
    success: true,
    measuring,
    measured_at_ms: Date.now() - 3 * 60_000,
    volumes: [
        {
            mount: 'C:\\',
            name: 'Local Disk',
            kind: 'ssd',
            total_bytes: 1000 * GB,
            available_bytes: 388 * GB,
            saba: { servers: 40 * GB, backups: 6 * GB, components: 1 * GB, app: 1 * GB },
        },
        {
            mount: 'D:\\',
            name: 'Data',
            kind: 'hdd',
            total_bytes: 4000 * GB,
            available_bytes: 1700 * GB,
            saba: { servers: 0, backups: 0, components: 0, app: 0 },
        },
    ],
    items: [
        {
            kind: 'server',
            category: 'servers',
            id: 'mc',
            name: 'Survival',
            path: 'C:\\saba\\instances\\mc\\server',
            mount: 'C:\\',
            bytes: 38 * GB,
            running: false,
        },
        {
            kind: 'server',
            category: 'servers',
            id: 'pal',
            name: 'Palworld',
            path: 'C:\\saba\\instances\\pal\\server',
            mount: 'C:\\',
            bytes: 2 * GB,
            running: true,
        },
        { kind: 'backups', category: 'backups', mount: 'C:\\', bytes: 6 * GB },
        { kind: 'modules', category: 'components', path: 'C:\\saba\\modules', mount: 'C:\\', bytes: 0.2 * GB },
        { kind: 'app', category: 'app', mount: 'C:\\', bytes: 1 * GB },
    ],
});

describe('설정 > 저장 공간', () => {
    beforeEach(() => {
        window.api = {
            ...window.api,
            systemStorage: vi.fn().mockResolvedValue(storage()),
            systemStorageMove: vi.fn(),
            systemStorageMoveStatus: vi.fn(),
            systemStorageMoveCancel: vi.fn().mockResolvedValue({ success: true }),
            openFolderDialog: vi.fn().mockResolvedValue('E:\\Games\\Survival'),
        } as unknown as typeof window.api;
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it('내장 디스크마다 남은 공간과 사바쨩 파일 크기를 보이고, 새로 고침은 데몬에 다시 재게 한다', async () => {
        await act(async () => render(<StorageTab daemonTarget={null} />));
        const disk = screen.getByRole('group', { name: /\(C:\)/ });
        expect(within(disk).getByText(/388 GB free of 1000 GB/)).toBeInTheDocument();
        expect(within(disk).getByText('48 GB')).toBeInTheDocument();
        expect(screen.getByText('Survival')).toBeInTheDocument();
        expect(window.api.systemStorage).toHaveBeenLastCalledWith(false);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Refresh' })));
        expect(window.api.systemStorage).toHaveBeenLastCalledWith(true);
    });

    it('파일 이동: 고른 드라이브의 자리를 데몬에 물어 보이고, 옮기는 동안 진행을 보인 뒤 다시 읽는다', async () => {
        vi.mocked(window.api.systemStorageMove).mockImplementation(async (body) =>
            body.dryRun
                ? {
                      success: true,
                      target: 'D:\\saba-chan\\servers\\Survival',
                      bytes: 38 * GB,
                      available_bytes: 1700 * GB,
                  }
                : {
                      success: true,
                      job: {
                          kind: 'server',
                          id: 'mc',
                          target: 'D:\\saba-chan\\servers\\Survival',
                          phase: 'copy',
                          done_bytes: 0,
                          total_bytes: 38 * GB,
                      },
                  },
        );
        vi.mocked(window.api.systemStorageMoveStatus)
            .mockResolvedValueOnce({
                success: true,
                job: {
                    kind: 'server',
                    id: 'mc',
                    target: 'D:\\saba-chan\\servers\\Survival',
                    phase: 'copy',
                    done_bytes: 19 * GB,
                    total_bytes: 38 * GB,
                },
            })
            .mockResolvedValue({
                success: true,
                job: {
                    kind: 'server',
                    id: 'mc',
                    target: 'D:\\saba-chan\\servers\\Survival',
                    phase: 'done',
                    done_bytes: 38 * GB,
                    total_bytes: 38 * GB,
                },
            });
        await act(async () => render(<StorageTab daemonTarget={null} />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Move files/ })));
        const dialog = screen.getByRole('dialog');
        pickOption(within(dialog).getByRole('combobox'), /Survival/);
        await act(async () => {});

        // 지금 드라이브는 고를 수 없다
        expect(within(dialog).getByRole('radio', { name: /\(C:\)/ })).toBeDisabled();
        await act(async () => fireEvent.click(within(dialog).getByRole('radio', { name: /\(D:\)/ })));
        expect(window.api.systemStorageMove).toHaveBeenLastCalledWith({
            kind: 'server',
            id: 'mc',
            mount: 'D:\\',
            dryRun: true,
        });
        expect(within(dialog).getByText('D:\\saba-chan\\servers\\Survival')).toBeInTheDocument();

        vi.useFakeTimers();
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: /^Move$/ })));
        expect(window.api.systemStorageMove).toHaveBeenLastCalledWith({ kind: 'server', id: 'mc', mount: 'D:\\' });
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(within(dialog).getByText(/19 GB \/ 38 GB/)).toBeInTheDocument();
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(within(dialog).getByText(/Moved to D:\\saba-chan\\servers\\Survival/)).toBeInTheDocument();
        expect(within(dialog).queryByRole('button', { name: /^Move$/ })).toBeNull();
        expect(window.api.systemStorage).toHaveBeenLastCalledWith(true);
        // 같은 탭의 폴더 위치도 옮긴 뒤의 경로로 다시 읽는다
        expect(window.api.systemPaths).toHaveBeenCalledTimes(2);
    });

    // 리뷰 11-18: 진행 읽기가 한 번 실패해도 조회를 멈추지 않고, 계속 실패하면 이유를 보이고 창을 닫을 수 있게 한다
    it('옮기는 중 진행 읽기가 실패해도 계속 묻고, 연달아 실패하면 오류 줄과 닫기를 보인다', async () => {
        const job = (phase: string) => ({
            kind: 'server',
            id: 'mc',
            target: 'D:\\saba-chan\\servers\\Survival',
            phase,
            done_bytes: 0,
            total_bytes: 38 * GB,
        });
        vi.mocked(window.api.systemStorageMove).mockImplementation(async (body) =>
            body.dryRun
                ? {
                      success: true,
                      target: 'D:\\saba-chan\\servers\\Survival',
                      bytes: 38 * GB,
                      available_bytes: 1700 * GB,
                  }
                : { success: true, job: job('moving') },
        );
        vi.mocked(window.api.systemStorageMoveStatus).mockResolvedValue({ error: 'connection refused' });
        await act(async () => render(<StorageTab daemonTarget={null} />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Move files/ })));
        const dialog = screen.getByRole('dialog');
        pickOption(within(dialog).getByRole('combobox'), /Survival/);
        await act(async () => {});
        await act(async () => fireEvent.click(within(dialog).getByRole('radio', { name: /\(D:\)/ })));
        vi.useFakeTimers();
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: /^Move$/ })));
        expect(within(dialog).queryByRole('button', { name: 'Close' })).toBeNull();
        await act(async () => vi.advanceTimersByTimeAsync(2000));
        expect(within(dialog).getByRole('alert')).toHaveTextContent('connection refused');
        expect(within(dialog).getByRole('button', { name: 'Close' })).toBeInTheDocument();
        vi.mocked(window.api.systemStorageMoveStatus).mockResolvedValue({ success: true, job: job('done') });
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(within(dialog).getByText(/Moved to D:\\saba-chan\\servers\\Survival/)).toBeInTheDocument();
        expect(within(dialog).queryByRole('alert')).toBeNull();
    });

    it('직접 고른 폴더로도 물어 보고, 데몬이 거절하면 이유를 보이며 옮기기를 막는다', async () => {
        vi.mocked(window.api.systemStorageMove).mockResolvedValue({
            success: false,
            error_code: 'storage_server_running',
            error: 'Palworld',
        });
        await act(async () => render(<StorageTab daemonTarget={null} />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Move files/ })));
        const dialog = screen.getByRole('dialog');
        pickOption(within(dialog).getByRole('combobox'), /Palworld/);
        await act(async () => {});
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: /Choose folder/ })));
        expect(window.api.systemStorageMove).toHaveBeenLastCalledWith({
            kind: 'server',
            id: 'pal',
            target: 'E:\\Games\\Survival',
            dryRun: true,
        });
        expect(within(dialog).getByRole('alert')).toHaveTextContent('Palworld is running. Stop it first.');
        expect(within(dialog).getByRole('button', { name: /^Move$/ })).toBeDisabled();
    });

    it('옮기는 중 취소는 데몬에 멈추라고 한다', async () => {
        vi.mocked(window.api.systemStorageMove).mockImplementation(async (body) =>
            body.dryRun
                ? {
                      success: true,
                      target: 'D:\\saba-chan\\servers\\Survival',
                      bytes: 38 * GB,
                      available_bytes: 1700 * GB,
                  }
                : {
                      success: true,
                      job: {
                          kind: 'server',
                          id: 'mc',
                          target: 'x',
                          phase: 'copy',
                          done_bytes: 0,
                          total_bytes: 38 * GB,
                      },
                  },
        );
        vi.mocked(window.api.systemStorageMoveStatus).mockResolvedValue({
            success: true,
            job: {
                kind: 'server',
                id: 'mc',
                target: 'x',
                phase: 'canceled',
                done_bytes: 1,
                total_bytes: 38 * GB,
                error_code: 'canceled',
            },
        });
        await act(async () => render(<StorageTab daemonTarget={null} />));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Move files/ })));
        const dialog = screen.getByRole('dialog');
        pickOption(within(dialog).getByRole('combobox'), /Survival/);
        await act(async () => {});
        await act(async () => fireEvent.click(within(dialog).getByRole('radio', { name: /\(D:\)/ })));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: /^Move$/ })));
        await act(async () => fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' })));
        expect(window.api.systemStorageMoveCancel).toHaveBeenCalled();
        await waitFor(() => expect(within(dialog).getByText(/Canceled/)).toBeInTheDocument());
    });
});
