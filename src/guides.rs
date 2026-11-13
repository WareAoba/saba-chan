//! 가이드 (사용자 2026-11-11): 기능 하나를 화면에서 짚어 가며 알려 주는 짧은 안내.
//!
//! 가이드 하나 = 폴더 하나: `guide.toml`(무리·아이콘·완료 조건·단계)과 `locales/<언어>.json`(제목·요약·단계별 문구).
//! 앱에 함께 싣는 것은 저장소 `guides/`를 빌드 때 넣고(`build.rs`), 따로 받은 것은 데이터 폴더 `guides/`에 둔다.
//! 같은 이름이면 받은 것이 이긴다. 완료·new!·진행 단계는 데몬이 판단하고 기억한다(config.json `guides` 칸).

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// 앱에 함께 싣는 가이드 하나: (이름, guide.toml, [(언어, 문구 json)])
type Builtin = (&'static str, &'static str, &'static [(&'static str, &'static str)]);
/// 폴더에서 읽은 가이드 하나: (이름, guide.toml, [(언어, 문구 json)])
type Folder = (String, String, Vec<(String, String)>);

const BUILTIN: &[Builtin] = include!(concat!(env!("OUT_DIR"), "/builtin_guides.rs"));

/// 카드·목록의 무리 (보이는 순서)
pub const GROUPS: &[&str] = &["start", "care", "discord", "more"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideFile {
    guide: Meta,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    group: String,
    icon: String,
    #[serde(default)]
    order: i32,
    /// 데몬이 실제 상태로 판단하는 완료 조건 (`Facts::met`). 없거나 이 판이 모르는 조건이면 끝까지 본 것으로 완료
    #[serde(default)]
    done: Option<String>,
    /// 이 가이드가 짚는 화면이 있는 가장 낮은 앱 판 (`>=0.1.15`)
    #[serde(default)]
    requires: Option<String>,
}

/// 단계 하나: 여는 화면(`go`, 화면 주소)·앱이 하는 일(`action`)·짚을 자리(`spot`, 화면의 `data-spot`)
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    title: String,
    summary: String,
    steps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Guide {
    pub id: String,
    pub group: String,
    pub icon: String,
    pub order: i32,
    pub done: Option<String>,
    pub steps: Vec<Step>,
    texts: HashMap<String, Text>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 폴더 하나를 읽는다. 틀린 가이드는 이유와 함께 뺀다 (다른 가이드는 그대로 보인다)
fn parse(id: &str, toml_text: &str, locales: &[(String, String)], core: &str) -> Result<Option<Guide>, String> {
    if !valid_id(id) {
        return Err("bad guide name".into());
    }
    let file: GuideFile = toml::from_str(toml_text).map_err(|e| e.to_string())?;
    if !GROUPS.contains(&file.guide.group.as_str()) {
        return Err(format!("unknown group '{}'", file.guide.group));
    }
    if file.steps.is_empty() {
        return Err("no steps".into());
    }
    if let Some(req) = &file.guide.requires {
        saba_chan_updater_lib::version::min_required(req.trim()).ok_or_else(|| format!("bad requires '{req}'"))?;
        // 이 앱에 아직 없는 화면을 짚는 가이드는 보이지 않는다 (앱을 올리면 보인다)
        if !saba_chan_updater_lib::version::satisfies(core, req.trim()) {
            return Ok(None);
        }
    }
    let mut texts = HashMap::new();
    for (lang, body) in locales {
        let text: Text = serde_json::from_str(body).map_err(|e| format!("locales/{lang}.json: {e}"))?;
        if text.steps.len() != file.steps.len() {
            return Err(format!("locales/{lang}.json has {} step texts for {} steps", text.steps.len(), file.steps.len()));
        }
        texts.insert(lang.clone(), text);
    }
    if !texts.contains_key("en") {
        return Err("no English text".into());
    }
    Ok(Some(Guide {
        id: id.to_string(),
        group: file.guide.group,
        icon: file.guide.icon,
        order: file.guide.order,
        done: file.guide.done,
        steps: file.steps,
        texts,
    }))
}

fn read_dir_guide(dir: &Path) -> Option<Folder> {
    let id = dir.file_name()?.to_str()?.to_string();
    let toml_text = std::fs::read_to_string(dir.join("guide.toml")).ok()?;
    let mut locales = Vec::new();
    for entry in std::fs::read_dir(dir.join("locales")).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "json") {
            if let (Some(lang), Ok(body)) = (path.file_stem().and_then(|s| s.to_str()), std::fs::read_to_string(&path)) {
                locales.push((lang.to_string(), body));
            }
        }
    }
    Some((id, toml_text, locales))
}

/// 받은 가이드를 두는 곳 (`<데이터 폴더>/guides`)
pub fn installed_dir() -> PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join("guides")
}

