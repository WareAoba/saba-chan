import { DataTransferSection, dataTransferAvailable } from '../DataTransferSection';
import { FolderPathsSection, folderPathsAvailable } from '../FolderPathsSection';
import { StorageSection, storageAvailable } from '../StorageSection';

/** 설정 › 저장 공간: 디스크·사바쨩 파일·파일 이동, 접어 둔 내보내기·가져오기와 폴더 위치. `daemonTarget`: 지금 다루는 데몬 (설정을 열 때 받는다) */
export function StorageTab({ daemonTarget }: { daemonTarget: string | null | undefined }) {
    // 원격 데몬을 다루는 동안은 그 PC의 디스크·폴더를 보인다 (경로 고르기는 RemotePathPicker, D2). 데몬을 바꾸면 다시 읽는다
    const key = daemonTarget || 'local';
    return (
        <>
            {storageAvailable() && <StorageSection key={key} />}
            {dataTransferAvailable() && <DataTransferSection key={key} defaultOpen={false} />}
            {folderPathsAvailable() && <FolderPathsSection key={key} defaultOpen={false} />}
        </>
    );
}
