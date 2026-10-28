import { act, fireEvent, screen, waitFor } from '@testing-library/react';

/** 새 서버 만들기 마법사의 "다음" (확인을 기다리는 단계는 확인이 끝난 뒤 누른다) */
export const next = async () => {
    const button = screen.getByRole('button', { name: 'Next' });
    await waitFor(() => expect(button).not.toBeDisabled());
    await act(async () => fireEvent.click(button));
};

/** 이름 칸이 있는 단계까지 */
export const toNameStep = async () => {
    while (!document.getElementById('as-server-name')) await next();
};

/** 확인 단계(서버 추가 버튼)까지 */
export const toConfirmStep = async () => {
    while (!screen.queryByRole('button', { name: /Add Server/i })) await next();
};

/** 확인 단계까지 가서 서버 추가를 누른다 */
export const submitWizard = async () => {
    await toConfirmStep();
    const create = screen.getByRole('button', { name: /Add Server/i });
    await waitFor(() => expect(create).not.toBeDisabled());
    await act(async () => fireEvent.click(create));
};
