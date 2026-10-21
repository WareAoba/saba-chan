import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listItems } from '../utils/validation';
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
    const types = Object.fromEntries(fields.map((f) => [f.name, f.field_type]));
    // CLI에 낼 모양으로 비교한다 (목록 ['a,b']와 ['a','b']는 다른 값)
    const same = (k, v) => String(cliValue(types[k], v) ?? '') === String(cliValue(types[k], openedWith[k]) ?? '');
    return Object.fromEntries(
        Object.entries(values)
            .filter(([k, v]) => !k.startsWith('_') && !same(k, v))
            .map(([k, v]) => [k, secret.has(k) || SECRET_KEY.test(k) ? `<${k}>` : cliValue(types[k], v)]),
    );
}

/** CLI `server set` 문법: list는 `a,b`(항목에 쉼표가 있으면 JSON 배열), multiline은 `\`→`\\` 뒤 줄바꿈→`\n` */
function cliValue(fieldType, v) {
    if (fieldType === 'list') {
        const items = listItems(v);
        // 쉼표나 앞뒤 공백이 든 항목, [로 시작하는 목록은 CLI 쉼표 경로가 그대로 두지 않아 JSON 배열로
        return items.some((i) => i.includes(',') || i !== i.trim()) || items[0]?.startsWith('[')
            ? JSON.stringify(items)
            : items.join(',');
    }
    if (fieldType === 'multiline')
        return String(v ?? '')
            .replace(/\\/g, '\\\\')
            .replace(/\n/g, '\\n');
    return v;
}

export function fillCli(template, { server, values = {}, extra = {} }) {
    let line = template.replace('<server>', arg(server));
    line = line.replace(/<([A-Za-z0-9_.-]+)>/g, (m, name) => (name in values ? arg(values[name]) : m));
    const rest = Object.entries(extra).map(([k, v]) => `${k}=${arg(v)}`);
    return rest.length ? `${line} ${rest.join(' ')}` : line;
}

/**
 * 데몬 split_quoted(src/chat/mod.rs)와 같은 규칙으로 나눈다: 공백으로 나누되, `"`로 시작한 낱말은
 * 뒤에 공백(또는 끝)이 오는 닫는 `"`까지를 한 칸으로 (따옴표는 뗀다)
 */
export function splitQuoted(text) {
    const out = [];
    let rest = text.trimStart();
    while (rest) {
        const ws = rest.search(/\s/);
        const end = ws < 0 ? rest.length : ws;
        const word = rest.slice(0, end);
        if (word.length > 1 && word.startsWith('"') && word.endsWith('"')) {
            out.push(word.slice(1, -1));
            rest = rest.slice(end).trimStart();
            continue;
        }
        if (word.startsWith('"')) {
            const body = rest.slice(1);
            const close = [...body.matchAll(/"(?=\s|$)/g)][0];
            if (close) {
                out.push(body.slice(0, close.index));
                rest = body.slice(close.index + 1).trimStart();
                continue;
            }
        }
        out.push(word);
        rest = rest.slice(end).trimStart();
    }
    return out;
}

/**
 * 명령 화면 입력 한 줄의 CLI 등가. 셸이 먼저 읽으므로 모든 칸을 따옴표로 지킨다.
 * 아는 명령이면 `server run`: 데몬과 같은 규칙으로 나눈 칸마다 한 인자.
 * 아니면 콘솔에 그대로 보내는 `server send`: 줄 전체를 한 인자 (CLI가 공백으로 다시 잇는다).
 */
export function commandCli(serverName, line, commands = []) {
    const text = line.trim();
    if (!text) return null;
    const words = splitQuoted(text);
    const name = (words[0] || '').toLowerCase();
    const known = commands.some(
        (c) => c.name.toLowerCase() === name || (c.aliases || []).some((a) => String(a).toLowerCase() === name),
    );
    const prefix = `saba-chan-cli server ${known ? 'run' : 'send'} ${arg(serverName)}`;
    return known ? `${prefix} ${words.map(arg).join(' ')}` : `${prefix} ${arg(text)}`;
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
