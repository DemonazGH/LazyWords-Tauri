# MSIX Candidate Packaging

This is a packaging-only path for the existing Store product `9PNTM7Q8VG8S`.
It does not rebuild Rust, run an installer, capture the machine, sign a package,
change certificate trust or publish anything. Ordinary MSI/NSIS builds are unchanged.

Run on Windows with Node.js and the Windows SDK:

```powershell
node scripts/build-msix.cjs --version 1.0.2.0 --exe C:\path\to\verified\lazywords.exe --exe-sha256 <SHA256> --output C:\path\to\new-output-directory
```

Use a deliberately selected, previously verified production x64 executable.
The script requires its exact SHA256; it does not infer that a stale release EXE
matches current sources. The package version is independent of the embedded
executable version. A new output directory is required; existing artifacts are
never overwritten. `--makeappx` can select a particular SDK tool explicitly.

Only eleven files are mapped: the manifest, EXE, three bundled dictionaries,
two locales, tray icon and three Store logos. The MSI cleanup DLL is an installer
helper, not an application runtime dependency, and is intentionally excluded.
Existing repository logos are reused without image conversion. MakeAppx semantic
validation stays enabled. After packing, the script unpacks and verifies the
exact file allowlist and every payload hash. `payload-audit.json` and tool logs
remain beside the unsigned `.msix`.

Identity, publisher, application ID `LAZYWORDS` and the executable's VFS path are
preserved from the historical manifest. No machine-captured registry hives,
user settings/progress/statistics/imports, WebView caches, secrets or uninstallers
are inputs. The old WindowsAppRuntime dependency is not declared: this native
Tauri executable does not use Windows App SDK; runtime/deployment verification
must still confirm prerequisites, notably WebView2. The VM's installed runtime
does not prove operation on machines without WebView2.

The manifest declares the stable desktop startup task `LazyWordsStartup` for the
same VFS executable. Build a fresh production EXE with the matching
`src-tauri/src/autostart.rs` bridge: an older EXE still uses the wrong Run-key
mechanism, so the manifest change alone is not a complete fix. Package identity
selects Windows StartupTask APIs; MSI/NSIS keep the existing autostart plugin.
Errors never fall back to Run-key writes inside the package. User/policy-disabled
tasks are respected and Settings reads the actual OS state. Saving a startup
change requires successful OS registration and settings persistence.
The backward-compatible `msixStartupInitialized` setting migrates an existing
saved preference only once. Later package starts read Windows' task state, so
external enable as well as disable is not overwritten by a stale preference.
The manifest starts the task disabled; first activation applies the saved
preference (default enabled) before marking migration complete. Installation
alone must not override an existing disabled preference.

Retest enabled AND disabled startup after real reboots, including external
disable in Windows Startup apps. Unit tests and a checked box are not proof of
login startup. A changed executable also requires fresh installer/update smoke
tests; previously pinned MSI/NSIS artifacts remain historical evidence only.

Before any release: test upgrade from the real Store baseline, data preservation,
startup, restart, coexistence, removal, and WACK. An unsigned Store candidate
cannot be installed directly for same-identity testing; local signing/trust is
a separate explicit step. Never use `-AllowUnsigned` with a changed publisher
identity and call it an update of the existing Store product. Keep an untouched
unsigned candidate and any locally signed test copy separate. Do not upload or
submit while preparation-only authorization is in effect.

References:
- https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool
- https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements
- https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide
- https://learn.microsoft.com/en-us/windows/msix/package/unsigned-package
- https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask
