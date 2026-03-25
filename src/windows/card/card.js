// card.js — renderer process for the word card overlay

const cardEl = document.getElementById('card');
const termEl = document.getElementById('term');
const definitionEl = document.getElementById('definition');

let currentSettings = {};
let hideTimer = null;
let currentWordIndex = null; // index of the currently displayed card (null when hidden)
let isCardVisible = false;

async function init() {
  currentSettings = await window.api.getSettings();
  applySettings(currentSettings);

  window.api.onShowWord(({ term, definition, index }) => {
    termEl.textContent = term;
    definitionEl.textContent = definition;
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
    currentWordIndex = null; // status card — not a learnable entry
    termEl.textContent = headline ?? 'All cards learned! 🎉';
    definitionEl.textContent = `${dictLabel ?? 'Dictionary:'} ${dictName ?? ''}`;
    showCardTemporarily();
  });

  window.api.onSwitchedDict(({ headline, dictName }) => {
    currentWordIndex = null;
    termEl.textContent = headline ?? 'Switched to:';
    definitionEl.textContent = dictName ?? '';
    showCardTemporarily();
  });

  window.api.onAllDictsLearned(({ headline, sub }) => {
    currentWordIndex = null;
    termEl.textContent = headline ?? 'All dictionaries learned! 🎉';
    definitionEl.textContent = sub ?? 'Restore cards in settings';
    showCardTemporarily();
  });

  // Ctrl+Shift+K: mark current card as learned
  window.api.onMarkKnown(() => {
    if (!isCardVisible || currentWordIndex === null) return;
    const idx = currentWordIndex;
    currentWordIndex = null; // prevent double-marking
    window.api.markLearned(idx);
    hideCard();
  });
}

function applySettings(settings) {
  termEl.style.fontSize = `${settings.fontSize ?? 22}px`;
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
