# Installer — Known Issues

인스톨러의 현재 알려진 구조적 문제점을 기술한다.

> **2026-03-05 업데이트**: 아래 5건의 문제 모두 해결 완료.

---

## 1. ~~인스톨러 자체 버전 관리 부재~~ ✅ 해결

**해결**: `release-manifest.json`에 `installer` 컴포넌트를 추가하고, 설치 시작 전 매니페스트의 인스톨러 요구 버전과 현재 바이너리 버전(`CARGO_PKG_VERSION`)을 비교한다. 구버전 인스톨러로는 설치가 차단되며, 최신 인스톨러 다운로드를 안내하는 에러 메시지가 표시된다.

---

## 2. ~~모듈 목록 하드코딩~~ ✅ 해결

**해결**: `get_available_modules` 커맨드가 GitHub Contents API + 각 모듈의 `module.toml` RAW 파일을 동적으로 페치하여 모듈 메타데이터(이름, 설명, 아이콘, 의존 익스텐션)를 파싱한다. 네트워크 실패 시에만 하드코딩 폴백을 사용한다.

---

## 3. ~~모듈 의존 익스텐션의 자동 설치 부재~~ ✅ 해결

**해결**: 모듈 설치 후 `module.toml`의 `[install].requires_extensions` 를 참조하여 필요한 익스텐션을 자동으로 다운로드·설치한다. 익스텐션 매니페스트(`saba-chan-extensions/manifest.json`)에서 다운로드 URL을 조회한다. UI에도 각 모듈 카드에 의존 익스텐션 배지가 표시된다.

---

## 4. ~~다운로드 진행 상황 안내의 불투명성~~ ✅ 해결

**해결**: `reqwest`의 `bytes_stream()`을 사용한 스트리밍 다운로드로 변경. 바이트 단위 진행률(`received/total MB — n%`)이 실시간으로 UI에 표시된다. `InstallProgress` 구조체에 `download_file`, `download_received`, `download_total` 필드를 추가했다. 모듈 zipball 다운로드도 동일하게 적용.

---

## 5. ~~재설치 시 기존 컴포넌트 처리 미흡~~ ✅ 해결

**해결**: `setup_python_with_repair()` / `setup_node_with_repair()` 를 추가. 기존 런타임이 존재하더라도 `verify_python()` / `verify_node()` 검증에 실패하면 해당 디렉토리를 삭제하고 재다운로드한다. venv도 동일하게 손상 감지 시 재생성한다.
