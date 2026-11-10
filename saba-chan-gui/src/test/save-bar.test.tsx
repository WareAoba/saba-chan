import { act, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { SaveBar, SaveBarHost, usePendingChange } from '../components/ui/SaveBar';

// 바뀐 게 있을 때만 아래에서 올라와 저장·되돌리기를 묻는 막대 (사용자 11-11)
describe('SaveBar', () => {
    it('바뀐 게 없으면 그리지 않는다', () => {
        render(<SaveBar dirty={false} saving={false} onSave={vi.fn()} onRevert={vi.fn()} />);
        expect(document.querySelector('.saba-savebar')).toBeNull();
    });

    it('바뀐 게 있으면 안내 글과 되돌리기·저장을 보이고 각각 부른다', () => {
        const onSave = vi.fn();
        const onRevert = vi.fn();
        render(<SaveBar dirty saving={false} onSave={onSave} onRevert={onRevert} />);
        const bar = screen.getByRole('region', { name: 'You have unsaved changes' });
        expect(bar.classList).toContain('dirty');
        fireEvent.click(screen.getByRole('button', { name: 'Revert' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));
        expect(onRevert).toHaveBeenCalledTimes(1);
        expect(onSave).toHaveBeenCalledTimes(1);
    });

    it('저장 중이거나 막혔으면 저장을 누를 수 없다', () => {
        const { rerender } = render(<SaveBar dirty saving onSave={vi.fn()} onRevert={vi.fn()} />);
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
        rerender(<SaveBar dirty saving={false} blocked onSave={vi.fn()} onRevert={vi.fn()} />);
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    });

    it('위험한 저장은 안내 글과 단추 이름을 바꾸고 빨간 단추로 보인다', () => {
        render(
            <SaveBar
                dirty
                saving={false}
                danger
                message="Really?"
                saveLabel="Save anyway"
                onSave={vi.fn()}
                onRevert={vi.fn()}
            />,
        );
        expect(screen.getByRole('region', { name: 'Really?' })).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Save anyway' }).classList).toContain('btn-danger');
    });
});

function Field({ id, onSave }: { id: string; onSave: (v: string) => void }) {
    const [saved, setSaved] = useState('a');
    const [draft, setDraft] = useState('a');
    usePendingChange(id, {
        dirty: draft !== saved,
        blocked: draft === '',
        save: () => {
            onSave(draft);
            setSaved(draft);
        },
        revert: () => setDraft(saved),
    });
    return <input aria-label={id} value={draft} onChange={(e) => setDraft(e.target.value)} />;
}

describe('SaveBarHost', () => {
    it('여러 칸의 바뀐 것을 막대 하나로 모아 한꺼번에 저장한다', async () => {
        const onSave = vi.fn();
        render(
            <SaveBarHost>
                <Field id="port" onSave={onSave} />
                <Field id="url" onSave={onSave} />
            </SaveBarHost>,
        );
        expect(document.querySelector('.saba-savebar')).toBeNull();
        fireEvent.change(screen.getByLabelText('port'), { target: { value: 'b' } });
        fireEvent.change(screen.getByLabelText('url'), { target: { value: 'c' } });
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Save' })));
        expect(onSave.mock.calls).toEqual([['b'], ['c']]);
        expect(document.querySelector('.saba-savebar.dirty')).toBeNull();
    });

    it('되돌리기는 바뀐 칸 모두를 저장된 값으로 돌리고, 틀린 칸이 있으면 저장을 막는다', () => {
        render(
            <SaveBarHost>
                <Field id="port" onSave={vi.fn()} />
            </SaveBarHost>,
        );
        fireEvent.change(screen.getByLabelText('port'), { target: { value: '' } });
        expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
        fireEvent.click(screen.getByRole('button', { name: 'Revert' }));
        expect((screen.getByLabelText('port') as HTMLInputElement).value).toBe('a');
    });
});
