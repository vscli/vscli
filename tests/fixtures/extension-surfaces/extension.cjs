const vscode = require('vscode');
exports.activate = context => {
  const output=vscode.window.createOutputChannel('Native Fixture Output');
  output.appendLine('LOG 猫🙂');
  const token={value:'INSERTED猫'}; token.self=token;
  const status=vscode.window.createStatusBarItem('fixture-status',vscode.StatusBarAlignment.Left,10);
  status.text='Native Ready'; status.tooltip='Runs opaque action';
  status.command={command:'surfaces.edit',title:'Insert',arguments:[token]}; status.show();
  const changed=new vscode.EventEmitter();
  let version=0, slow=false;
  const parent={label:'Group'}, child={label:'Leaf 猫',token};
  const provider={onDidChangeTreeData:changed.event,
    async getChildren(element) {
      if(slow) await new Promise(resolve=>setTimeout(resolve,150));
      return element?[child]:[parent];
    },
    getTreeItem(element) {
      const item=new vscode.TreeItem(element.label+(version?' refreshed':''),element===parent?vscode.TreeItemCollapsibleState.Collapsed:vscode.TreeItemCollapsibleState.None);
      if(element===child) item.command={command:'surfaces.edit',title:'Insert',arguments:[element.token]};
      return item;
    }
  };
  const tree=vscode.window.createTreeView('fixture.surfaceTree',{treeDataProvider:provider});
  tree.onDidChangeVisibility(e=>output.appendLine('visible='+e.visible));
  const register=(name,callback)=>context.subscriptions.push(vscode.commands.registerCommand('surfaces.'+name,callback));
  register('show',()=>output.show());
  register('preserve',()=>output.show(true));
  register('refresh',()=>{version++;changed.fire();});
  register('slow',()=>{slow=true;changed.fire();});
  register('dispose',()=>{output.dispose();status.dispose();tree.dispose();});
  register('crash',()=>process.exit(7));
  register('edit',async argument=>{
    if(argument!==token||argument.self!==token) throw new Error('Opaque argument identity lost');
    const editor=vscode.window.activeTextEditor;
    if(!editor) return vscode.window.showInformationMessage('No active editor; no document created');
    const applied=await editor.edit(edit=>edit.insert(new vscode.Position(0,0),argument.value));
    await vscode.window.showInformationMessage('Surface applied='+applied);
  });
  register('prompt',async()=>{
    output.show();
    const value=await vscode.window.showInputBox({title:'Surface input',value:'answer'});
    await vscode.window.showInformationMessage('Surface input='+value);
  });
  context.subscriptions.push(output,status,tree,changed);
};
