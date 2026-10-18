#!/usr/bin/env python3
"""Deterministic pinned-source type elision/extraction; not a TypeScript compiler."""
from pathlib import Path
import hashlib, json, re
import argparse
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--upstream", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
EXPECTED = {'src/vs/base/common/uri.ts': '65bc6eefd07704297435d71a9da87dec1ca9103c0575bf29ce00f291a24114c9', 'src/vs/base/common/path.ts': '3ee9852efa3c404fbb5b8015b9633f5b17b38b9ed619cfbbc38a89d2169dbacf', 'src/vs/base/common/charCode.ts': 'ddb45e08c0f66a253a52ca5eaafc705687ceda45229ce5c9a1aa64d0fb34e510', 'src/vs/base/common/marshallingIds.ts': '3048bf85eed631d42e32ef8684a64019bcc5befe9e7eb94de445fd4c66cb9acd', 'src/vs/base/common/platform.ts': 'c6e8f86c52ca1b521ec5a3a852791e621a632b96e63325fc2892eea281f90d98', 'src/vs/base/common/process.ts': 'c72b499f137a9f52134cd816db076f02712b7ee279f2957b53740639f80d9083', 'src/vscode-dts/vscode.d.ts': 'a4321528e86dcdcb8450287f2a263055c46a622119cdb186de85ac3b19ce9608', 'LICENSE.txt': '9480271317925265e806a9a196aaa33410a962fa9d4d1e248a4a5187bc8c9df9'}

def checked(rel):
    data = (args.upstream / rel).read_bytes()
    if hashlib.sha256(data).hexdigest() != EXPECTED[rel]:
        raise ValueError('Pinned source digest mismatch: ' + rel)
    return data.decode('utf-8')

def replace(text, before, after):
    if before not in text:
        raise ValueError('Source transform anchor absent: ' + before)
    return text.replace(before, after)

def method(source, object_name, name):
    start = source.index('\n\t' + name + '(', source.index('export const ' + object_name + ': IPath'))
    end = source.index('\n\t},', start) + len('\n\t},')
    return source[start:end].rstrip(',')

uri = checked('src/vs/base/common/uri.ts')
path = checked('src/vs/base/common/path.ts')
char = checked('src/vs/base/common/charCode.ts')
marshalling = checked('src/vs/base/common/marshallingIds.ts')
license_text = checked('LICENSE.txt')

# All four revive declarations are type-only overloads; retain its implementation.
uri, overload_count = re.subn(r'^\s*static revive\([^\n]+;\n', '', uri, flags=re.M)
if overload_count != 4:
    raise ValueError('Pinned URI revive overload declarations changed')

# Preserve exact runtime URI logic; remove only imports, declarations, overloads,
# assertions and annotations. This exact transform refuses changed source hashes.
uri = re.sub(r'^import .*\n', '', uri, flags=re.M)
uri = uri[:uri.index('export type UriDto<T>')].rstrip() + '\n'
start = uri.index('export interface UriComponents {')
end = uri.index('export function isUriComponents', start)
uri = uri[:start] + uri[end:]
start = uri.index('interface UriState extends UriComponents {')
end = uri.index('const _pathSepMarker', start)
uri = uri[:start] + uri[end:]
uri = re.sub(r'^\s*readonly \w+: string;\n', '', uri, flags=re.M)
uri = re.sub(r'^\s*protected constructor\([^\n]+\);\n', '', uri, flags=re.M)
uri = re.sub(r'^\s*static revive\([^\n]+\);\n', '', uri, flags=re.M)
for before, after in [
    ('export class URI implements UriComponents', 'class URI'),
    ('function _validateUri(ret: URI, _strict?: boolean): void', 'function _validateUri(ret, _strict)'),
    ('function _schemeFix(scheme: string, _strict: boolean): string', 'function _schemeFix(scheme, _strict)'),
    ('function _referenceResolution(scheme: string, path: string): string', 'function _referenceResolution(scheme, path)'),
    ('static isUri(thing: unknown): thing is URI', 'static isUri(thing)'),
    ('protected constructor(schemeOrData: string | UriComponents, authority?: string, path?: string, query?: string, fragment?: string, _strict: boolean = false)', 'constructor(schemeOrData, authority, path, query, fragment, _strict = false)'),
    ('get fsPath(): string', 'get fsPath()'),
    ('with(change: { scheme?: string; authority?: string | null; path?: string | null; query?: string | null; fragment?: string | null }): URI', 'with(change)'),
    ('static parse(value: string, _strict: boolean = false): URI', 'static parse(value, _strict = false)'),
    ('static file(path: string): URI', 'static file(path)'),
    ('static from(components: UriComponents, strict?: boolean): URI', 'static from(components, strict)'),
    ('static joinPath(uri: URI, ...pathFragment: string[]): URI', 'static joinPath(uri, ...pathFragment)'),
    ('let newPath: string;', 'let newPath;'),
    ('toString(skipEncoding: boolean = false): string', 'toString(skipEncoding = false)'),
    ('toJSON(): UriComponents', 'toJSON()'),
    ('static revive(data: UriComponents | URI | undefined | null): URI | undefined | null', 'static revive(data)'),
    ('export function isUriComponents(thing: unknown): thing is UriComponents', 'function isUriComponents(thing)'),
    ('_formatted: string | null = null;', '_formatted = null;'),
    ('_fsPath: string | null = null;', '_fsPath = null;'),
    ('const res = <UriState>{', 'const res = {'),
    ('const encodeTable: { [ch: number]: string } = {', 'const encodeTable = {'),
    ('function encodeURIComponentFast(uriComponent: string, isPath: boolean, isAuthority: boolean): string', 'function encodeURIComponentFast(uriComponent, isPath, isAuthority)'),
    ('let res: string | undefined = undefined;', 'let res = undefined;'),
    ('function encodeURIComponentMinimal(path: string): string', 'function encodeURIComponentMinimal(path)'),
    ('export function uriToFsPath(uri: URI, keepDriveLetterCasing: boolean): string', 'function uriToFsPath(uri, keepDriveLetterCasing)'),
    ('let value: string;', 'let value;'),
    ('function _asFormatted(uri: URI, skipEncoding: boolean): string', 'function _asFormatted(uri, skipEncoding)'),
    ('function decodeURIComponentGraceful(str: string): string', 'function decodeURIComponentGraceful(str)'),
    ('function percentDecode(str: string): string', 'function percentDecode(str)'),
]:
    uri = replace(uri, before, after)
