const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

function renderer(file, api = {}) {
  const elements = new Map();
  const timers = new Map();
  const errors = [];
  let timerId = 0;
  function element() {
    const classes = new Set();
    return {
      value: '', style: {}, dataset: {}, children: [], listeners: {}, attributes: {},
      classList: {
        add: value => classes.add(value), remove: value => classes.delete(value),
        contains: value => classes.has(value),
        toggle: (value, on) => on ? classes.add(value) : classes.delete(value),
      },
      addEventListener(name, cb) { this.listeners[name] = cb; },
      setAttribute(name, value) { this.attributes[name] = value; },
      focus() { this.focused = true; },
      showModal() { this.open = true; },
      appendChild(child) { this.children.push(child); if (child.selected) this.value = child.value; },
      set innerHTML(value) { this.children = []; this.value = ''; },
      getBoundingClientRect: () => ({ width: 500, height: 360 }),
    };
  }
  const context = vm.createContext({
    console: { ...console, error: (...args) => errors.push(args) },
    window: { api, addEventListener() {} },
    document: {
      getElementById(id) {
        if (!elements.has(id)) elements.set(id, element());
        return elements.get(id);
      },
      querySelectorAll: () => [], querySelector: () => null, createElement: element,
    },
    setTimeout(cb, delay) { const id = ++timerId; timers.set(id, { cb, delay }); return id; },
    clearTimeout: id => timers.delete(id),
  });
  const source = readFileSync(path.join(__dirname, '..', file), 'utf8');
  vm.runInContext(source.replace(/init\(\);\s*$/, ''), context, { filename: file });
  return {
    run: code => vm.runInContext(code, context), elements, errors,
    async tick(delay) {
      for (const [id, timer] of [...timers]) {
        if (timer.delay === delay && timers.delete(id)) await timer.cb();
      }
    },
  };
}

test('Quick Add prefills a term without saving and only offers an exact suggestion', async () => {
  let draft = { term: 'One', suggestion: 'A number' };
  let opens;
  let closes = 0;
  const app = renderer('src/windows/quick-add/quick-add.js', {
    onQuickAddOpened: cb => { opens = cb; },
    onLocaleUpdated: () => {},
    getLocale: async () => ({ strings: {} }),
    takeQuickAddDraft: async () => { const next = draft; draft = null; return next; },
    closeQuickAdd: async () => { closes++; },
    saveQuickCard: () => assert.fail('Opening, suggesting and cancelling must never save'),
  });
  await app.run('init()');
  const input = app.elements.get('quick-add-term');
  const definition = app.elements.get('quick-add-definition');
  const suggestion = app.elements.get('quick-add-suggestion');
  assert.equal(input.value, 'One');
  assert.equal(definition.value, '');
  assert.equal(definition.focused, true);
  assert.equal(suggestion.hidden, false);
  input.value = 'Another';
  input.listeners.input();
  assert.equal(suggestion.hidden, true);
  input.value = ' one ';
  input.listeners.input();
  app.elements.get('quick-add-use').listeners.click();
  assert.equal(definition.value, 'A number');
  await opens();
  assert.equal(definition.value, 'A number', 'a duplicate open event keeps the draft');
  await app.elements.get('quick-add-form').listeners.keydown({ key: 'Escape', preventDefault() {} });
  assert.equal(closes, 1);
  assert.equal(definition.value, '');
  draft = { term: '', suggestion: null };
  await opens();
  assert.equal(input.value, '');
  assert.equal(input.focused, true);
});

test('Quick Add serializes saves, retains rejected drafts and closes on success', async () => {
  const saves = [];
  let finish;
  let closes = 0;
  const app = renderer('src/windows/quick-add/quick-add.js', {
    saveQuickCard: (term, definition) => {
      saves.push([term, definition]);
      return new Promise((resolve, reject) => { finish = { resolve, reject }; });
    },
    closeQuickAdd: async () => { closes++; },
  });
  const term = app.elements.get('quick-add-term');
  const definition = app.elements.get('quick-add-definition');
  const message = app.elements.get('quick-add-status');
  const submit = () => app.elements.get('quick-add-form').listeners.submit({ preventDefault() {} });
  term.value = '  term  ';
  definition.value = '   ';
  await submit();
  assert.equal(saves.length, 0);
  definition.value = '  definition  ';
  const first = submit();
  await submit();
  assert.deepEqual(saves, [['term', 'definition']]);
  finish.reject('quickAdd.duplicate');
  await first;
  assert.match(message.textContent, /already in My Words/);
  assert.equal(definition.value, '  definition  ');
  assert.equal(closes, 0);

  definition.value = ' another definition ';
  const second = submit();
  finish.resolve();
  await second;
  assert.equal(closes, 1);
  assert.deepEqual(saves[1], ['term', 'another definition']);
  assert.equal(term.value, '');
  assert.equal(definition.value, '');
});

