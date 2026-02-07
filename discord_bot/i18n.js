const i18next = require('i18next');
const path = require('path');
const fs = require('fs');

// Load translation files
const loadTranslations = () => {
    const localesPath = path.join(__dirname, '..', 'locales');
    const resources = {};
    
    const languages = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];
    
    languages.forEach(lang => {
        const commonPath = path.join(localesPath, lang, 'common.json');
        const botPath = path.join(localesPath, lang, 'bot.json');
        
        if (fs.existsSync(commonPath) && fs.existsSync(botPath)) {
            const common = JSON.parse(fs.readFileSync(commonPath, 'utf8'));
            const bot = JSON.parse(fs.readFileSync(botPath, 'utf8'));
            
            resources[lang] = {
                common: common,
                bot: bot,
            };
        }
    });
    
    return resources;
};

// Initialize i18next
i18next.init({
    lng: process.env.SABA_LANG || 'en', // Use environment variable or default to English
    fallbackLng: 'en',
    defaultNS: 'bot',
    resources: loadTranslations(),
    interpolation: {
        escapeValue: false,
    },
});

module.exports = i18next;
