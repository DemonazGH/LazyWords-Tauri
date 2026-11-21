// settings.js — Settings window renderer (auto-save pattern)

let currentSettings = {};
let L = {}; // current locale strings

// ── Locale ─────────────────────────────────────────────────

function applyLocale(strings) {
  L = strings;

  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = el.dataset.i18n;
    if (strings[key] !== undefined) el.textContent = strings[key];
  });
  document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
    const key = el.dataset.i18nPlaceholder;
    if (strings[key] !== undefined) el.placeholder = strings[key];
  });
  document.querySelectorAll('[data-i18n-title]').forEach(el => {
    const key = el.dataset.i18nTitle;
    if (strings[key] !== undefined) el.title = strings[key];
  });

  // Also update the auto option text (it can vary by locale)
  const autoOpt = document.querySelector('#language option[value="auto"]');
  if (autoOpt && strings['settings.language.auto']) {
    autoOpt.textContent = strings['settings.language.auto'];
  }

  // Re-render dynamic parts with new strings
  renderAnswerDelay();
  renderLearnedList();
  if (myWordsManager.open && !myWordsBusy) loadMyWords();
  if (document.getElementById('tab-stats').classList.contains('active')) {
    loadStats();
  }
}

// ── Auto-save ───────────────────────────────────────────────────────────────

let debounceTimer = null;
let indicatorTimer = null;
let pendingSettingsPatch = {};
let saveQueue = Promise.resolve();
let settingsRevision = 0;

function showSavedIndicator() {
  const tag = document.getElementById('autosave-tag');
  tag.classList.add('visible');
  clearTimeout(indicatorTimer);
  indicatorTimer = setTimeout(() => tag.classList.remove('visible'), 1000);
}

async function saveImmediate(patch) {
  Object.assign(currentSettings, patch);
  settingsRevision++;
  Object.assign(pendingSettingsPatch, patch);
  return flushPendingSettings();
}

function saveDebounced(patch) {
  Object.assign(currentSettings, patch);
  settingsRevision++;
  Object.assign(pendingSettingsPatch, patch);
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => flushPendingSettings().catch(console.error), 200);
}

function flushPendingSettings() {
  clearTimeout(debounceTimer);
  const patch = pendingSettingsPatch;
  pendingSettingsPatch = {};
  if (Object.keys(patch).length === 0) return saveQueue;
  // Serialize writes so an older request cannot overwrite a newer slider value.
  saveQueue = saveQueue.catch(() => {}).then(async () => {
    clearTimeout(indicatorTimer);
    document.getElementById('autosave-tag').classList.remove('visible');
    const saved = await window.api.saveSettings(patch);
    if (saved === false) return false;
    showSavedIndicator();
    return true;
  });
  return saveQueue;
}

window.addEventListener('blur', () => flushPendingSettings().catch(console.error));

// ── Tab switching ──────────────────────────────────────────

document.querySelectorAll('.tab').forEach(tab => {
  tab.addEventListener('click', () => {
    document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById(`tab-${tab.dataset.tab}`).classList.add('active');
    if (tab.dataset.tab === 'stats') loadStats();
  });
});

// ── Language selector ──────────────────────────────────────

document.getElementById('language').addEventListener('change', async (e) => {
  await saveImmediate({ language: e.target.value });
  // Manually fetch and apply new language strings instantly
  const localeData = await window.api.getLocale();
  applyLocale(localeData.strings);
  loadDictionaries();
});

// ── Position buttons ───────────────────────────────────────

document.querySelectorAll('.pos-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.pos-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    saveImmediate({ position: btn.dataset.pos });
  });
});

function setActivePosition(pos) {
  document.querySelectorAll('.pos-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.pos === pos);
  });
}

// ── Range sliders ──────────────────────────────────────────

function bindRange(id, key, decimals = 0) {
  const input = document.getElementById(id);
  const valEl = document.getElementById(`${id}-val`);
  input.addEventListener('input', () => {
    const v = parseFloat(input.value);
    valEl.textContent = decimals > 0 ? v.toFixed(decimals) : v;
    saveDebounced({ [key]: v });
  });
}

bindRange('fontSize', 'fontSize');
bindRange('showDuration', 'showDuration');
bindRange('interval', 'interval');
bindRange('fadeDuration', 'fadeDuration', 1);

