import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setRemotePathPicker } from '../../api/remotePathPicker';
import { useModalClose } from '../../hooks/useModalClose';
import { Icon } from '../Icon';
import '../Modals/Modals.css';
import './RemotePathPicker.css';

const TITLES = { file: 'title_file', folder: 'title_folder', save: 'title_save' };

/**
 * 원격 데몬 쪽 경로 고르기 (D2). 원격 데몬을 다루는 동안 대화상자를 여는 브리지 메서드
 * (openFileDialog·openFolderDialog·dataExport 등)가 이 창을 띄운다. 폴더 목록은 그 데몬이 읽는다 (GET /api/system/browse)
 */
function RemotePathPicker() {
    const { t } = useTranslation('gui');
    const [request, setRequest] = useState(null); // {kind, extensions, defaultName, resolve}
    const [listing, setListing] = useState(null);
    const [typed, setTyped] = useState('');
    const [chosen, setChosen] = useState(null);
    const [name, setName] = useState('');
    const [error, setError] = useState(null);
    const [busy, setBusy] = useState(false);
    const requestRef = useRef(null);

    const finish = useCallback((value) => {
        requestRef.current?.resolve(value);
        requestRef.current = null;
        setRequest(null);
    }, []);
    const { isClosing, requestClose } = useModalClose(() => finish(null));

    const open = useCallback(
        async (path) => {
            const req = requestRef.current;
            if (!req) return;
            setBusy(true);
            setError(null);
            const res = await window.api.systemBrowse(
                path,
                req.kind === 'folder' ? null : req.extensions?.length ? req.extensions : ['*'],
            );
            setBusy(false);
            if (!res?.ok) {
                const key = `remote_picker.err_${res?.error_code}`;
                setError(t(key, { defaultValue: t('remote_picker.err_generic') }));
                return;
            }
            setListing(res);
            setTyped(res.path || '');
            setChosen(null);
        },
        [t],
    );

    useEffect(() => {
        setRemotePathPicker(
            (req) =>
                new Promise((resolve) => {
                    // 앞서 연 창이 있으면 그것은 취소로 끝낸다
                    requestRef.current?.resolve(null);
                    requestRef.current = { ...req, resolve };
                    setRequest(requestRef.current);
                    setListing(null);
                    setChosen(null);
                    setError(null);
                    setName(req.defaultName || '');
                    (async () => {
                        const roots = await window.api.systemBrowse('', null);
                        if (requestRef.current?.resolve !== resolve) return;
                        if (roots?.ok && roots.home) await open(roots.home);
                        else if (roots?.ok) {
                            setListing(roots);
                            setTyped('');
                        } else setError(t('remote_picker.err_generic'));
                    })();
                }),
        );
        return () => {
            setRemotePathPicker(null);
            requestRef.current?.resolve(null);
            requestRef.current = null;
        };
    }, [open, t]);

    if (!request) return null;

    const { kind, extensions } = request;
    const here = listing?.path || null;
    const sep = listing?.sep || '/';
    const saveName = () => {
        const n = name.trim();
        const ext = extensions?.[0];
        if (!n || !ext || ext === '*' || n.toLowerCase().endsWith(`.${ext.toLowerCase()}`)) return n;
        return `${n}.${ext}`;
    };
    const result = () => {
        if (kind === 'file') return chosen;
        if (kind === 'folder') return here;
        const n = saveName();
        return here && n ? `${here.endsWith(sep) ? here : here + sep}${n}` : null;
    };
    const ready = !busy && !!result();
    const choose = () => {
        const value = result();
        if (value) finish(value);
    };
    const up = () => (listing?.parent ? open(listing.parent) : open(''));

    return (
        <div className={clsx('modal-overlay', 'remote-picker-overlay', { closing: isClosing })} onClick={requestClose}>
            <div
                className="modal remote-picker"
                role="dialog"
                aria-label={t(`remote_picker.${TITLES[kind]}`)}
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                    if (e.key === 'Escape') requestClose();
                }}
            >
                <h2 className="modal-title">{t(`remote_picker.${TITLES[kind]}`)}</h2>
                <div className="remote-picker-bar">
                    <button
                        type="button"
                        className="icon-btn"
                        onClick={up}
                        disabled={busy || !here}
                        aria-label={t('remote_picker.up')}
                        data-tooltip={t('remote_picker.up')}
                    >
                        <Icon name="arrowUp" size="sm" />
                    </button>
                    {listing?.home && (
                        <button
                            type="button"
                            className="icon-btn"
                            onClick={() => open(listing.home)}
                            disabled={busy}
                            aria-label={t('remote_picker.home')}
                            data-tooltip={t('remote_picker.home')}
                        >
                            <Icon name="home" size="sm" />
                        </button>
                    )}
                    <input
                        className="setting-input-text remote-picker-path mono"
                        aria-label={t('remote_picker.path')}
                        value={typed}
                        placeholder={t('remote_picker.roots')}
                        onChange={(e) => setTyped(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') open(typed.trim());
                        }}
                    />
                </div>
                {error && (
                    <p className="remote-picker-error" role="alert">
                        {error}
                    </p>
                )}
                <ul className="remote-picker-list">
                    {listing?.entries?.map((e) => (
                        <li key={e.path}>
                            <button
                                type="button"
                                className={clsx('remote-picker-entry', { selected: chosen === e.path, file: !e.dir })}
                                aria-label={e.name}
                                onClick={() =>
                                    e.dir ? open(e.path) : kind === 'file' ? setChosen(e.path) : setName(e.name)
                                }
                                onDoubleClick={() => !e.dir && kind === 'file' && finish(e.path)}
                                disabled={busy}
                            >
                                <Icon name={e.dir ? 'folder' : 'file'} size="sm" />
                                <span>{e.name}</span>
                            </button>
                        </li>
                    ))}
                    {listing && !listing.entries?.length && (
                        <li className="remote-picker-empty">{t('remote_picker.empty')}</li>
                    )}
                </ul>
                {listing?.truncated && <p className="setting-description">{t('remote_picker.truncated')}</p>}
                {kind === 'save' && (
                    <input
                        className="setting-input-text remote-picker-name"
                        aria-label={t('remote_picker.file_name')}
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') choose();
                        }}
                    />
                )}
                <div className="modal-buttons-group">
                    <button type="button" className="modal-button question-confirm" onClick={choose} disabled={!ready}>
                        {t(
                            `remote_picker.${kind === 'folder' ? 'choose_folder' : kind === 'save' ? 'save' : 'choose'}`,
                        )}
                    </button>
                    <button type="button" className="modal-button question-cancel" onClick={requestClose}>
                        {t('remote_picker.cancel')}
                    </button>
                </div>
            </div>
        </div>
    );
}

export default RemotePathPicker;
