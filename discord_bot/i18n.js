const i18next = require('i18next');
const path = require('path');
const fs = require('fs');

// Load translation files
const loadTranslations = () => {
    const localesPath = path.join(__dirname, '..', 'locales');
    const resources = {};
    
    // Load English
    const enCommon = JSON.parse(fs.readFileSync(path.join(localesPath, 'en', 'common.json'), 'utf8'));
    const enBot = JSON.parse(fs.readFileSync(path.join(localesPath, 'en', 'bot.json'), 'utf8'));
    
    // Load Korean
    const koCommon = JSON.parse(fs.readFileSync(path.join(localesPath, 'ko', 'common.json'), 'utf8'));
    const koBot = JSON.parse(fs.readFileSync(path.join(localesPath, 'ko', 'bot.json'), 'utf8'));
    
    // Load Japanese
    const jaCommon = JSON.parse(fs.readFileSync(path.join(localesPath, 'ja', 'common.json'), 'utf8'));
    const jaBot = JSON.parse(fs.readFileSync(path.join(localesPath, 'ja', 'bot.json'), 'utf8'));
    
    resources.en = {
        common: enCommon,
        bot: enBot,
    };
    
    resources.ko = {
        common: koCommon,
        bot: koBot,
    };
    
    resources.ja = {
        common: jaCommon,
        bot: jaBot,
    };
    
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
