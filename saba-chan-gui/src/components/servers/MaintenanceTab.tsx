import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import type { ApiBody } from '../../types/daemon';
import { draftProblems, formatBytes, fromDraft, toDraft, WEEKDAYS } from '../../utils/schedule';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner } from '../ui/SabaUI';
import { failed } from '../../utils/apiResult';
import { BackupsCard } from './MaintenanceBackups';
import { ScheduleCard, type ScheduleInfo } from './MaintenanceSchedule';
import { errorText } from './maintenanceText';

// 예전 경로로 가져오는 곳(시험)을 위해 다시 내보낸다
export { draftProblems, formatBytes, fromDraft, toDraft, WEEKDAYS };

/** 이 화면을 띄울 IPC가 있는지 (구버전 브리지에는 없다) */
export const maintenanceAvailable = () => !!window.api?.instanceScheduleGet;

/**
 * 서버별 백업·예약 작업 탭 (데몬 A2·A3).
 * 백업 목록은 모듈이 [backup]을 선언했을 때만, 공지·예고는 공지 명령이 있을 때만 보인다.
 */
export function MaintenanceTab({ server, moduleCommands = [] }: { server: Server; moduleCommands?: ApiBody[] }) {
    const { t } = useTranslation('gui');
    const [info, setInfo] = useState<ScheduleInfo | null>(null);
    const [loadError, setLoadError] = useState<string | null>(null);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.instanceScheduleGet(server.id);
            if (failed(res)) throw new Error(errorText(res, t('maintenance.load_failed'), t));
            setInfo({
                schedule: Array.isArray(res?.schedule) ? res.schedule : [],
                backupKeep: res?.backup_keep ?? 10,
                backupSupported: !!res?.backup_supported,
                announceSupported: !!res?.announce_supported,
                saveSupported: !!res?.save_supported,
                // 작업마다 다음 실행 시각 (유닉스 초). 보는 사람의 시간대로 보인다
                nextRuns: Array.isArray(res?.next_runs) ? res.next_runs : [],
                history: Array.isArray(res?.history) ? res.history : [],
                // 작업 시각은 서버 PC의 시계다
                utcOffset: typeof res?.utc_offset_minutes === 'number' ? res.utc_offset_minutes : null,
                // 저장할 때 다시 보낸다: 그사이 다른 곳에서 바뀌었으면 데몬이 덮어쓰지 않고 거절한다 (L51)
                revision: res?.revision,
            });
        } catch (err) {
            setLoadError((err as Error).message || String(err));
        }
    }, [server.id, t]);

    useEffect(() => {
        setInfo(null);
        load();
    }, [load]);

    if (loadError) {
        return <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />;
    }
    if (!info) {
        return (
            <div className="empty-inline">
                <SabaSpinner />
                <p>{t('shell.loading')}</p>
            </div>
        );
    }
    return (
        <div className="mt-tab">
            {info.backupSupported && typeof window.api?.instanceBackups === 'function' && <BackupsCard server={server} />}
            <ScheduleCard server={server} info={info} onSaved={load} moduleCommands={moduleCommands} />
        </div>
    );
}
