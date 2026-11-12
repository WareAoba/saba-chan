import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { AddServerPayload } from '../../hooks/useServerActions';
import type { Server } from '../../stores/useServerStore';
import { ConfirmIcon } from '../Modals/Modals';
import { Dialog } from '../ui/Dialog';

/**
 * 브랜치 서버 만들기 창: 상위 서버 아래에 묶이는 새 서버를 만든다. 모듈·버전·설정은 데몬이 상위 서버에서 한 번 가져오므로
 * 여기서 정하는 것은 이름뿐이다. 포트는 데몬이 비어 있는 것으로 정해 알려 준다 (`POST /api/instances/check`).
 */
export function BranchServerDialog({
    parent,
    onClose,
    onCreate,
}: {
    parent: Server;
    onClose: () => void;
    onCreate: (payload: AddServerPayload) => Promise<boolean> | boolean;
}) {
    const { t } = useTranslation('gui');
    const [name, setName] = useState(() => t('branch.default_name', { name: parent.name }));
    const [port, setPort] = useState<number | null>(null);
    const [busy, setBusy] = useState(false);
    const edited = useRef(false);
    const firstName = useRef(name);

    useEffect(() => {
        let live = true;
        Promise.resolve(window.api?.instanceCheck?.({ name: firstName.current, branch_of: parent.id }))
            .then((res) => {
                if (!live || !res) return;
                const ports = (res.ports || []) as { field?: string; port?: number }[];
                const found = (ports.find((p) => p.field === 'port') || ports[0])?.port;
                if (typeof found === 'number') setPort(found);
                // 같은 이름이 이미 있으면 데몬이 권하는 이름으로 (직접 고친 뒤에는 두지 않는다)
                if (res.name?.taken && typeof res.name.suggested === 'string' && !edited.current) setName(res.name.suggested);
            })
            .catch(() => {});
        return () => {
            live = false;
        };
    }, [parent.id]);

    const create = async () => {
        setBusy(true);
        const made = await onCreate({
            name: name.trim(),
            module_name: parent.module,
            branch_of: parent.id,
            ...(port != null ? { port } : {}),
        });
        setBusy(false);
        if (made) onClose();
    };

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && !busy && onClose()}
            title={t('branch.create')}
            description={t('branch.desc', { name: parent.name })}
            media={<ConfirmIcon icon="branch" tone="info" />}
            size="sm"
            footer={
                <>
                    <button type="button" className="btn btn-ghost" onClick={onClose} disabled={busy}>
                        {t('modals.cancel')}
                    </button>
                    <button type="button" className="btn btn-primary" onClick={create} disabled={busy || !name.trim()}>
                        {t('branch.submit')}
                    </button>
                </>
            }
        >
            <div className="branch-form">
                <label>
                    <span>{t('branch.name')}</span>
                    <input
                        type="text"
                        className="input"
                        aria-label={t('branch.name')}
                        value={name}
                        onChange={(e) => {
                            edited.current = true;
                            setName(e.target.value);
                        }}
                    />
                </label>
                <div>
                    <span>{t('shell.info_port')}</span>
                    <div className="input branch-port">
                        {port != null ? (
                            <>
                                {port}
                                <span className="muted">{t('branch.port_hint')}</span>
                            </>
                        ) : (
                            <span className="muted">{t('branch.port_auto')}</span>
                        )}
                    </div>
                </div>
            </div>
        </Dialog>
    );
}
