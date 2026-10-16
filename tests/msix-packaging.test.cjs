const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { validateVersion, manifestFor, mappingText, verifyPayload, parseArgs, FILES } = require('../scripts/build-msix.cjs');

test('MSIX preserves Store identity, application ID and executable location', () => {
  const xml = manifestFor('1.0.2.0');
  assert.match(xml, /Name="DZMITRYSALAVEIMORNINGSTAR\.LazyWords"/);
  assert.match(xml, /Publisher="CN=3048F451-806C-4706-9A09-F8DB743D657E"/);
  assert.match(xml, /Version="1\.0\.2\.0" ProcessorArchitecture="x64"/);
  assert.match(xml, /Id="LAZYWORDS"/);
  assert.match(xml, /Executable="VFS\\ProgramFilesX64\\LazyWords\\lazywords\.exe"/);
  assert.doesNotMatch(xml, /__PACKAGE_VERSION__|WindowsAppRuntime|windows\.shortcut/);
});

test('MSIX Store version validation rejects malformed, reserved and oversized values', () => {
  assert.equal(validateVersion('1.0.2.0'), '1.0.2.0');
  for (const value of ['', '1.0.2', '0.1.0.0', '1.0.2.1', '1.65536.0.0', '1.0.0.0"/>']) {
    assert.throws(() => validateVersion(value));
  }
});

test('MSIX startup task uses the same stable ID as the runtime bridge', () => {
  const xml = manifestFor('1.0.2.0');
  const runtime = fs.readFileSync(path.join(__dirname, '../src-tauri/src/autostart.rs'), 'utf8');
  const id = runtime.match(/STARTUP_TASK_ID: &str = "([^"]+)"/)[1];
  assert.equal((xml.match(/Category="windows\.startupTask"/g) || []).length, 1);
  assert.match(xml, /xmlns:desktop="http:\/\/schemas\.microsoft\.com\/appx\/manifest\/desktop\/windows10"/);
  assert.match(xml, new RegExp(`TaskId="${id}" Enabled="false" DisplayName="LazyWords"`));
  assert.match(xml, /desktop:Extension Category="windows\.startupTask"\s+Executable="VFS\\ProgramFilesX64\\LazyWords\\lazywords\.exe"\s+EntryPoint="Windows.FullTrustApplication"/);
});

test('MSIX payload is a fixed allowlist without user data, caches or uninstall helpers', () => {
  assert.equal(FILES.length, 9);
  for (const [source, destination] of FILES) {
    assert.doesNotMatch(source + destination, /AppData|EBWebView|settings\.json|learned\.json|stats\.json|\.dll|\.pdb|\.pfx|\.cer/i);
  }
});

test('MSIX mapping rejects injection, duplicate and unsafe target paths', () => {
  assert.match(mappingText([{ source: 'C:\\a b\\icon.png', destination: 'Assets/icon.png' }]), /"Assets\\icon\.png"/);
  for (const destination of ['../secrets', '/absolute', '\\absolute', 'C:/absolute', 'a"\nb']) {
    assert.throws(() => mappingText([{ source: 'x', destination }]));
  }
  assert.throws(() => mappingText([{ source: 'x"\ny', destination: 'a' }]));
  assert.throws(() => mappingText([{ source: 'x', destination: 'A' }, { source: 'y', destination: 'a' }]));
});

test('MSIX validation rejects extra payload files and altered bytes', (t) => {
  const temporaryRoot = fs.realpathSync(os.tmpdir());
  const dir = fs.mkdtempSync(path.join(temporaryRoot, 'lazywords-msix-test-'));
  t.after(() => {
    if (path.dirname(dir) !== temporaryRoot || !path.basename(dir).startsWith('lazywords-msix-test-')) {
      throw new Error('Refusing cleanup outside the generated test directory');
    }
    fs.rmSync(dir, { recursive: true, force: true });
  });
  fs.writeFileSync(path.join(dir, 'payload'), 'expected');
  const entries = [{ destination: 'payload', sha256: crypto.createHash('sha256').update('expected').digest('hex') }];
  verifyPayload(dir, entries);
  fs.writeFileSync(path.join(dir, 'stats.json'), '{}');
  assert.throws(() => verifyPayload(dir, entries), /allowlist/);
  fs.unlinkSync(path.join(dir, 'stats.json'));
  fs.writeFileSync(path.join(dir, 'payload'), 'changed');
  assert.throws(() => verifyPayload(dir, entries), /bytes changed/);
});

test('MSIX CLI rejects unknown, duplicate and incomplete options', () => {
  assert.deepEqual(parseArgs(['--version', '1.0.2.0']), { version: '1.0.2.0' });
  for (const args of [['--upload', 'yes'], ['--version'], ['--version', 'a', '--version', 'b']]) {
    assert.throws(() => parseArgs(args));
  }
});
