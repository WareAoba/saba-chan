//! 효과음 — 데몬 이벤트와 업데이트 확인 결과에 맞춰 짧은 소리를 낸다 (설정 `sounds`: 전체·종류별 스위치, 기본 켬).
//!
//! 셸이 내므로 창 없이 트레이에만 떠 있어도 들린다. 앱에서 직접 켜고 끈 것(`source:"gui"`)은
//! 누른 사람이 이미 보고 있으므로 울리지 않는다. 원격 데몬을 이 PC에서 켜고 끈 것(`source:"remote"`, `remoteHost`가 이 PC)도
//! 같다. 그 원격 PC에서는 밖에서 한 일이라 울린다. 소리 파일은 `sounds/`에 있고 실행 파일에 싣는다.

use crate::events::SseEvent;
use crate::{window, AppState};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    InstallSuccess,
    InstallFailed,
    Started,
    Stopped,
    Crashed,
    UpdateAvailable,
}

impl Cue {
    fn file_name(self) -> &'static str {
        match self {
            Cue::InstallSuccess => "instance-install-success.wav",
            Cue::InstallFailed => "instance-install-failed.wav",
            Cue::Started => "instance-started.wav",
            Cue::Stopped => "instance-stopped.wav",
            Cue::Crashed => "instance-crashed.wav",
            Cue::UpdateAvailable => "update-available.wav",
        }
    }

    /// 이 소리를 켜고 끄는 `sounds` 설정 키
    fn setting_key(self) -> &'static str {
        match self {
            Cue::Started => "serverStart",
            Cue::Stopped => "serverStop",
            Cue::Crashed => "crash",
            Cue::InstallSuccess | Cue::InstallFailed => "install",
            Cue::UpdateAvailable => "update",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Cue::InstallSuccess => include_bytes!("../sounds/instance-install-success.wav"),
            Cue::InstallFailed => include_bytes!("../sounds/instance-install-failed.wav"),
            Cue::Started => include_bytes!("../sounds/instance-started.wav"),
            Cue::Stopped => include_bytes!("../sounds/instance-stopped.wav"),
            Cue::Crashed => include_bytes!("../sounds/instance-crashed.wav"),
            Cue::UpdateAvailable => include_bytes!("../sounds/update-available.wav"),
        }
    }
}

/// 이벤트 → 낼 소리. 설치 결과는 끝난 때 한 번만 낸다 (같은 끝 상태가 다시 와도 울리지 않는다)
#[derive(Default)]
pub struct EventCues {
    finished_installs: HashSet<String>,
}

impl EventCues {
    pub fn cue_for(&mut self, ev: &SseEvent) -> Option<Cue> {
        let d = &ev.data;
        match ev.kind.as_str() {
            // 디스코드·웹·CLI·예약·자동 재시작·사바쨩 밖에서 켠 것. 앱 버튼은 빼고
            "instance.started" => (d["source"] != "gui").then_some(Cue::Started),
            "instance.stopped" => (d["source"] != "gui").then_some(Cue::Stopped),
            // 예기치 않은 종료는 크래시와 원인 모를 종료(작업 관리자 강제 종료 등)뿐이다 (01 §이벤트)
            "instance.exit" => matches!(d["kind"].as_str(), Some("crash" | "unknown_exit")).then_some(Cue::Crashed),
            "provision.progress" => {
                let name = d["instance"].as_str()?;
                if d["done"].as_bool() != Some(true) {
                    self.finished_installs.remove(name);
                    return None;
                }
                if !self.finished_installs.insert(name.to_string()) {
                    return None;
                }
                let failed = d["error"].as_str().is_some_and(|e| !e.is_empty());
                Some(if failed {
                    Cue::InstallFailed
                } else {
                    Cue::InstallSuccess
                })
            }
            _ => None,
        }
    }
}

/// 원격 데몬에 켜기·끄기를 시킨 PC (`remoteHost`). 이 PC의 hostId와 같으면 이 앱에서 누른 것이다
pub fn remote_requester(ev: &SseEvent) -> Option<&str> {
    ev.data["remoteHost"].as_str().filter(|h| !h.is_empty())
}

