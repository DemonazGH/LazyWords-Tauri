let strings = {};
let busy = false;

window.api.getLocale().then(({ strings: localeStrings }) => {
  strings = localeStrings;
  document.querySelectorAll('[data-i18n]').forEach(el => {
    if (strings[el.dataset.i18n] !== undefined) el.textContent = strings[el.dataset.i18n];
  });
}).catch(() => {});

function setStatus(message) {
  document.getElementById('status').textContent = message;
}

function setBusy(value) {
  busy = value;
  document.querySelectorAll('.pack-btn').forEach(button => { button.disabled = value; });
}

function showError(error) {
  setBusy(false);
  setStatus('Error: ' + (error?.message ?? String(error)));
}

document.querySelectorAll('.pack-btn[data-pack]').forEach(button => {
  button.addEventListener('click', async () => {
    if (busy) return;
    setBusy(true);
    setStatus(strings['onboarding.loading'] ?? 'Loading...');
    try {
      await window.api.completeOnboarding(button.dataset.pack);
    } catch (error) {
      showError(error);
    }
  });
});

document.getElementById('btn-import').addEventListener('click', async () => {
  if (busy) return;
  setBusy(true);
  setStatus(strings['onboarding.loading'] ?? 'Loading...');
  try {
    const result = await window.api.importDictionary();
    if (!result) {
      setBusy(false);
      setStatus('');
      return;
    }
    if (result.error) throw new Error(result.error);
    await window.api.completeOnboarding(result.id);
  } catch (error) {
    showError(error);
  }
});
