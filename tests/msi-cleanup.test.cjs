const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const root = path.join(__dirname, '../src-tauri');
const config = JSON.parse(readFileSync(path.join(root, 'tauri.conf.json'), 'utf8'));
const wix = readFileSync(path.join(root, 'windows/autostart-cleanup.wxs'), 'utf8');

test('MSI links the native cleanup fragment through its hidden feature', () => {
  assert.ok(config.bundle.windows.wix.fragmentPaths.includes('windows/autostart-cleanup.wxs'));
  assert.ok(config.bundle.windows.wix.featureRefs.includes('OwnedAutostartCleanup'));
  assert.match(wix, /<Feature Id="OwnedAutostartCleanup"[^>]*Display="hidden"/);
  assert.match(wix, /SourceFile="\$\(var.SourceDir\)\.autostart\.dll"/);
});

test('cleanup runs only on successful full uninstall, not upgrade or rollback', () => {
  assert.match(wix, /DllEntry="CleanupOwnedAutostart" Execute="commit"/);
  assert.match(wix, /Impersonate="yes" Return="ignore"/);
  assert.equal(wix.match(/Installed AND REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE/g).length, 2);
  assert.match(wix, /Value="\[UserSID\]\|\[#Path\]\|\[ProductName\]"/);
  assert.match(wix, /<Custom Action="CleanupOwnedAutostart" Before="RemoveFiles">/);
});
