const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

test('apply waits for the progress listener before starting', async () => {
    const source = fs.readFileSync(path.join(__dirname, '../src/updater.js'), 'utf8');
    const start = source.search(/(?:async )?function enterApplyMode\(/);
    const calls = [];
    let ready;
    const context = {
        window: { __TAURI__: { event: { listen: () => new Promise(resolve => {
            ready = () => { calls.push('listening'); resolve(() => {}); };
        }) } } },
        invoke: async command => calls.push(command),
        showRing() {}, showProgress() {}, tr: key => key,
    };
    vm.createContext(context);
    vm.runInContext(source.slice(start), context);
    const pending = context.enterApplyMode({});
    assert.deepEqual(calls, []);
    ready();
    await pending;
    assert.deepEqual(calls, ['listening', 'start_apply']);
});