test('debounce preserves patches from different sliders', async () => {
  const saved = {};
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: async patch => Object.assign(saved, patch),
  });
  app.run('saveDebounced({fontSize: 28}); saveDebounced({interval: 5});');
  await app.tick(200);
  assert.deepEqual(saved, { fontSize: 28, interval: 5 });
});

test('legacy marker review never links on close without a fresh confirmation', async () => {
  const calls = [];
  let response = { dictId: 'deck', words: [], pendingTotal: 3, pendingHere: 2, orderToken: 'order-a' };
  const app = renderer('src/windows/settings/settings.js', {
    getLearnedList: async () => response,
    resolveLegacyLearned: async (...args) => { calls.push(args); return 0; },
  });
  app.run('currentSettings.activeDictionary = "deck"');
  await app.run('loadLearnedList()');
  const dialog = app.elements.get('learned-legacy-dialog');
  const button = app.elements.get('btn-review-legacy');
  assert.match(app.elements.get('learned-legacy-count').textContent, /3 \(2 in this dictionary\)/);
  button.listeners.click();
  assert.equal(dialog.open, true);
  await dialog.listeners.close();
  assert.deepEqual(calls, []);

  button.listeners.click();
  response = { ...response, orderToken: 'order-b' };
  await app.run('loadLearnedList()');
  dialog.returnValue = 'confirm';
  await dialog.listeners.close();
  assert.deepEqual(calls, [['deck', 'order-a']], 'only the reviewed order can be linked');
  assert.equal(dialog.returnValue, '');
  assert.match(app.elements.get('learned-status').textContent, /remain pending/);
});

test('smart scheduling is opt-in and reverts to the last saved state after failure', async () => {
  const patches = [];
  let finish;
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: patch => {
      patches.push(patch);
      return new Promise(resolve => { finish = resolve; });
    },
  });
  const checkbox = app.elements.get('smartScheduling');
  const change = () => checkbox.listeners.change({ target: checkbox });
  app.run('renderSmartScheduling()');
  assert.equal(checkbox.checked, false);

  checkbox.checked = true;
  const enabled = change();
  assert.equal(checkbox.disabled, true);
  await Promise.resolve();
  await Promise.resolve();
  finish(true);
  await enabled;
  assert.equal(checkbox.checked, true);
  assert.equal(checkbox.disabled, false);

  checkbox.checked = false;
  const disabled = change();
  await Promise.resolve();
  await Promise.resolve();
  finish(false);
  await disabled;
  assert.equal(checkbox.checked, true);
  assert.equal(checkbox.disabled, false);
  assert.equal(app.elements.get('smart-scheduling-error').hidden, false);
  assert.deepEqual(JSON.parse(JSON.stringify(patches)), [
    { smartScheduling: true }, { smartScheduling: false },
  ]);
});

test('immediate save flushes pending sliders and newest value wins', async () => {
  const saved = {};
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: async patch => Object.assign(saved, patch),
  });
  app.run('saveDebounced({fontSize: 26, interval: 5});');
  await app.run('saveImmediate({fontSize: 28, position: "bottom-right"})');
  await app.tick(200);
  assert.deepEqual(saved, { fontSize: 28, interval: 5, position: 'bottom-right' });
});

test('dictionary list uses current selection when an older request completes', async () => {
  let finish;
  const app = renderer('src/windows/settings/settings.js', {
    getDictionaries: () => new Promise(resolve => { finish = resolve; }),
  });
  app.run('currentSettings.activeDictionary = "old"');
  const pending = app.run('loadDictionaries("old")');
  app.run('currentSettings.activeDictionary = "new"');
  finish([{ id: 'old', name: 'Old', source: 'bundled' }, { id: 'new', name: 'New', source: 'user' }]);
  await pending;
  assert.equal(app.elements.get('activeDictionary').value, 'new');
});

test('switching dictionary invalidates the displayed card before mark-known', async () => {
  const callbacks = {};
  const marked = [];
  const api = {
    getSettings: async () => ({ activeDictionary: 'old', showDuration: 15 }),
    getCardLayout: async () => ({ width: 500, maxHeight: 1000 }),
    resizeCard: async () => {},
    markLearned: (...args) => marked.push(args),
  };
  for (const name of ['onShowWord', 'onHideWord', 'onUpdateSettings', 'onAllLearned',
    'onSwitchedDict', 'onAllDictsLearned', 'onMarkKnown', 'onDictAutoSwitched']) {
    api[name] = cb => { callbacks[name] = cb; };
  }
  const app = renderer('src/windows/card/card.js', api);
  await app.run('init()');
  await callbacks.onShowWord({ term: 'old card', definition: 'old definition', cardId: 'old-id', dictId: 'old' });
  await callbacks.onUpdateSettings({ activeDictionary: 'new' });
  callbacks.onMarkKnown();
  assert.equal(marked.length, 0, 'old index must not be submitted for the new dictionary');
});

