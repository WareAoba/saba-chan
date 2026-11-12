//! 업데이터 `--apply` 한 번의 흐름: 받아 둔 목록 읽기 → 대상 정하기 → 적용 → 완료 마커 → 목록 지우기.
//! 창 업데이터와 창 없는 적용(`--headless`)이 같이 쓴다

use super::*;

/// 적용 진행 (창 업데이터는 `apply:progress` 이벤트로 그대로 보낸다)
#[derive(Debug, Clone, Serialize)]
pub struct ApplyProgress {
    pub step: String,
    /// 진단용 영어 원문. 화면 문구는 `applied`·`failed` 수로 번역한다
    pub message: String,
    pub percent: i32,
    pub applied: Vec<String>,
    /// 실패하거나 건너뛴 키 수
    pub failed: usize,
}

impl ApplyProgress {
    fn new(step: &str, message: &str, percent: i32, applied: &[String], failed: usize) -> Self {
        Self { step: step.into(), message: message.into(), percent, applied: applied.to_vec(), failed }
    }
}

impl UpdateManager {
    /// 받아 둔 업데이트를 적용하고 결과를 완료 마커에 남긴다 (성공·실패 모두).
    /// `(적용한 키, 실패·건너뛴 키와 이유)`. 받아 둔 목록을 못 읽으면 실패 마커를 남기고 `Err`
    pub async fn apply_staged(&mut self, mut report: impl FnMut(&ApplyProgress)) -> std::result::Result<(Vec<String>, Vec<String>), String> {
        report(&ApplyProgress::new("manifest", "Loading manifest...", 10, &[], 0));
        let count = match self.load_pending_manifest() {
            Ok(count) => count,
            Err(e) => {
                let msg = format!("Failed to load manifest: {}", e);
                report(&ApplyProgress::new("error", &msg, 0, &[], 1));
                UpdateCompletionMarker::failure(msg.clone()).save().ok();
                return Err(msg);
            }
        };
        report(&ApplyProgress::new("manifest", &format!("{} components ready", count), 25, &[], 0));

        // apply-targets.json이 없거나 비어있으면 받아 둔 것 전체를 적용 순서대로. 실패하면 남은 것은 건너뛴 것으로 남긴다
        let mut target_keys = self.load_updater_apply_targets().unwrap_or_default();
        tracing::info!("[Apply] Targets: {:?}", target_keys);
        if target_keys.is_empty() {
            target_keys = self.pending_keys_in_apply_order();
        }
        // 목록을 남긴 뒤 설치기로 같거나 더 새 판을 깐 키는 적용할 것이 없다 (실패로 남은 것까지 막지 않게)
        target_keys.retain(|k| !self.already_installed.contains(k));
        // 코어가 바뀌는 묶음에서 지금 코어로 돌지 않는 모듈·익스텐션은 받아 둔 채 둔다: 새 데몬이 건강하지 않아 코어만
        // 되돌아가면(`live_apply`) 옛 코어가 거절하는 판이 남는다. 건강한 새 데몬이 첫 확인의 자동 교체로 바꾼다
        let waiting = self.addons_waiting_for_the_new_core(&target_keys);
        target_keys.retain(|k| !waiting.contains(k));
        // 적용 대상 목록이 일부만 고른 것이면 나머지는 받아 둔 채 다음 적용을 기다린다
        let rest: Vec<String> = self
            .status
            .components
            .iter()
            .map(|c| c.component.manifest_key())
            .filter(|k| !target_keys.contains(k))
            .collect();
        let total = target_keys.len();
        let (applied, failed) = self
            .apply_target_keys(&target_keys, |i, key, done| {
                let pct = 30 + ((i as i32) * 60 / std::cmp::max(total as i32, 1));
                report(&ApplyProgress::new("applying", &format!("Applying {} ({}/{})...", key, i + 1, total), pct, done, 0));
            })
            .await;

        // 성공·실패 모두 남겨 "됐는지 안 됐는지" 볼 수 있게 한다
        let marker = UpdateCompletionMarker {
            message: Some(if failed.is_empty() {
                format!("{} updates applied: {}", applied.len(), applied.join(", "))
            } else {
                format!("{} applied, {} failed", applied.len(), failed.len())
            }),
            success: failed.is_empty(),
            failed: failed.clone(),
            // 적용한 릴리즈의 버전·패치노트: 앱이 결과 알림을 누르면 보여 준다 (A5)
            release: self.pending_release(),
            ..UpdateCompletionMarker::success(applied.clone())
        };
        marker.save().ok();
        self.finish_pending_manifest(&rest);

        if failed.is_empty() {
            let msg = if applied.is_empty() { "No updates to apply.".to_string() } else { format!("{} updates applied!", applied.len()) };
            report(&ApplyProgress::new("complete", &msg, 100, &applied, 0));
        } else {
            let msg = format!("{} applied, {} failed: {}", applied.len(), failed.len(), failed.join("; "));
            report(&ApplyProgress::new("error", &msg, 0, &applied, failed.len()));
        }
        Ok((applied, failed))
    }
}
