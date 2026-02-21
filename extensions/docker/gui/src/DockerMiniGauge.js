/**
 * DockerMiniGauge — 서버 카드 헤더의 미니 메모리 게이지
 * 슬롯: ServerCard.headerGauge
 */
import React from 'react';
import { MemoryGauge } from './MemoryGauge';

export default function DockerMiniGauge({ server }) {
  const isDocker = server?.extension_data?.docker_enabled;
  if (!isDocker) return null;
  if (server.provisioning) return null;
  if (server.status !== 'running') return null;

  const memPct = server.extension_status?.docker?.memory_percent;
  if (memPct == null) return null;

  return (
    <MemoryGauge
      percent={memPct}
      size={44}
      compact
      title={server.extension_status?.docker?.memory_usage || `${Math.round(memPct)}%`}
    />
  );
}
