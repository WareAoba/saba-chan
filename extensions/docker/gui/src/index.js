/**
 * Docker Extension — GUI Entry Point
 *
 * UMD 번들로 빌드되어 window.SabaExtDocker로 등록됨.
 * registerSlots()를 통해 호스트 앱의 슬롯에 컴포넌트를 등록.
 */
import DockerBadge from './DockerBadge';
import DockerMiniGauge from './DockerMiniGauge';
import DockerStatsRow from './DockerStatsRow';
import DockerProvision from './DockerProvision';
import DockerTab from './DockerTab';
import DockerToggle from './DockerToggle';
import { MemoryGauge } from './MemoryGauge';

/**
 * 슬롯 레지스트리 — 호스트 앱의 ExtensionContext가 호출
 * @returns {Object} slotId → Component[] 매핑
 */
export function registerSlots() {
  return {
    'ServerCard.badge': [DockerBadge],
    'ServerCard.headerGauge': [DockerMiniGauge],
    'ServerCard.expandedStats': [DockerStatsRow],
    'ServerCard.provision': [DockerProvision],
    'ServerSettings.tab': [DockerTab],
    'AddServer.options': [DockerToggle],
  };
}

// Named exports for direct usage
export {
  DockerBadge,
  DockerMiniGauge,
  DockerStatsRow,
  DockerProvision,
  DockerTab,
  DockerToggle,
  MemoryGauge,
};
