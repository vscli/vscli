#!/usr/bin/env python3
"""Portable source application inference reaches a no-Node native profile workflow."""
from pathlib import Path
import hashlib,json,os,subprocess,tempfile
from pty_smoke import BINARY,CTRL_S,CTRL_Z,Editor,eventually
from imported_appearance_pty import painted
from profile_migration_pty import finish_after_failure

def run():
    with tempfile.TemporaryDirectory(prefix='vscli-portable-profile-') as temporary:
        root=Path(temporary);data=root/'data';user=data/'user-data/User';user.mkdir(parents=True);extensions=data/'extensions';extensions.mkdir();(extensions/'extensions.json').write_text('[]')
        app_root=root/'resources/app';package=app_root/'extensions/portable-builtin';package.mkdir(parents=True)
        (app_root/'package.json').write_text('{"version":"1.118.0"}');(app_root/'product.json').write_text('{"applicationName":"code"}')
        (package/'package.json').write_text(json.dumps({'publisher':'fixture','name':'portable-builtin','version':'1.0.0','main':'main.cjs','contributes':{'themes':[{'id':'fixture.portable.theme','label':'%theme%','path':'theme.json'}]}}))
        (package/'package.nls.json').write_text('{"theme":"Portable built-in theme"}');(package/'main.cjs').write_text("throw new Error('portable source code must not execute')");(package/'theme.json').write_text('{"name":"Portable built-in theme","colors":{"editor.foreground":"#1a2b3c"}}')
        (user/'settings.json').write_text('// portable source comments\n{"workbench.colorTheme":"fixture.portable.theme"}')
        (user/'keybindings.json').write_text(json.dumps([{'key':'f6','command':'type','args':{'text':'portable '},'when':'editorTextFocus && resourceExtname == .txt'}]))
        originals={p:hashlib.sha256(p.read_bytes()).digest() for directory in [data,app_root] for p in directory.rglob('*') if p.is_file()}
        environment={**os.environ,'VSCODE_PORTABLE':str(data)}
        discovered=json.loads(subprocess.run([BINARY,'--list-vscode-profiles'],env=environment,check=True,capture_output=True,timeout=20).stdout)
        profile=next(p for p in discovered['profiles'] if p['product']=='VS Code Portable');assert Path(profile['application']['root'])==app_root.resolve();assert profile['application']['version']=='1.118.0'
        native=root/'native';report=json.loads(subprocess.run([BINARY,'--import-vscode','--vscode-profile','VS Code Portable:default','--config-dir',str(native),'--import-extensions','--apply-import'],env=environment,check=True,capture_output=True,timeout=20).stdout)
        assert report['extensions_imported']==1 and report['extensions_execution_enabled']==0 and report['extensions']['built_in_packages']==1
        active=Path(report['activated_profile']);assert (active/'settings.json').read_bytes()==(user/'settings.json').read_bytes();assert (active/'keybindings.json').read_bytes()==(user/'keybindings.json').read_bytes()
        file=root/'native.txt';original='λ🙂 original\r\n'.encode();file.write_bytes(original)
        app=Editor(root,'--config-dir',native,'--extension-node',root/'no-node',file,enhanced=True)
        try:
            eventually(lambda:painted(app,(26,43,60)),timeout=10);app.send(b'\x1b[17~');app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==b'portable '+original);app.send(CTRL_Z);app.send(CTRL_S);eventually(lambda:app.read() and file.read_bytes()==original);app.finish()
        except BaseException:finish_after_failure(app);raise
        assert all(hashlib.sha256(p.read_bytes()).digest()==digest for p,digest in originals.items())
        print('PASS: portable environment → adjacent application/version/built-in discovery → independent profile/package copy → no-Node theme/imported shortcut/Unicode-CRLF save-Undo; source bytes unchanged and no execution grants')

if __name__=='__main__':run()