/// 이 PC(`own`, 릴레이 hostId)가 원격으로 누른 것인가. 이 PC의 hostId를 모르면 울린다
pub fn asked_by_this_pc(requester: &str, own: Option<&str>) -> bool {
    own.is_some_and(|own| own == requester)
}

/// 전체 스위치와 그 종류의 스위치가 모두 꺼지지 않았으면 울린다 (값이 없으면 켬)
fn enabled(settings: &Value, cue: Cue) -> bool {
    let sounds = &settings["sounds"];
    sounds["enabled"] != json!(false) && sounds[cue.setting_key()] != json!(false)
}

pub fn play(app: &AppHandle, cue: Cue) {
    if !enabled(&app.state::<AppState>().settings(), cue) {
        return tracing::info!("[Sound] {:?} — turned off in settings", cue);
    }
    tracing::info!("[Sound] {:?}", cue);
    play_bytes(app, cue);
}

/// 업데이트를 찾았다: 작업 표시줄 단추를 깜박이고(Windows, 창을 앞으로 가져올 때까지) Dock 아이콘을 튀긴다(macOS).
/// 창 없이 트레이에만 떠 있으면 단추가 없으므로 OS 알림으로 충분하다
pub fn request_attention(app: &AppHandle) {
    let Some(w) = app.get_webview_window(window::MAIN) else {
        return;
    };
    // Windows의 Informational이 "앞으로 올 때까지 깜박임"이다. macOS는 Critical이어야 앱을 열 때까지 튄다
    let kind = if cfg!(windows) {
        tauri::UserAttentionType::Informational
    } else {
        tauri::UserAttentionType::Critical
    };
    if let Err(e) = w.request_user_attention(Some(kind)) {
        tracing::warn!("[Sound] Could not request attention: {}", e);
    }
}

#[cfg(windows)]
fn play_bytes(_app: &AppHandle, cue: Cue) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    // SND_MEMORY + SND_ASYNC는 다 울릴 때까지 버퍼가 살아 있어야 한다: 실행 파일에 실은 'static 바이트다.
    // 새 소리는 울리던 소리를 끊는다
    let ok = unsafe {
        PlaySoundW(
            cue.bytes().as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        )
    };
    if ok == 0 {
        tracing::warn!("[Sound] PlaySound failed for {}", cue.file_name());
    }
}

