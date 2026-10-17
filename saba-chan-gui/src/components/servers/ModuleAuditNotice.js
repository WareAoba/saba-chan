import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

// 모듈 코드는 바뀔 때만 다시 검사하면 된다: 이 창이 떠 있는 동안 모듈 이름별로 한 번
const cache = new Map();

export function clearModuleAuditCache() {
    cache.clear();
}

function loadAudit(moduleName) {
    if (!cache.has(moduleName)) {
        cache.set(
            moduleName,
            Promise.resolve(window.api.moduleAudit(moduleName))
                .catch(() => null)
                .then((res) => {
                    // 실패(데몬 꺼짐·4xx/5xx)는 기억하지 않는다: 다음에 이 서버를 열 때 다시 묻는다
                    if (!Array.isArray(res?.undeclared)) cache.delete(moduleName);
                    return res;
                }),
        );
    }
    return cache.get(moduleName);
}

/**
 * 모듈 코드 검사 경고 (데몬 M12: GET /api/module/:name/audit).
 * 모듈이 [permissions]에 선언하지 않은 능력(네트워크·프로세스 실행 등)을 코드에서 쓰면 알린다.
 * 정적 검사라 다 잡지는 못한다: 선언과 다른 곳을 보여 주는 한 겹이다.
 */
export function ModuleAuditNotice({ moduleName }) {
    const { t } = useTranslation('gui');
    const [undeclared, setUndeclared] = useState([]);
    const [open, setOpen] = useState(false);

    useEffect(() => {
        setUndeclared([]);
        setOpen(false);
        if (!moduleName || !window.api?.moduleAudit) return;
        let alive = true;
        loadAudit(moduleName).then((res) => {
            if (alive && Array.isArray(res?.undeclared)) setUndeclared(res.undeclared);
        });
        return () => {
            alive = false;
        };
    }, [moduleName]);

    if (undeclared.length === 0) return null;
    const kinds = [...new Set(undeclared.map((f) => f.permission))];
    const label = (p) => t(`module_audit.perm_${p}`, { defaultValue: p });

    return (
        <div className="notice tone-warn module-audit" role="alert">
            <Icon name="alertTriangle" size="sm" />
            <span className="module-audit-body">
                <strong>{t('module_audit.title', { module: moduleName })}</strong>{' '}
                {t('module_audit.summary', { perms: kinds.map(label).join(', ') })}
                <button type="button" className="btn-link" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
                    {open ? t('module_audit.hide') : t('module_audit.show', { count: undeclared.length })}
                </button>
                {open && (
                    <ul className="module-audit-list">
                        {undeclared.map((f) => (
                            <li key={`${f.file}:${f.line}:${f.permission}`}>
                                <span>{label(f.permission)}</span>
                                <code>
                                    {f.file}:{f.line}
                                </code>
                                {f.detail && <span className="muted">{f.detail}</span>}
                            </li>
                        ))}
                    </ul>
                )}
            </span>
        </div>
    );
}
