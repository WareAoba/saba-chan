import { describe, expect, it } from 'vitest';
import { markDaemonSwitch, openingGate, RELAY_WAIT_MS, takeDaemonSwitch } from '../utils/opening';

const base = {
    daemonOk: true,
    configLoaded: true,
    relayConfigured: true,
    relayChecked: true,
    relayConnected: true,
    waitedMs: 0,
};

describe('opening chime (정상 기동음)', () => {
    it('plays with the opening once the daemon and the relay are both up', () => {
        expect(openingGate(base)).toEqual({ hold: false, chime: true });
    });

    it('holds the opening until the first relay check answers, so sound and animation start together', () => {
        expect(openingGate({ ...base, relayChecked: false, relayConnected: false })).toEqual({ hold: true, chime: false });
        expect(openingGate({ ...base, configLoaded: false, relayChecked: false })).toEqual({ hold: true, chime: false });
    });

    it('stops waiting for a slow relay and opens silently', () => {
        expect(
            openingGate({ ...base, relayChecked: false, relayConnected: false, waitedMs: RELAY_WAIT_MS }),
        ).toEqual({ hold: false, chime: false });
    });

    it('opens silently when the relay answered but is not connected', () => {
        expect(openingGate({ ...base, relayConnected: false })).toEqual({ hold: false, chime: false });
    });

    it('plays when no relay is set up yet (nothing failed)', () => {
        expect(openingGate({ ...base, relayConfigured: false, relayChecked: false, relayConnected: false })).toEqual({
            hold: false,
            chime: true,
        });
    });

    it('never plays when the app opened without a daemon', () => {
        expect(openingGate({ ...base, daemonOk: false })).toEqual({ hold: false, chime: false });
    });
});

describe('PC 전환 (사용자 11-12: 전환할 때는 오프닝 없음)', () => {
    it('opens straight away after a switch: no wait for the relay and no chime', () => {
        expect(openingGate({ ...base, switched: true, relayChecked: false, relayConnected: false })).toEqual({
            hold: false,
            chime: false,
        });
        expect(openingGate({ ...base, switched: true })).toEqual({ hold: false, chime: false });
    });

    it('remembers a switch across the reload only once', () => {
        expect(takeDaemonSwitch()).toBe(false);
        markDaemonSwitch();
        expect(takeDaemonSwitch()).toBe(true);
        expect(takeDaemonSwitch()).toBe(false);
    });
});
