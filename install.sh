#!/bin/sh
# 사바쨩 한 줄 설치: 리눅스 (화면 없는 서버와 데스크톱, x86_64·ARM64), 맥 (M칩)
#
#   curl -fsSL https://raw.githubusercontent.com/WareAoba/saba-chan/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --dir ~/saba --modules minecraft palworld
#
# 하는 일
#   1. 릴레이에서 이 플랫폼(linux-x64·linux-arm64·macos-arm64)의 최신 릴리즈를 찾아 CLI zip만 받고, manifest의 sha256·크기와 대조한다
#   2. 받은 CLI로 `saba-chan-cli install`을 돌린다. 나머지(데몬·업데이터·GUI·모듈)는 CLI가 받고,
#      서명된 manifest를 번들 공개키로 검증한 뒤 설치한다 (창 인스톨러와 같은 saba-chan-installer-core).
#      화면(DISPLAY·WAYLAND_DISPLAY)이 있으면 GUI까지 든 데스크톱 구성, 없으면 헤드리스 구성이다 (--gui·--no-gui로 정한다)
#   3. CLI를 ~/.local/bin/saba-chan-cli로 링크한다. 헤드리스면 systemd 사용자 서비스(saba-chan.service)로 데몬을 켜고,
#      데스크톱이면 앱 메뉴에 사바쨩을 넣고(CLI 설치가 한다) GUI를 띄운다 (GUI가 데몬을 켠다. Windows 데스크톱 설치와 같다)
#      맥은 ~/Applications/Saba-chan.app에 설치하고(CLI 설치가 앱 묶음을 만든다) 그 앱을 연다
#
# 인자는 `saba-chan-cli install`의 것이다 (--dir, --modules, --components-dir, --gui, --no-gui, --yes).
# 정식 채널만 설치한다 (CLI 설치도 정식 채널을 받는다). 채널은 설치 뒤 `saba-chan-cli update config channel=beta`로
# 필요: curl, sha256sum, python3(manifest JSON 읽기, zip 풀기. 클라우드 서버 이미지에는 unzip이 없는 일이 많다).
# 맥은 OS에 든 것만 쓴다: shasum, osascript(JSON), ditto(zip). 맥의 python3는 개발 도구를 깔라는 창을 띄울 수 있다
set -eu

RELAY=https://saba-chan.online
CHANNEL=stable

case "${LANG:-}${LC_ALL:-}" in
    *ko*) KO=1 ;;
    *) KO= ;;
esac
say() { if [ -n "$KO" ]; then printf '%s\n' "$1"; else printf '%s\n' "$2"; fi; }
die() { say "오류: $1" "Error: $2" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) PLATFORM=linux-x64 ;;
    Linux-aarch64 | Linux-arm64) PLATFORM=linux-arm64 ;;
    Darwin-arm64) PLATFORM=macos-arm64 ;;
    Darwin-*) die "맥은 M칩만 지원해요." "Only Apple silicon Macs are supported." ;;
    Linux-*) die "x86_64·ARM64 PC만 지원해요." "Only x86_64 and ARM64 are supported." ;;
    *) die "리눅스와 맥에서만 설치할 수 있어요." "This installer is for Linux and macOS only." ;;
esac
MAC=
[ "$PLATFORM" != macos-arm64 ] || MAC=1
if [ -n "$MAC" ]; then TOOLS="curl shasum osascript ditto"; else TOOLS="curl sha256sum python3"; fi
for tool in $TOOLS; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool 이(가) 필요해요." "$tool is required."
done

# JSON 파일 읽기: 리눅스는 python3, 맥은 osascript(JavaScript). `js 파일 인자... <<'JS'`로 run(a)의 몸을 넘긴다 (a[0]이 파일)
js() {
    { printf '%s\n' "ObjC.import('Foundation');" 'function run(a) {' \
        'var j = JSON.parse($.NSString.stringWithContentsOfFileEncodingError(a[0], $.NSUTF8StringEncoding, null).js);'
      cat; printf '%s\n' '}'; } >"$TMP/read.js"
    osascript -l JavaScript "$TMP/read.js" "$@"
}

# 기본 설치 폴더는 CLI와 같다. 끝나고 링크·서비스를 걸려고 여기서 정해 넘긴다
if [ -n "$MAC" ]; then
    DIR="$HOME/Applications/Saba-chan.app/Contents/MacOS"
else
    DIR="${XDG_DATA_HOME:-$HOME/.local/share}/saba-chan"
fi
prev=
for arg in "$@"; do
    [ "$prev" = --dir ] && DIR=$arg
    case "$arg" in --dir=*) DIR=${arg#--dir=} ;; esac
    prev=$arg
done
case " $* " in
    *" --dir "* | *" --dir="*) ;;
    *) set -- --dir "$DIR" "$@" ;;