function renderAnswerDelay() {
  const seconds = currentSettings.answerDelay ?? 0;
  const input = document.getElementById('answerDelay');
  const label = seconds === 0
    ? (L['settings.answerDelay.off'] ?? 'Off')
    : `${seconds} ${L['settings.showDuration.unit'] ?? 'sec'}`;
  input.value = seconds;
  input.setAttribute('aria-valuetext', label);
  document.getElementById('answerDelay-val').textContent = label;
}

document.getElementById('answerDelay').addEventListener('input', (e) => {
  saveDebounced({ answerDelay: Number(e.target.value) });
  renderAnswerDelay();
});

// ── Checkbox ───────────────────────────────────────────────

function renderSmartScheduling() {
  document.getElementById('smartScheduling').checked = currentSettings.smartScheduling ?? false;
}

document.getElementById('smartScheduling').addEventListener('change', async (e) => {
  const checkbox = e.target;
  const previous = currentSettings.smartScheduling ?? false;
  const error = document.getElementById('smart-scheduling-error');
  checkbox.disabled = true;
  error.hidden = true;
  try {
    if (await saveImmediate({ smartScheduling: checkbox.checked }) === false) {
      throw new Error('Smart scheduling change was not saved');
    }
  } catch (failure) {
    console.error(failure);
    currentSettings.smartScheduling = previous;
    renderSmartScheduling();
    error.textContent = L['settings.smartScheduling.error'] ??
      'Could not save smart scheduling. Try again.';
    error.hidden = false;
  } finally {
    checkbox.disabled = false;
  }
});

document.getElementById('autoStart').addEventListener('change', async (e) => {
  const checkbox = e.target;
  const error = document.getElementById('autostart-error');
  checkbox.disabled = true;
  error.hidden = true;
  try {
    if (await saveImmediate({ autoStart: checkbox.checked }) === false) {
      throw new Error('Autostart change was not saved');
    }
  } catch (failure) {
    console.error(failure);
    try {
      const settings = await window.api.getSettings();
      currentSettings.autoStart = settings.autoStart;
      checkbox.checked = settings.autoStart;
    } catch (refreshError) {
      console.error(refreshError);
    }
    error.textContent = L['settings.autostart.error'] ??
      'Could not change startup. Check Windows Startup apps and permissions, then try again.';
    error.hidden = false;
  } finally {
    checkbox.disabled = false;
  }
});

// ── Dictionary selector ────────────────────────────────────

let availableDicts = [];
let dictionaryListRequest = 0;

async function loadDictionaries() {
  const request = ++dictionaryListRequest;
  const dictionaries = await window.api.getDictionaries();
  if (request !== dictionaryListRequest) return;
  availableDicts = dictionaries;
  const selectedId = currentSettings.activeDictionary;
  const sel = document.getElementById('activeDictionary');
  sel.innerHTML = '';
  const importLabel = L['settings.import.label'] ?? 'import';
  for (const d of availableDicts) {
    const opt = document.createElement('option');
    opt.value = d.id;
    opt.textContent = d.source === 'user' ? `${d.name} (${importLabel})` : d.name;
    if (d.id === selectedId) opt.selected = true;
    sel.appendChild(opt);
  }
  updateDeleteBtn();
}

function updateDeleteBtn() {
  const selId = document.getElementById('activeDictionary').value;
  const dict = availableDicts.find(d => d.id === selId);
  document.getElementById('btn-delete-dict').style.display =
    (dict && dict.source === 'user') ? '' : 'none';
}

document.getElementById('activeDictionary').addEventListener('change', async (e) => {
  await saveImmediate({ activeDictionary: e.target.value });
  updateDeleteBtn();
  loadLearnedList();
});

document.getElementById('btn-delete-dict').addEventListener('click', async () => {
  const selId = document.getElementById('activeDictionary').value;
  const dict = availableDicts.find(d => d.id === selId);
  if (!dict || dict.source !== 'user') return;

  const confirmMsg = (L['settings.deleteConfirm'] ?? 'Delete dictionary "{name}"?')
    .replace('{name}', dict.name);
  if (!confirm(confirmMsg)) return;

  const result = await window.api.deleteDictionary(selId);
  if (!result || result.error) {
    const errMsg = (L['settings.import.error'] ?? 'Error: {msg}')
      .replace('{msg}', result?.error ?? 'unknown error');
    alert(errMsg);
    return;
  }
  currentSettings.activeDictionary = result.newActive;
  settingsRevision++;
  await loadDictionaries();
  await loadLearnedList();
});