uri = re.sub(r'<(?:URI|UriState|UriComponents)>', '', uri)
uri = uri.replace('override ', '')

# Extract precisely the path functions reached by URI.joinPath. No cwd,
# filesystem, provider, resolve, platform shims or omitted path APIs are used.
path_head = path[:path.index("import * as process")]
constants = path[path.index('const CHAR_UPPERCASE_A'):path.index('function validateObject')]
helpers = path[path.index('function validateString'):path.index('function formatExt')]
helpers = helpers.replace("const platformIsWin32 = (process.platform === 'win32');", '')
path_body = constants + helpers + 'const win32 = {' + method(path, 'win32', 'normalize') + ',' + method(path, 'win32', 'join') + '\n};\n'
path_body += 'const posix = {' + method(path, 'posix', 'normalize') + ',' + method(path, 'posix', 'join') + '\n};\nconst paths = { win32, posix };\n'
for before, after in [
    ("\tcode: 'ERR_INVALID_ARG_TYPE';\n", ''),
    ('constructor(name: string, expected: string, actual: unknown)', 'constructor(name, expected, actual)'),
    ('function validateString(value: string, name: string)', 'function validateString(value, name)'),
    ('function isPathSeparator(code: number | undefined)', 'function isPathSeparator(code)'),
    ('function isPosixPathSeparator(code: number | undefined)', 'function isPosixPathSeparator(code)'),
    ('function isWindowsDeviceRoot(code: number)', 'function isWindowsDeviceRoot(code)'),
    ('function normalizeString(path: string, allowAboveRoot: boolean, separator: string, isPathSeparator: (code?: number) => boolean)', 'function normalizeString(path, allowAboveRoot, separator, isPathSeparator)'),
    ('normalize(path: string): string', 'normalize(path)'),
    ('join(...paths: string[]): string', 'join(...paths)'),
    ('let firstPart: string | undefined;', 'let firstPart;'),
]:
    path_body = replace(path_body, before, after)

used = sorted(set(re.findall(r'CharCode\.(\w+)', uri)))
values = {}
for name in used:
    found = re.search(r'^\s*' + re.escape(name) + r'\s*=\s*(\d+)\s*,', char, re.M)
    if not found:
        raise ValueError('Unrecognized CharCode constant: ' + name)
    values[name] = int(found.group(1))
if not re.search(r'\bUri\s*=\s*1\s*,', marshalling):
    raise ValueError('MarshalledId.Uri assignment changed')
constants_js = 'const CharCode = Object.freeze(' + json.dumps(values, sort_keys=True) + ');\nconst MarshalledId = Object.freeze({ Uri: 1 });\n'
license_comment = '/*\n' + '\n'.join(' * ' + line for line in license_text.splitlines()) + '\n */\n'
header = "'use strict';\n\n" + license_comment + path_head
header += '// Generated by package generate-uri.py from exact Microsoft stable sources.\n// All URI runtime algorithms are preserved; only TypeScript erasure, CommonJS\n// binding, enum materialization and an isolated platform factory are adapted.\n'
output = header + path_body + '\nfunction createUriForPlatform(isWindows) {\n'
output += "\tif (typeof isWindows !== 'boolean') throw new TypeError('URI platform requires a boolean');\n" + constants_js + uri
output += '\nreturn URI;\n}\n\nconst Uri = createUriForPlatform(process.platform === \'win32\');\nmodule.exports = { Uri, createUriForPlatform };\n'
out = args.output;out.parent.mkdir(parents=True,exist_ok=True);out.write_text(output)
print(json.dumps({'path': str(out), 'sha256': hashlib.sha256(out.read_bytes()).hexdigest(), 'bytes': out.stat().st_size}))
