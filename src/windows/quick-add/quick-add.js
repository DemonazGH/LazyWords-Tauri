const form = document.getElementById('quick-add-form');
const term = document.getElementById('quick-add-term');
const definition = document.getElementById('quick-add-definition');
const suggestion = document.getElementById('quick-add-suggestion');
const options = document.getElementById('quick-add-options');
const status = document.getElementById('quick-add-status');
const save = document.getElementById('quick-add-save');
const practice = document.getElementById('quick-add-practice');
const cancel = document.getElementById('quick-add-cancel');

const messages = {
  'quickAdd.invalidTerm': 'Enter a term on one line.',
  'quickAdd.invalidDefinition': 'Enter a definition or translation.',
  'quickAdd.duplicate': 'This card is already in My Words.',
  'quickAdd.saveError': 'Could not save to My Words. Your draft is still here.',
  'quickAdd.closeError': 'Could not close Quick Add.',
  'quickAdd.saved': 'Saved to My Words.',
  'quickAdd.savedWithoutSwitch': 'Card saved. Your practice dictionary did not change. You can select My Words in Settings.',
  'quickAdd.englishDefinition': 'English definition',
  'quickAdd.russianTranslation': 'Russian translation',
  'quickAdd.verbForms': 'Verb forms',
};
const sourceLabels = new Map([
  ['en-definitions', 'quickAdd.englishDefinition'],
  ['en-ru', 'quickAdd.russianTranslation'],
  ['en-irregular-verbs', 'quickAdd.verbForms'],
]);
let strings = {};
let choices = [];
let lookupTimer;
let lookupVersion = 0;
let busy = false;
let openQueue = Promise.resolve();

function applyLocale(locale) {
  strings = locale;
  document.querySelectorAll('[data-i18n]').forEach(el => {
    if (strings[el.dataset.i18n]) el.textContent = strings[el.dataset.i18n];
  });
  renderSuggestions();
}

function showStatus(key) {
  status.textContent = strings[key] || messages[key] || '';
}

function renderSuggestions() {
  options.replaceChildren();
  suggestion.hidden = choices.length === 0;
  const version = lookupVersion;
  for (const { source, answer } of choices) {
    const button = document.createElement('button');
    button.type = 'button';
    const label = document.createElement('strong');
    const key = sourceLabels.get(source);
    label.textContent = strings[key] || messages[key];
    const preview = document.createElement('span');
    preview.textContent = answer;
    button.appendChild(label);
    button.appendChild(preview);
    button.addEventListener('click', () => {
      if (busy || version !== lookupVersion) return;
      definition.value = answer;
      definition.focus();
    });
    options.appendChild(button);
  }
}

function clearSuggestions() {
  clearTimeout(lookupTimer);
  lookupVersion++;
  choices = [];
  renderSuggestions();
}

function lookupSuggestions(immediate = false) {
  clearSuggestions();
  const query = term.value.trim();
  if (!query) return;
  const version = lookupVersion;
  async function lookup() {
    try {
      const rows = await window.api.quickAddSuggestions(query);
      if (version !== lookupVersion) return;
      choices = rows.filter(row => sourceLabels.has(row.source) && row.answer);
      renderSuggestions();
    } catch {
      // An unavailable suggestion must never block entering an answer manually.
    }
  }
  if (immediate) return lookup();
  lookupTimer = setTimeout(lookup, 180);
}

function resetForm() {
  term.value = '';
  definition.value = '';
  clearSuggestions();
  showStatus();
}

function refreshDraft() {
  // An open may arrive before the webview has registered its listener.
  openQueue = openQueue.catch(() => {}).then(async () => {
    const next = await window.api.takeQuickAddDraft();
    if (!next) return;
    resetForm();
    term.value = next.term;
    (next.term ? definition : term).focus();
    if (next.term) void lookupSuggestions(true);
  });
  return openQueue;
}

function setBusy(value) {
  busy = value;
  for (const field of [term, definition, save, practice, cancel, suggestion]) {
    field.disabled = value;
  }
}

async function dismiss() {
  if (busy) return;
  try {
    await window.api.closeQuickAdd();
    resetForm();
  } catch {
    showStatus('quickAdd.closeError');
  }
}

term.addEventListener('input', () => lookupSuggestions());
cancel.addEventListener('click', dismiss);
form.addEventListener('keydown', event => {
  if (event.key === 'Escape') {
    event.preventDefault();
    return dismiss();
  }
});

form.addEventListener('submit', async event => {
  event.preventDefault();
  if (busy) return;
  const front = term.value.trim();
  const back = definition.value.trim();
  if (!front) return showStatus('quickAdd.invalidTerm');
  if (!back) return showStatus('quickAdd.invalidDefinition');

  setBusy(true);
  showStatus();
  try {
    const outcome = await window.api.saveQuickCard(front, back, event.submitter === practice);
    resetForm();
    if (outcome === 'savedWithoutSwitch') {
      showStatus('quickAdd.savedWithoutSwitch');
      return;
    }
    try {
      await window.api.closeQuickAdd();
    } catch {
      showStatus('quickAdd.saved');
    }
  } catch (error) {
    showStatus(Object.hasOwn(messages, error) ? error : 'quickAdd.saveError');
  } finally {
    setBusy(false);
  }
});

async function init() {
  await window.api.onQuickAddOpened(() => refreshDraft().catch(console.error));
  await refreshDraft().catch(console.error);
  await window.api.onLocaleUpdated(({ strings }) => applyLocale(strings));
  try {
    applyLocale((await window.api.getLocale()).strings);
  } catch (error) {
    console.error(error);
  }
}

init();
