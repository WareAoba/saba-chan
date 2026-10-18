import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';

/**
 * "CLI로 보기" (I4-c): 이 화면과 같은 일을 하는 CLI 한 줄. 데몬 카탈로그(GET /api/actions)의
 * `cli` 템플릿을 화면 값으로 채운다. 채널이나 템플릿이 없으면 그리지 않는다.
 */
let catalog = null; // 액션 id → cli 템플릿 (앱 실행 동안 한 번만 묻는다)

export function resetCliCache() {
    catalog = null;
}

function loadCatalog() {
    if (!catalog) {
        catalog = Promise.resolve(window.api.actionsList({ surface: 'gui' }))
            .then((res) => {
                const list = Array.isArray(res) ? res : res?.actions || [];
                return Object.fromEntries(list.filter((a) => a.cli).map((a) => [a.id, a.cli]));
            })
            .catch(() => {
                catalog = null;
                return {};
            });
    }
    return catalog;
}

/** 셸 한 칸: 객체·배열은 JSON을 작은따옴표로, 공백·따옴표가 있는 글은 큰따옴표로 */
function arg(value) {
    if (value !== null && typeof value === 'object') return `'${JSON.stringify(value)}'`;
    const s = String(value ?? '');
    return /[\s"']/.test(s) || s === '' ? `"${s.replace(/"/g, '\\"')}"` : s;
}

export function fillCli(template, { server, values = {}, extra = {} }) {
    let line = template.replace('<server>', arg(server));
    line = line.replace(/<([A-Za-z0-9_.-]+)>/g, (m, name) => (name in values ? arg(values[name]) : m));
    const rest = Object.entries(extra).map(([k, v]) => `${k}=${arg(v)}`);
    return rest.length ? `${line} ${rest.join(' ')}` : line;
}

export function CliLine({ actionId, server, values, extra }) {
    const { t } = useTranslation('gui');
    const [template, setTemplate] = useState(null);
    const [copied, setCopied] = useState(false);

    useEffect(() => {
        if (!window.api?.actionsList) return;
        let alive = true;
        loadCatalog().then((map) => alive && setTemplate(map[actionId] || null));
        return () => {
            alive = false;
        };
    }, [actionId]);

    if (!template) return null;
    const line = fillCli(template, { server, values, extra });

    return (
        <div className="cli-line">
            <Icon name="terminal" size="sm" />
            <code title={line}>{line}</code>
            <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={async () => {
                    await navigator.clipboard?.writeText(line);
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                }}
                aria-label={t('cli_line.copy')}
                title={copied ? t('cli_line.copied') : t('cli_line.copy')}
            >
                <Icon name={copied ? 'check' : 'copy'} size="sm" />
            </button>
        </div>
    );
}