/// 앱에 실은 가이드 + 받은 가이드 (같은 이름이면 받은 것). 무리·순서대로
pub fn load(installed: Option<&Path>) -> Vec<Guide> {
    let core = env!("CARGO_PKG_VERSION");
    let mut by_id: BTreeMap<String, Guide> = BTreeMap::new();
    for (id, toml_text, locales) in BUILTIN {
        let locales: Vec<_> = locales.iter().map(|(l, b)| (l.to_string(), b.to_string())).collect();
        match parse(id, toml_text, &locales, core) {
            Ok(Some(g)) => {
                by_id.insert(g.id.clone(), g);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("Built-in guide '{}' is broken: {}", id, e),
        }
    }
    if let Some(dir) = installed {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some((id, toml_text, locales)) = read_dir_guide(&entry.path()) else { continue };
            match parse(&id, &toml_text, &locales, core) {
                Ok(Some(g)) => {
                    by_id.insert(g.id.clone(), g);
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Skipping guide '{}': {}", id, e),
            }
        }
    }
    let mut guides: Vec<Guide> = by_id.into_values().collect();
    guides.sort_by_key(|g| (GROUPS.iter().position(|x| *x == g.group), g.order));
    guides
}

/// 서버(릴레이 `GET /v1/guides`)가 주는 가이드 하나: 폴더 하나를 글로 옮긴 것
#[derive(Debug, Clone, Deserialize)]
pub struct Served {
    pub id: String,
    pub toml: String,
    #[serde(default)]
    pub locales: BTreeMap<String, String>,
}

/// 한 번에 받는 가이드 수 상한
const MAX_SERVED: usize = 200;

fn valid_lang(lang: &str) -> bool {
    (2..=10).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 받은 가이드로 `dir`(받은 가이드 폴더)을 서버와 같게 맞춘다: 맞는 가이드만 쓰고(잘못된 판이면 있던 사본을 둔다. 이 앱 판에 아직 없는 화면을 짚는 것도
/// 앱을 올리면 보이게 둔다), 서버가 더는 주지 않는 것은 지운다. 바뀐 수(쓰거나 지운 가이드)를 돌려준다
pub fn install(dir: &Path, served: &[Served]) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let core = env!("CARGO_PKG_VERSION");
    let mut keep = std::collections::HashSet::new();
    let mut written = 0;
    for g in served.iter().take(MAX_SERVED) {
        // 서버가 아직 주는 가이드는 이번 판이 잘못됐어도 있던 사본을 지우지 않는다 (맞는 판만 덮어쓴다)
        keep.insert(g.id.clone());
        if !g.locales.keys().all(|l| valid_lang(l)) {
            tracing::warn!("Skipping served guide '{}': bad language name", g.id);
            continue;
        }
        let locales: Vec<(String, String)> = g.locales.iter().map(|(l, b)| (l.clone(), b.clone())).collect();
        if let Err(e) = parse(&g.id, &g.toml, &locales, core) {
            tracing::warn!("Skipping served guide '{}': {}", g.id, e);
            continue;
        }
        let target = dir.join(&g.id);
        let same = read_dir_guide(&target).is_some_and(|(_, toml_text, mut have)| {
            have.sort();
            toml_text == g.toml && have == locales
        });
        if same {
            continue;
        }
        // 다 쓴 뒤 한 번에 바꾼다 (읽는 쪽이 반쯤 쓴 가이드를 보지 않게)
        let tmp = dir.join(format!(".{}.tmp", g.id));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("locales"))?;
        std::fs::write(tmp.join("guide.toml"), &g.toml)?;
        for (lang, body) in &locales {
            std::fs::write(tmp.join("locales").join(format!("{lang}.json")), body)?;
        }
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&tmp, &target)?;
        written += 1;
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() && !keep.contains(&name) {
            // 쓰다 남은 임시 폴더(`.이름.tmp`)는 지우기만 한다
            if std::fs::remove_dir_all(entry.path()).is_ok() && !name.starts_with('.') {
                written += 1;
            }
        }
    }
    Ok(written)
}