/// macOS는 afplay, 리눅스는 paplay(없으면 aplay)로 앱 캐시 폴더에 풀어 둔 파일을 튼다.
/// 여러 사용자가 함께 쓰는 임시 폴더는 쓰지 않는다 (남이 미리 만든 링크를 따라 덮어쓸 수 있다)
#[cfg(not(windows))]
fn play_bytes(app: &AppHandle, cue: Cue) {
    let dir = match app.path().app_cache_dir() {
        Ok(d) => d.join("sounds"),
        Err(e) => return tracing::warn!("[Sound] No cache folder: {}", e),
    };
    std::thread::spawn(move || {
        let path = dir.join(cue.file_name());
        let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() == cue.bytes().len() as u64);
        if !fresh {
            if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, cue.bytes())) {
                return tracing::warn!("[Sound] Could not write {}: {}", path.display(), e);
            }
        }
        let players: &[&str] = if cfg!(target_os = "macos") {
            &["afplay"]
        } else {
            &["paplay", "aplay"]
        };
        for player in players {
            if let Ok(mut child) = std::process::Command::new(player).arg(&path).spawn() {
                let _ = child.wait();
                return;
            }
        }
        tracing::warn!("[Sound] No audio player found for {}", cue.file_name());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, data: Value) -> SseEvent {
        SseEvent {
            id: None,
            kind: kind.into(),
            data,
        }
    }

    #[test]
    fn a_remote_start_from_this_pc_is_this_apps_own_button() {
        let from_a = ev("instance.started", json!({"source": "remote", "remoteHost": "host-a"}));
        assert_eq!(remote_requester(&from_a), Some("host-a"));
        // 원격 데몬을 고른 이 PC(host-a)가 누른 것: 울리지 않는다
        assert!(asked_by_this_pc("host-a", Some("host-a")));
        // 다른 PC가 이 PC의 데몬을 켰다: 밖에서 한 일이다
        assert!(!asked_by_this_pc("host-a", Some("host-b")));
        assert!(!asked_by_this_pc("host-a", None));
        assert_eq!(
            remote_requester(&ev("instance.started", json!({"source": "discord"}))),
            None
        );
    }

    #[test]
    fn start_and_stop_sound_only_when_done_outside_the_app() {
        let mut cues = EventCues::default();
        for source in [
            "discord", "web", "remote", "cli", "chatbot", "schedule", "restart", "external", "game",
        ] {
            assert_eq!(
                cues.cue_for(&ev("instance.started", json!({"source": source}))),
                Some(Cue::Started),
                "{source}"
            );
            assert_eq!(
                cues.cue_for(&ev("instance.stopped", json!({"source": source}))),
                Some(Cue::Stopped),
                "{source}"
            );
        }
        assert_eq!(cues.cue_for(&ev("instance.started", json!({"source": "gui"}))), None);
        assert_eq!(
            cues.cue_for(&ev("instance.stopped", json!({"source": "gui", "kind": "user_stop"}))),
            None
        );
    }

    #[test]
    fn crash_sound_only_for_unexpected_exits() {
        let mut cues = EventCues::default();
        let exit = |kind: &str| ev("instance.exit", json!({"kind": kind}));
        assert_eq!(cues.cue_for(&exit("crash")), Some(Cue::Crashed));
        assert_eq!(cues.cue_for(&exit("unknown_exit")), Some(Cue::Crashed));
        assert_eq!(cues.cue_for(&exit("start_failed")), None);
        assert_eq!(cues.cue_for(&exit("normal")), None);
    }

    #[test]
    fn install_result_sounds_once_when_it_finishes() {
        let mut cues = EventCues::default();
        let progress = |name: &str, done: bool, error: Value| {
            ev(
                "provision.progress",
                json!({"instance": name, "done": done, "error": error}),
            )
        };
        assert_eq!(cues.cue_for(&progress("Survival", false, Value::Null)), None);
        assert_eq!(
            cues.cue_for(&progress("Survival", true, Value::Null)),
            Some(Cue::InstallSuccess)
        );
        assert_eq!(
            cues.cue_for(&progress("Survival", true, Value::Null)),
            None,
            "same end state again"
        );
        assert_eq!(
            cues.cue_for(&progress("Creative", true, json!("download failed"))),
            Some(Cue::InstallFailed)
        );
        // 다음 설치는 다시 알린다
        assert_eq!(cues.cue_for(&progress("Survival", false, Value::Null)), None);
        assert_eq!(
            cues.cue_for(&progress("Survival", true, json!(""))),
            Some(Cue::InstallSuccess)
        );
    }

    #[test]
    fn settings_turn_sounds_off_all_at_once_or_by_kind() {
        assert!(enabled(&json!({}), Cue::Crashed), "on until set");
        let only_crash_off = json!({"sounds": {"enabled": true, "crash": false}});
        assert!(!enabled(&only_crash_off, Cue::Crashed));
        assert!(enabled(&only_crash_off, Cue::Started));
        let install_off = json!({"sounds": {"install": false}});
        assert!(!enabled(&install_off, Cue::InstallSuccess));
        assert!(!enabled(&install_off, Cue::InstallFailed));
        let all_off = json!({"sounds": {"enabled": false, "update": true}});
        assert!(!enabled(&all_off, Cue::UpdateAvailable));
    }

    #[test]
    fn every_sound_is_a_pcm_wav() {
        for cue in [
            Cue::InstallSuccess,
            Cue::InstallFailed,
            Cue::Started,
            Cue::Stopped,
            Cue::Crashed,
            Cue::UpdateAvailable,
        ] {
            let b = cue.bytes();
            assert!(b.starts_with(b"RIFF") && &b[8..12] == b"WAVE", "{}", cue.file_name());
            assert_eq!(
                u16::from_le_bytes([b[20], b[21]]),
                1,
                "{} is plain PCM",
                cue.file_name()
            );
        }
    }
}
