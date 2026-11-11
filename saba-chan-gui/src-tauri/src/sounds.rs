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
    /// 앱 밖에서 켠 서버가 켜지지 못했거나 크래시 뒤 자동 재시작이 실패했다
    StartFailed,
    /// 이 GUI가 띄운 데몬(사바쨩 본체)이 정상 종료가 아니게 끝났다
    DaemonDown,
    UpdateAvailable,
    /// 정상 기동음: 데몬과 릴레이가 붙어 오프닝이 시작될 때 (화면이 `sound_play`로 부른다)
    Startup,
    /// 처음 사용자 가이드를 열 때 (화면이 `sound_play`로 부른다)
    GuideOpen,
}

impl Cue {
    fn file_name(self) -> &'static str {
        match self {
            Cue::InstallSuccess => "instance-install-success.wav",
            Cue::InstallFailed => "instance-install-failed.wav",
            Cue::Started => "instance-started.wav",
            Cue::Stopped => "instance-stopped.wav",
            Cue::Crashed => "instance-crashed.wav",
            Cue::StartFailed => "instance-start-failed.wav",
            Cue::DaemonDown => "daemon-down.wav",
            Cue::UpdateAvailable => "update-available.wav",
            Cue::Startup => "app-startup.wav",
            Cue::GuideOpen => "guide-open.wav",
        }
    }

    /// 이 소리를 켜고 끄는 `sounds` 설정 키
    fn setting_key(self) -> &'static str {
        match self {
            Cue::Started => "serverStart",
            Cue::Stopped => "serverStop",
            Cue::Crashed | Cue::StartFailed | Cue::DaemonDown => "crash",
            Cue::InstallSuccess | Cue::InstallFailed => "install",
            Cue::UpdateAvailable => "update",
            Cue::Startup => "startup",
            // 가이드는 사람이 연 순간이라 종류 스위치 없이 전체 스위치만 따른다
            Cue::GuideOpen => "enabled",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Cue::InstallSuccess => include_bytes!("../sounds/instance-install-success.wav"),
            Cue::InstallFailed => include_bytes!("../sounds/instance-install-failed.wav"),
            Cue::Started => include_bytes!("../sounds/instance-started.wav"),
            Cue::Stopped => include_bytes!("../sounds/instance-stopped.wav"),
            Cue::Crashed => include_bytes!("../sounds/instance-crashed.wav"),
            Cue::StartFailed => include_bytes!("../sounds/instance-start-failed.wav"),
            Cue::DaemonDown => include_bytes!("../sounds/daemon-down.wav"),
            Cue::UpdateAvailable => include_bytes!("../sounds/update-available.wav"),
            Cue::Startup => include_bytes!("../sounds/app-startup.wav"),
            Cue::GuideOpen => include_bytes!("../sounds/guide-open.wav"),
        }
    }
}

/// 화면이 직접 고를 수 있는 소리. 이벤트로 내는 소리(크래시 등)는 셸만 낸다
pub fn gui_cue(name: &str) -> Option<Cue> {
    match name {
        "startup" => Some(Cue::Startup),
        "guideOpen" => Some(Cue::GuideOpen),
        _ => None,
    }
}

