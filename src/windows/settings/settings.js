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
  renderLearnedList();
  if (document.getElementById('tab-stats').classList.contains('active')) {
    loadStats();
  }
}

// ── Auto-save ───────────────────────────────────────────────────────────────

let debounceTimer = null;
let indicatorTimer = null;

function showSavedIndicator() {
  const tag = document.getElementById('autosave-tag');
  tag.classList.add('visible');
  clearTimeout(indicatorTimer);
  indicatorTimer = setTimeout(() => tag.classList.remove('visible'), 1000);
}

async function saveImmediate(patch) {
  Object.assign(currentSettings, patch);
  await window.api.saveSettings(patch);
  showSavedIndicator();
}

function saveDebounced(patch) {
  Object.assign(currentSettings, patch);
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(async () => {
    await window.api.saveSettings(patch);
    showSavedIndicator();
  }, 200);
}

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
  loadDictionaries(currentSettings.activeDictionary);
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

// ── Checkbox ───────────────────────────────────────────────

document.getElementById('autoStart').addEventListener('change', (e) => {
  saveImmediate({ autoStart: e.target.checked });
});

// ── Dictionary selector ────────────────────────────────────

let availableDicts = [];

async function loadDictionaries(selectedId) {
  availableDicts = await window.api.getDictionaries();
  const sel = document.getElementById('activeDictionary');
  sel.innerHTML = '';
  const importLabel = L['settings.import.label'] ?? 'import';
  for (const d of availableDicts) {
    const opt = document.createElement('option');
    opt.value = d.id;
    opt.textContent = d.source === 'bundled' ? d.name : `${d.name} (${importLabel})`;
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

document.getElementById('activeDictionary').addEventListener('change', (e) => {
  saveImmediate({ activeDictionary: e.target.value });
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
  await loadDictionaries(result.newActive);
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
    await loadDictionaries(result.id);
    saveImmediate({ activeDictionary: result.id });
    document.getElementById('activeDictionary').value = result.id;
  }

  setTimeout(() => msg.classList.remove('visible'), 4000);
});

// ── Learned list ───────────────────────────────────────────

const LEARNED_PAGE = 20;
let allLearnedWords = []; // newest first
let learnedShowAll = false;
let restoreAllConfirming = false;
let restoreAllTimer = null;

async function loadLearnedList() {
  const { words } = await window.api.getLearnedList();
  allLearnedWords = [...words].reverse(); // newest first
  renderLearnedList();
}

function renderLearnedList() {
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
        w.word.toLowerCase().includes(query) ||
        w.translation.toLowerCase().includes(query))
    : allLearnedWords;

  const expandedBySearch = query.length > 0;
  const visible = (learnedShowAll || expandedBySearch)
    ? filtered
    : filtered.slice(0, LEARNED_PAGE);

  list.classList.toggle('learned-list--expanded', learnedShowAll || expandedBySearch);
  list.style.display = filtered.length ? 'block' : 'none';
  list.innerHTML = '';

  const restoreLabel = L['settings.learned.restore'] ?? 'Restore';
  visible.forEach(({ word, translation, index }) => {
    const li = document.createElement('li');
    li.innerHTML = `
      <div class="word-pair">
        <span class="word">${escHtml(word)}</span>
        <span class="translation">${escHtml(translation)}</span>
      </div>
      <button class="btn-restore" data-index="${index}">${escHtml(restoreLabel)}</button>
    `;
    li.querySelector('.btn-restore').addEventListener('click', async (e) => {
      await window.api.removeLearned(parseInt(e.target.dataset.index, 10));
      await loadLearnedList();
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

document.getElementById('btn-restore-all').addEventListener('click', async () => {
  const btn = document.getElementById('btn-restore-all');
  if (!restoreAllConfirming) {
    restoreAllConfirming = true;
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
  await window.api.clearLearned();
  learnedShowAll = false;
  await loadLearnedList();
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

  document.getElementById('autoStart').checked = currentSettings.autoStart ?? true;

  await loadDictionaries(currentSettings.activeDictionary);
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
    loadDictionaries(newId);
    loadLearnedList();
  });

  // Live locale updates when user changes language
  window.api.onLocaleUpdated(({ strings }) => {
    applyLocale(strings);
    // Refresh dictionary list to update "(import)" label in current locale
    loadDictionaries(currentSettings.activeDictionary);
  });
}

init();
