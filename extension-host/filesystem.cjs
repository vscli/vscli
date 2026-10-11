'use strict';

// Consumer/error/provider contracts follow VS Code stable 1.141.0,
// 2a59476c9bfcb90b3ddc372c36762471b7dfad1c, extHostFileSystemConsumer.ts,
// extHostFileSystem.ts, extHostTypes.ts and platform/files/common/files.ts.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License; see filesystem.LICENSE.txt.
// Disk operations run asynchronously in the optional compatibility process.
// They do not edit or save the native document model.
const disk = require('node:fs/promises');
const path = require('node:path');
const { Uri, Disposable, EventEmitter } = require('./api-types.cjs');

function enumeration(entries) {
  const result = {};
  for (const [name, value] of Object.entries(entries)) { result[name] = value; result[value] = name; }
  return result;
}
const FileType = enumeration({ Unknown: 0, File: 1, Directory: 2, SymbolicLink: 64 });
const FilePermission = enumeration({ Readonly: 1 });
const FileChangeType = enumeration({ Changed: 1, Created: 2, Deleted: 3 });
const codes = {
  FileExists: 'EntryExists', FileNotFound: 'EntryNotFound', FileNotADirectory: 'EntryNotADirectory',
  FileIsADirectory: 'EntryIsADirectory', NoPermissions: 'NoPermissions', Unavailable: 'Unavailable',
};
class FileSystemError extends Error {
  constructor(messageOrUri, code = 'Unknown', terminator) {
    super(Uri.isUri?.(messageOrUri) || messageOrUri instanceof Uri ? messageOrUri.toString(true) : messageOrUri);
    this.code = terminator?.name ?? 'Unknown';
    this.name = `${code} (FileSystemError)`;
    Object.setPrototypeOf(this, FileSystemError.prototype);
    if (typeof Error.captureStackTrace === 'function' && typeof terminator === 'function') Error.captureStackTrace(this, terminator);
  }
  static FileExists(message) { return new FileSystemError(message, codes.FileExists, FileSystemError.FileExists); }
  static FileNotFound(message) { return new FileSystemError(message, codes.FileNotFound, FileSystemError.FileNotFound); }
  static FileNotADirectory(message) { return new FileSystemError(message, codes.FileNotADirectory, FileSystemError.FileNotADirectory); }
  static FileIsADirectory(message) { return new FileSystemError(message, codes.FileIsADirectory, FileSystemError.FileIsADirectory); }
  static NoPermissions(message) { return new FileSystemError(message, codes.NoPermissions, FileSystemError.NoPermissions); }
  static Unavailable(message) { return new FileSystemError(message, codes.Unavailable, FileSystemError.Unavailable); }
}
function normalizeError(error) {
  if (error instanceof FileSystemError) return error;
  if (!(error instanceof Error)) return new FileSystemError(String(error));
  const nativeCodes = { EEXIST: 'FileExists', ENOENT: 'FileNotFound', ENOTDIR: 'FileNotADirectory', EISDIR: 'FileIsADirectory', EACCES: 'NoPermissions', EPERM: 'NoPermissions', EBUSY: 'Unavailable' };
  const factory = nativeCodes[error.code] || Object.keys(codes).find(key => codes[key] === error.name);
  if (factory) return FileSystemError[factory](error.message);
  if (error.name === 'ENOPRO' || error.message.includes('ENOPRO')) return FileSystemError.Unavailable(error.message);
  return new FileSystemError(error.message, error.name);
}
function typeOf(stat) { return stat.isDirectory() ? FileType.Directory : stat.isFile() ? FileType.File : FileType.Unknown; }
function byteSnapshot(value) {
  if (!(value instanceof Uint8Array)) throw FileSystemError.Unavailable('Filesystem provider returned invalid file bytes');
  return new Uint8Array(value);
}
function localPath(uri) {
  if (uri.scheme !== 'file' || !uri.path || uri.path.includes('\0')) throw FileSystemError.Unavailable('Invalid local file URI');
  if (process.platform !== 'win32' && uri.authority && uri.authority.toLowerCase() !== 'localhost') {
    throw FileSystemError.Unavailable('Remote file authorities require a remote filesystem provider');
  }
  return uri.fsPath;
}
const diskProvider = {
  async stat(uri) {
    const name = localPath(uri);
    let stat = await disk.lstat(name), type = typeOf(stat);
    if (stat.isSymbolicLink()) {
      try { stat = await disk.stat(name); type = typeOf(stat); }
      catch (error) { if (error.code !== 'ENOENT') throw error; type = FileType.Unknown; }
      type |= FileType.SymbolicLink;
    }
    return { type, ctime: stat.birthtimeMs, mtime: stat.mtimeMs, size: stat.size };
  },
  async readDirectory(uri) {
    const children = await disk.readdir(localPath(uri), { withFileTypes: true });
    const result = [];
    // Sequential link resolution bounds I/O even for enormous directories.
    for (const child of children) {
      if (child.isSymbolicLink()) {
        try { result.push([child.name, (await this.stat(Uri.joinPath(uri, child.name))).type]); }
        catch { /* upstream omits individual unreadable entries */ }
      } else result.push([child.name, typeOf(child)]);
    }
    return result;
  },
  createDirectory: uri => disk.mkdir(localPath(uri)),
  async readFile(uri) { return new Uint8Array(await disk.readFile(localPath(uri))); },
  async writeFile(uri, bytes, options) { await disk.writeFile(localPath(uri), bytes, { flag: options.overwrite ? 'w' : 'wx' }); },
  async delete(uri, options) {
    const name = localPath(uri), stat = await disk.lstat(name);
    if (stat.isDirectory()) {
      if (options.recursive) await disk.rm(name, { recursive: true });
      else await disk.rmdir(name);
    } else await disk.unlink(name);
  },
  async rename(from, to, options) {
    const source = localPath(from), target = localPath(to);
    const caseRename = process.platform !== 'linux' && source.toLowerCase() === target.toLowerCase();
    if (!caseRename) {
      let previous;
      try { previous = await disk.lstat(target); } catch (error) { if (error.code !== 'ENOENT') throw error; }
      if (previous) {
        if (!options.overwrite) throw FileSystemError.FileExists(to);
        const original = await disk.lstat(source);
        if (!original.isFile() || !previous.isFile()) await disk.rm(target, { recursive: true });
      }
    }
    await disk.rename(source, target);
  },
  async copy(from, to, options) {
    let previous;
    try { previous = await disk.lstat(localPath(to)); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (previous) {
      if (!options.overwrite) throw FileSystemError.FileExists(to);
      const original = await disk.lstat(localPath(from));
      if (!original.isFile() || !previous.isFile()) await disk.rm(localPath(to), { recursive: true });
    }
    await disk.cp(localPath(from), localPath(to), { recursive: true, force: options.overwrite, errorOnExist: !options.overwrite, verbatimSymlinks: true });
  },
};

function createFileSystem({ assertOwner = () => {}, track = (_owner, value) => value } = {}) {
  const changed = new EventEmitter();
  const providers = new Map([['file', { impl: diskProvider, readonly: false, caseSensitive: process.platform === 'linux', builtin: true }]]);
  // Retain admission until actual completion, including never-settling providers.
  let pending = 0, queuedBytes = 0, mutationTail = Promise.resolve();
  const MAX_PENDING = 64, MAX_QUEUED_BYTES = 64 * 1024 * 1024;
  function lookup(uri, writable = false) {
    if (!uri || typeof uri.scheme !== 'string' || typeof uri.path !== 'string' || typeof uri.with !== 'function') throw new TypeError('Invalid URI');
    const provider = providers.get(uri.scheme);
    if (!provider) throw FileSystemError.Unavailable(`ENOPRO: No filesystem provider for '${uri.scheme}'`);
    if (!provider.builtin) assertOwner(provider.owner);
    if (writable && provider.readonly) throw FileSystemError.NoPermissions(`Filesystem '${uri.scheme}' is readonly`);
    return provider;
  }
  function invoke(entry, method, ...args) {
    if (!entry.builtin) {
      assertOwner(entry.owner);
      if (providers.get(args[0].scheme) !== entry) throw FileSystemError.Unavailable('Filesystem provider was disposed');
    }
    return entry.impl[method](...args);
  }
  function parent(uri) {
    let directory;
    if (uri.scheme === 'file') directory = Uri.file(path.dirname(localPath(uri))).path;
    else directory = path.posix.dirname(uri.path);
    return uri.with({ path: directory });
  }
  function key(uri, entry) {
    const normalized = uri.scheme === 'file' ? path.resolve(localPath(uri)).replaceAll(path.sep, '/') : uri.path;
    const name = entry.caseSensitive ? normalized : normalized.toLowerCase();
    return `${uri.scheme.toLowerCase()}://${uri.authority.toLowerCase()}${name}`;
  }
  async function mkdirp(entry, uri) {
    const directories = [];
    while (key(uri, entry) !== key(parent(uri), entry)) {
      try {
        if (!((await invoke(entry, 'stat', uri)).type & FileType.Directory)) throw FileSystemError.FileExists(uri);
        break;
      } catch (error) {
        if (normalizeError(error).code !== 'FileNotFound') throw error;
        directories.push(path.posix.basename(uri.path)); uri = parent(uri);
      }
    }
    for (const component of directories.reverse()) {
      uri = Uri.joinPath(uri, component);
      try { await invoke(entry, 'createDirectory', uri); }
      catch (error) { if (normalizeError(error).code !== 'FileExists') throw error; }
    }
  }
  async function exists(entry, uri) {
    try { return await invoke(entry, 'stat', uri); }
    catch (error) { if (normalizeError(error).code === 'FileNotFound') return undefined; throw error; }
  }
  async function copyTree(sourceEntry, source, targetEntry, target, overwrite, depth = 0) {
    if (depth >= 256) throw FileSystemError.Unavailable('Filesystem copy traversal is too deep');
    const stat = await invoke(sourceEntry, 'stat', source);
    const previous = await exists(targetEntry, target);
    if (previous && !overwrite) throw FileSystemError.FileExists(target);
    if (stat.type & FileType.Directory) {
      if (previous && !(previous.type & FileType.Directory)) await invoke(targetEntry, 'delete', target, { recursive: true });
      await mkdirp(targetEntry, target);
      for (const [name] of await invoke(sourceEntry, 'readDirectory', source)) {
        if (!name || name === '.' || name === '..' || /[\\/]/.test(name)) throw FileSystemError.Unavailable('Invalid filesystem directory entry');
        await copyTree(sourceEntry, Uri.joinPath(source, name), targetEntry, Uri.joinPath(target, name), overwrite, depth + 1);
      }
    } else {
      if (previous?.type & FileType.Directory) await invoke(targetEntry, 'delete', target, { recursive: true });
      await mkdirp(targetEntry, parent(target));
      const bytes = byteSnapshot(await invoke(sourceEntry, 'readFile', source));
      await invoke(targetEntry, 'writeFile', target, bytes, { create: true, overwrite });
    }
  }
  async function transfer(source, target, options, move) {
    const from = lookup(source, move), to = lookup(target, true), overwrite = options?.overwrite ?? false;
    const fromKey = key(source, from), toKey = key(target, to);
    if (source.toString() === target.toString()) return;
    await invoke(from, 'stat', source);
    const caseRename = from === to && fromKey === toKey && source.path !== target.path && move;
    if (from === to && fromKey === toKey && !caseRename) {
      if (move) return;
      throw FileSystemError.FileExists(target);
    }
    if (from === to && toKey.startsWith(`${fromKey.replace(/\/$/, '')}/`)) throw FileSystemError.Unavailable('Cannot copy or move a folder into itself');
    if (from === to && fromKey.startsWith(`${toKey.replace(/\/$/, '')}/`)) throw FileSystemError.Unavailable('Cannot overwrite an ancestor of the source');
    const previous = caseRename ? undefined : await exists(to, target);
    if (previous && !overwrite) throw FileSystemError.FileExists(target);
    if (previous && overwrite && from.builtin && to.builtin) {
      const sourcePath = await disk.realpath(localPath(source)), targetPath = await disk.realpath(localPath(target));
      const relative = path.relative(targetPath, sourcePath);
      if (relative === '' || relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)) {
        throw FileSystemError.Unavailable('Cannot overwrite a filesystem alias containing the source');
      }
    }
    await mkdirp(to, parent(target));
    if (from === to && (move || typeof from.impl.copy === 'function')) {
      await invoke(from, move ? 'rename' : 'copy', source, target, { overwrite });
    } else {
      if (previous && overwrite) await invoke(to, 'delete', target, { recursive: true });
      await copyTree(from, source, to, target, overwrite);
      // Delete only after all target writes actually acknowledge completion.
      if (move) await invoke(from, 'delete', source, { recursive: true });
    }
  }
  function operation(owner, mutation, call, bytes = 0, resources = [], prepare) {
    try {
      assertOwner(owner);
      const admitted = resources.map(uri => [uri, lookup(uri)]);
      if (pending >= MAX_PENDING || queuedBytes + bytes > MAX_QUEUED_BYTES) throw FileSystemError.Unavailable('Filesystem operation queue is full');
      pending++; queuedBytes += bytes;
      try { prepare?.(); }
      catch (error) { pending--; queuedBytes -= bytes; throw error; }
      const run = () => {
        assertOwner(owner);
        for (const [uri, entry] of admitted) {
          if (lookup(uri) !== entry) throw FileSystemError.Unavailable('Filesystem provider changed while the operation was queued');
        }
        return call();
      };
      const result = mutation ? mutationTail.then(run) : Promise.resolve().then(run);
      if (mutation) mutationTail = result.catch(() => {});
      return result.catch(error => { throw normalizeError(error); }).finally(() => { pending--; queuedBytes -= bytes; });
    } catch (error) { return Promise.reject(normalizeError(error)); }
  }
  function forOwner(owner) {
    const fs = Object.freeze({
      stat: uri => operation(owner, false, async () => {
        const stat = await invoke(lookup(uri), 'stat', uri);
        return { type: stat.type, ctime: stat.ctime, mtime: stat.mtime, size: stat.size, permissions: stat.permissions === FilePermission.Readonly ? FilePermission.Readonly : undefined };
      }, 0, [uri]),
      readDirectory: uri => operation(owner, false, async () => (await invoke(lookup(uri), 'readDirectory', uri)).slice(), 0, [uri]),
      readFile: uri => operation(owner, false, async () => byteSnapshot(await invoke(lookup(uri), 'readFile', uri)), 0, [uri]),
      createDirectory: uri => operation(owner, true, () => mkdirp(lookup(uri, true), uri), 0, [uri]),
      writeFile(uri, content) {
        if (!(content instanceof Uint8Array)) return Promise.reject(new TypeError('File content must be a Uint8Array'));
        // Copy at admission; a caller mutating its array cannot corrupt a queued write.
        if (content.byteLength > MAX_QUEUED_BYTES) return Promise.reject(FileSystemError.Unavailable('Filesystem operation queue is full'));
        let bytes;
        return operation(owner, true, async () => {
          const entry = lookup(uri, true); await mkdirp(entry, parent(uri));
          await invoke(entry, 'writeFile', uri, bytes, { create: true, overwrite: true });
        }, content.byteLength, [uri], () => { bytes = new Uint8Array(content); });
      },
      delete: (uri, options) => operation(owner, true, async () => {
        const entry = lookup(uri, true);
        if (options?.useTrash) throw FileSystemError.Unavailable('Trash integration is not yet implemented');
        await invoke(entry, 'delete', uri, { recursive: false, ...options });
      }, 0, [uri]),
      rename: (source, target, options) => operation(owner, true, () => transfer(source, target, options, true), 0, [source, target]),
      copy: (source, target, options) => operation(owner, true, () => transfer(source, target, options, false), 0, [source, target]),
      isWritableFileSystem(scheme) { assertOwner(owner); const entry = providers.get(scheme); return entry ? !entry.readonly : undefined; },
    });
    return {
      fs,
      registerFileSystemProvider(scheme, provider, options = {}) {
        assertOwner(owner);
        if (!provider) throw new Error('MISSING provider');
        for (const method of ['watch', 'stat', 'readDirectory', 'createDirectory', 'readFile', 'writeFile', 'delete', 'rename']) {
          if (typeof provider[method] !== 'function') throw new Error(`Provider does NOT implement ${method}`);
        }
        if (typeof provider.onDidChangeFile !== 'function') throw new Error('Provider does NOT implement onDidChangeFile');
        if (typeof scheme !== 'string' || !/^[A-Za-z][A-Za-z0-9+.-]*$/.test(scheme)) throw new TypeError('Invalid filesystem scheme');
        if (providers.has(scheme)) throw new Error(`a provider for the scheme '${scheme}' is already registered`);
        // Reserve before evaluating extension-owned option getters or events.
        const entry = { impl: provider, owner, readonly: false, caseSensitive: false };
        providers.set(scheme, entry);
        // Retain subscription ownership. Native document/watch event routing is a
        // separate integration step; these events must not mutate editor mirrors.
        let subscription, registration;
        try {
          entry.readonly = !!options.isReadonly; entry.caseSensitive = !!options.isCaseSensitive;
          subscription = provider.onDidChangeFile(events => {
          if (providers.get(scheme) !== entry) return;
          const valid = [];
          for (const event of events) {
            if (event.uri.scheme !== scheme) continue;
            if (![FileChangeType.Changed, FileChangeType.Created, FileChangeType.Deleted].includes(event.type)) throw new Error('Unknown FileChangeType');
            valid.push({ uri: event.uri, type: event.type });
          }
          assertOwner(owner);
          if (providers.get(scheme) !== entry) return;
          changed.fire(valid);
          });
          assertOwner(owner);
          if (providers.get(scheme) !== entry) throw FileSystemError.Unavailable('Filesystem registration changed');
          registration = new Disposable(() => {
            if (providers.get(scheme) === entry) providers.delete(scheme);
            subscription.dispose();
          });
          return track(owner, registration);
        } catch (error) {
          if (registration) registration.dispose();
          else {
            if (providers.get(scheme) === entry) providers.delete(scheme);
            subscription?.dispose();
          }
          throw error;
        }
      },
    };
  }
  return { forOwner, onDidChangeFiles: changed.event, pathCaseSensitive: scheme => providers.get(scheme)?.caseSensitive };
}
module.exports = { createFileSystem, FileSystemError, FileType, FilePermission, FileChangeType };
