const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

function onboarding(overrides = {}) {
  const completed = [];
  const buttons = ['en-ru', 'en-definitions', 'en-irregular-verbs'].map(pack => ({
    dataset: { pack }, disabled: false,
    addEventListener(name, callback) { this[name] = callback; },
  }));
  const importButton = { disabled: false, addEventListener(name, callback) { this[name] = callback; } };
  const status = { textContent: '' };
  const context = vm.createContext({
    window: {
      close() { assert.fail('Native window must be closed by the backend, not DOM window.close'); },
      api: {
        getLocale: async () => ({ strings: {} }),
        completeOnboarding: async id => { completed.push(id); },
        importDictionary: async () => null,
        saveSettings() { assert.fail('Selection and completion must use a single backend command'); },
        ...overrides,
      },
    },
    document: {
      getElementById: id => id === 'status' ? status : importButton,
      querySelectorAll: selector => selector === '.pack-btn' ? [...buttons, importButton]
        : selector === '.pack-btn[data-pack]' ? buttons : [],
    },
  });
  vm.runInContext(readFileSync(path.join(__dirname, '../src/windows/onboarding/onboarding.js'), 'utf8'), context);
  return { buttons, importButton, status, completed };
}

test('each starter pack completes through the native backend without DOM close', async () => {
  for (let index = 0; index < 3; index++) {
    const app = onboarding();
    await app.buttons[index].click();
    assert.deepEqual(app.completed, [app.buttons[index].dataset.pack]);
    assert.ok([...app.buttons, app.importButton].every(button => button.disabled));
  }
});

test('a pending completion prevents duplicate selection', async () => {
  let finish;
  const calls = [];
  const app = onboarding({ completeOnboarding: id => new Promise(resolve => { calls.push(id); finish = resolve; }) });
  const first = app.buttons[0].click();
  await app.buttons[1].click();
  assert.deepEqual(calls, ['en-ru']);
  finish();
  await first;
});

test('cancelled import leaves onboarding retryable without completing it', async () => {
  const app = onboarding();
  await app.importButton.click();
  assert.equal(app.status.textContent, '');
  assert.ok([...app.buttons, app.importButton].every(button => !button.disabled));
  assert.deepEqual(app.completed, []);
  await app.buttons[0].click();
  assert.deepEqual(app.completed, ['en-ru']);
});

test('successful import completes using the returned dictionary id', async () => {
  const app = onboarding({ importDictionary: async () => ({ id: 'custom', wordCount: 2 }) });
  await app.importButton.click();
  assert.deepEqual(app.completed, ['custom']);
});

test('invalid import displays an error and leaves starter packs available', async () => {
  const app = onboarding({ importDictionary: async () => ({ error: 'Invalid CSV' }) });
  await app.importButton.click();
  assert.equal(app.status.textContent, 'Error: Invalid CSV');
  assert.ok(app.buttons.every(button => !button.disabled));
  assert.deepEqual(app.completed, []);
});

test('picker rejection does not strand disabled buttons', async () => {
  const app = onboarding({ importDictionary: async () => { throw new Error('Picker unavailable'); } });
  await app.importButton.click();
  assert.equal(app.status.textContent, 'Error: Picker unavailable');
  assert.equal(app.importButton.disabled, false);
});

test('failed persistence or native completion leaves selection retryable', async () => {
  let calls = 0;
  const app = onboarding({ completeOnboarding: async () => {
    if (++calls === 1) throw new Error('Failed to save dictionary selection');
  } });
  await app.buttons[0].click();
  assert.equal(app.status.textContent, 'Error: Failed to save dictionary selection');
  assert.equal(app.buttons[0].disabled, false);
  await app.buttons[0].click();
  assert.equal(calls, 2);
});

test('failed completion of an imported dictionary re-enables all choices', async () => {
  const app = onboarding({
    importDictionary: async () => ({ id: 'custom' }),
    completeOnboarding: async () => { throw 'Write failed'; },
  });
  await app.importButton.click();
  assert.equal(app.status.textContent, 'Error: Write failed');
  assert.ok([...app.buttons, app.importButton].every(button => !button.disabled));
});
