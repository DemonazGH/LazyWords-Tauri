const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

const ROOT = path.resolve(__dirname, '..');
const APP = 'VFS/ProgramFilesX64/LazyWords';
const FILES = [
  ['src-tauri/icons/icon.png', `${APP}/icons/icon.png`],
  ['src/dictionaries/en-ru.csv', `${APP}/dictionaries/en-ru.csv`],
  ['src/dictionaries/en-definitions.csv', `${APP}/dictionaries/en-definitions.csv`],
  ['src/dictionaries/en-irregular-verbs.csv', `${APP}/dictionaries/en-irregular-verbs.csv`],
  ['src/locales/en.json', `${APP}/locales/en.json`],
  ['src/locales/ru.json', `${APP}/locales/ru.json`],
  ['src-tauri/icons/StoreLogo.png', 'Assets/StoreLogo.png'],
  ['src-tauri/icons/Square150x150Logo.png', 'Assets/Square150x150Logo.png'],
  ['src-tauri/icons/Square44x44Logo.png', 'Assets/Square44x44Logo.png'],
];

function validateVersion(version) {
  if (!/^[1-9]\d*\.\d+\.\d+\.0$/.test(version || '') ||
      version.split('.').some((part) => Number(part) > 65535)) {
    throw new Error('Use a four-part Store version with revision 0, e.g. 1.0.2.0');
  }
  return version;
}

function hash(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function regularFile(file) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0) {
    throw new Error(`Expected a nonempty regular file: ${file}`);
  }
  if (fs.realpathSync(file).toLowerCase() !== path.resolve(file).toLowerCase()) {
    throw new Error(`Refusing redirected input: ${file}`);
  }
  return file;
}

function x64Executable(file) {
  const data = fs.readFileSync(regularFile(file));
  if (data.length < 64 || data.toString('ascii', 0, 2) !== 'MZ') {
    throw new Error('Input is not a Windows executable');
  }
  const pe = data.readUInt32LE(0x3c);
  if (pe + 24 > data.length || data.readUInt32LE(pe) !== 0x4550 ||
      data.readUInt16LE(pe + 4) !== 0x8664 ||
      (data.readUInt16LE(pe + 22) & 0x2000)) {
    throw new Error('Input must be an x64 PE executable, not a DLL');
  }
}

function manifestFor(version) {
  const template = fs.readFileSync(path.join(ROOT, 'packaging/msix/AppxManifest.xml'), 'utf8');
  if (template.split('__PACKAGE_VERSION__').length !== 2) {
    throw new Error('Manifest must contain exactly one version placeholder');
  }
  return template.replace('__PACKAGE_VERSION__', validateVersion(version));
}

function mappingText(entries) {
  const destinations = new Set();
  return '[Files]\r\n' + entries.map(({ source, destination }) => {
    if (/["\r\n]/.test(source) || /["\r\n]/.test(destination) ||
        path.win32.isAbsolute(destination) || destination.includes('..') || destination.includes(':')) {
      throw new Error('Unsafe mapping path');
    }
    const target = destination.replaceAll('/', '\\');
    if (destinations.has(target.toLowerCase())) throw new Error('Duplicate package destination');
    destinations.add(target.toLowerCase());
    return `"${source}" "${target}"`;
  }).join('\r\n') + '\r\n';
}

function sdkTool(explicit) {
  if (explicit) return regularFile(path.resolve(explicit));
  const base = path.join(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'Windows Kits/10/bin');
  const versions = fs.readdirSync(base).filter((v) => /^10\.0\.\d+\.\d+$/.test(v))
    .sort((a, b) => b.localeCompare(a, 'en', { numeric: true }));
  for (const version of versions) {
    const tool = path.join(base, version, 'x64/makeappx.exe');
    if (fs.existsSync(tool)) return tool;
  }
  throw new Error('Windows SDK MakeAppx.exe not found; pass --makeappx explicitly');
}

function walk(directory, prefix = '') {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((item) => {
    const name = prefix + item.name;
    if (item.isSymbolicLink()) throw new Error('Unexpected link in unpacked package');
    return item.isDirectory() ? walk(path.join(directory, item.name), name + '/') : [name];
  });
}