/// 완료 조건을 판단하는 지금 상태
#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub paired: bool,
    pub modules: usize,
    pub servers: usize,
    pub started: bool,
    pub chatbot_key: bool,
    pub backup_any: bool,
    pub schedule_any: bool,
    pub game_chat_on: bool,
    /// 처음 시작하기 체크리스트 (`GET /api/onboarding`과 같은 값)
    pub onboarding: Value,
}

impl Facts {
    /// 조건이 맞으면 Some(true/false), 이 판이 모르는 조건이면 None (끝까지 본 것으로 판단)
    fn met(&self, condition: &str) -> Option<bool> {
        Some(match condition {
            "onboarding" => self.onboarding.get("next").is_some_and(Value::is_null),
            "discord.linked" => self.paired,
            "module.any" => self.modules > 0,
            "server.any" => self.servers > 0,
            "server.started" => self.started,
            "chatbot.key" => self.chatbot_key,
            "backup.any" => self.backup_any,
            "schedule.any" => self.schedule_any,
            "game_chat.on" => self.game_chat_on,
            _ => return None,
        })
    }
}

/// config.json `guides` 칸
pub const SECTION: &str = "guides";

#[derive(Debug, Default, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct State {
    /// 처음 본 때 이미 있던 가이드와 그 뒤 연 가이드. 여기 없는 가이드가 new!다. 칸이 없으면(None) 지금 것을 모두 담는다
    #[serde(default)]
    pub known: Option<Vec<String>>,
    /// 끝까지 본 가이드
    #[serde(default)]
    pub finished: Vec<String>,
    /// 완료 조건이 있는 가이드 중 어느 PC에서든 실제로 해낸 것 (내 PC 모두가 함께 본다. 끝까지 보기만 한 것은 아니다)
    #[serde(default)]
    pub achieved: Vec<String>,
    /// 보던 단계 (0부터)
    #[serde(default)]
    pub step: BTreeMap<String, usize>,
}

impl State {
    pub fn from_section(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

fn union(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    for x in b {
        if !out.contains(x) {
            out.push(x.clone());
        }
    }
    out
}

/// 이 PC의 진행도에 내 PC 모두가 함께 보는 진행도(릴레이)를 합친다: 아는·끝낸 가이드는 합치고, 보던 단계는 함께 보는 값을 따른다
pub fn merge(local: &State, shared: &State) -> State {
    let known = match (&local.known, &shared.known) {
        (None, None) => None,
        (a, b) => Some(union(a.as_deref().unwrap_or_default(), b.as_deref().unwrap_or_default())),
    };
    let mut step = local.step.clone();
    step.extend(shared.step.iter().map(|(k, v)| (k.clone(), *v)));
    State { known, finished: union(&local.finished, &shared.finished), achieved: union(&local.achieved, &shared.achieved), step }
}

/// `view`가 남기라고 주는 값: 처음 본 때 아는 가이드(`known`), 이 PC의 실제 상태로 해낸 가이드(`achieved`, 다른 PC도 끝난 것으로 본다)
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Remember {
    pub known: Option<Vec<String>>,
    pub achieved: Vec<String>,
}

impl Remember {
    pub fn is_empty(&self) -> bool {
        self.known.is_none() && self.achieved.is_empty()
    }

    /// 남길 칸 (`config.json guides`에 합칠 값)
    pub fn patch(&self, state: &State) -> Value {
        let mut out = serde_json::Map::new();
        if let Some(known) = &self.known {
            out.insert("known".into(), json!(known));
        }
        if !self.achieved.is_empty() {
            out.insert("achieved".into(), json!(union(&state.achieved, &self.achieved)));
        }
        Value::Object(out)
    }
}

/// 언어 → 없으면 영어
fn text_for<'a>(g: &'a Guide, lang: &str) -> &'a Text {
    g.texts.get(lang).or_else(|| g.texts.get("en")).expect("parse requires English")
}

