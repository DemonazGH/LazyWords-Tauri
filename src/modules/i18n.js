// i18n.js — Internationalization module (main process only)
// Loads locale JSON files and provides t(key) translation function.

const fs = require('fs');
const path = require('path');

const LOCALES_DIR = path.join(__dirname, '../../locales');
const SUPPORTED = ['en', 'ru'];

let strings = {};
let currentLocale = 'en';

function resolveCode(code) {
  if (!code || code === 'auto') return 'en'; // auto resolved before calling here
  if (code.startsWith('ru')) return 'ru';
  return 'en';
}

function setLocale(code) {
  const resolved = resolveCode(code);
  const file = path.join(LOCALES_DIR, `${resolved}.json`);
  try {
    strings = JSON.parse(fs.readFileSync(file, 'utf8'));
    currentLocale = resolved;
  } catch {
    // Fallback to English
    try {
      strings = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, 'en.json'), 'utf8'));
      currentLocale = 'en';
    } catch {
      strings = {};
      currentLocale = 'en';
    }
  }
}

function t(key) {
  return strings[key] ?? key;
}

function getLocale() {
  return currentLocale;
}

function getStrings() {
  return { ...strings };
}

module.exports = { setLocale, t, getLocale, getStrings, SUPPORTED };
