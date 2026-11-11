#!/usr/bin/env bash
# apt 패키지를 깐다. 러너 기본 미러(azure.archive·ports.ubuntu.com)가 막히면 다른 미러로 바꿔 다시 한다.
# 2026-11-11 GitHub 러너에서 Canonical 미러가 모두 응답하지 않아 발행이 세 번 멈췄다.
# 사용: apt-install.sh <패키지>...
set -uo pipefail
APT=(-o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)

try_install() {
  sudo timeout 300 apt-get "${APT[@]}" update -q -o APT::Update::Error-Mode=any \
    && sudo timeout 900 apt-get "${APT[@]}" install -y -q "$@"
}

if try_install "$@"; then exit 0; fi

case "$(dpkg --print-architecture)" in
  arm64) MIRRORS=(https://mirror.us.leaseweb.net/ubuntu-ports https://ftp.tsukuba.wide.ad.jp/Linux/ubuntu-ports) ;;
  *)     MIRRORS=(https://mirrors.edge.kernel.org/ubuntu https://mirror.us.leaseweb.net/ubuntu) ;;
esac
CODENAME="$(. /etc/os-release && echo "$VERSION_CODENAME")"
# 기본 ubuntu 소스를 치우고(마이크로소프트 등 다른 소스는 둔다) 대체 미러 하나만 쓴다
sudo mv /etc/apt/sources.list /etc/apt/sources.list.runner 2>/dev/null || true
sudo find /etc/apt/sources.list.d -name 'ubuntu*.sources' -exec mv {} {}.runner \; 2>/dev/null || true
for m in "${MIRRORS[@]}"; do
  echo "기본 미러 실패, $m 로 다시 한다"
  printf 'deb %s %s main restricted universe multiverse\ndeb %s %s-updates main restricted universe multiverse\ndeb %s %s-security main restricted universe multiverse\n' \
    "$m" "$CODENAME" "$m" "$CODENAME" "$m" "$CODENAME" | sudo tee /etc/apt/sources.list >/dev/null
  if try_install "$@"; then exit 0; fi
done
echo "모든 미러에서 apt 실패" >&2
exit 1