function verifyPayload(directory, entries) {
  const generated = new Set(['AppxBlockMap.xml', '[Content_Types].xml']);
  const actual = walk(directory).filter((name) => !generated.has(name)).sort();
  const expected = entries.map((entry) => entry.destination).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error('Unpacked files differ from the explicit payload allowlist');
  }
  for (const entry of entries) {
    if (hash(regularFile(path.join(directory, entry.destination))) !== entry.sha256) {
      throw new Error(`Packaged bytes changed: ${entry.destination}`);
    }
  }
}

function build(options) {
  if (process.platform !== 'win32') throw new Error('MSIX packaging requires Windows SDK');
  const version = validateVersion(options.version);
  if (!options.exe || !options.output || !/^[a-f0-9]{64}$/i.test(options['exe-sha256'] || '')) {
    throw new Error('Required: --version --exe --exe-sha256 --output');
  }
  const exe = path.resolve(options.exe);
  x64Executable(exe);
  if (hash(exe) !== options['exe-sha256'].toLowerCase()) throw new Error('EXE SHA256 mismatch');
  const output = path.resolve(options.output);
  if (fs.existsSync(output)) throw new Error('Output directory must not already exist');
  const tool = sdkTool(options.makeappx);
  const entries = FILES.map(([source, destination]) => ({
    source: regularFile(path.join(ROOT, source)), destination,
  }));
  entries.push({ source: exe, destination: `${APP}/lazywords.exe` });
  fs.mkdirSync(output, { recursive: true });
  const manifest = path.join(output, 'AppxManifest.xml');
  fs.writeFileSync(manifest, manifestFor(version), { encoding: 'utf8', flag: 'wx' });
  entries.push({ source: manifest, destination: 'AppxManifest.xml' });
  for (const entry of entries) entry.sha256 = hash(entry.source);
  const mapping = path.join(output, 'mapping.txt');
  fs.writeFileSync(mapping, mappingText(entries), { encoding: 'utf8', flag: 'wx' });
  const packageFile = path.join(output, `DZMITRYSALAVEIMORNINGSTAR.LazyWords_${version}_x64.msix`);
  const run = (command, args) => {
    const result = spawnSync(tool, [command, ...args], { encoding: 'utf8', windowsHide: true });
    fs.writeFileSync(path.join(output, `${command}.log`), (result.stdout || '') + (result.stderr || ''));
    if (result.error || result.status !== 0) {
      throw new Error(`MakeAppx ${command} failed; see ${command}.log: ${result.error?.message || result.status}`);
    }
  };
  // Map only allowlisted files; never capture an installation or scan AppData.
  run('pack', ['/f', mapping, '/p', packageFile, '/h', 'SHA256', '/no']);
  const verified = path.join(output, 'verified');
  run('unpack', ['/p', packageFile, '/d', verified, '/no']);
  verifyPayload(verified, entries);
  const audit = {
    packageVersion: version, executableVersionIndependent: true,
    packageFile, packageSHA256: hash(packageFile), signed: false,
    payloadVerified: true, files: entries,
    scope: 'Packaging only; no native rebuild, signing, installation or publication',
  };
  fs.writeFileSync(path.join(output, 'payload-audit.json'), JSON.stringify(audit, null, 2) + '\n');
  return audit;
}

function parseArgs(args) {
  const result = {};
  const allowed = new Set(['version', 'exe', 'exe-sha256', 'output', 'makeappx']);
  for (let i = 0; i < args.length; i += 2) {
    const name = args[i].replace(/^--/, '');
    if (!args[i].startsWith('--') || !allowed.has(name) || !args[i + 1] || result[name]) {
      throw new Error(`Unknown, duplicate or incomplete option: ${args[i]}`);
    }
    result[name] = args[i + 1];
  }
  return result;
}

module.exports = { validateVersion, manifestFor, mappingText, verifyPayload, parseArgs, build, FILES };
if (require.main === module) {
  try {
    const audit = build(parseArgs(process.argv.slice(2)));
    console.log(`Verified ${audit.files.length} payload files: ${audit.packageFile}`);
    console.log(`SHA256 ${audit.packageSHA256}`);
    console.log('Unsigned candidate only. Store update, startup and WACK remain separate tests.');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
