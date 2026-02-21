/**
 * DockerStatsRow — 서버 카드 확장 시 Docker 리소스 통계
 * 슬롯: ServerCard.expandedStats
 */
import React from 'react';
import { MemoryGauge } from './MemoryGauge';

export default function DockerStatsRow({ server }) {
  const isDocker = server?.extension_data?.docker_enabled;
  if (!isDocker) return null;
  if (server.status !== 'running') return null;

  const dockerStats = server.extension_status?.docker;
  if (dockerStats?.memory_percent == null) return null;

  return (
    <div className="docker-stats-row" style={{
      display: 'flex',
      alignItems: 'center',
      gap: '12px',
      padding: '8px 0',
      borderBottom: '1px solid var(--border-subtle, rgba(255,255,255,0.06))',
      marginBottom: '6px',
    }}>
      <MemoryGauge
        percent={dockerStats.memory_percent}
        usage={dockerStats.memory_usage}
        size={130}
      />
      {dockerStats.cpu_percent != null && (
        <div style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: '2px',
        }}>
          <span style={{
            fontSize: '10px',
            color: 'var(--text-tertiary, #888)',
            textTransform: 'uppercase',
            fontWeight: 600,
            letterSpacing: '0.5px',
          }}>CPU</span>
          <span style={{
            fontSize: '16px',
            fontWeight: 700,
            color: 'var(--text-primary, #fff)',
          }}>{dockerStats.cpu_percent.toFixed(1)}%</span>
        </div>
      )}
    </div>
  );
}
