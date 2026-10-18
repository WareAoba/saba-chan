import { useTranslation } from 'react-i18next';
import { useExtensions } from '../contexts/ExtensionContext';

/**
 * 데이터 slot — 익스텐션 manifest가 선언한 값을 그린다 (코드 실행 없음, CLI와 같은 어휘).
 * 조건 판정과 값 찾기는 데몬이 한다: `GET /api/servers`의 `server.slots`,
 * `GET /api/extensions`의 `ext.slots`. 없으면 아무것도 그리지 않는다.
 */
const slotItems = (server, slotId) => server?.slots?.[slotId] || [];

/** `ServerCard.badge` — 서버 이름 옆 짧은 표시 */
export function DataBadges({ server }) {
    const items = slotItems(server, 'ServerCard.badge');
    if (items.length === 0) return null;
    return items.map((b, i) => (
        <span className="data-slot-badge" key={`${b.extension}-${i}`} title={b.extension}>
            {b.text}
        </span>
    ));
}

/** `ServerDetail.status` — 정보 탭에 더할 [라벨, 값] */
export const dataStatusRows = (server) => slotItems(server, 'ServerDetail.status').map((r) => [r.label, r.value]);

/** 켜진 익스텐션의 `AddServer.options` 칸 */
const addServerFields = (extensions) =>
    extensions
        .filter((ext) => ext.enabled)
        .flatMap((ext) => [].concat(ext.slots?.['AddServer.options'] || []).map((f) => ({ ...f, ext: ext.id })))
        .filter((f) => f.field);

/** 생성 요청에 실을 `extension_data` — 손대지 않은 칸은 기본값 (CLI와 같음) */
export function addServerExtensionData(extensions, values) {
    const entries = addServerFields(extensions)
        .map((f) => [f.field, values[f.field] ?? f.default])
        .filter(([, v]) => v != null && v !== '');
    return entries.length ? Object.fromEntries(entries) : null;
}

/** `AddServer.options` — 서버 만들 때 물어볼 칸. 답은 `extension_data[field]`로 보낸다 */
export function AddServerOptions({ values, onChange, disabled }) {
    const { t } = useTranslation('gui');
    const { extensions } = useExtensions();
    const fields = addServerFields(extensions);
    if (fields.length === 0) return null;

    return fields.map((f) => {
        const label = t(`ext_${f.ext}:options.${f.field}`, { defaultValue: f.label || f.field });
        const value = values[f.field] ?? f.default;
        const id = `add-opt-${f.ext}-${f.field}`;
        return (
            <div className="as-section as-section-card" key={id}>
                {f.type === 'boolean' ? (
                    <label className="as-label" htmlFor={id}>
                        <input
                            id={id}
                            type="checkbox"
                            checked={!!value}
                            onChange={(e) => onChange(f.field, e.target.checked)}
                            disabled={disabled}
                        />
                        {label}
                    </label>
                ) : (
                    <>
                        <label className="as-label" htmlFor={id}>
                            {label}
                        </label>
                        <input
                            id={id}
                            className="as-input"
                            type="text"
                            value={value ?? ''}
                            onChange={(e) => onChange(f.field, e.target.value)}
                            disabled={disabled}
                        />
                    </>
                )}
            </div>
        );
    });
}