// ── Import ─────────────────────────────────────────────────

document.getElementById('btn-import').addEventListener('click', async () => {
  const msg = document.getElementById('import-msg');
  msg.className = 'import-msg';
  msg.textContent = '';

  const result = await window.api.importDictionary();
  if (!result) return; // cancelled

  msg.classList.add('visible');
  if (result.error) {
    msg.classList.add('err');
    msg.textContent = (L['settings.import.error'] ?? 'Error: {msg}')
      .replace('{msg}', result.error);
  } else {
    msg.classList.add('ok');
    msg.textContent = (L['settings.import.success'] ?? 'Imported: {n} words')
      .replace('{n}', result.wordCount);
    await saveImmediate({ activeDictionary: result.id });
    await loadDictionaries();
    await loadLearnedList();
  }

  setTimeout(() => msg.classList.remove('visible'), 4000);
});

// My Words

const myWordsManager = document.getElementById('my-words');
const myWordsList = document.getElementById('my-words-list');
const myWordsStatus = document.getElementById('my-words-status');
const editMyWordDialog = document.getElementById('my-words-edit');
const deleteMyWordDialog = document.getElementById('my-words-delete');
const myWordsTerm = document.getElementById('my-words-term');
const myWordsDefinition = document.getElementById('my-words-definition');
const myWordsCancelEdit = document.getElementById('my-words-cancel-edit');
let myWordsRequest = 0;
let myWordsBusy = false;
let myWordsTarget = null;

async function loadMyWords(message) {
  const request = ++myWordsRequest;
  try {
    const cards = await window.api.getMyWords();
    if (request !== myWordsRequest || !myWordsManager.open) return;
    myWordsList.replaceChildren(...cards.map(renderMyWord));
    myWordsList.hidden = cards.length === 0;
    myWordsStatus.textContent = message || (cards.length ? '' :
      (L['myWords.empty'] ?? 'My Words is empty. Use Quick Add to save a card.'));
  } catch {
    if (request !== myWordsRequest || !myWordsManager.open) return;
    myWordsList.replaceChildren();
    myWordsList.hidden = true;
    myWordsStatus.textContent = L['myWords.loadError'] ?? 'Could not read My Words.';
  }
}

function renderMyWord(card) {
  const row = document.createElement('li');
  const pair = document.createElement('div');
  pair.className = 'my-words-pair';
  const term = document.createElement('strong');
  const definition = document.createElement('span');
  term.textContent = card.term;
  definition.textContent = card.definition;
  pair.appendChild(term);
  pair.appendChild(definition);
  row.appendChild(pair);
  for (const [action, fallback, dialog] of [
    ['edit', 'Edit', editMyWordDialog],
    ['delete', 'Delete', deleteMyWordDialog],
  ]) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn-secondary';
    button.textContent = L[`myWords.${action}`] ?? fallback;
    button.setAttribute('aria-label', `${button.textContent}: ${card.term}`);
    button.addEventListener('click', () => {
      if (myWordsBusy) return;
      myWordsTarget = card.cardId;
      if (action === 'edit') {
        myWordsTerm.value = card.term;
        myWordsDefinition.value = card.definition;
        myWordsTerm.removeAttribute('aria-invalid');
        myWordsDefinition.removeAttribute('aria-invalid');
      } else {
        document.getElementById('my-words-delete-preview').textContent = `${card.term}\n${card.definition}`;
      }
      document.getElementById(`my-words-${action}-status`).textContent = '';
      dialog.showModal();
    });
    row.appendChild(button);
  }
  return row;
}

