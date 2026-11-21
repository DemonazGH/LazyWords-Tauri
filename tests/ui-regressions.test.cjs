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
      removeAttribute(name) { delete this.attributes[name]; },
      focus() { this.focused = true; },
      showModal() { this.open = true; },
      close() { this.open = false; },
      replaceChildren(...children) { this.children = children; },
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

test('Quick Add offers labeled choices for copied and typed words without changing a draft or saving', async () => {
  let draft = { term: 'One' };
  let opens;
  let closes = 0;
  const app = renderer('src/windows/quick-add/quick-add.js', {
    onQuickAddOpened: cb => { opens = cb; },
    onLocaleUpdated: () => {},
    getLocale: async () => ({ strings: {} }),
    takeQuickAddDraft: async () => { const next = draft; draft = null; return next; },
    quickAddSuggestions: async term => term.toLowerCase() === 'one' ? [
      { source: 'en-definitions', answer: 'A number' }, { source: 'en-ru', answer: 'один' },
    ] : [],
    closeQuickAdd: async () => { closes++; },
    saveQuickCard: () => assert.fail('Opening, suggesting and cancelling must never save'),
  });
  await app.run('init()');
  const input = app.elements.get('quick-add-term');
  const definition = app.elements.get('quick-add-definition');
  const suggestion = app.elements.get('quick-add-suggestion');
  const options = app.elements.get('quick-add-options');
  assert.equal(input.value, 'One');
  assert.equal(definition.value, '');
  assert.equal(definition.focused, true);
  assert.equal(suggestion.hidden, false);
  assert.deepEqual(options.children.map(button => button.type), ['button', 'button']);
  assert.deepEqual(options.children.map(button => button.children[0].textContent),
    ['English definition', 'Russian translation']);
  const stale = options.children[0];
  definition.value = 'My own answer';
  input.value = 'Another';
  input.listeners.input();
  stale.listeners.click();
  assert.equal(definition.value, 'My own answer');
  await app.tick(180);
  assert.equal(suggestion.hidden, true);
  input.value = ' one ';
  input.listeners.input();
  await app.tick(180);
  assert.equal(definition.value, 'My own answer');
  options.children[1].listeners.click();
  assert.equal(definition.value, 'один');
  options.children[0].listeners.click();
  assert.equal(definition.value, 'A number');
  await opens();
  assert.equal(definition.value, 'A number', 'a duplicate open event keeps the draft');
  await app.elements.get('quick-add-form').listeners.keydown({ key: 'Escape', preventDefault() {} });
  assert.equal(closes, 1);
  assert.equal(definition.value, '');
  draft = { term: '' };
  await opens();
  assert.equal(input.value, '');
  assert.equal(input.focused, true);
});

test('Quick Add ignores late lookups after typing or cancel, and manual answers survive lookup failure', async () => {
  const pending = new Map();
  const app = renderer('src/windows/quick-add/quick-add.js', {
    quickAddSuggestions: term => new Promise((resolve, reject) => pending.set(term, { resolve, reject })),
    closeQuickAdd: async () => {},
  });
  const input = app.elements.get('quick-add-term');
  const options = app.elements.get('quick-add-options');
  const answer = app.elements.get('quick-add-definition');
  function type(term) { input.value = term; input.listeners.input(); }
  type('skipped');
  type('first');
  const first = app.tick(180);
  assert.equal(pending.has('skipped'), false, 'rapid typing debounces native lookups');
  type('second');
  const second = app.tick(180);
  answer.value = 'Manually typed';
  pending.get('second').resolve([{ source: 'en-ru', answer: 'Second answer' }]);
  await second;
  pending.get('first').resolve([{ source: 'en-definitions', answer: 'Stale answer' }]);
  await first;
  assert.equal(options.children.length, 1);
  assert.equal(options.children[0].children[1].textContent, 'Second answer');
  assert.equal(answer.value, 'Manually typed');
  type('third');
  const third = app.tick(180);
  pending.get('third').reject(new Error('unavailable'));
  await third;
  assert.equal(options.children.length, 0);
  assert.equal(answer.value, 'Manually typed');
  type('fourth');
  const fourth = app.tick(180);
  await app.elements.get('quick-add-cancel').listeners.click();
  pending.get('fourth').resolve([{ source: 'en-ru', answer: 'After close' }]);
  await fourth;
  assert.equal(options.children.length, 0);
  assert.equal(answer.value, '');
});