/// 화면이 고른 때 소리를 낸다 (정상 기동음·가이드 열림). 모르는 이름은 무시한다
#[tauri::command]
pub fn sound_play(app: AppHandle, name: String) {
    match gui_cue(&name) {
        Some(cue) => play(&app, cue),
        None => tracing::warn!("[Sound] Unknown sound from the window: {}", name),
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
            // 예기치 않은 종료는 크래시와 원인 모를 종료(작업 관리자 강제 종료 등)뿐이다 (01 §이벤트).
            // 시작 실패는 앱 밖에서 켠 것만: 시킨 쪽(`source`)이 없는 옛 데몬의 것은 앱 버튼일 수 있어 울리지 않는다
            "instance.exit" => match d["kind"].as_str() {
                Some("crash" | "unknown_exit") => Some(Cue::Crashed),
                Some("start_failed") => d["source"].as_str().is_some_and(|s| s != "gui").then_some(Cue::StartFailed),
                _ => None,
            },
            // 크래시 뒤 자동 재시작마저 실패해 서버가 꺼진 채다
            "instance.restart_failed" => Some(Cue::StartFailed),
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

/// 같은 일로 앱 효과음이 울리면 OS 알림은 소리 없이 띄운다 (두 소리가 겹치지 않게)
pub fn toast_silent(settings: &Value, cue: Option<Cue>) -> bool {
    cue.is_some_and(|cue| enabled(settings, cue))
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

/// macOS·리눅스는 앱 안 재생 스레드 하나가 실행 파일에 실은 바이트를 바로 튼다 (외부 재생 프로그램·임시 파일 없음).
/// Windows처럼 새 소리는 울리던 소리를 끊는다. 소리가 다 끝나면 출력 장치를 놓는다
#[cfg(not(windows))]
fn play_bytes(_app: &AppHandle, cue: Cue) {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Mutex, OnceLock};
    static PLAYER: OnceLock<Mutex<Option<Sender<Cue>>>> = OnceLock::new();
    let slot = PLAYER.get_or_init(|| Mutex::new(None));
    let mut tx = slot.lock().unwrap_or_else(|e| e.into_inner());
    if tx.as_ref().is_some_and(|t| t.send(cue).is_ok()) {
        return;
    }
    let (t, rx) = channel();
    let started = std::thread::Builder::new().name("sounds".into()).spawn(move || player_loop(rx));
    match started {
        Ok(_) if t.send(cue).is_ok() => *tx = Some(t),
        Ok(_) => tracing::warn!("[Sound] Sound thread stopped before {}", cue.file_name()),
        Err(e) => tracing::warn!("[Sound] Could not start the sound thread: {}", e),
    }
}

/// 받은 소리를 차례로 튼다. 장치를 열지 못하면 그 소리만 건너뛴다 (다음 소리 때 다시 연다)
#[cfg(not(windows))]
fn player_loop(rx: std::sync::mpsc::Receiver<Cue>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    while let Ok(first) = rx.recv() {
        let mut device = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("[Sound] No audio output for {}: {}", first.file_name(), e);
                continue;
            }
        };
        device.log_on_drop(false);
        let mut player = start(&device, first);
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(next) => {
                    if let Some(p) = player.take() {
                        p.stop();
                    }
                    player = start(&device, next);
                }
                Err(RecvTimeoutError::Timeout) if player.as_ref().is_none_or(|p| p.empty()) => break,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(not(windows))]
fn start(device: &rodio::MixerDeviceSink, cue: Cue) -> Option<rodio::Player> {
    match decode(cue) {
        Ok(source) => {
            let player = rodio::Player::connect_new(device.mixer());
            player.append(source);
            Some(player)
        }
        Err(e) => {
            tracing::warn!("[Sound] Could not decode {}: {}", cue.file_name(), e);
            None
        }
    }
}

#[cfg(not(windows))]
fn decode(cue: Cue) -> Result<rodio::Decoder<std::io::Cursor<&'static [u8]>>, rodio::decoder::DecoderError> {
    rodio::Decoder::new_wav(std::io::Cursor::new(cue.bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Cue; 10] = [
        Cue::InstallSuccess,
        Cue::InstallFailed,
        Cue::Started,
        Cue::Stopped,
        Cue::Crashed,
        Cue::StartFailed,
        Cue::DaemonDown,
        Cue::UpdateAvailable,
        Cue::Startup,
        Cue::GuideOpen,
    ];

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
        assert_eq!(cues.cue_for(&exit("normal")), None);
    }

    #[test]
    fn start_failure_sounds_unless_started_from_the_app() {
        let mut cues = EventCues::default();
        let failed = |source: Value| ev("instance.exit", json!({"kind": "start_failed", "source": source}));
        for source in ["discord", "schedule", "restart", "web", "remote", "cli"] {
            assert_eq!(cues.cue_for(&failed(json!(source))), Some(Cue::StartFailed), "{source}");
        }
        assert_eq!(cues.cue_for(&failed(json!("gui"))), None, "앱 버튼으로 켠 것은 보고 있다");
        assert_eq!(cues.cue_for(&failed(Value::Null)), None, "시킨 쪽을 싣지 않는 옛 데몬");
        assert_eq!(
            cues.cue_for(&ev("instance.restart_failed", json!({"noteCode": "restart_failed"}))),
            Some(Cue::StartFailed)
        );
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
        assert!(!enabled(&only_crash_off, Cue::StartFailed));
        assert!(!enabled(&only_crash_off, Cue::DaemonDown));
        assert!(enabled(&only_crash_off, Cue::Started));
        let install_off = json!({"sounds": {"install": false}});
        assert!(!enabled(&install_off, Cue::InstallSuccess));
        assert!(!enabled(&install_off, Cue::InstallFailed));
        let all_off = json!({"sounds": {"enabled": false, "update": true}});
        assert!(!enabled(&all_off, Cue::UpdateAvailable));
    }

    #[test]
    fn the_window_may_only_play_its_own_moments() {
        assert_eq!(gui_cue("startup"), Some(Cue::Startup));
        assert_eq!(gui_cue("guideOpen"), Some(Cue::GuideOpen));
        // 이벤트로 내는 소리(크래시 등)를 화면이 대신 울릴 수는 없다
        assert_eq!(gui_cue("crash"), None);
        assert_eq!(gui_cue("instance-crashed.wav"), None);
    }

    #[test]
    fn startup_chime_has_its_own_switch() {
        assert!(enabled(&json!({}), Cue::Startup));
        let off = json!({"sounds": {"startup": false}});
        assert!(!enabled(&off, Cue::Startup));
        assert!(enabled(&off, Cue::Crashed));
        assert!(!enabled(&json!({"sounds": {"enabled": false}}), Cue::GuideOpen));
        assert!(enabled(&json!({}), Cue::GuideOpen));
    }

    #[test]
    fn os_notification_is_silent_only_when_our_sound_plays_with_it() {
        let on = json!({});
        assert!(toast_silent(&on, Some(Cue::Crashed)), "앱 효과음과 겹치면 OS 알림음은 끈다");
        assert!(!toast_silent(&on, None), "효과음이 없는 알림은 OS 알림음을 낸다");
        let crash_off = json!({"sounds": {"crash": false}});
        assert!(!toast_silent(&crash_off, Some(Cue::Crashed)), "효과음을 껐으면 OS 알림음은 남긴다");
    }

    // 앱 안 재생(맥·리눅스)은 실은 wav(48kHz 24비트)를 직접 디코딩한다
    #[cfg(not(windows))]
    #[test]
    fn every_sound_decodes_for_in_app_playback() {
        use rodio::Source;
        for cue in ALL {
            let source = decode(cue).unwrap_or_else(|e| panic!("{}: {e}", cue.file_name()));
            assert!(source.total_duration().is_some_and(|d| d.as_millis() > 100), "{}", cue.file_name());
        }
    }

    #[test]
    fn every_sound_is_a_pcm_wav() {
        for cue in ALL {
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