/// `GET /api/guides`의 목록. 처음 부르면(`known`이 없음) 지금 가이드를 모두 아는 것으로, 실제 상태로 끝난 가이드는 끝낸 것으로 남길 값을 함께 준다.
/// 완료 조건이 있으면 이 PC에서 맞거나 어느 PC에서든 해냈으면(`achieved`) 끝이고, 이 판이 모르는 조건이나 조건 없는 가이드는 끝까지 봤으면 끝이다
pub fn view(guides: &[Guide], state: &State, facts: &Facts, lang: &str) -> (Value, Remember) {
    let first_look = state.known.is_none();
    let known: Vec<String> = state.known.clone().unwrap_or_else(|| guides.iter().map(|g| g.id.clone()).collect());
    let onboarding_steps: Vec<(String, bool)> = facts.onboarding["steps"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| !s["optional"].as_bool().unwrap_or(false))
                .map(|s| (s["id"].as_str().unwrap_or_default().to_string(), s["done"].as_bool().unwrap_or(false)))
                .collect()
        })
        .unwrap_or_default();
    let mut newly_done = Vec::new();
    let list: Vec<Value> = guides
        .iter()
        .map(|g| {
            let text = text_for(g, lang);
            let finished = state.finished.contains(&g.id);
            let achieved = state.achieved.contains(&g.id);
            let done = match g.done.as_deref().and_then(|c| facts.met(c)) {
                Some(true) => {
                    if !achieved {
                        newly_done.push(g.id.clone());
                    }
                    true
                }
                Some(false) => achieved,
                None => finished,
            };
            let mut item = json!({
                "id": g.id,
                "group": g.group,
                "icon": g.icon,
                "title": text.title,
                "summary": text.summary,
                "steps": g.steps.iter().zip(&text.steps).map(|(s, t)| {
                    let mut v = serde_json::to_value(s).unwrap_or_default();
                    v["text"] = json!(t);
                    v
                }).collect::<Vec<_>>(),
                "done": done,
                "new": !known.contains(&g.id),
                "step": state.step.get(&g.id).copied().unwrap_or(0).min(g.steps.len() - 1),
            });
            // 처음 시작하기: 체크리스트의 다음 단계부터, 진행은 필수 단계 수로
            if g.done.as_deref() == Some("onboarding") && !onboarding_steps.is_empty() {
                let next = facts.onboarding["next"].as_str();
                if let Some(i) = next.and_then(|n| g.steps.iter().position(|s| s.id.as_deref() == Some(n))) {
                    item["step"] = json!(i);
                }
                item["progress"] = json!({
                    "done": onboarding_steps.iter().filter(|(_, d)| *d).count(),
                    "total": onboarding_steps.len(),
                });
            }
            item
        })
        .collect();
    (json!({ "guides": list, "groups": GROUPS }), Remember { known: first_look.then_some(known), achieved: newly_done })
}