async function mutateMyWords(action, dialog, submitId, mutate) {
  if (myWordsBusy || !dialog.open || !myWordsTarget) return;
  const status = document.getElementById(`my-words-${action}-status`);
  const submit = document.getElementById(submitId);
  myWordsBusy = true;
  submit.disabled = true;
  if (action === 'edit') myWordsTerm.readOnly = myWordsDefinition.readOnly = true;
  status.textContent = L['myWords.working'] ?? 'Saving to My Words...';
  try {
    await mutate(myWordsTarget);
  } catch (error) {
    const messages = {
      'quickAdd.invalidTerm': 'Enter a term on one line (up to 160 characters).',
      'quickAdd.invalidDefinition': 'Enter a definition or translation (up to 4000 characters).',
      'quickAdd.duplicate': 'This card is already in My Words.',
      'myWords.stale': 'This card changed. Close this dialog and reopen My Words.',
      'myWords.ambiguous': 'There are multiple identical cards. No changes were made.',
    };
    status.textContent = Object.hasOwn(messages, error)
      ? (L[error] ?? messages[error])
      : (L['myWords.writeError'] ?? 'Could not update My Words. No changes were saved.');
    const invalidField = error === 'quickAdd.invalidTerm' ? myWordsTerm :
      error === 'quickAdd.invalidDefinition' ? myWordsDefinition : null;
    invalidField?.setAttribute('aria-invalid', 'true');
    invalidField?.focus();
    return;
  } finally {
    submit.disabled = false;
    if (action === 'edit') myWordsTerm.readOnly = myWordsDefinition.readOnly = false;
    myWordsBusy = false;
  }
  dialog.close();
  document.getElementById('my-words-summary').focus();
  await loadMyWords(action === 'edit' ? (L['myWords.saved'] ?? 'Card updated.') :
    (L['myWords.deleted'] ?? 'Card deleted.'));
}

myWordsManager.addEventListener('toggle', () => {
  if (myWordsManager.open) return loadMyWords();
  myWordsRequest++;
});

myWordsCancelEdit.addEventListener('click', () => {
  if (!myWordsBusy) editMyWordDialog.close();
});
for (const field of [myWordsTerm, myWordsDefinition]) {
  field.addEventListener('input', () => field.removeAttribute('aria-invalid'));
}
for (const dialog of [editMyWordDialog, deleteMyWordDialog]) {
  dialog.addEventListener('cancel', event => {
    if (myWordsBusy) event.preventDefault();
  });
}

document.getElementById('my-words-edit-form').addEventListener('submit', event => {
  event.preventDefault();
  return mutateMyWords('edit', editMyWordDialog, 'my-words-save', id =>
    window.api.editMyWord(id, myWordsTerm.value.trim(), myWordsDefinition.value.trim()));
});
document.getElementById('my-words-delete-form').addEventListener('submit', event => {
  if (event.submitter?.value === 'cancel' && !myWordsBusy) return;
  event.preventDefault();
  if (event.submitter?.id !== 'my-words-confirm-delete') return;
  return mutateMyWords('delete', deleteMyWordDialog, 'my-words-confirm-delete', id =>
    window.api.deleteMyWord(id));
});

// ── Learned list ───────────────────────────────────────────

const LEARNED_PAGE = 20;
let allLearnedWords = []; // newest first
let learnedShowAll = false;
let restoreAllConfirming = false;
let restoreAllTimer = null;
let learnedListRequest = 0;
let learnedDeckId = null;
let legacyReview = null;
let legacyConfirmation = null;
let previewRequest = 0;
const reviewDeck = document.getElementById('legacy-deck');

async function loadLearnedList() {
  const request = ++learnedListRequest;
  const { dictId, words, pendingTotal = 0, pendingHere = 0, pendingDecks = {} } = await window.api.getLearnedList();
  if (request !== learnedListRequest || dictId !== currentSettings.activeDictionary) return;
  allLearnedWords = [...words].reverse(); // newest first
  learnedDeckId = dictId;
  legacyReview = { dictId, pendingTotal, pendingHere, pendingDecks };
  renderLearnedList();
}

