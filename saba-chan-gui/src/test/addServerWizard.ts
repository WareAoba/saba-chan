import { act, fireEvent, screen, waitFor } from '@testing-library/react';

/*
 * 새 서버 만들기 (한 장 요약, 사용자 11-11). 게임을 고르면 이름·버전·포트·위치·실행 방식이 한 장에 있다.
 * 옛 마법사 시험이 쓰던 도우미 이름은 남겨 둔다: 이제 단계가 없어 "다음"은 없다
 */

/** 요약 한 장까지 (게임을 고르면 바로 거기다) */
export const toNameStep = async () => {
    await waitFor(() => expect(document.getElementById('as-server-name')).toBeTruthy());
};
export const toConfirmStep = toNameStep;

/** 만들기 버튼 */
export const createButton = () => screen.getByRole('button', { name: /^Create$/ });

/** 요약에서 만들기를 누른다 (겹치는지 확인이 끝나 버튼이 풀린 뒤) */
export const submitWizard = async () => {
    await toNameStep();
    const create = createButton();
    await waitFor(() => expect(create).not.toBeDisabled());
    await act(async () => fireEvent.click(create));
};
