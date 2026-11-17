// card.js — renderer process for the word card overlay

const cardEl = document.getElementById('card');
const termEl = document.getElementById('term');
const definitionEl = document.getElementById('definition');

let currentSettings = {};
let hideTimer = null;
let revealTimer = null;
let currentWordIndex = null; // index of the currently displayed card (null when hidden)
let currentDictionaryId = null;
let isCardVisible = false;
let layoutRevision = 0;

async function init() {
  currentSettings = await window.api.getSettings();
  applySettings(currentSettings);

  window.api.onShowWord(({ term, definition, index, dictId, showDuration, answerDelay }) => {
    if (dictId !== currentSettings.activeDictionary) return;
    termEl.textContent = term;
    definitionEl.textContent = definition;
    currentWordIndex = index ?? null;
    currentDictionaryId = dictId;
    return showCardTemporarily(showDuration, answerDelay).catch(console.error);
  });

  window.api.onUpdateSettings((settings) => {
    if (settings.activeDictionary !== currentSettings.activeDictionary) hideCard();
    currentSettings = settings;
    applySettings(settings);
  });

  window.api.onDictAutoSwitched(({ newId }) => {
    hideCard();
    currentSettings.activeDictionary = newId;
  });

  window.api.onAllLearned(({ headline, dictLabel, dictName }) => {
    currentWordIndex = null; // status card — not a learnable entry
    termEl.textContent = headline ?? 'All cards learned! 🎉';
    definitionEl.textContent = `${dictLabel ?? 'Dictionary:'} ${dictName ?? ''}`;
    return showCardTemporarily().catch(console.error);
  });

  window.api.onSwitchedDict(({ headline, dictName }) => {
    currentWordIndex = null;
    termEl.textContent = headline ?? 'Switched to:';
    definitionEl.textContent = dictName ?? '';
    return showCardTemporarily().catch(console.error);
  });

  window.api.onAllDictsLearned(({ headline, sub }) => {
    currentWordIndex = null;
    termEl.textContent = headline ?? 'All dictionaries learned! 🎉';
    definitionEl.textContent = sub ?? 'Restore cards in settings';
    return showCardTemporarily().catch(console.error);
  });

  // Ctrl+Shift+K: mark current card as learned
  window.api.onMarkKnown(() => {
    if (!isCardVisible || currentWordIndex === null) return;
    const idx = currentWordIndex;
    currentWordIndex = null; // prevent double-marking
    window.api.markLearned(idx, currentDictionaryId);
    hideCard();
  });
}

function applySettings(settings) {
  termEl.style.fontSize = `${settings.fontSize ?? 22}px`;
  cardEl.style.transition = `opacity ${settings.fadeDuration ?? 0.5}s ease`;
  if (isCardVisible) fitCard().catch(console.error);
}

async function fitCard() {
  const revision = ++layoutRevision;
  const { width, maxHeight } = await window.api.getCardLayout();
  if (revision !== layoutRevision) return;
  cardEl.style.width = `${width}px`;
  termEl.style.fontSize = `${currentSettings.fontSize ?? 22}px`;
  definitionEl.style.fontSize = '15px';
  // Keep unusually large imports within the monitor's usable area.
  for (let attempt = 0; attempt < 20; attempt++) {
    const height = Math.ceil(cardEl.getBoundingClientRect().height);
    if (height <= maxHeight) break;
    const ratio = Math.min(0.9, (maxHeight - 32) / Math.max(1, height - 32));
    termEl.style.fontSize = `${parseFloat(termEl.style.fontSize) * Math.max(0.1, ratio)}px`;
    definitionEl.style.fontSize = `${parseFloat(definitionEl.style.fontSize) * Math.max(0.1, ratio)}px`;
  }
  await window.api.resizeCard(Math.ceil(cardEl.getBoundingClientRect().height));
  return revision === layoutRevision;
}

function showCard() {
  cardEl.classList.add('visible');
  isCardVisible = true;
}

function cancelCardTimers() {
  clearTimeout(hideTimer);
  clearTimeout(revealTimer);
  hideTimer = revealTimer = null;
}

async function showCardTemporarily(showDuration = currentSettings.showDuration ?? 4, answerDelay = 0) {
  cancelCardTimers();
  isCardVisible = false;
  cardEl.classList.remove('visible');
  definitionEl.classList.toggle('answer-pending', answerDelay > 0);
  if (!await fitCard()) return;
  showCard();
  const reveal = () => {
    revealTimer = null;
    definitionEl.classList.remove('answer-pending');
    hideTimer = setTimeout(hideCard, showDuration * 1000);
  };
  if (answerDelay > 0) revealTimer = setTimeout(reveal, answerDelay * 1000);
  else reveal();
}

function hideCard() {
  layoutRevision++;
  currentWordIndex = null;
  currentDictionaryId = null;
  cardEl.classList.remove('visible');
  isCardVisible = false;
  cancelCardTimers();
}

init();
