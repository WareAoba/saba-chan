# 기여 규칙

## 커밋 제목

```
<종류>(<범위>): <요약>
```

- 종류: `feat` 새 기능, `fix` 버그 수정, `perf` 성능, `refactor` 동작이 같은 정리, `docs` 문서, `test` `build` `ci` `chore`
- 범위(선택): 부품(`daemon` `gui` `cli` `updater` `runner` `module` `ext`)
- 호환이 깨지면 종류 뒤에 `!`: `feat(api)!: GET /api/module/:name이 raw toml을 주지 않는다`
- 요약은 사용자가 보는 변화를 쓴다. 본문에 이유와 방법을 적는다.

릴리즈 노트는 `release-notes/<버전>.json`(`{언어: 본문}`)에 사람이 쓴다. 발행 워크플로가 이 파일을 manifest와 GitHub Release에 싣는다(없으면 발행이 멈춘다).

## 버전

버전은 `versions.toml` 한 곳에서 올린다. `python scripts/versions.py sync`로 각 파일에 써 넣고, `check`로 어긋남을 확인한다. 실행 중인 데몬의 버전은 `GET /api/version`이 코어·모듈·익스텐션·인터페이스를 한 번에 보여 준다.

번호가 채널을 말한다. 베타는 `X.Y.Z-beta.N`, RC는 `X.Y.Z-rc.N`, 정식은 `X.Y.Z`다(`[release] channel`과 맞아야 `versions.py check`가 통과한다). 꼬리표 없는 번호로 베타를 내지 않는다. 베타 채널은 같은 번호의 RC·정식까지 이어 받는다(beta → rc → 정식 → 다음 번호의 베타).

모듈(`module.toml`)·익스텐션(`manifest.json`)은 꼬리표 없는 번호(`X.Y.Z`)만 쓴다. 꼬리표가 붙으면 발행(`versions.py manifest`·`addons`)이 멈춘다.