esac
# CLI는 상대 경로를 지금 폴더 기준으로 푼다: 링크가 ~/.local/bin 기준으로 풀리지 않게 같은 절대 경로로 둔다
case "$DIR" in
    /*) ;;
    *) DIR="$PWD/$DIR" ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

say "최신 릴리즈를 찾는 중..." "Looking up the latest release..."
curl -fsS -H 'X-Saba-Client: 1' \
    "$RELAY/v1/updates/check?channel=$CHANNEL&platform=$PLATFORM&current=0.0.0&client_id=install-sh" >"$TMP/check.json" ||
    die "릴레이에 닿지 못했어요." "Could not reach the relay."
if [ -n "$MAC" ]; then
    MANIFEST_URL=$(js "$TMP/check.json" <<'JS'
return (j.latest || {}).manifestUrl || "";
JS
)
else
    MANIFEST_URL=$(python3 -I -c 'import json,sys;print((json.load(open(sys.argv[1])).get("latest") or {}).get("manifestUrl",""))' "$TMP/check.json")
fi
[ -n "$MANIFEST_URL" ] || die "이 PC용 릴리즈가 아직 없어요." "No release for this platform yet."
curl -fsS -H 'X-Saba-Client: 1' "$MANIFEST_URL" >"$TMP/manifest.json" || die "manifest를 받지 못했어요." "Could not download the manifest."

# CLI 항목: 맨 위 플랫폼이 이 플랫폼이면 components, 아니면 platforms.<플랫폼>.components
# 창 앱 유무를 맨 앞에 둔다: 크기가 없는 manifest에서도 read가 칸을 밀리지 않게. https가 아닌 주소는 받지 않는다
if [ -n "$MAC" ]; then
    js "$TMP/manifest.json" "$PLATFORM" >"$TMP/cli.txt" <<'JS'
var m = j.manifest;
var own = m.platform === a[1] ? m.components : ((m.platforms || {})[a[1]] || {}).components || {};
var cli = own.cli;
if (!cli || (cli.url || "").indexOf("https://") !== 0) return "";
return [own.gui ? "gui" : "nogui", cli.url, cli.sha256, cli.size_bytes == null ? "" : cli.size_bytes].join(" ");
JS
else
    python3 -I - "$TMP/manifest.json" "$PLATFORM" >"$TMP/cli.txt" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))["manifest"]
own = m["components"] if m.get("platform") == sys.argv[2] else m.get("platforms", {}).get(sys.argv[2], {}).get("components", {})
cli = own.get("cli")
if not cli or not cli.get("url", "").startswith("https://"):
    sys.exit(1)
print("gui" if own.get("gui") else "nogui", cli["url"], cli["sha256"], cli.get("size_bytes", ""))
PY
fi
[ -n "$(tr -d ' \n' <"$TMP/cli.txt")" ] || die "이 PC용 CLI가 릴리즈에 없어요." "The release has no CLI for this platform."
read -r HAS_GUI CLI_URL CLI_SHA CLI_SIZE <"$TMP/cli.txt"
# 이 릴리스에 이 플랫폼의 창 앱이 없으면(창 앱 이전 릴리스) 화면이 있어도 헤드리스로 깐다. 받은 CLI가 데스크톱 구성을 고르면 없는 gui로 멈춘다
if [ "$HAS_GUI" = nogui ]; then
    case " $* " in
        *" --gui "* | *" --no-gui "*) ;;
        *) set -- "$@" --no-gui ;;
    esac
fi

say "CLI를 받는 중..." "Downloading the CLI..."
curl -fsSL "$CLI_URL" -o "$TMP/cli.zip" || die "CLI를 받지 못했어요." "Could not download the CLI."
[ -z "$CLI_SIZE" ] || [ "$(wc -c <"$TMP/cli.zip" | tr -d ' ')" = "$CLI_SIZE" ] || die "받은 CLI 크기가 달라요." "The CLI download has the wrong size."
if [ -n "$MAC" ]; then
    echo "$CLI_SHA  $TMP/cli.zip" | shasum -a 256 -c -s - || die "받은 CLI가 릴리즈와 달라요." "The CLI download does not match the release."
    ditto -x -k "$TMP/cli.zip" "$TMP/cli"
else
    echo "$CLI_SHA  $TMP/cli.zip" | sha256sum -c --status - || die "받은 CLI가 릴리즈와 달라요." "The CLI download does not match the release."
    python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$TMP/cli.zip" "$TMP/cli"
fi
chmod +x "$TMP/cli/saba-chan-cli"

# 묻는 말(폴더·모듈·확인)은 터미널에서 받는다: `curl | sh`면 표준 입력은 이 스크립트다
# 제어 터미널이 없으면(ssh 원격 명령, CI) 권한이 있어도 열리지 않는다: 실제로 열어 본다
if (: </dev/tty) 2>/dev/null; then
    "$TMP/cli/saba-chan-cli" install "$@" </dev/tty
else
    "$TMP/cli/saba-chan-cli" install "$@" </dev/null
fi

# 비어 있지 않은 폴더를 고르면 CLI는 그 아래 Saba-chan 폴더에 설치한다
[ -x "$DIR/saba-chan-cli" ] || [ ! -x "$DIR/Saba-chan/saba-chan-cli" ] || DIR="$DIR/Saba-chan"
CLI="$DIR/saba-chan-cli"
[ -x "$CLI" ] || die "설치 폴더에 CLI가 없어요: $DIR" "No CLI in the install folder: $DIR"
mkdir -p "$HOME/.local/bin"
ln -sf "$CLI" "$HOME/.local/bin/saba-chan-cli"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) say "saba-chan-cli는 ~/.local/bin에 있어요. 이 폴더를 셸 설정의 PATH에 넣으면 이름만으로 쓸 수 있어요." \
        "saba-chan-cli is in ~/.local/bin. Add that folder to PATH in your shell profile to run it by name." ;;
esac

# 데스크톱 구성: GUI가 데몬을 띄운다. GUI가 쓰는 시스템 라이브러리(WebKitGTK)가 없으면 설치 명령만 알린다
# 일반 사용자의 PATH에는 ldconfig(/sbin)가 없을 수 있다. 확인할 길이 없으면 있다고 보고 GUI를 띄운다
has_webkit() {
    for l in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
        if command -v "$l" >/dev/null 2>&1; then
            "$l" -p 2>/dev/null | grep -q 'libwebkit2gtk-4\.1\.so\.0'
            return
        fi
    done
    return 0
}
BUNDLE=
case "$DIR" in *.app/Contents/MacOS) BUNDLE="$DIR/../.." ;; esac
if [ -n "$MAC" ] && [ -n "$BUNDLE" ] && [ -x "$DIR/saba-chan-gui" ]; then
    # 앱 묶음을 연다 (GUI가 데몬을 켠다). 응용 프로그램 폴더의 사바쨩으로도 열 수 있다
    say "응용 프로그램 폴더에서 사바쨩을 열 수 있어요. 지금 엽니다." "Saba-chan is in your Applications folder. Opening it now."
    open "$BUNDLE" || "$CLI" daemon start
elif [ -n "$MAC" ]; then
    # 화면 없는 맥이나 묶음 밖 설치: 로그인할 때 켜기(LaunchAgent)는 아직 없다. 데몬만 켠다
    "$CLI" daemon start
elif [ -x "$DIR/saba-chan-gui" ]; then
    if ! has_webkit; then
        say "창 앱에 WebKitGTK가 필요해요. 예: sudo apt install libwebkit2gtk-4.1-0 libayatana-appindicator3-1 (Fedora: sudo dnf install webkit2gtk4.1 libayatana-appindicator-gtk3)" \
            "The app needs WebKitGTK. For example: sudo apt install libwebkit2gtk-4.1-0 libayatana-appindicator3-1 (Fedora: sudo dnf install webkit2gtk4.1 libayatana-appindicator-gtk3)"
        # 앱을 열 수 있을 때까지 CLI는 쓸 수 있게 데몬은 켠다 (앱을 열면 이 데몬에 붙는다)
        "$CLI" daemon start
    else
        say "앱 메뉴에서 사바쨩을 열 수 있어요. 지금 엽니다." "Saba-chan is in your app menu. Opening it now."
        (cd "$DIR" && setsid "$DIR/saba-chan-gui" </dev/null >/dev/null 2>&1 &)
    fi
# 헤드리스 구성: 로그인할 때 데몬을 켜는 systemd 사용자 서비스를 등록하고, 지금도 그 서비스로 켠다.
# 등록은 데몬 API(`config autostart daemon`)라 잠깐 데몬을 띄웠다가 끈다
elif command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
    "$CLI" daemon start >/dev/null
    if "$CLI" config autostart daemon; then
        "$CLI" daemon stop >/dev/null
        systemctl --user start saba-chan.service
    fi
else
    say "systemd 사용자 세션이 없어 자동 실행은 걸지 않았어요. 데몬만 켭니다." \
        "No systemd user session; starting the daemon without autostart."
    "$CLI" daemon start
fi

# SteamCMD(팰월드·좀보이드 등)는 32비트 프로그램이다: 32비트 C 라이브러리가 없으면 설치가 "required file not found"로 실패한다.
# ARM에는 SteamCMD 게임이 없으니 알리지 않는다
[ "$PLATFORM" != linux-x64 ] || [ -e /lib/ld-linux.so.2 ] || [ -e /lib32/ld-linux.so.2 ] ||
    say "Steam 게임을 설치하려면 32비트 라이브러리가 필요해요. 예: sudo apt install lib32gcc-s1" \
        "Steam games need 32-bit libraries. For example: sudo apt install lib32gcc-s1"

case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) say "~/.local/bin을 PATH에 넣으면 어디서든 saba-chan-cli를 쓸 수 있어요." \
        "Add ~/.local/bin to PATH to run saba-chan-cli from anywhere." ;;
esac
say "설치를 마쳤어요. saba-chan-cli 로 시작해 보세요." "Done. Try saba-chan-cli."
