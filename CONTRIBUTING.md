# 기여 규칙

## 커밋 제목

```
<종류>(<범위>): <요약>
```

- 종류: `feat` 새 기능, `fix` 버그 수정, `perf` 성능, `refactor` 동작이 같은 정리, `docs` 문서, `test` `build` `ci` `chore` (릴리즈 노트에 안 나온다)
- 범위(선택): 로드맵 ID(`A1`, `C12`) 또는 부품(`daemon` `gui` `cli` `updater` `runner` `module` `ext`)
- 호환이 깨지면 종류 뒤에 `!`: `feat(api)!: GET /api/module/:name이 raw toml을 주지 않는다`
- 요약은 사용자가 보는 변화를 쓴다. 본문에 이유와 방법을 적는다.

릴리즈 노트는 이 제목으로 만든다(`python scripts/changelog.py`, 릴리즈 워크플로가 `CHANGELOG.md`를 릴리즈에 붙인다). 규칙을 따르지 않은 제목은 "기타"로 들어간다.

## 버전

버전은 `versions.toml` 한 곳에서 올린다. `python scripts/versions.py sync`로 각 파일에 써 넣고, `check`로 어긋남을 확인한다. 실행 중인 데몬의 버전은 `GET /api/version`이 코어·모듈·익스텐션·인터페이스를 한 번에 보여 준다.