function renderLegacyReview() {
  const { pendingTotal = 0, pendingHere = 0, pendingDecks = {} } = legacyReview ?? {};
  const decks = Object.entries(pendingDecks);
  const selected = reviewDeck.value;
  reviewDeck.replaceChildren(...decks.map(([id, count]) => {
    const option = document.createElement('option');
    option.value = id;
    option.textContent = `${id} (${count})`;
    return option;
  }));
  reviewDeck.value = [selected, currentSettings.activeDictionary].find(id => decks.some(([key]) => key === id)) ?? decks[0]?.[0] ?? '';
  document.getElementById('legacy-notice').hidden = !decks.length || !!currentSettings.legacyNoticeDismissed;
  document.getElementById('legacy-notice-message').textContent =
    (L['learned.notice.message'] ?? 'Dictionaries with marks to review: {count}. You can leave them pending without losing them.')
      .replace('{count}', decks.length);
  document.getElementById('learned-legacy').hidden = pendingTotal === 0;
  document.getElementById('btn-review-legacy').hidden = !decks.length;
  document.getElementById('learned-legacy-count').textContent =
    (L['learned.legacy.pending'] ?? 'Older markers pending: {total} ({deck} in the active dictionary). They do not hide cards until linked.')
      .replace('{total}', pendingTotal).replace('{deck}', pendingHere);
}

function learnedStatus(key, fallback) {
  document.getElementById('learned-status').textContent = L[key] ?? fallback;
}

const legacyDialog = document.getElementById('learned-legacy-dialog');
document.getElementById('legacy-notice-review').addEventListener('click', () => reviewDeck.focus());
document.getElementById('legacy-notice-dismiss').addEventListener('click', async () => {
  try {
    if (await saveImmediate({ legacyNoticeDismissed: true }) === false) throw new Error('Save failed');
    renderLegacyReview();
    reviewDeck.focus();
  } catch (error) {
    console.error(error);
    currentSettings.legacyNoticeDismissed = false;
    renderLegacyReview();
    document.getElementById('legacy-notice-status').textContent = L['learned.notice.saveError'] ?? 'Could not save the dismissal.';
  }
});
reviewDeck.addEventListener('change', () => {
  previewRequest++;
  document.getElementById('learned-status').textContent = '';
});
document.getElementById('btn-review-legacy').addEventListener('click', async () => {
  const dictId = reviewDeck.value;
  const request = ++previewRequest;
  if (!dictId || legacyDialog.open) return;
  let preview;
  try { preview = await window.api.previewLegacyLearned(dictId); } catch (error) { console.error(error); }
  if (request !== previewRequest || dictId !== reviewDeck.value) return;
  if (!preview?.orderToken || !preview.sample.length) {
    learnedStatus('learned.legacy.unavailable', 'No matching rows could be reviewed. Your marks are still saved.');
    return;
  }
  document.getElementById('legacy-review-name').textContent = dictId;
  document.getElementById('legacy-preview').replaceChildren(...preview.sample.map(({ row, term, definition }) => {
    const li = document.createElement('li');
    li.textContent = `#${row}: ${term} · ${definition}`;
    return li;
  }));
  legacyConfirmation = { dictId, orderToken: preview.orderToken };
  legacyDialog.showModal();
});

legacyDialog.addEventListener('cancel', () => { legacyConfirmation = null; });
// A queued close event from an older preview must never consume a new confirmation.
document.getElementById('legacy-review-form').addEventListener('submit', async event => {
  const review = legacyConfirmation;
  legacyConfirmation = null;
  if (!legacyDialog.open || event.submitter?.id !== 'legacy-review-confirm' || !review) return;
  try {
    const linked = await window.api.resolveLegacyLearned(review.dictId, review.orderToken);
    if (linked > 0) {
      learnedStatus('learned.legacy.linked', 'Matched older markers were linked. Any unmatched markers remain pending.');
    } else {
      learnedStatus('learned.legacy.notLinked', 'No markers were linked. They remain pending.');
    }
    await loadLearnedList();
  } catch (error) {
    console.error(error);
    learnedStatus('learned.legacy.notLinked', 'No markers were linked. They remain pending.');
  }
});

