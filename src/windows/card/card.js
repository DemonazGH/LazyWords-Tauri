// card.js — renderer process for the word card overlay

const cardEl = document.getElementById('card');
const wordEl = document.getElementById('word');
const translationEl = document.getElementById('translation');

let currentSettings = {};
let hideTimer = null;
let currentWordIndex = null; // index of the currently displayed word (null when hidden)
let isCardVisible = false;

async function init() {
  currentSettings = await window.api.getSettings();
  applySettings(currentSettings);

  window.api.onShowWord(({ word, translation, index }) => {
    wordEl.textContent = word;
    translationEl.textContent = translation;
    currentWordIndex = index ?? null;
    showCardTemporarily();
  });

  window.api.onHideWord(() => {
    hideCard();
  });

  window.api.onUpdateSettings((settings) => {
    currentSettings = settings;
    applySettings(settings);
  });

  window.api.onAllLearned(({ headline, dictLabel, dictName }) => {
    currentWordIndex = null; // status card — not a learnable word
    wordEl.textContent = headline ?? 'All words learned! 🎉';
    translationEl.textContent = `${dictLabel ?? 'Dictionary:'} ${dictName ?? ''}`;
    showCardTemporarily();
  });

  window.api.onSwitchedDict(({ headline, dictName }) => {
    currentWordIndex = null;
    wordEl.textContent = headline ?? 'Switched to:';
    translationEl.textContent = dictName ?? '';
    showCardTemporarily();
  });

  window.api.onAllDictsLearned(({ headline, sub }) => {
    currentWordIndex = null;
    wordEl.textContent = headline ?? 'All dictionaries learned! 🎉';
    translationEl.textContent = sub ?? 'Restore words in settings';
    showCardTemporarily();
  });

  // Ctrl+Shift+K: mark current word as learned
  window.api.onMarkKnown(() => {
    if (!isCardVisible || currentWordIndex === null) return;
    const idx = currentWordIndex;
    currentWordIndex = null; // prevent double-marking
    window.api.markLearned(idx);
    hideCard();
  });
}

function applySettings(settings) {
  wordEl.style.fontSize = `${settings.fontSize ?? 22}px`;
  cardEl.style.transition = `opacity ${settings.fadeDuration ?? 0.5}s ease`;
}

function showCard() {
  cardEl.classList.add('visible');
  isCardVisible = true;
}

function showCardTemporarily() {
  showCard();
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => {
    hideCard();
  }, (currentSettings.showDuration ?? 4) * 1000);
}

function hideCard() {
  cardEl.classList.remove('visible');
  isCardVisible = false;
  clearTimeout(hideTimer);
}

init();
