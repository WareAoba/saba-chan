import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddServerOptions, addServerExtensionData, DataBadges, dataStatusRows } from '../components/DataSlots';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({
        extensions: [
            {
                id: 'box',
                enabled: true,
                slots: {
                    'AddServer.options': [
                        { field: 'boxed', type: 'boolean', label: 'Use box', default: true },
                        { field: 'image', type: 'string', label: 'Image' },
                    ],
                },
            },
            { id: 'off', enabled: false, slots: { 'AddServer.options': [{ field: 'x', type: 'string', label: 'X' }] } },
        ],
    }),
}));

describe('데이터 slot (데몬이 판정한 server.slots)', () => {
    it('ServerCard.badge 텍스트를 그리고, slots가 없으면 아무것도 그리지 않는다', () => {
        const server = { id: 'a', slots: { 'ServerCard.badge': [{ extension: 'box', text: '📦' }] } };
        render(<DataBadges server={server} />);
        expect(screen.getByText('📦')).toBeInTheDocument();
        const { container } = render(<DataBadges server={{ id: 'b' }} />);
        expect(container).toBeEmptyDOMElement();
    });

    it('ServerDetail.status를 (라벨, 값) 줄로 바꾼다', () => {
        const server = { slots: { 'ServerDetail.status': [{ extension: 'box', label: 'Box', value: 'up' }] } };
        expect(dataStatusRows(server)).toEqual([['Box', 'up']]);
        expect(dataStatusRows({})).toEqual([]);
    });

    it('켜진 익스텐션의 AddServer.options 칸을 그리고 바뀐 값을 field로 알린다', () => {
        const onChange = vi.fn();
        render(<AddServerOptions values={{}} onChange={onChange} />);
        const box = screen.getByRole('checkbox', { name: 'Use box' });
        expect(box).toBeChecked();
        expect(screen.queryByText('X')).toBeNull();
        fireEvent.click(box);
        expect(onChange).toHaveBeenCalledWith('boxed', false);
        fireEvent.change(screen.getByRole('textbox', { name: 'Image' }), { target: { value: 'alpine' } });
        expect(onChange).toHaveBeenCalledWith('image', 'alpine');
    });

    it('생성 요청의 extension_data는 손대지 않은 칸을 기본값으로 채우고 빈 칸은 뺀다', () => {
        const extensions = [
            {
                id: 'box',
                enabled: true,
                slots: {
                    'AddServer.options': [{ field: 'boxed', type: 'boolean', default: true }, { field: 'image' }],
                },
            },
        ];
        expect(addServerExtensionData(extensions, {})).toEqual({ boxed: true });
        expect(addServerExtensionData(extensions, { boxed: false, image: 'alpine' })).toEqual({
            boxed: false,
            image: 'alpine',
        });
    });
});
