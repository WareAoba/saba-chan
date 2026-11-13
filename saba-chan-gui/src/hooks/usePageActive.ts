import { createContext, useContext } from 'react';

/**
 * 상단 탭이 지금 보이는가 (App `KeptPage`). 떠난 탭은 지우지 않고 숨겨 두므로,
 * "탭을 열 때마다" 할 일(새로 읽기·바로 가기 칸 열기)은 마운트 대신 이 값이 `true`가 될 때 한다.
 * 탭 밖(서버 상세의 작업 기록 등)에서는 늘 `true`다
 */
export const PageActiveContext = createContext(true);

export const usePageActive = () => useContext(PageActiveContext);