test('Quick Add serializes saves, retains rejected drafts and closes on success', async () => {
  const saves = [];
  let finish;
  let closes = 0;
  const app = renderer('src/windows/quick-add/quick-add.js', {
    saveQuickCard: (term, definition, practice) => {
      saves.push([term, definition, practice]);
      return new Promise((resolve, reject) => { finish = { resolve, reject }; });
    },
    closeQuickAdd: async () => { closes++; },
  });
  const term = app.elements.get('quick-add-term');
  const definition = app.elements.get('quick-add-definition');
  const message = app.elements.get('quick-add-status');
  const submit = submitter => app.elements.get('quick-add-form').listeners.submit({ submitter, preventDefault() {} });
  term.value = '  term  ';
  definition.value = '   ';
  await submit();
  assert.equal(saves.length, 0);
  definition.value = '  definition  ';
  const first = submit();
  const practice = app.elements.get('quick-add-practice');
  assert.equal(practice.disabled, true);
  await submit(practice);
  assert.deepEqual(saves, [['term', 'definition', false]]);
  finish.reject('quickAdd.duplicate');
  await first;
  assert.match(message.textContent, /already in My Words/);
  assert.equal(definition.value, '  definition  ');
  assert.equal(closes, 0);

  definition.value = ' another definition ';
  const second = submit(practice);
  finish.resolve();
  await second;
  assert.equal(closes, 1);
  assert.deepEqual(saves[1], ['term', 'another definition', true]);
  assert.equal(term.value, '');
  assert.equal(definition.value, '');
});

test('Quick Add distinguishes a failed switch from a failed card save', async () => {
  const app = renderer('src/windows/quick-add/quick-add.js', {
    saveQuickCard: async (_term, _definition, practice) => {
      assert.equal(practice, true);
      return 'savedWithoutSwitch';
    },
    closeQuickAdd: () => assert.fail('Keep the saved-card warning visible'),
  });
  app.elements.get('quick-add-term').value = 'saved';
  app.elements.get('quick-add-definition').value = 'once';
  await app.elements.get('quick-add-form').listeners.submit({
    submitter: app.elements.get('quick-add-practice'), preventDefault() {},
  });
  assert.match(app.elements.get('quick-add-status').textContent, /Card saved.*did not change/);
  assert.equal(app.elements.get('quick-add-term').value, '');
  assert.equal(app.elements.get('quick-add-practice').disabled, false);
});

