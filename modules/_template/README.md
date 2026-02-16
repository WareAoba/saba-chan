# saba-chan Module Template

이 디렉토리는 새로운 게임 서버 모듈을 만들 때 참고할 수 있는 템플릿입니다.

## 새 모듈 만들기

1. 이 `_template` 디렉토리를 복사하여 `modules/your_game/`으로 이동
2. `module.toml`의 `[module]` 섹션을 게임에 맞게 수정
3. `lifecycle.py`에서 필수 함수를 구현
4. (선택) 모듈 아이콘을 `icon.png`로 추가

## 디렉토리 구조

```
modules/your_game/
  module.toml       ← 모듈 메타데이터 및 설정 스키마
  lifecycle.py      ← 서버 라이프사이클 관리 (Python)
  i18n.py           ← 다국어 지원 헬퍼 (선택)
  icon.png          ← 모듈 아이콘 (선택)
  locales/          ← 번역 파일 (선택)
```

## lifecycle.py 필수 함수

| 함수 | 설명 | 반환값 |
|---|---|---|
| `validate(config)` | 서버 실행 전 필수 조건 검증 | `{success, issues[]}` |
| `get_launch_command(config)` | 서버 실행 명령어 생성 | `{success, program, args[], working_dir}` |
| `status(config)` | 서버 상태 확인 | `{success, status, ...}` |

## lifecycle.py 선택 함수

| 함수 | 설명 | 반환값 |
|---|---|---|
| `start(config)` | 레거시 서버 시작 | `{success, pid?, message}` |
| `stop(config)` | 서버 정상 종료 | `{success, message}` |
| `command(config)` | 명령어 실행 | `{success, message, data?}` |
| `configure(config)` | 설정 파일 동기화 | `{success, updated_keys[]}` |
| `read_properties(config)` | 서버 설정 읽기 | `{success, properties{}}` |
| `accept_eula(config)` | EULA 동의 처리 | `{success, message}` |
| `diagnose_log(config)` | 에러 로그 진단 | `{success, issues[]}` |
| `list_versions(config)` | 설치 가능 버전 조회 | `{success, versions[]}` |
| `install_server(config)` | 서버 다운로드/설치 | `{success, install_path}` |
| `reset_server(config)` | 서버 초기화 | `{success, message}` |

## module.toml 필수 필드

```toml
[module]
name = "your_game"           # 영문 식별자 (필수)
version = "1.0.0"            # 모듈 버전 (필수)
description = "설명"         # 모듈 설명 (필수)
display_name = "Your Game"   # GUI 표시 이름 (필수)
entry = "lifecycle.py"       # 진입점 (필수)
```

## 통신 프로토콜

모든 lifecycle.py 함수는 JSON 기반으로 통신합니다:
- **입력**: `config` dict (stdin으로 전달)
- **출력**: JSON dict (stdout으로 출력)
- **로그**: stderr로 출력 (데몬이 캡처하여 콘솔에 표시)

```python
# 기본 패턴
def my_function(config):
    # config에서 필요한 값 추출
    working_dir = config.get("working_dir", ".")
    
    # 작업 수행
    result = do_something(working_dir)
    
    # 결과 반환
    return {"success": True, "message": "Done", "data": result}
```
