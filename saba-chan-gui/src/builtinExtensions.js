/**
 * builtinExtensions — 내장 익스텐션 정적 레지스트리
 *
 * 내장 익스텐션의 GUI 컴포넌트를 메인 번들에 직접 포함시킨다.
 * 별도 UMD 빌드 없이 vite가 소스를 그대로 번들링.
 *
 * 형식: { [extensionId]: { registerSlots: () => slotMap } }
 */
import { registerSlots as dockerRegisterSlots } from '@ext/docker/gui/src/index';

const builtinExtensions = {
  docker: { registerSlots: dockerRegisterSlots },
};

export default builtinExtensions;
