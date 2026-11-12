//! macOS 알림 — UserNotifications(`UNUserNotificationCenter`).
//! 알림 플러그인의 맥 구현(mac-notification-sys)은 macOS 11에서 폐기된 `NSUserNotificationCenter`를 쓰므로 맥에서는 이것으로 낸다.
//! 처음 알릴 때 OS가 알림 허용을 묻고, 거절했으면 내지 않는다(기록만 남긴다)

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationSound,
    UNUserNotificationCenter,
};

/// 알림을 낸다. 앱 묶음 밖(번들 ID가 없는 개발 실행)이면 false: `currentNotificationCenter`가 예외를 던지므로 부르지 않는다.
/// 번들 유무는 처음 한 번만 본다: 폴백한 알림 플러그인(mac-notification-sys)이 `bundleIdentifier`를 가짜 ID로 바꿔 끼우기 때문이다
pub fn show(title: &str, body: &str, silent: bool) -> bool {
    static BUNDLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*BUNDLED.get_or_init(|| NSBundle::mainBundle().bundleIdentifier().is_some()) {
        return false;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    if !silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let id = NSString::from_str(&uuid::Uuid::new_v4().to_string());
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let target = center.clone();
    let asked = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            return tracing::warn!("[Notify] Notifications are not allowed{}", describe(error));
        }
        let added = RcBlock::new(|error: *mut NSError| match error.is_null() {
            true => tracing::info!("[Notify] Notification shown"),
            false => tracing::warn!("[Notify] Failed to show notification{}", describe(error)),
        });
        target.addNotificationRequest_withCompletionHandler(&request, Some(&added));
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &asked,
    );
    true
}

fn describe(error: *mut NSError) -> String {
    // SAFETY: 완료 처리기가 넘긴 NSError는 null이거나 처리기가 도는 동안 살아 있다
    match unsafe { error.as_ref() } {
        Some(e) => format!(": {}", e.localizedDescription()),
        None => String::new(),
    }
}
