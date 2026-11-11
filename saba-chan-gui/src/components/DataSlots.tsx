import { useTranslation } from 'react-i18next';
import type { ExtensionListItem, Json, ServerInfo } from '../types/daemon';
import { useExtensions } from '../contexts/ExtensionContext';
import { SabaToggle } from './ui/SabaUI';

/**
 * 데이터 slot — 익스텐션 manifest가 선언한 값을 그린다 (코드 실행 없음, CLI와 같은 어휘).
 * 조건 판정과 값 찾기는 데몬이 한다: `GET /api/servers`의 `server.slots`,
 * `GET /api/extensions`의 `ext.slots`. 없으면 아무것도 그리지 않는다.
 */
const slotItems = (server: Pick<ServerInfo, 'slots'> | null | undefined, slotId: string) => (server?.slots?.[slotId] as DataSlotItem[] | undefined) || [];

/** `ServerCard.badge`는 text, `ServerDetail.status`는 label·value */
interface DataSlotItem {
    extension: string;
    text?: string;
    label?: string;
    value?: string | number;
}

/** `ServerCard.badge` — 서버 이름 옆 짧은 표시 */
export function DataBadges({ server }: { server: Pick<ServerInfo, 'slots'> | null | undefined }) {
    const items = slotItems(server, 'ServerCard.badge');
    if (items.length === 0) return null;
    return items.map((b, i) => (
        <span
            className="data-slot-badge"
            // biome-ignore lint/suspicious/noArrayIndexKey: 읽기 전용 목록이라 순서가 바뀌지 않는다
            key={`${b.extension}-${i}`}
            data-tooltip={b.extension}
        >
            {b.text}
        </span>
    ));
}

/** `ServerDetail.status` — 정보 탭에 더할 [라벨, 값] */
export const dataStatusRows = (server: Pick<ServerInfo, 'slots'> | null | undefined) => slotItems(server, 'ServerDetail.status').map((r) => [r.label, r.value]);

/** manifest `AddServer.options` 한 칸 */
interface AddServerField {
    field: string;
    type?: string;
    label?: string;
    default?: Json;
}

type AddServerValues = Record<string, Json | undefined>;

/** 켜진 익스텐션의 `AddServer.options` 칸 */
const addServerFields = (extensions: ExtensionListItem[]) =>
    extensions
        .filter((ext) => ext.enabled)
        .flatMap((ext) =>
            ([] as AddServerField[])
                .concat((ext.slots?.['AddServer.options'] || []) as unknown as AddServerField | AddServerField[])
                .map((f) => ({ ...f, ext: ext.id })),
        )
        .filter((f) => f.field);

/** 생성 요청에 실을 `extension_data` — 손대지 않은 칸은 기본값 (CLI와 같음) */
export function addServerExtensionData(extensions: ExtensionListItem[], values: AddServerValues) {
    const entries = addServerFields(extensions)
        .map((f) => [f.field, values[f.field] ?? f.default])
        .filter(([, v]) => v != null && v !== '');
    return entries.length ? Object.fromEntries(entries) : null;
}

/** `AddServer.options` — 서버 만들 때 물어볼 칸. 답은 `extension_data[field]`로 보낸다 */
interface AddServerOptionsProps {
    values: AddServerValues;
    onChange: (field: string, value: Json) => void;
    disabled?: boolean;
}

export function AddServerOptions({ values, onChange, disabled }: AddServerOptionsProps) {
    const { t } = useTranslation('gui');
    const { extensions } = useExtensions();
    const fields = addServerFields(extensions);
    if (fields.length === 0) return null;

    // 서버 추가 창의 다른 칸과 같은 줄 모양 (설정 이름 왼쪽, 입력 오른쪽)
    return (
        <section className="dc-card ss-card">
            <div className="ss-rows">
                {fields.map((f) => {
                    const label = t(`ext_${f.ext}:options.${f.field}`, { defaultValue: f.label || f.field });
                    const value = values[f.field] ?? f.default;
                    const id = `add-opt-${f.ext}-${f.field}`;
                    return (
                        <div className="ss-row" key={id}>
                            <label className="ss-name" htmlFor={id}>
                                {label}
                            </label>
                            <div className="ss-control">
                                {f.type === 'boolean' ? (
                                    <SabaToggle
                                        id={id}
                                        checked={!!value}
                                        onChange={(on) => onChange(f.field, on)}
                                        disabled={disabled}
                                    />
                                ) : (
                                    <input
                                        id={id}
                                        className="input ss-text"
                                        type="text"
                                        value={(value as string | number | undefined) ?? ''}
                                        onChange={(e) => onChange(f.field, e.target.value)}
                                        disabled={disabled}
                                    />
                                )}
                            </div>
                        </div>
                    );
                })}
            </div>
        </section>
    );
}