function renderLearnedList() {
  renderLegacyReview();
  const list    = document.getElementById('learned-list');
  const empty   = document.getElementById('learned-empty');
  const count   = document.getElementById('learned-count');
  const search  = document.getElementById('learned-search');
  const showBtn = document.getElementById('btn-show-all');
  const restAll = document.getElementById('btn-restore-all');

  count.textContent = allLearnedWords.length;

  if (allLearnedWords.length === 0) {
    list.style.display = 'none';
    empty.style.display = 'block';
    search.style.display = 'none';
    showBtn.style.display = 'none';
    restAll.style.display = 'none';
    return;
  }

  empty.style.display = 'none';
  search.style.display = '';
  restAll.style.display = '';

  // Keep restore-all button text in sync with locale (not confirming state)
  if (!restoreAllConfirming) {
    restAll.textContent = L['settings.learned.restoreAll'] ?? 'Restore all';
  }

  const query = search.value.trim().toLowerCase();
  const filtered = query
    ? allLearnedWords.filter(w =>
        w.term.toLowerCase().includes(query) ||
        w.definition.toLowerCase().includes(query))
    : allLearnedWords;

  const expandedBySearch = query.length > 0;
  const visible = (learnedShowAll || expandedBySearch)
    ? filtered
    : filtered.slice(0, LEARNED_PAGE);

  list.classList.toggle('learned-list--expanded', learnedShowAll || expandedBySearch);
  list.style.display = filtered.length ? 'block' : 'none';
  list.innerHTML = '';

  const restoreLabel = L['settings.learned.restore'] ?? 'Restore';
  const sourceDeck = learnedDeckId;
  visible.forEach(({ term, definition, cardId }) => {
    const li = document.createElement('li');
    li.innerHTML = `
      <div class="word-pair">
        <span class="term">${escHtml(term)}</span>
        <span class="definition">${escHtml(definition)}</span>
      </div>
      <button class="btn-restore" type="button">${escHtml(restoreLabel)}</button>
    `;
    li.querySelector('.btn-restore').addEventListener('click', async () => {
      await restoreLearned(() => window.api.removeLearned(cardId, sourceDeck));
    });
    list.appendChild(li);
  });

  if (!expandedBySearch && filtered.length > LEARNED_PAGE) {
    showBtn.style.display = '';
    showBtn.textContent = learnedShowAll
      ? (L['settings.learned.collapse'] ?? 'Collapse')
      : (L['settings.learned.showAll'] ?? 'Show all ({n})').replace('{n}', filtered.length);
  } else {
    showBtn.style.display = 'none';
  }
}

document.getElementById('learned-search').addEventListener('input', renderLearnedList);

document.getElementById('btn-show-all').addEventListener('click', () => {
  learnedShowAll = !learnedShowAll;
  renderLearnedList();
});

async function restoreLearned(action) {
  try {
    if (!await action()) throw new Error('Learned marker was not restored');
    learnedStatus('', '');
    await loadLearnedList();
  } catch (error) {
    console.error(error);
    learnedStatus('learned.restoreFailed', 'Could not restore learned cards. Refresh Settings and try again.');
  }
}

let restoreAllDeck = null;
document.getElementById('btn-restore-all').addEventListener('click', async () => {
  const btn = document.getElementById('btn-restore-all');
  if (!restoreAllConfirming) {
    restoreAllConfirming = true;
    restoreAllDeck = learnedDeckId;
    btn.textContent = L['settings.learned.restoreAllConfirm'] ?? 'Really restore all?';
    btn.classList.add('confirming');
    clearTimeout(restoreAllTimer);
    restoreAllTimer = setTimeout(() => {
      restoreAllConfirming = false;
      btn.textContent = L['settings.learned.restoreAll'] ?? 'Restore all';
      btn.classList.remove('confirming');
    }, 3000);
    return;
  }
  clearTimeout(restoreAllTimer);
  restoreAllConfirming = false;
  btn.textContent = L['settings.learned.restoreAll'] ?? 'Restore all';
  btn.classList.remove('confirming');
  await restoreLearned(() => window.api.clearLearned(restoreAllDeck));
  learnedShowAll = false;
});

