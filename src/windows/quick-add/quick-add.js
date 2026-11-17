const form = document.getElementById('quick-add-form');
const term = document.getElementById('quick-add-term');
const definition = document.getElementById('quick-add-definition');
const suggestion = document.getElementById('quick-add-suggestion');
const preview = document.getElementById('quick-add-preview');
const status = document.getElementById('quick-add-status');
const save = document.getElementById('quick-add-save');
const cancel = document.getElementById('quick-add-cancel');
const useSuggestion = document.getElementById('quick-add-use');

const messages = {
  'quickAdd.invalidTerm': 'Enter a term on one line.',
  'quickAdd.invalidDefinition': 'Enter a definition or translation.',
  'quickAdd.duplicate': 'This card is already in My Words.',
  'quickAdd.saveError': 'Could not save to My Words. Your draft is still here.',
  'quickAdd.closeError': 'Could not close Quick Add.',
  'quickAdd.saved': 'Saved to My Words.',
};
let strings = {};
let draft = null;
let busy = false;
let openQueue = Promise.resolve();

function applyLocale(locale) {
  strings = locale;
  document.querySelectorAll('[data-i18n]').forEach(el => {
    if (strings[el.dataset.i18n]) el.textContent = strings[el.dataset.i18n];
  });
}

function showStatus(key) {
  status.textContent = strings[key] || messages[key] || '';
}

function renderSuggestion() {
  const matches = term.value.trim().toLowerCase() === draft?.term.toLowerCase();
  preview.textContent = matches ? draft.suggestion || '' : '';
  suggestion.hidden = !preview.textContent;
}

function resetForm() {
  draft = null;
  term.value = '';
  definition.value = '';
  renderSuggestion();
  showStatus();
}

function refreshDraft() {
  // An open may arrive before the webview has registered its listener.
  openQueue = openQueue.catch(() => {}).then(async () => {
    const next = await window.api.takeQuickAddDraft();
    if (!next) return;
    resetForm();
    draft = next;
    term.value = next.term;
    renderSuggestion();
    (next.term ? definition : term).focus();
  });
  return openQueue;
}

function setBusy(value) {
  busy = value;
  for (const field of [term, definition, save, cancel, useSuggestion]) {
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

term.addEventListener('input', renderSuggestion);
useSuggestion.addEventListener('click', () => {
  if (!suggestion.hidden && !busy) {
    definition.value = preview.textContent;
    definition.focus();
  }
});
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
    await window.api.saveQuickCard(front, back);
    resetForm();
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
