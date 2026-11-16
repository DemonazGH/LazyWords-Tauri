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
      value: '', style: {}, dataset: {}, children: [], listeners: {},
      classList: {
        add: value => classes.add(value), remove: value => classes.delete(value),
        contains: value => classes.has(value),
        toggle: (value, on) => on ? classes.add(value) : classes.delete(value),
      },
      addEventListener(name, cb) { this.listeners[name] = cb; },
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

test('debounce preserves patches from different sliders', async () => {
  const saved = {};
  const app = renderer('src/windows/settings/settings.js', {
    saveSettings: async patch => Object.assign(saved, patch),
  });
  app.run('saveDebounced({fontSize: 28}); saveDebounced({interval: 5});');
  await app.tick(200);
  assert.deepEqual(saved, { fontSize: 28, interval: 5 });
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
  await callbacks.onShowWord({ term: 'old card', definition: 'old definition', index: 0, dictId: 'old' });
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
  await callbacks.onShowWord({ term: 'long card', definition: 'long text', index: 1, dictId: 'old' });
  assert.deepEqual(heights, [360]);
  callbacks.onMarkKnown();
  callbacks.onMarkKnown();
  assert.deepEqual(marked, [[1, 'old']]);
  callbacks.onDictAutoSwitched({ newId: 'new' });
  await callbacks.onShowWord({ term: 'new card', definition: 'new text', index: 0, dictId: 'new' });
  callbacks.onMarkKnown();
  assert.deepEqual(marked, [[1, 'old'], [0, 'new']]);
});
