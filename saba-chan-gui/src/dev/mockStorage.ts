// 미리보기(npm run dev)의 설정 › 저장 공간: 내장 디스크 셋과 사바쨩 파일. 파일 이동은 몇 번 읽으면 끝난다
const GB = 1024 ** 3;
const ok = <T>(v: T) => Promise.resolve(v);
const DATA = 'C:\\Users\\me\\AppData\\Roaming\\saba-chan';

type MockJob = Record<string, unknown> & { phase: string; done_bytes: number; total_bytes: number };
let job: MockJob | null = null;

export const mockStorage = {
    systemPaths: () =>
        ok({
            success: true,
            data_dir: DATA,
            install_dir: 'C:\\Program Files\\saba-chan',
            default_install_dir: 'C:\\Program Files\\saba-chan',
            instances_dir: `${DATA}\\instances`,
            modules: { path: `${DATA}\\modules`, default: `${DATA}\\modules`, source: 'default' },
            extensions: { path: `${DATA}\\extensions`, default: `${DATA}\\extensions`, source: 'default' },
        }),
    systemStorage: () =>
        ok({
            success: true,
            measuring: false,
            measured_at_ms: Date.now() - 3 * 60_000,
            volumes: [
                {
                    mount: 'C:\\',
                    name: '로컬 디스크',
                    kind: 'ssd',
                    total_bytes: 931 * GB,
                    available_bytes: 319 * GB,
                    saba: { servers: 38.8 * GB, backups: 6.3 * GB, components: 0.6 * GB, app: 1.3 * GB },
                },
                {
                    mount: 'D:\\',
                    name: '데이터',
                    kind: 'hdd',
                    total_bytes: 3726 * GB,
                    available_bytes: 1574 * GB,
                    saba: { servers: 23.9 * GB, backups: 0, components: 0, app: 0 },
                },
                {
                    mount: 'E:\\',
                    name: '게임',
                    kind: 'ssd',
                    total_bytes: 465 * GB,
                    available_bytes: 27 * GB,
                    saba: { servers: 0, backups: 0, components: 0, app: 0 },
                },
            ],
            items: [
                {
                    kind: 'server',
                    category: 'servers',
                    id: 'mc-1',
                    name: '마크 서버',
                    path: `${DATA}\\instances\\mc-1\\server`,
                    mount: 'C:\\',
                    bytes: 38.2 * GB,
                    running: false,
                },
                {
                    kind: 'server',
                    category: 'servers',
                    id: 'pal-1',
                    name: '팰월드',
                    path: 'D:\\saba-chan\\servers\\팰월드',
                    mount: 'D:\\',
                    bytes: 14.1 * GB,
                    running: true,
                },
                {
                    kind: 'server',
                    category: 'servers',
                    id: 'pz-1',
                    name: '좀보이드',
                    path: 'D:\\saba-chan\\servers\\좀보이드',
                    mount: 'D:\\',
                    bytes: 9.8 * GB,
                    running: false,
                },
                {
                    kind: 'server',
                    category: 'servers',
                    id: 'tr-1',
                    name: '테라리아',
                    path: `${DATA}\\instances\\tr-1\\server`,
                    mount: 'C:\\',
                    bytes: 0.6 * GB,
                    running: false,
                },
                { kind: 'backups', category: 'backups', mount: 'C:\\', bytes: 6.3 * GB },
                { kind: 'modules', category: 'components', path: `${DATA}\\modules`, mount: 'C:\\', bytes: 0.18 * GB },
                {
                    kind: 'extensions',
                    category: 'components',
                    path: `${DATA}\\extensions`,
                    mount: 'C:\\',
                    bytes: 0.42 * GB,
                },
                { kind: 'app', category: 'app', mount: 'C:\\', bytes: 1.3 * GB },
            ],
        }),
    systemStorageMove: (body: { kind: string; id?: string; mount?: string; target?: string; dryRun?: boolean }) => {
        const target = body.target || `${body.mount}saba-chan\\servers\\${body.id}`;
        if (body.dryRun) return ok({ success: true, target, bytes: 38.2 * GB, available_bytes: 1574 * GB });
        job = { kind: body.kind, id: body.id, target, phase: 'copy', done_bytes: 0, total_bytes: 38.2 * GB };
        return ok({ success: true, job });
    },
    systemStorageMoveStatus: () => {
        if (job?.phase === 'copy') {
            const done = Math.min(job.total_bytes, job.done_bytes + 2.5 * GB);
            job = { ...job, done_bytes: done, phase: done >= job.total_bytes ? 'done' : 'copy' };
        }
        return ok({ success: true, job });
    },
    systemStorageMoveCancel: () => {
        if (job) job = { ...job, phase: 'canceled', error_code: 'canceled' };
        return ok({ success: true });
    },
};