function escHtml(str) {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// ── Statistics ─────────────────────────────────────────────

async function loadStats() {
  const { today, streak, last7, learnedCount, totalWords } = await window.api.getStats();

  document.getElementById('stat-shown-today').textContent = today.shown;
  document.getElementById('stat-learned-today').textContent = today.learned;
  document.getElementById('stat-streak').textContent = streak;
  document.getElementById('stat-learned-total').textContent = learnedCount;
  document.getElementById('stat-total').textContent = totalWords;

  const pct = totalWords > 0 ? Math.round((learnedCount / totalWords) * 100) : 0;
  document.getElementById('stat-progress-fill').style.width = `${pct}%`;
  document.getElementById('stat-progress-pct').textContent = `${pct}%`;

  renderChart(last7);
}

function renderChart(days) {
  const chart = document.getElementById('chart');
  chart.innerHTML = '';

  const maxShown = Math.max(1, ...days.map(d => d.shown));

  days.forEach(d => {
    const group = document.createElement('div');
    group.className = 'chart-bar-group';

    const bars = document.createElement('div');
    bars.className = 'chart-bars';

    const shownH = Math.round((d.shown / maxShown) * 64);
    const learnedH = d.shown > 0 ? Math.round((d.learned / maxShown) * 64) : 0;

    const bShown = document.createElement('div');
    bShown.className = 'bar shown';
    bShown.style.height = `${shownH}px`;
    bShown.title = `${L['stats.shown'] ?? 'shown'}: ${d.shown}`;

    const bLearned = document.createElement('div');
    bLearned.className = 'bar learned';
    bLearned.style.height = `${learnedH}px`;
    bLearned.title = `${L['stats.learned'] ?? 'learned'}: ${d.learned}`;

    bars.appendChild(bShown);
    bars.appendChild(bLearned);

    const label = document.createElement('div');
    label.className = 'chart-date';
    const [, month, day] = d.date.split('-');
    label.textContent = `${day}/${month}`;

    group.appendChild(bars);
    group.appendChild(label);
    chart.appendChild(group);
  });
}

// ── Init ───────────────────────────────────────────────────

async function init() {
  // Load settings and locale in parallel
  const [settingsResult, localeData] = await Promise.all([
    window.api.getSettings(),
    window.api.getLocale()
  ]);
  currentSettings = settingsResult;

  // Apply locale first so all labels are translated before data is filled in
  applyLocale(localeData.strings);

  // Set language selector to current value
  document.getElementById('language').value = currentSettings.language ?? 'auto';

  setActivePosition(currentSettings.position ?? 'center');

  const setRange = (id, key, decimals = 0) => {
    const input = document.getElementById(id);
    const valEl = document.getElementById(`${id}-val`);
    const v = currentSettings[key];
    input.value = v;
    valEl.textContent = decimals > 0 ? parseFloat(v).toFixed(decimals) : v;
  };

  setRange('fontSize', 'fontSize');
  setRange('showDuration', 'showDuration');
  setRange('interval', 'interval');
  setRange('fadeDuration', 'fadeDuration', 1);

  renderSmartScheduling();
  document.getElementById('autoStart').checked = currentSettings.autoStart ?? true;

  await loadDictionaries();
  await loadLearnedList();

  // Real-time stats updates pushed from main process
  window.api.onStatsUpdated(() => {
    if (document.getElementById('tab-stats').classList.contains('active')) {
      loadStats();
    }
  });

  // Auto-switch: main process switched activeDictionary — refresh UI
  window.api.onDictAutoSwitched(({ newId }) => {
    currentSettings.activeDictionary = newId;
    settingsRevision++;
    loadDictionaries();
    loadLearnedList();
  });

  // Live locale updates when user changes language
  window.api.onLocaleUpdated(({ strings }) => {
    applyLocale(strings);
    // Refresh dictionary list to update "(import)" label in current locale
    loadDictionaries();
  });

  // Dict list changed (e.g. import from onboarding) — reload dropdown
  window.api.onDictListUpdated(() => {
    if (myWordsManager.open && !myWordsBusy) loadMyWords();
    return refreshSettingsData();
  });

  // Fallback: refresh data every time the settings window receives focus
  window.addEventListener('focus', refreshSettingsData);
}

async function refreshSettingsData() {
  const revision = settingsRevision;
  await saveQueue.catch(() => {});
  const settings = await window.api.getSettings();
  if (revision !== settingsRevision) return;
  currentSettings = { ...settings, ...pendingSettingsPatch };
  renderAnswerDelay();
  renderSmartScheduling();
  document.getElementById('autoStart').checked = currentSettings.autoStart ?? true;
  await loadDictionaries();
  await loadLearnedList();
}

init();
