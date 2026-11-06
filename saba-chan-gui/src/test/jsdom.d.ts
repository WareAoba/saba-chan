// jsdom은 vitest 환경으로만 깔려 있고 타입 패키지가 없다. 시험에서 쓰는 만큼만 적는다 (installer-pages.test.ts)
declare module 'jsdom' {
    export class VirtualConsole {
        sendTo(console: Console, options?: { omitJSDOMErrors?: boolean }): VirtualConsole;
    }
    export class JSDOM {
        constructor(html: string, options?: Record<string, unknown>);
        readonly window: Window;
    }
}