test('late getSettings response cannot replace a local dictionary change', async () => {
  let finish;
  const app = renderer('src/windows/settings/settings.js', {
    getSettings: () => new Promise(resolve => { finish = resolve; }),
    saveSettings: async () => true,
  });
  const pending = app.run('refreshSettingsData()');
  await Promise.resolve();
  await Promise.resolve();
  await app.run('saveImmediate({activeDictionary: "new"})');
  finish({ activeDictionary: 'old' });
  await pending;
  assert.equal(app.run('currentSettings.activeDictionary'), 'new');
});

test('settings writes are serialized, including immediate saves', async () => {
  const writes = [];
  const completions = [];
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: patch => new Promise(resolve => { writes.push({ ...patch }); completions.push(resolve); }),
  });
  const first = app.run('saveImmediate({fontSize: 26})');
  const second = app.run('saveImmediate({fontSize: 28})');
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(writes, [{ fontSize: 26 }]);
  completions[0](true);
  await first;
  await Promise.resolve();
  assert.deepEqual(writes, [{ fontSize: 26 }, { fontSize: 28 }]);
  completions[1](true);
  await second;
});

test('failed settings save does not show a success indicator and the queue recovers', async () => {
  let success = false;
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: async () => success,
  });
  app.elements.get('autoStart').checked = true;
  assert.equal(await app.run('saveImmediate({autoStart: true})'), false);
  assert.equal(app.elements.get('autosave-tag').classList.contains('visible'), false);
  success = true;
  assert.equal(await app.run('saveImmediate({autoStart: false})'), true);
  assert.equal(app.elements.get('autosave-tag').classList.contains('visible'), true);
});

for (const reject of [false, true]) {
  test(`autostart failure restores actual OS state and shows an error (reject=${reject})`, async () => {
    const app = renderer('src/windows/settings/settings.js', {
      saveSettings: async () => { if (reject) throw new Error('blocked'); return false; },
      getSettings: async () => ({ autoStart: false }),
    });
    const checkbox = app.elements.get('autoStart');
    checkbox.checked = true;
    await checkbox.listeners.change({ target: checkbox });
    assert.equal(checkbox.checked, false);
    assert.equal(checkbox.disabled, false);
    assert.equal(app.elements.get('autostart-error').hidden, false);
    assert.equal(app.errors.length, 1);
    assert.match(app.elements.get('autostart-error').textContent, /Could not change startup/);
    assert.equal(app.elements.get('autosave-tag').classList.contains('visible'), false);
  });
}

test('settings focus refresh reflects externally disabled startup', async () => {
  const app = renderer('src/windows/settings/settings.js', {
    getSettings: async () => ({ autoStart: false, activeDictionary: 'test' }),
    getDictionaries: async () => [],
    getLearnedList: async () => ({ dictId: 'test', words: [] }),
  });
  app.elements.get('autoStart').checked = true;
  await app.run('refreshSettingsData()');
  assert.equal(app.elements.get('autoStart').checked, false);
});

test('autostart toggle remains disabled until successful persistence finishes', async () => {
  let finish;
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: () => new Promise(resolve => { finish = resolve; }),
  });
  const checkbox = app.elements.get('autoStart');
  checkbox.checked = true;
  const pending = checkbox.listeners.change({ target: checkbox });
  assert.equal(checkbox.disabled, true);
  assert.equal(app.elements.get('autostart-error').hidden, true);
  await Promise.resolve();
  await Promise.resolve();
  finish(true);
  await pending;
  assert.equal(checkbox.disabled, false);
  assert.equal(checkbox.checked, true);
  assert.equal(app.elements.get('autosave-tag').classList.contains('visible'), true);
});

test('card grows to measured content and submits source dictionary only while visible', async () => {
  const callbacks = {};
  const heights = [];
  const marked = [];
  const api = {
    getSettings: async () => ({ activeDictionary: 'old', showDuration: 15 }),
    getCardLayout: async () => ({ width: 500, maxHeight: 1000 }),
    resizeCard: async height => heights.push(height),
    markLearned: (...args) => marked.push(args),
  };
  for (const name of ['onShowWord', 'onHideWord', 'onUpdateSettings', 'onAllLearned',
    'onSwitchedDict', 'onAllDictsLearned', 'onMarkKnown', 'onDictAutoSwitched']) {
    api[name] = cb => { callbacks[name] = cb; };
  }
  const app = renderer('src/windows/card/card.js', api);
  await app.run('init()');
  await callbacks.onShowWord({ term: 'long card', definition: 'long text', cardId: 'long-id', dictId: 'old' });
  assert.deepEqual(heights, [360]);
  callbacks.onMarkKnown();
  callbacks.onMarkKnown();
  assert.deepEqual(marked, [['long-id', 'old']]);
  callbacks.onDictAutoSwitched({ newId: 'new' });
  await callbacks.onShowWord({ term: 'new card', definition: 'new text', cardId: 'new-id', dictId: 'new' });
  callbacks.onMarkKnown();
  assert.deepEqual(marked, [['long-id', 'old'], ['new-id', 'new']]);
});

