import { useState } from 'react';
import { DataTransferSection } from '../DataTransferSection';
import { FolderPathsSection } from '../FolderPathsSection';
import { StorageSection } from '../StorageSection';

/** 설정 › 저장 공간: 디스크·사바쨩 파일·파일 이동, 접어 둔 내보내기·가져오기와 폴더 위치. `daemonTarget`: 지금 다루는 데몬 (설정을 열 때 받는다) */
export function StorageTab({ daemonTarget }: { daemonTarget: string | null | undefined }) {
    // 원격 데몬을 다루는 동안은 그 PC의 디스크·폴더를 보인다 (경로 고르기는 RemotePathPicker, D2). 데몬을 바꾸면 다시 읽는다
    const key = daemonTarget || 'local';
    // 파일을 옮기면 폴더 위치도 다시 읽는다
    const [moves, setMoves] = useState(0);
    return (
        <>
            <StorageSection key={key} onMoved={() => setMoves((n) => n + 1)} />
            <DataTransferSection key={key} defaultOpen={false} />
            <FolderPathsSection key={key} defaultOpen={false} revision={moves} />
        </>
    );
}
