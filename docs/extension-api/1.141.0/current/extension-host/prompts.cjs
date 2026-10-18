'use strict';
function createPrompts(request, session, owner, budget) {
  const text = (value, name) => {
    if (value === undefined) return '';
    if (typeof value !== 'string' || Buffer.byteLength(value) > 4096) throw new Error(`Invalid or oversized prompt ${name}`);
    return value;
  };
  function options(value, allowed) {
    if (value === undefined) return {};
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid prompt options');
    for (const key of Object.keys(value)) {
      if (!allowed.includes(key)) throw new Error(`VSCLI prompt option is not implemented: ${key}`);
    }
    return value;
  }
  async function run(kind, source, input, token) {
    if (token !== undefined) throw new Error('VSCLI prompt cancellation tokens are not implemented');
    if (budget.pending >= 8) throw new Error('Extension prompt queue limit reached');
    budget.pending++;
    let bytes = 0, charged = false;
    try {
      const quick = kind === 'quickPick';
      const opts = options(input, quick ? ['title', 'placeHolder', 'matchOnDescription', 'matchOnDetail', 'canPickMany', 'ignoreFocusOut'] : ['title', 'placeHolder', 'prompt', 'value', 'ignoreFocusOut']);
      if (opts.canPickMany) throw new Error('VSCLI multi-select prompts are not implemented');
      if (opts.ignoreFocusOut) throw new Error('VSCLI ignoreFocusOut prompts are not implemented');
      for (const name of ['canPickMany', 'ignoreFocusOut', 'matchOnDescription', 'matchOnDetail']) if (opts[name] !== undefined && typeof opts[name] !== 'boolean') throw new Error(`Invalid prompt ${name}`);
      const raw = quick ? await source : [];
      if (!Array.isArray(raw) || raw.length > 128) throw new Error('Extension prompt item limit exceeded');
      const original = raw.slice();
      const items = original.map(item => {
        if (typeof item === 'string') return { label: text(item, 'label') };
        if (!item || typeof item !== 'object' || typeof item.label !== 'string') throw new Error('Invalid QuickPick item');
        if (item.kind !== undefined || item.picked || item.alwaysShow || item.buttons !== undefined) throw new Error('VSCLI advanced QuickPick items are not implemented');
        return { label: text(item.label, 'label'), description: text(item.description, 'description'), detail: text(item.detail, 'detail') };
      });
      const params = { session, owner, kind, items, title: text(opts.title, 'title'), placeHolder: text(opts.placeHolder, 'placeHolder'), prompt: text(opts.prompt, 'prompt'), value: text(opts.value, 'value'), matchOnDescription: !!opts.matchOnDescription, matchOnDetail: !!opts.matchOnDetail };
      bytes = Buffer.byteLength(JSON.stringify(params));
      if (budget.bytes + bytes > 64 * 1024) throw new Error('Extension prompt presentation exceeds 64 KiB session budget');
      budget.bytes += bytes;
      charged = true;
      const result = await request('prompt', params);
      if (result?.value === null || result?.value === undefined) return undefined;
      if (quick) {
        if (!Number.isInteger(result.value) || result.value < 0 || result.value >= original.length) throw new Error('Invalid native QuickPick response');
        return original[result.value];
      }
      return text(result.value, 'result');
    } finally { budget.pending--; if (charged) budget.bytes -= bytes; }
  }
  return { showQuickPick: (items, options, token) => run('quickPick', items, options, token), showInputBox: (options, token) => run('inputBox', undefined, options, token) };
}
module.exports = { createPrompts };
