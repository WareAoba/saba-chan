/// 모듈·익스텐션 폴더에 파일을 쓰는 작업(설치·제거)과 폴더 옮기기를 가른다.
/// 작업은 읽기 잠금을 쥐고 하고, 옮기기는 쓰기 잠금을 바로 잡지 못하면 거절한다
pub(crate) static IN_USE: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

/// 모듈·익스텐션 폴더에 쓰는 동안 쥔다 (옮기는 중이면 끝나기를 기다린다)
pub(crate) async fn in_use() -> tokio::sync::RwLockReadGuard<'static, ()> {
    IN_USE.read().await
}
