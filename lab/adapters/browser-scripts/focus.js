(()=>{const deepAll=__DEEP__;const el=deepAll(__SELECTOR__)[0];if(!el)throw new Error('no element for selector');el.focus();if(typeof el.select==='function')el.select();return true})()
