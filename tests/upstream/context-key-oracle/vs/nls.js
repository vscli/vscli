exports.localize=(_key,message,...args)=>message.replace(/\{([0-9]+)\}/g,(m,n)=>args[n]??m);
