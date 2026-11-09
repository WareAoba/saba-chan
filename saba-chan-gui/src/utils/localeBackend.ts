/**
 * 번역 런타임 읽기 (05 §번역 런타임): i18next 백엔드는 설치 폴더 번역(데몬 `GET /api/locales/{lang}/{ns}`)을
 * 먼저 쓰고, 데몬이 없거나 키가 빠지면 빌드에 묶인 번역으로 채운다. 번역 판(`locales_version`)이 바뀌면 다시 읽는다.
 */
import type { BackendModule, ResourceKey } from 'i18next';

type Bundle = Record<string, unknown>;
/** 언어·묶음 하나. 없으면 null */
export type BundleLoader = (language: string, namespace: string) => Promise<Bundle | null>;

const isBundle = (v: unknown): v is Bundle => !!v && typeof v === 'object' && !Array.isArray(v);

/** 내장본 위에 설치본을 덮는다. 글과 묶음만 받고, 다른 모양이면 내장본을 남긴다 */
export function overlay(base: Bundle, top: unknown): Bundle {
    if (!isBundle(top)) return base;
    const out: Bundle = { ...base };
    for (const [key, value] of Object.entries(top)) {
        if (typeof value === 'string') {
            if (!isBundle(base[key])) out[key] = value;
        } else if (isBundle(value) && (base[key] === undefined || isBundle(base[key]))) {
            out[key] = overlay(isBundle(base[key]) ? (base[key] as Bundle) : {}, value);
        }
    }
    return out;
}

/** `daemonServed`: 데몬이 묶음 하나를 준 때 (판 감시가 첫 판에서 다시 읽을지 정한다) */
export function createLocaleBackend(
    embedded: BundleLoader,
    daemon: BundleLoader,
    daemonServed?: () => void,
): BackendModule {
    return {
        type: 'backend',
        init() {},
        read(language, namespace, callback) {
            const fromDaemon = daemon(language, namespace).catch(() => null);
            Promise.all([embedded(language, namespace), fromDaemon]).then(
                ([base, top]) => {
                    if (isBundle(top)) daemonServed?.();
                    callback(null, overlay(base ?? {}, top) as ResourceKey);
                },
                // 내장본을 못 읽으면 데몬 것만이라도
                (err) =>
                    fromDaemon.then((top) => {
                        if (!isBundle(top)) return callback(err, false);
                        daemonServed?.();
                        callback(null, overlay({}, top) as ResourceKey);
                    }),
            );
        },
    };
}

/** `/api/updates/status`의 `locales_version`을 보고, 바뀌면 `reload` (재시작·알약 없음) */
export function createLocalesVersionWatcher(reload: () => unknown) {
    let seen: string | null | undefined;
    let daemonServed = false;
    return {
        markDaemonServed() {
            daemonServed = true;
        },
        apply(version: unknown) {
            // 판을 모르는 예전 데몬(칸 없음)은 무시
            if (version !== null && typeof version !== 'string') return;
            if (version === seen) return;
            const first = seen === undefined;
            seen = version;
            // 처음 본 판: 처음 읽을 때 데몬 것을 이미 썼거나 설치 번역이 없으면 그대로
            if (first && (daemonServed || version === null)) return;
            reload();
        },
    };
}
