/**
 * 원격 데몬 쪽 경로 고르기 (D2). 원격 데몬을 다루는 중에는 이 PC의 대화상자 대신
 * 그 데몬의 폴더를 보여 주는 창(shell/RemotePathPicker)이 여기에 자기를 건다.
 * picker({kind: 'file'|'folder'|'save', extensions: string[]|null, defaultName: string|null}) → 고른 경로 또는 null
 */
let picker = null;

export function setRemotePathPicker(fn) {
    picker = fn || null;
}

export function pickRemotePath(request) {
    return picker ? picker(request) : Promise.resolve(null);
}

export function hasRemotePathPicker() {
    return !!picker;
}
