import { useState, useRef } from 'react';
import { debugLog, debugWarn } from '../utils/helpers';

/**
 * Manages drag-and-drop reordering of server cards using Pointer Events.
 *
 * @param {Array} servers - Current server list
 * @param {Function} setServers - Server state setter
 * @returns {Object} Drag state and handler
 */
export function useDragReorder(servers, setServers) {
    const cardRefs = useRef({});
    const dragRef = useRef({ active: false, draggedName: null });
    const [draggedName, setDraggedName] = useState(null);
    const skipNextClick = useRef(false);

    const handleCardPointerDown = (e, index) => {
        if (e.button !== 0) return;
        if (e.target.closest('button') || e.target.closest('.action-icon')) return;

        const name = servers[index].name;
        const card = cardRefs.current[name];
        if (!card) return;

        const rect = card.getBoundingClientRect();

        // Snapshot all card slot positions at drag start
        const slotPositions = servers.map(s => {
            const el = cardRefs.current[s.name];
            if (!el) return null;
            const r = el.getBoundingClientRect();
            return { x: r.left, y: r.top, w: r.width, h: r.height };
        });

        dragRef.current = {
            active: false,
            draggedName: name,
            fromSlot: index,
            targetSlot: index,
            startX: e.clientX,
            startY: e.clientY,
            offsetX: e.clientX - rect.left,
            offsetY: e.clientY - rect.top,
            slotPositions,
            originalOrder: servers.map(s => s.name),
            nameToId: Object.fromEntries(servers.map(s => [s.name, s.id])),
        };

        const onMove = (me) => {
            const d = dragRef.current;
            if (!d.draggedName) return;

            const dx = me.clientX - d.startX;
            const dy = me.clientY - d.startY;

            // Activation threshold (6px)
            if (!d.active) {
                if (Math.abs(dx) < 6 && Math.abs(dy) < 6) return;
                d.active = true;
                setDraggedName(d.draggedName);
                const dragCard = cardRefs.current[d.draggedName];
                if (dragCard) {
                    dragCard.style.transition = 'box-shadow 0.2s ease, opacity 0.2s ease';
                }
            }

            // Move dragged card with cursor
            const dragCard = cardRefs.current[d.draggedName];
            if (dragCard) {
                dragCard.style.transform = `translate(${dx}px, ${dy}px)`;
            }

            // Find nearest slot
            let targetSlot = d.targetSlot;
            let minDist = Infinity;
            for (let i = 0; i < d.slotPositions.length; i++) {
                const slot = d.slotPositions[i];
                if (!slot) continue;
                const cx = slot.x + slot.w / 2;
                const cy = slot.y + slot.h / 2;
                const dist = Math.hypot(me.clientX - cx, me.clientY - cy);
                if (dist < minDist) {
                    minDist = dist;
                    targetSlot = i;
                }
            }

            if (targetSlot !== d.targetSlot) {
                d.targetSlot = targetSlot;

                // Compute visual reorder
                const order = [...d.originalOrder];
                const draggedIdx = order.indexOf(d.draggedName);
                const [item] = order.splice(draggedIdx, 1);
                order.splice(targetSlot, 0, item);

                // Animate other cards to target slot positions
                order.forEach((cardName, newSlotIdx) => {
                    if (cardName === d.draggedName) return;
                    const el = cardRefs.current[cardName];
                    if (!el) return;

                    const origSlotIdx = d.originalOrder.indexOf(cardName);
                    const origPos = d.slotPositions[origSlotIdx];
                    const targetPos = d.slotPositions[newSlotIdx];
                    if (!origPos || !targetPos) return;

                    const tx = targetPos.x - origPos.x;
                    const ty = targetPos.y - origPos.y;

                    if (Math.abs(tx) < 1 && Math.abs(ty) < 1) {
                        el.style.transform = '';
                    } else {
                        el.style.transform = `translate(${tx}px, ${ty}px)`;
                    }
                });
            }
        };

        const onUp = async () => {
            document.removeEventListener('pointermove', onMove);
            document.removeEventListener('pointerup', onUp);

            const d = dragRef.current;

            // Clean up all inline styles
            Object.values(cardRefs.current).forEach(el => {
                if (el) {
                    el.style.transform = '';
                    el.style.transition = '';
                }
            });

            const wasActive = d.active;
            const { targetSlot, fromSlot, originalOrder, nameToId } = d;

            dragRef.current = { active: false, draggedName: null };
            setDraggedName(null);

            // Prevent click after drag
            if (wasActive) {
                skipNextClick.current = true;
                requestAnimationFrame(() => { skipNextClick.current = false; });
            }

            if (!wasActive || targetSlot === fromSlot) return;

            // Compute and apply final order
            const order = [...originalOrder];
            const draggedIdx = order.indexOf(d.draggedName);
            const [item] = order.splice(draggedIdx, 1);
            order.splice(targetSlot, 0, item);

            setServers(prev => {
                const byName = {};
                prev.forEach(s => { byName[s.name] = s; });
                return order.map(n => byName[n]);
            });

            // Persist order to backend
            try {
                const orderedIds = order.map(n => nameToId[n]);
                await window.api.instanceReorder(orderedIds);
                debugLog('Server order saved:', orderedIds);
            } catch (err) {
                debugWarn('Failed to save server order:', err);
            }
        };

        document.addEventListener('pointermove', onMove);
        document.addEventListener('pointerup', onUp);
    };

    return { draggedName, cardRefs, skipNextClick, handleCardPointerDown };
}
