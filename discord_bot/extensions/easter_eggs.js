/**
 * 🥚 사바쨩 Easter Eggs Extension
 * 
 * 단답형 이스터에그 반응을 처리하는 익스텐션.
 * prefix + 트리거 단어에 반응합니다.
 */

const i18n = require('../i18n');

// ── 단답 반응 테이블 ──
// 키: 트리거 단어(들), 값: 응답 문자열 또는 함수
const SIMPLE_EGGS = {
    '물':     '🫗',
    '섹스':   '🔞',
    '사랑해': '❤️',
};

// ── 확률 반응 테이블 ──
// { triggers: [...], responses: [{ weight, text }] }
const RANDOM_EGGS = [
    {
        triggers: ['할건해야제', 'ㅎㄱㅎㅇㅈ'],
        responses: [
            { weight: 0.9, text: '반드시 가야제 ㅋㅋ' },
            { weight: 0.1, text: '이건 에바제...' },
        ],
    },
    {
        triggers: ['갈래말래', 'ㄱㄹㅁㄹ'],
        responses: [
            { weight: 0.9, text: '반드시 가야제 ㅋㅋ' },
            { weight: 0.1, text: '안감 ㅈㅈㅇㅇ' },
        ],
    },
];

/**
 * 가중치 기반 무작위 선택
 */
function weightedRandom(responses) {
    const r = Math.random();
    let cumulative = 0;
    for (const resp of responses) {
        cumulative += resp.weight;
        if (r < cumulative) return resp.text;
    }
    return responses[responses.length - 1].text;
}

/**
 * 이스터에그 메시지 처리
 * @param {import('discord.js').Message} message
 * @param {string[]} args - prefix 이후 토큰 배열
 * @returns {boolean} 처리했으면 true
 */
async function handleEasterEgg(message, args) {
    if (args.length !== 1) return false;

    const word = args[0];

    // 1) 단답 반응
    if (SIMPLE_EGGS[word]) {
        await message.reply(SIMPLE_EGGS[word]);
        return true;
    }

    // 2) 확률 반응
    for (const egg of RANDOM_EGGS) {
        if (egg.triggers.includes(word)) {
            const reply = weightedRandom(egg.responses);
            await message.reply(reply);
            return true;
        }
    }

    return false;
}

module.exports = { handleEasterEgg };
