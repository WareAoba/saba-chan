import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import type { ApiBody } from '../../types/daemon';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner } from '../ui/SabaUI';
import { failed } from '../../utils/apiResult';
import { BackupsCard } from './MaintenanceBackups';
import { ScheduleCard, type ScheduleInfo } from './MaintenanceSchedule';
import { errorText } from './maintenanceText';

/** 받은 예약 정보: 서버마다 하나. 탭을 다시 열면 이것부터 바로 보이고 새 정보는 뒤에서 받아 바꾼다 */
const infos = new Map<string, ScheduleInfo>();

/**
 * 서버별 백업·예약 작업 탭 (데몬 A2·A3).
 * 백업 목록은 모듈이 [backup]을 선언했을 때만, 공지·예고는 공지 명령이 있을 때만 보인다.
 */
export function MaintenanceTab({ server, moduleCommands = [] }: { server: Server; moduleCommands?: ApiBody[] }) {
    const { t } = useTranslation('gui');
    const [info, setInfo] = useState<ScheduleInfo | null>(() => infos.get(server.id) ?? null);
    const [loadError, setLoadError] = useState<string | null>(null);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.instanceScheduleGet(server.id);
            if (failed(res)) throw new Error(errorText(res, t('maintenance.load_failed'), t));
            const next: ScheduleInfo = {
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
                // 저장할 때 다시 보낸다: 그사이 다른 곳에서 바뀌었으면 데몬이 덮어쓰지 않고 거절한다
                revision: res?.revision,
            };
            infos.set(server.id, next);
            // 보이는 것과 같으면 그대로 둔다: 먼저 보인 탭에서 고치던 내용을 새 객체로 지우지 않게
            setInfo((shown) => (shown && JSON.stringify(shown) === JSON.stringify(next) ? shown : next));
        } catch (err) {
            setLoadError((err as Error).message || String(err));
        }
    }, [server.id, t]);

    useEffect(() => {
        setInfo(infos.get(server.id) ?? null);
        load();
    }, [load, server.id]);

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
            {info.backupSupported && <BackupsCard server={server} />}
            <ScheduleCard server={server} info={info} onSaved={load} moduleCommands={moduleCommands} />
        </div>
    );
}