/// `POST /api/guides/:id`: 열었음(new! 끄기)·보던 단계·끝까지 봄을 남길 칸
pub fn patch(state: &State, id: &str, opened: bool, step: Option<usize>, finished: bool) -> Value {
    let mut out = serde_json::Map::new();
    if opened {
        let mut known = state.known.clone().unwrap_or_default();
        if !known.iter().any(|k| k == id) {
            known.push(id.to_string());
            out.insert("known".into(), json!(known));
        }
    }
    if let Some(step) = step {
        let mut steps = state.step.clone();
        steps.insert(id.to_string(), step);
        out.insert("step".into(), json!(steps));
    }
    if finished && !state.finished.iter().any(|f| f == id) {
        let mut done = state.finished.clone();
        done.push(id.to_string());
        out.insert("finished".into(), json!(done));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
[guide]
group = "care"
icon = "save"
done = "backup.any"

[[steps]]
go = "server/:first/maintenance"
spot = "server.tab.maintenance"

[[steps]]
spot = "backup.create"
"#;

    fn en(steps: usize) -> (String, String) {
        ("en".into(), json!({ "title": "Backups", "summary": "s", "steps": vec!["a"; steps] }).to_string())
    }

    #[test]
    fn a_guide_is_one_folder_and_needs_english_with_a_text_per_step() {
        let g = parse("backup", TOML, &[en(2)], "0.1.15").unwrap().unwrap();
        assert_eq!((g.group.as_str(), g.steps.len()), ("care", 2));
        assert!(parse("backup", TOML, &[en(1)], "0.1.15").is_err(), "a step without text");
        assert!(parse("backup", TOML, &[], "0.1.15").is_err(), "no English");
        assert!(parse("Bad Name", TOML, &[en(2)], "0.1.15").is_err());
        assert!(parse("x", &TOML.replace("care", "games"), &[en(2)], "0.1.15").is_err(), "unknown group");
    }

    #[test]
    fn a_guide_for_a_newer_app_stays_hidden_until_the_app_catches_up() {
        let newer = TOML.replace("done = \"backup.any\"", "requires = \">=0.2.0\"");
        assert!(parse("x", &newer, &[en(2)], "0.1.15").unwrap().is_none());
        assert!(parse("x", &newer, &[en(2)], "0.2.0").unwrap().is_some());
    }

    #[test]
    fn every_built_in_guide_loads_in_every_language() {
        let core = env!("CARGO_PKG_VERSION");
        for (id, toml_text, locales) in BUILTIN {
            let locales: Vec<_> = locales.iter().map(|(l, b)| (l.to_string(), b.to_string())).collect();
            let g = parse(id, toml_text, &locales, core).unwrap_or_else(|e| panic!("{id}: {e}"));
            let g = g.unwrap_or_else(|| panic!("{id} requires a newer app than this one"));
            for lang in saba_chan_updater_lib::constants::SUPPORTED_LANGUAGES {
                assert!(g.texts.contains_key(*lang), "{id} has no {lang} text");
            }
            if let Some(cond) = &g.done {
                assert!(Facts::default().met(cond).is_some(), "{id}: unknown done condition '{cond}'");
            }
        }
    }

    fn guides() -> Vec<Guide> {
        vec![
            parse("backup", TOML, &[en(2)], "0.1.15").unwrap().unwrap(),
            parse("music", &TOML.replace("done = \"backup.any\"", ""), &[en(2)], "0.1.15").unwrap().unwrap(),
        ]
    }

    #[test]
    fn guides_present_at_first_look_are_not_new_but_later_ones_are_until_opened() {
        let (v, remember) = view(&guides(), &State::default(), &Facts::default(), "en");
        assert_eq!(v["guides"][0]["new"], false);
        let mut state = State { known: remember.known, ..Default::default() };
        assert_eq!(state.known.as_deref(), Some(&["backup".to_string(), "music".to_string()][..]));

        state.known = Some(vec!["backup".into()]);
        let (v, remember) = view(&guides(), &state, &Facts::default(), "en");
        assert_eq!((v["guides"][1]["new"].clone(), remember), (json!(true), Remember::default()));
        let p = patch(&state, "music", true, None, false);
        assert_eq!(p["known"], json!(["backup", "music"]));
    }

    #[test]
    fn done_comes_from_real_state_and_unknown_conditions_from_finishing_the_steps() {
        let state = State { known: Some(vec![]), finished: vec!["music".into(), "backup".into()], ..Default::default() };
        let (v, _) = view(&guides(), &state, &Facts::default(), "en");
        assert_eq!(v["guides"][0]["done"], false, "finishing the backup guide is not a backup");
        assert_eq!(v["guides"][1]["done"], true);
        let facts = Facts { backup_any: true, ..Default::default() };
        let (v, _) = view(&guides(), &State::default(), &facts, "en");
        assert_eq!(v["guides"][0]["done"], true);
    }

    /// 가이드 진행도는 내 PC 모두가 함께 본다 (사용자 11-12): 한 PC에서 실제로 해낸 가이드는 끝낸 것으로 남겨 다른 PC에도 끝난 것으로 보인다
    #[test]
    fn a_guide_done_on_one_pc_stays_done_on_the_others() {
        let facts = Facts { backup_any: true, ..Default::default() };
        let state = State { known: Some(vec![]), ..Default::default() };
        let (_, remember) = view(&guides(), &state, &facts, "en");
        assert_eq!(remember, Remember { known: None, achieved: vec!["backup".into()] });
        // 다른 PC (백업 없음)
        let shared = State { known: Some(vec![]), achieved: vec!["backup".into()], ..Default::default() };
        let (v, remember) = view(&guides(), &shared, &Facts::default(), "en");
        assert_eq!((v["guides"][0]["done"].clone(), remember), (json!(true), Remember::default()));
    }

    #[test]
    fn shared_progress_merges_with_this_pcs_and_the_shared_step_wins() {
        let local = State {
            known: None,
            finished: vec!["backup".into()],
            achieved: vec![],
            step: BTreeMap::from([("music".into(), 1), ("backup".into(), 1)]),
        };
        let shared = State {
            known: Some(vec!["music".into()]),
            finished: vec!["music".into()],
            achieved: vec!["backup".into()],
            step: BTreeMap::from([("music".into(), 0)]),
        };
        assert_eq!(
            merge(&local, &shared),
            State {
                known: Some(vec!["music".into()]),
                finished: vec!["backup".into(), "music".into()],
                achieved: vec!["backup".into()],
                step: BTreeMap::from([("music".into(), 0), ("backup".into(), 1)]),
            }
        );
        assert_eq!(merge(&State::default(), &State::default()).known, None, "nobody has looked yet");
    }

    #[test]
    fn the_first_start_guide_resumes_at_the_next_checklist_step() {
        let toml_text = "[guide]\ngroup = \"start\"\nicon = \"rocket\"\ndone = \"onboarding\"\n\n[[steps]]\nid = \"discord\"\n\n[[steps]]\nid = \"module\"\n\n[[steps]]\nid = \"server\"\n";
        let g = parse("first-start", toml_text, &[en(3)], "0.1.15").unwrap().unwrap();
        let facts = Facts {
            onboarding: json!({ "steps": [
                { "id": "discord", "done": true, "optional": false },
                { "id": "module", "done": false, "optional": false },
                { "id": "server", "done": false, "optional": false },
                { "id": "chatbot", "done": false, "optional": true },
            ], "next": "module" }),
            ..Default::default()
        };
        let (v, _) = view(&[g], &State::default(), &facts, "ko");
        assert_eq!(v["guides"][0]["step"], 1);
        assert_eq!(v["guides"][0]["progress"], json!({ "done": 1, "total": 3 }));
        assert_eq!(v["guides"][0]["title"], "Backups", "falls back to English");
    }

    fn served(id: &str, toml: &str, steps: usize) -> Served {
        Served { id: id.into(), toml: toml.into(), locales: BTreeMap::from([en(steps)]) }
    }

    #[test]
    fn served_guides_mirror_into_the_folder_and_bad_ones_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad_lang = served("lang", TOML, 2);
        bad_lang.locales.insert("../x".into(), "{}".into());
        let first = [served("new-thing", TOML, 2), served("../escape", TOML, 2), served("broken", TOML, 1), bad_lang];
        assert_eq!(install(dir.path(), &first).unwrap(), 1);
        let ids: Vec<String> = load(Some(dir.path())).into_iter().map(|g| g.id).collect();
        assert!(ids.contains(&"new-thing".to_string()));
        assert!(!dir.path().join("broken").exists() && !dir.path().join("lang").exists());
        // 같은 것을 다시 받으면 쓰지 않고, 서버가 빼면 지운다 (지운 것도 바뀐 수에 센다: 열린 목록이 다시 읽게)
        assert_eq!(install(dir.path(), &first).unwrap(), 0);
        assert_eq!(install(dir.path(), &[]).unwrap(), 1);
        assert!(!dir.path().join("new-thing").exists());
    }

    #[test]
    fn a_broken_update_of_a_served_guide_keeps_the_copy_already_there() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install(dir.path(), &[served("new-thing", TOML, 2)]).unwrap(), 1);
        // 서버가 같은 가이드를 잘못된 문구로 보내도 있던 것을 지우지 않는다
        assert_eq!(install(dir.path(), &[served("new-thing", TOML, 1)]).unwrap(), 0);
        let ids: Vec<String> = load(Some(dir.path())).into_iter().map(|g| g.id).collect();
        assert!(ids.contains(&"new-thing".to_string()));
    }
}
