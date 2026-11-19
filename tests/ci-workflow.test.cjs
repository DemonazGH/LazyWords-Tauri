const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const workflow = readFileSync(path.join(__dirname, '../.github/workflows/ci.yml'), 'utf8');
const lines = workflow.split(/\r?\n/);

function job(id) {
  const start = lines.indexOf(`  ${id}:`);
  assert.notEqual(start, -1, `Missing CI job: ${id}`);
  let end = start + 1;
  while (end < lines.length && !/^\S|^ {2}\S/.test(lines[end])) end++;
  return lines.slice(start, end).join('\n');
}

test('PR CI preserves main triggers and runs separate Linux and Windows checks', () => {
  assert.match(workflow, /^  push:\r?\n    branches: \[main\]/m);
  assert.match(workflow, /^  pull_request:\r?\n    branches: \[main\]/m);
  assert.match(job('test'), /^    runs-on: ubuntu-latest$/m);
  assert.match(job('windows-test'), /^    runs-on: windows-latest$/m);
  for (const id of ['test', 'windows-test']) {
    const config = job(id);
    assert.match(config, /^    timeout-minutes: 30$/m);
    assert.doesNotMatch(config, /^\s+(?:needs|if|continue-on-error):/m);
  }
});

test('each PR platform runs Node regressions and locked Rust tests without suppressing failure', () => {
  for (const id of ['test', 'windows-test']) {
    const config = job(id);
    assert.match(config, /uses: actions\/setup-node@v4/);
    assert.match(config, /^          node-version: 22$/m);
    assert.match(config, /^        run: npm ci --ignore-scripts$/m);
    assert.match(config, /^        run: npm test$/m);
    assert.match(config, /^        run: cargo test --locked\n        working-directory: src-tauri$/m);
    assert.doesNotMatch(config, /--if-present|\|\|\s*true|continue-on-error/);
  }
});

test('PR CI stays read-only and excludes packaging and release steps', () => {
  assert.match(workflow, /^permissions:\r?\n  contents: read$/m);
  assert.doesNotMatch(workflow, /^\s*[\w-]+:\s*write\s*$/m);
  assert.doesNotMatch(workflow, /pull_request_target|secrets\.|persist-credentials:\s*true/);
  assert.doesNotMatch(workflow,
    /action-gh-release|upload-artifact|(?:gh|git) (?:release|push|tag)|build-msix|signtool|tauri build|npm publish/);
});
