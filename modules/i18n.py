#!/usr/bin/env python3
"""
Simple i18n utility for Python modules.
Loads translations from JSON files in the module's locales/ directory.
Keys a module does not define fall back to the shared phrases in common_locales.json
(next to this file, shipped with the daemon, saba-core >=0.1.11-0).
"""

import json
import os
import re


class I18n:
    """Simple internationalization helper"""
    
    def __init__(self, module_dir, default_lang='en'):
        """
        Initialize i18n with module directory
        
        Args:
            module_dir: Directory containing the locales/ folder
            default_lang: Default language code (default: 'en')
        """
        self.module_dir = module_dir
        self.locales_dir = os.path.join(module_dir, 'locales')
        self.default_lang = default_lang
        self.current_lang = os.environ.get('SABA_LANG', default_lang)
        self.translations = {}
        self.common = {}
        self._load_translations()
    
    def _load_translations(self):
        """Load translation files"""
        # Load default language
        default_file = os.path.join(self.locales_dir, f'{self.default_lang}.json')
        if os.path.exists(default_file):
            with open(default_file, 'r', encoding='utf-8') as f:
                self.translations[self.default_lang] = json.load(f)
        
        # Load current language if different
        if self.current_lang != self.default_lang:
            current_file = os.path.join(self.locales_dir, f'{self.current_lang}.json')
            if os.path.exists(current_file):
                with open(current_file, 'r', encoding='utf-8') as f:
                    self.translations[self.current_lang] = json.load(f)
    
        # 공용 문구: 모듈 locales에 없는 범용 키(정지·설치 오류 등)
        common_file = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'common_locales.json')
        if os.path.exists(common_file):
            try:
                with open(common_file, 'r', encoding='utf-8') as f:
                    self.common = json.load(f)
            except (OSError, ValueError):
                self.common = {}

    def _get_nested_value(self, obj, path):
        """Get nested value from object using dot notation"""
        keys = path.split('.')
        value = obj
        for key in keys:
            if isinstance(value, dict) and key in value:
                value = value[key]
            else:
                return None
        return value
    
    def t(self, key, **kwargs):
        """
        Translate a key with optional interpolation
        
        Args:
            key: Translation key (supports dot notation like 'errors.failed_to_start')
            **kwargs: Variables for interpolation (e.g., error='connection failed')
        
        Returns:
            Translated string with interpolated values
        
        Example:
            t('errors.failed_to_start', error='connection failed')
            # Returns: "Failed to start: connection failed"
        """
        # Try current language first
        translation = None
        if self.current_lang in self.translations:
            translation = self._get_nested_value(self.translations[self.current_lang], key)
        
        # Fall back to default language
        if translation is None and self.default_lang in self.translations:
            translation = self._get_nested_value(self.translations[self.default_lang], key)
        
        # Fall back to the shared phrases (current language, then default)
        for lang in (self.current_lang, self.default_lang):
            if translation is None and lang in self.common:
                translation = self._get_nested_value(self.common[lang], key)

        # Fall back to key itself
        if translation is None:
            translation = key
        
        # Interpolate variables (replace {{var}} with values)
        if kwargs:
            for var_name, var_value in kwargs.items():
                translation = translation.replace(f'{{{{{var_name}}}}}', str(var_value))
        
        return translation
