/**
 * ExtensionSlot — 익스텐션이 UI를 삽입하는 범용 슬롯 컴포넌트
 *
 * 사용법:
 *   <ExtensionSlot slotId="ServerCard.badge" server={server} />
 *
 * 등록된 컴포넌트가 없으면 null 렌더링 (빈 슬롯)
 */
import React from 'react';
import { useExtensions } from '../contexts/ExtensionContext';

export default function ExtensionSlot({ slotId, ...props }) {
  const { slots } = useExtensions();
  const components = slots[slotId] || [];

  if (components.length === 0) return null;

  return (
    <>
      {components.map((Comp, i) => (
        <Comp key={`${slotId}-${i}`} {...props} />
      ))}
    </>
  );
}
