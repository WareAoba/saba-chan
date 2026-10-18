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

/** `<이름>` 자리 표시는 그대로 둔다 (비밀 값 대신, 사용자가 채운다) */
const PLACEHOLDER = /^<[A-Za-z0-9_.-]+>$/;

/** 셸 한 칸: 안전한 글자만 있으면 그대로, 아니면 작은따옴표로 (POSIX: 안의 '는 '\''). 객체·배열은 JSON */
function arg(value) {
    const s = value !== null && typeof value === 'object' ? JSON.stringify(value) : String(value ?? '');
    if (PLACEHOLDER.test(s) || /^[A-Za-z0-9_./:@%+=,-]+$/.test(s)) return s;
    return `'${s.replace(/'/g, `'\\''`)}'`;
}

const SECRET_KEY = /(password|passwd|token|secret)$/i;

/**
 * 설정 화면에서 CLI 줄에 덧붙일 값: 화면을 연 뒤 바꾼 것만 (`_`로 시작하는 GUI 내부 값 제외).
 * 비밀번호 칸(field_type 'password')과 *_password·*_token·*_secret 키는 값 대신 `<이름>`을 둔다.
 */
export function changedForCli(values, openedWith, fields = []) {
    const secret = new Set(fields.filter((f) => f.field_type === 'password').map((f) => f.name));
    return Object.fromEntries(
        Object.entries(values)
            .filter(([k, v]) => !k.startsWith('_') && String(v ?? '') !== String(openedWith[k] ?? ''))
            .map(([k, v]) => [k, secret.has(k) || SECRET_KEY.test(k) ? `<${k}>` : v]),
    );
}

export function fillCli(template, { server, values = {}, extra = {} }) {
    let line = template.replace('<server>', arg(server));
    line = line.replace(/<([A-Za-z0-9_.-]+)>/g, (m, name) => (name in values ? arg(values[name]) : m));
    const rest = Object.entries(extra).map(([k, v]) => `${k}=${arg(v)}`);
    return rest.length ? `${line} ${rest.join(' ')}` : line;
}

/** 명령 한 줄과 복사 버튼 */
export function CliText({ line }) {
    const { t } = useTranslation('gui');
    const [copied, setCopied] = useState(false);
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

export function CliLine({ actionId, server, values, extra }) {
    const [template, setTemplate] = useState(null);

    useEffect(() => {
        if (!window.api?.actionsList) return;
        let alive = true;
        loadCatalog().then((map) => alive && setTemplate(map[actionId] || null));
        return () => {
            alive = false;
        };
    }, [actionId]);

    if (!template) return null;
    return <CliText line={fillCli(template, { server, values, extra })} />;
}
