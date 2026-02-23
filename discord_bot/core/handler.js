/**
 * 🎮 핸들러 — 봇 자체 기능 (익스텐션) 관리
 * 
 * 봇이 IPC 없이 자체적으로 처리하는 기능들을 등록하고 디스패치합니다.
 * 각 익스텐션은 (message, args, botConfig) => boolean 형태의 핸들러를 제공합니다.
 */

const musicExtension = require('../extensions/music');
const easterEggsExtension = require('../extensions/easter_eggs');
const rpsExtension = require('../extensions/rps');

/**
 * 등록된 익스텐션 파이프라인.
 * 순서대로 시도하며, 하나라도 true를 반환하면 중단.
 */
const extensions = [
    {
        name: 'Music:Shortcut',
        handler: (msg, args, cfg) => musicExtension.handleMusicShortcut(msg, args, cfg),
    },
    {
        name: 'Music',
        handler: (msg, args, cfg) => musicExtension.handleMusicMessage(msg, args, cfg),
    },
    {
        name: 'EasterEgg',
        handler: (msg, args, _cfg) => easterEggsExtension.handleEasterEgg(msg, args),
    },
    {
        name: 'RPS',
        handler: (msg, args, _cfg) => rpsExtension.handleRPS(msg, args),
    },
];

/**
 * 모든 익스텐션을 순서대로 시도합니다.
 * @param {import('discord.js').Message} message
 * @param {string[]} args
 * @param {object} botConfig
 * @returns {boolean} 어떤 익스텐션이 처리했으면 true
 */
async function handle(message, args, botConfig) {
    for (const ext of extensions) {
        try {
            const handled = await ext.handler(message, args, botConfig);
            if (handled) return true;
        } catch (e) {
            console.error(`[${ext.name}] Extension error:`, e.message);
        }
    }
    return false;
}

module.exports = { handle };
