/**
 * ✊✌️✋ 사바쨩 Rock-Paper-Scissors Extension
 * 
 * 가위바위보 게임을 Discord 버튼 UI로 제공합니다.
 * "prefix 가위바위보" 로 시작하면 ✊✌️✋ 버튼이 나타나고,
 * 클릭하면 결과를 표시합니다. 무승부 시 자동 재도전!
 */

const { ActionRowBuilder, ButtonBuilder, ButtonStyle } = require('discord.js');
const i18n = require('../i18n');

const CHOICES = ['가위', '바위', '보'];
const CHOICE_EMOJI = { '가위': '✌️', '바위': '✊', '보': '✋' };
const TRIGGERS = ['가위바위보', 'rps', 'ㄱㅂㅂ'];

// 승패 판정: user가 이기면 'win', 지면 'lose', 비기면 'draw'
function judge(user, bot) {
    if (user === bot) return 'draw';
    if (
        (user === '가위' && bot === '보') ||
        (user === '바위' && bot === '가위') ||
        (user === '보'   && bot === '바위')
    ) return 'win';
    return 'lose';
}

/**
 * 버튼 행 생성
 */
function createButtons(round = 1) {
    return new ActionRowBuilder().addComponents(
        new ButtonBuilder()
            .setCustomId(`rps_가위_${round}`)
            .setLabel('가위')
            .setEmoji('✌️')
            .setStyle(ButtonStyle.Primary),
        new ButtonBuilder()
            .setCustomId(`rps_바위_${round}`)
            .setLabel('바위')
            .setEmoji('✊')
            .setStyle(ButtonStyle.Primary),
        new ButtonBuilder()
            .setCustomId(`rps_보_${round}`)
            .setLabel('보')
            .setEmoji('✋')
            .setStyle(ButtonStyle.Primary),
    );
}

/**
 * 비활성 버튼으로 결과 표시
 */
function createDisabledButtons(userChoice, round) {
    return new ActionRowBuilder().addComponents(
        ...CHOICES.map(c => {
            const btn = new ButtonBuilder()
                .setCustomId(`rps_${c}_${round}`)
                .setLabel(c)
                .setEmoji(CHOICE_EMOJI[c])
                .setDisabled(true);
            if (c === userChoice) {
                btn.setStyle(ButtonStyle.Success);
            } else {
                btn.setStyle(ButtonStyle.Secondary);
            }
            return btn;
        })
    );
}

/**
 * 가위바위보 진입점
 * @param {import('discord.js').Message} message
 * @param {string[]} args
 * @returns {boolean} 처리했으면 true
 */
async function handleRPS(message, args) {
    if (args.length !== 1) return false;
    if (!TRIGGERS.includes(args[0])) return false;

    const userId = message.author.id;
    let round = 1;

    const sent = await message.reply({
        content: i18n.t('bot:rps.prompt', { defaultValue: '✊✌️✋ 하나를 골라주세요!' }),
        components: [createButtons(round)],
    });

    // 라운드 루프 — 무승부 시 새 버튼으로 교체
    const playRound = async () => {
        const filter = (interaction) => {
            return interaction.user.id === userId &&
                   interaction.customId.startsWith('rps_') &&
                   interaction.customId.endsWith(`_${round}`);
        };

        try {
            const interaction = await sent.awaitMessageComponent({
                filter,
                time: 15_000,
            });

            const userChoice = interaction.customId.split('_')[1];
            const botChoice = CHOICES[Math.floor(Math.random() * 3)];
            const result = judge(userChoice, botChoice);

            if (result === 'draw') {
                round++;
                await interaction.update({
                    content: i18n.t('bot:rps.draw', {
                        bot: `${CHOICE_EMOJI[botChoice]} ${botChoice}`,
                        defaultValue: `{{bot}}! 비겼다! 다시~ 🔄`,
                    }),
                    components: [createButtons(round)],
                });
                // 재귀적으로 다음 라운드
                await playRound();
            } else {
                const emoji = result === 'win' ? '😵' : '😋';
                const resultText = result === 'win'
                    ? i18n.t('bot:rps.user_win', {
                        bot: `${CHOICE_EMOJI[botChoice]} ${botChoice}`,
                        defaultValue: `{{bot}}! 졌다... ${emoji}`,
                    })
                    : i18n.t('bot:rps.bot_win', {
                        bot: `${CHOICE_EMOJI[botChoice]} ${botChoice}`,
                        defaultValue: `{{bot}}! 이겼다~ ${emoji}`,
                    });

                await interaction.update({
                    content: resultText,
                    components: [createDisabledButtons(userChoice, round)],
                });
            }
        } catch (err) {
            // 시간 초과
            await sent.edit({
                content: i18n.t('bot:rps.timeout', { defaultValue: '⏰ 시간 초과! 다음에 다시 도전하세요~' }),
                components: [],
            }).catch(() => {});
        }
    };

    await playRound();
    return true;
}

module.exports = { handleRPS };
