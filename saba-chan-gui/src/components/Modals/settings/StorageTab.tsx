import { DataTransferSection, dataTransferAvailable } from '../DataTransferSection';
import { FolderPathsSection, folderPathsAvailable } from '../FolderPathsSection';

/** 설정 › 저장 공간: 폴더 경로·데이터 옮기기. `daemonTarget`: 지금 다루는 데몬 (설정을 열 때 받는다) */
export function StorageTab({ daemonTarget }: { daemonTarget: string | null | undefined }) {
    return (
        <>
            {/* 원격 데몬을 다루는 동안은 경로를 그 PC의 폴더에서 고른다 (RemotePathPicker, D2). 데몬을 바꾸면 다시 읽는다 */}
            {folderPathsAvailable() && <FolderPathsSection key={daemonTarget || 'local'} />}

            {dataTransferAvailable() && <DataTransferSection key={daemonTarget || 'local'} />}
        </>
    );
}