test('answer delay displays a localized value and saves the number of seconds', async () => {
  const writes = [];
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: async patch => { writes.push({ ...patch }); return true; },
  });
  const input = app.elements.get('answerDelay');
  app.run('L = {"settings.answerDelay.off":"Выкл.", "settings.showDuration.unit":"сек"}; renderAnswerDelay()');
  assert.equal(input.attributes['aria-valuetext'], 'Выкл.');
  input.value = '4';
  input.listeners.input({ target: input });
  assert.equal(app.elements.get('answerDelay-val').textContent, '4 сек');
  assert.equal(input.attributes['aria-valuetext'], '4 сек');
  await app.tick(200);
  assert.deepEqual(writes, [{ answerDelay: 4 }]);
});

async function delayedCard(apiOverrides = {}) {
  const callbacks = {};
  const api = {
    getSettings: async () => ({ activeDictionary: 'deck', showDuration: 4, answerDelay: 2 }),
    getCardLayout: async () => ({ width: 500, maxHeight: 1000 }),
    resizeCard: async () => {},
    ...apiOverrides,
  };
  for (const name of ['onShowWord', 'onUpdateSettings', 'onAllLearned',
    'onSwitchedDict', 'onAllDictsLearned', 'onMarkKnown', 'onDictAutoSwitched']) {
    api[name] = cb => { callbacks[name] = cb; };
  }
  const app = renderer('src/windows/card/card.js', api);
  await app.run('init()');
  return {
    ...app, callbacks,
    show: (def, answerDelay = 2) => callbacks.onShowWord({
      term: def, definition: def, cardId: 'card-id', dictId: 'deck', showDuration: 4, answerDelay,
    }),
    visible: () => app.elements.get('card').classList.contains('visible'),
    pending: () => app.elements.get('definition').classList.contains('answer-pending'),
  };
}

test('delayed answer is measured before reveal and stays visible for the full show duration', async () => {
  const heights = [];
  const app = await delayedCard({ resizeCard: async height => heights.push(height) });
  await app.show('definition');
  assert.equal(app.visible(), true);
  assert.equal(app.pending(), true);
  assert.deepEqual(heights, [360]);
  await app.tick(4000);
  assert.equal(app.visible(), true, 'the old show duration does not hide a pending answer');
  await app.tick(2000);
  assert.equal(app.pending(), false);
  assert.equal(app.visible(), true);
  assert.deepEqual(heights, [360], 'reveal does not trigger another layout');
  await app.tick(4000);
  assert.equal(app.visible(), false);
});

test('a replacement card cancels both an old reveal and an old hide', async () => {
  const app = await delayedCard();
  await app.show('first');
  await app.show('second', 5);
  await app.tick(2000);
  assert.equal(app.pending(), true, 'the first reveal cannot reveal the second card');
  await app.tick(5000);
  assert.equal(app.pending(), false);
  await app.show('third');
  await app.tick(4000);
  assert.equal(app.visible(), true, 'the second hide cannot dismiss the third card');
  assert.equal(app.pending(), true);
  await app.tick(2000);
  assert.equal(app.pending(), false);
});

test('mark-known and dictionary changes cancel pending reveals; status cards do not wait', async () => {
  const marked = [];
  const app = await delayedCard({ markLearned: (...args) => marked.push(args) });
  await app.show('learned early');
  app.callbacks.onMarkKnown();
  assert.deepEqual(marked, [['card-id', 'deck']]);
  await app.tick(2000);
  assert.equal(app.visible(), false);
  assert.equal(app.pending(), true, 'answer stays hidden when the card fades');

  await app.show('old dictionary');
  app.callbacks.onUpdateSettings({ activeDictionary: 'another', answerDelay: 2, showDuration: 4 });
  await app.tick(2000);
  assert.equal(app.visible(), false);
  assert.equal(app.pending(), true);
  await app.callbacks.onAllLearned({ headline: 'Done', dictName: 'Deck' });
  assert.equal(app.visible(), true);
  assert.equal(app.pending(), false);
  await app.tick(4000);
  assert.equal(app.visible(), false);
});

test('off shows the answer immediately and uses the original duration', async () => {
  const app = await delayedCard();
  await app.show('definition', 0);
  assert.equal(app.visible(), true);
  assert.equal(app.pending(), false);
  await app.tick(4000);
  assert.equal(app.visible(), false);
});