test('Quick Add keeps ordinary Save as the default HTML submit button', () => {
  const html = readFileSync(path.join(__dirname, '../src/windows/quick-add/quick-add.html'), 'utf8');
  const buttons = html.match(/<button\b[^>]*>/gi)
    .filter(tag => !/\btype="(?:button|reset)"/i.test(tag));
  assert.deepEqual(buttons.map(tag => tag.match(/\bid="([^"]+)"/)[1]),
    ['quick-add-save', 'quick-add-practice']);
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

test('My Words edits target the displayed card ID and preserve a rejected draft', async () => {
  let cards = [{ cardId: 'old-id', term: '<word>', definition: 'Old answer' }];
  const saves = [];
  let complete;
  const app = renderer('src/windows/settings/settings.js', {
    getMyWords: async () => cards,
    editMyWord: (...args) => {
      saves.push(args);
      return new Promise((resolve, reject) => { complete = { resolve, reject }; });
    },
  });
  const manager = app.elements.get('my-words');
  manager.open = true;
  await manager.listeners.toggle();
  const list = app.elements.get('my-words-list');
  const edit = () => list.children[0].children[1].listeners.click();
  const dialog = app.elements.get('my-words-edit');
  const answer = app.elements.get('my-words-definition');
  const submit = () => app.elements.get('my-words-edit-form').listeners.submit({ preventDefault() {} });
  assert.equal(list.children[0].children[0].children[0].textContent, '<word>');
  edit();
  assert.equal(answer.value, 'Old answer');
  app.elements.get('my-words-cancel-edit').listeners.click();
  assert.equal(dialog.open, false);
  assert.deepEqual(saves, []);

  edit();
  answer.value = ' new answer ';
  const first = submit();
  await submit();
  assert.deepEqual(saves, [['old-id', '<word>', 'new answer']]);
  complete.reject('quickAdd.duplicate');
  await first;
  assert.match(app.elements.get('my-words-edit-status').textContent, /already in My Words/);
  assert.equal(dialog.open, true);
  assert.equal(answer.value, ' new answer ');
  assert.equal(answer.readOnly, false);

  const second = submit();
  cards = [{ cardId: 'new-id', term: '<word>', definition: 'new answer' }];
  complete.resolve();
  await second;
  assert.equal(dialog.open, false);
  assert.equal(list.children[0].children[0].children[1].textContent, 'new answer');
  assert.equal(app.elements.get('my-words-summary').focused, true);
});

test('My Words deletion only runs for the explicit confirm button', async () => {
  let cards = [{ cardId: 'last-card', term: 'Last', definition: 'Answer, with punctuation' }];
  const deletes = [];
  const app = renderer('src/windows/settings/settings.js', {
    getMyWords: async () => cards,
    deleteMyWord: async id => { deletes.push(id); cards = []; },
  });
  const manager = app.elements.get('my-words');
  manager.open = true;
  await manager.listeners.toggle();
  const list = app.elements.get('my-words-list');
  list.children[0].children[2].listeners.click();
  const dialog = app.elements.get('my-words-delete');
  assert.equal(dialog.open, true);
  assert.match(app.elements.get('my-words-delete-preview').textContent, /Answer, with punctuation/);
  const submit = submitter =>
    app.elements.get('my-words-delete-form').listeners.submit({ submitter, preventDefault() {} });
  await submit({ value: 'cancel' });
  dialog.close(); // Native method=dialog closes on Cancel.
  await submit({ id: 'my-words-confirm-delete' });
  list.children[0].children[2].listeners.click();
  await submit(undefined);
  assert.deepEqual(deletes, []);
  await submit({ id: 'my-words-confirm-delete' });
  assert.deepEqual(deletes, ['last-card']);
  assert.equal(list.hidden, true);
  assert.equal(dialog.open, false);
});

test('legacy marker review never links on close without a fresh confirmation', async () => {
  const calls = [];
  const response = { dictId: 'deck', words: [], pendingTotal: 3, pendingHere: 2, pendingDecks: { deck: 2, import: 1 } };
  let token = 'order-a';
  const app = renderer('src/windows/settings/settings.js', {
    getLearnedList: async () => response,
    previewLegacyLearned: async () => ({orderToken: token, sample: [{ row: 2, term: '<term>', definition: 'answer' }]}),
    resolveLegacyLearned: async (...args) => { calls.push(args); return 0; },
  });
  app.run('currentSettings.activeDictionary = "deck"');
  await app.run('loadLearnedList()');
  const dialog = app.elements.get('learned-legacy-dialog');
  const button = app.elements.get('btn-review-legacy');
  assert.match(app.elements.get('learned-legacy-count').textContent, /3 \(2 in the active dictionary\)/);
  app.elements.get('legacy-notice-review').listeners.click();
  const select = app.elements.get('legacy-deck');
  assert.equal(select.focused, true);
  select.value = 'import';
  await button.listeners.click();
  assert.equal(app.elements.get('legacy-preview').children[0].textContent, '#2: <term> · answer');
  assert.equal(dialog.open, true);
  const submit = id => app.elements.get('legacy-review-form').listeners.submit({ submitter: { id } });
  await submit('cancel');
  assert.deepEqual(calls, []);

  dialog.open = false;
  await button.listeners.click();
  dialog.listeners.cancel();
  await submit('legacy-review-confirm');
  assert.deepEqual(calls, [], 'an escaped preview no longer grants confirmation');
  dialog.open = false;
  await button.listeners.click();
  token = 'order-b';
  await app.run('loadLearnedList()');
  await submit('legacy-review-confirm');
  await submit('legacy-review-confirm');
  assert.deepEqual(calls, [['import', 'order-a']], 'only the reviewed deck and order can be linked');
  assert.equal(app.run('currentSettings.activeDictionary'), 'deck');
  assert.match(app.elements.get('learned-status').textContent, /remain pending/);
});

test('dismissing the notice hides no pending records and does not change practice decks', async () => {
  const writes = [];
  const app = renderer('src/windows/settings/settings.js', {
    getLearnedList: async () => ({ dictId: 'practice', words: [], pendingTotal: 3, pendingDecks: { missing: 3 } }),
    saveSettings: async patch => { writes.push(patch); return writes.length > 1; },
    previewLegacyLearned: async () => null,
  });
  app.run('currentSettings.activeDictionary = "practice"');
  await app.run('loadLearnedList()');
  assert.equal(app.elements.get('legacy-notice').hidden, false);
  await app.elements.get('btn-review-legacy').listeners.click();
  assert.match(app.elements.get('learned-status').textContent, /still saved/);
  await app.elements.get('legacy-notice-dismiss').listeners.click();
  assert.equal(app.elements.get('legacy-notice').hidden, false);
  await app.elements.get('legacy-notice-dismiss').listeners.click();
  assert.equal(app.elements.get('legacy-notice').hidden, true);
  assert.equal(app.elements.get('learned-legacy').hidden, false);
  assert.equal(app.elements.get('legacy-deck').value, 'missing');
  assert.deepEqual(writes.map(patch => Object.keys(patch)), [['legacyNoticeDismissed'], ['legacyNoticeDismissed']]);
});

test('a late preview cannot reopen review for a different dictionary', async () => {
  let finish;
  let response = { dictId: 'practice', words: [], pendingTotal: 2, pendingDecks: { first: 1, second: 1 } };
  const app = renderer('src/windows/settings/settings.js', {
    getLearnedList: async () => response,
    previewLegacyLearned: () => new Promise(resolve => { finish = resolve; }),
  });
  app.run('currentSettings.activeDictionary = "practice"');
  await app.run('loadLearnedList()');
  const open = app.elements.get('btn-review-legacy').listeners.click();
  const select = app.elements.get('legacy-deck');
  select.value = 'second';
  select.listeners.change();
  finish({ orderToken: 'stale', sample: [{ row: 1, term: 'old', definition: 'deck' }] });
  await open;
  assert.ok(!app.elements.get('learned-legacy-dialog').open);
  response = { dictId: 'practice', words: [], pendingTotal: 0, pendingDecks: {} };
  await app.run('loadLearnedList()');
  assert.equal(app.elements.get('legacy-notice').hidden, true);
  assert.equal(app.elements.get('learned-legacy').hidden, true);
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
