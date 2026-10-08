(async()=>{
  const deepAll=__DEEP__;
  const end=performance.now()+__TIMEOUT__;let previous=null,previousAt=0,stable=0;
  while(performance.now()<end){
    await new Promise(resolve=>requestAnimationFrame(resolve));
    const now=performance.now();if(now-previousAt<16)continue;
    const elements=deepAll(__SELECTOR__);if(elements.length>1)throw new Error('ambiguous target');const el=elements[0];
    if(el){const box=el.getBoundingClientRect(),style=getComputedStyle(el),current=[box.x,box.y,box.width,box.height].join(',');
      stable=current===previous?stable+1:0;previous=current;previousAt=now;
      const root=el.getRootNode(),hit=(root.elementFromPoint?root:document).elementFromPoint(box.x+box.width/2,box.y+box.height/2);
      if(!el.disabled&&box.width>0&&box.height>0&&style.visibility!=='hidden'&&style.display!=='none'&&(hit===el||el.contains(hit))&&stable>=2)return true;
    }else{previous=null;stable=0;previousAt=now;}
  }throw new Error('actionability timeout');
})()
