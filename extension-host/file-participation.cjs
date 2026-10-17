'use strict';
// Native transport for the pinned main-thread cancellation race. Logical replies
// never stand in for actual participant completion; this module applies no edits.
const {CancellationTokenSource} = require('./cancellation.cjs');
const {reviveFileOperation} = require('./file-operations.cjs');
function createFileParticipation(engine, options) {
  const now = performance.now.bind(performance);
  let active;
  function validate(params) {
    if (!params || Object.keys(params).some(key=>!['session','request','frame','timeout'].includes(key)) ||
        params.session !== options.session || !Number.isSafeInteger(params.request) || params.request <= 0 ||
        params.frame?.sequence !== params.request || !Number.isSafeInteger(params.timeout) || params.timeout <= 0 || params.timeout > 2147483647) throw new Error('Invalid native file participation request');
    reviveFileOperation(params.frame, options.session);
  }
  function released(request, accepted) {
    options.unowned(()=>options.notify('fileParticipantReleased',{session:options.session,request,accepted}));
  }
  function provide(params) {
    try { validate(params); }
    catch(error) {
      if(params?.session===options.session && Number.isSafeInteger(params.request) && params.request>0 && active?.request!==params.request) released(params.request,false);
      throw error;
    }
    // A duplicate live identity cannot claim that its original work never began.
    if (active?.request === params.request) throw new Error('Duplicate live file participation request');
    if (!options.enabled() || active || engine.busy()) {
      released(params.request, false);
      throw new Error('Native file participation is unavailable or actually occupied');
    }
    return options.unowned(()=>{
      const source = new CancellationTokenSource(), deadline = now()+params.timeout;
      let run;
      try { run = engine.fire(params.frame, params.timeout, source.token); }
      catch(error) { source.dispose();released(params.request,false);throw error; }
      const record = {request:params.request,source};active = record;
      let timer,subscription;
      const canceled = new Promise(resolve=>{subscription=source.token.onCancellationRequested(()=>resolve(undefined));});
      function expire() {
        const remaining=deadline-now();
        if(remaining>0){timer=setTimeout(expire,Math.ceil(remaining));return;}
        source.cancel();
      }
      expire();
      run.settled.then(()=>{
        if(active===record)active=undefined;
        clearTimeout(timer);source.dispose();released(record.request,true);
      });
      return Promise.race([run.result,canceled]).finally(()=>{clearTimeout(timer);subscription.dispose();});
    });
  }
  function cancel(params) {
    if(!params || Object.keys(params).some(key=>!['session','request'].includes(key)) || params.session!==options.session || !Number.isSafeInteger(params.request) || params.request<=0) return false;
    if(active?.request!==params.request)return false;
    options.unowned(()=>active.source.cancel());return true;
  }
  return Object.freeze({provide,cancel,busy:()=>!!active});
}
module.exports={createFileParticipation};
