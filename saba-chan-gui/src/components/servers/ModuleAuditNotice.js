import { useEffect, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

// 모듈 코드는 바뀔 때만 다시 검사하면 된다: 이 창이 떠 있는 동안 모듈 이름별로 한 번
// 설치·업데이트·삭제 뒤에는 비우고, 떠 있는 경고도 다시 묻게 세대를 올린다
const cache = new Map();
let generation = 0;
const listeners = new Set();

export function clearModuleAuditCache() {
    cache.clear();
    generation += 1;
    for (const l of listeners) l();
}

function subscribe(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

// 모듈과 익스텐션은 같은 검사·같은 응답 모양이다 (GET /api/module/:name/audit, /api/extensions/:id/audit)
const SOURCES = {
    module: { api: 'moduleAudit', title: 'module_audit.title', summary: 'module_audit.summary' },
    extension: { api: 'extensionAudit', title: 'module_audit.ext_title', summary: 'module_audit.ext_summary' },
};

function loadAudit(kind, id) {
    const key = `${kind}:${id}`;
    if (!cache.has(key)) {
        cache.set(
            key,
            Promise.resolve(window.api[SOURCES[kind].api](id))
                .catch(() => null)
                .then((res) => {
                    // 실패(데몬 꺼짐·4xx/5xx)는 기억하지 않는다: 다음에 이 서버를 열 때 다시 묻는다
                    if (!Array.isArray(res?.undeclared)) cache.delete(key);
                    return res;
                }),
        );
    }
    return cache.get(key);
}

/**
 * 모듈 코드 검사 경고 (데몬 M12: GET /api/module/:name/audit).
 * 모듈이 [permissions]에 선언하지 않은 능력(네트워크·프로세스 실행 등)을 코드에서 쓰면 알린다.
 * 정적 검사라 다 잡지는 못한다: 선언과 다른 곳을 보여 주는 한 겹이다.
 */
export function ModuleAuditNotice({ moduleName }) {
    return <AuditNotice kind="module" id={moduleName} name={moduleName} />;
}

/** 익스텐션 카드용: manifest `permissions`와 코드 비교 */
export function ExtensionAuditNotice({ extId, name }) {
    return <AuditNotice kind="extension" id={extId} name={name || extId} />;
}

function AuditNotice({ kind, id, name }) {
    const { t } = useTranslation('gui');
    const source = SOURCES[kind];
    const gen = useSyncExternalStore(subscribe, () => generation);
    const [undeclared, setUndeclared] = useState([]);
    const [open, setOpen] = useState(false);
    const [failed, setFailed] = useState(false);

    useEffect(() => {
        setUndeclared([]);
        setOpen(false);
        setFailed(false);
        if (!id || !window.api?.[source.api]) return;
        let alive = true;
        loadAudit(kind, id).then((res) => {
            if (!alive) return;
            if (Array.isArray(res?.undeclared)) setUndeclared(res.undeclared);
            else setFailed(true);
        });
        return () => {
            alive = false;
        };
    }, [kind, id, source.api, gen]);

    // 검사하지 못한 것을 깨끗한 것처럼 보이지 않게 한다
    if (failed) {
        return (
            <div className="notice tone-info module-audit" role="status">
                <Icon name="info" size="sm" />
                <span className="module-audit-body">{t('module_audit.unavailable', { name })}</span>
            </div>
        );
    }
    if (undeclared.length === 0) return null;
    const kinds = [...new Set(undeclared.map((f) => f.permission))];
    const label = (p) => t(`module_audit.perm_${p}`, { defaultValue: p });

    return (
        <div className="notice tone-warn module-audit" role="alert">
            <Icon name="alertTriangle" size="sm" />
            <span className="module-audit-body">
                <strong>{t(source.title, { module: name, name })}</strong>{' '}
                {t(source.summary, { perms: kinds.map(label).join(', ') })}
                <button type="button" className="btn-link" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
                    {open ? t('module_audit.hide') : t('module_audit.show', { count: undeclared.length })}
                </button>
                {open && (
                    <ul className="module-audit-list">
                        {undeclared.map((f) => (
                            <li key={`${f.file}:${f.line}:${f.permission}`}>
                                <span>{label(f.permission)}</span>
                                <code>{f.line > 0 ? `${f.file}:${f.line}` : f.file}</code>
                                {f.detail && <span className="muted">{f.detail}</span>}
                            </li>
                        ))}
                    </ul>
                )}
            </span>
        </div>
    );
}
