/**
 * DockerMiniGauge — 서버 카드 헤더의 미니 메모리 게이지
 * 슬롯: ServerCard.headerGauge
 */
import React from 'react';
import { MemoryGauge } from './MemoryGauge';

export default function DockerMiniGauge({ server }) {
  const isDocker = server?.extension_data?.docker?.enabled || server?.use_docker;
  if (!isDocker) return null;
  if (server.provisioning) return null;
  if (server.status !== 'running') return null;
  if (server.docker_memory_percent == null) return null;

  return (
    <MemoryGauge
      percent={server.docker_memory_percent}
      size={44}
      compact
      title={server.docker_memory_usage || `${Math.round(server.docker_memory_percent)}%`}
    />
  );
}
