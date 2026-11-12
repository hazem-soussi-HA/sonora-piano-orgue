/* Node test-bed for the Rust Sonor core (WASM): renders voices and analyses audio. */
'use strict';
const fs=require('fs');
const path=require('path');
const LOG=path.join(__dirname,'test-core.log');
try{fs.unlinkSync(LOG);}catch(e){}
const console={log:(...a)=>fs.appendFileSync(LOG,a.join(' ')+'\n'),error:(...a)=>fs.appendFileSync(LOG,'ERR '+a.join(' ')+'\n')};

function mag(buf,f,start,len){
  let re=0,im=0;
  for(let i=0;i<len;i++){
    const w=0.5-0.5*Math.cos(2*Math.PI*i/len);
    const x=buf[start+i]*w;
    const a=2*Math.PI*f*i/44100;
    re+=x*Math.cos(a); im-=x*Math.sin(a);
  }
  return 2*Math.sqrt(re*re+im*im)/len;
}
function stats(buf){
  let peak=0,sum=0;
  for(let i=0;i<buf.length;i++){const v=Math.abs(buf[i]);if(v>peak)peak=v;sum+=v*v;}
  const rms=Math.sqrt(sum/buf.length);
  const seg=(a,b)=>{let s=0,c=0;for(let i=a;i<b;i++){s+=buf[i]*buf[i];c++;}return Math.sqrt(s/c);};
  return {peak:+peak.toFixed(4),rms:+rms.toFixed(4),
    head:+seg(0,4410).toFixed(4),tail:+seg(buf.length*0.85|0,buf.length).toFixed(4)};
}

(async()=>{
  const wasmPath=path.join(__dirname,'target','wasm32-unknown-unknown','release','sonor_core.wasm');
  const bytes=fs.readFileSync(wasmPath);
  const {instance}=await WebAssembly.instantiate(bytes,{});
  const ex=instance.exports;
  console.log('ABI version:',ex.sonor_abi_version());
  console.log('exports:',Object.keys(ex).join(', '));

  const paramsPtr=ex.sonor_params_ptr();
  const analysisPtr=ex.sonor_analysis_ptr();
  const SR=44100;
  const frames=SR*2;
  const N=4096;
  /* allocate BEFORE taking views: growing the memory detaches existing views */
  const bufPtr=ex.sonor_alloc(frames);
  const inPtr=ex.sonor_alloc(N);
  let P=new Float32Array(ex.memory.buffer,paramsPtr,32);
  let A=new Float32Array(ex.memory.buffer,analysisPtr,10+12+64);
  let Ai=new Int32Array(ex.memory.buffer,analysisPtr,22);  /* root/scale are i32 */
  let out=new Float32Array(ex.memory.buffer,bufPtr,frames);
  let input=new Float32Array(ex.memory.buffer,inPtr,N);
  const refresh=()=>{P=new Float32Array(ex.memory.buffer,paramsPtr,32);
    A=new Float32Array(ex.memory.buffer,analysisPtr,10+12+64);
    Ai=new Int32Array(ex.memory.buffer,analysisPtr,22);
    out=new Float32Array(ex.memory.buffer,bufPtr,frames);
    input=new Float32Array(ex.memory.buffer,inPtr,N);};

  function setParams(o){
    const base={sample_rate:SR,blend:0.5,draw:[0,8,8,0,0,0,0,0],tone:0.3,chiff:0.5,vib_rate:5.4,
      vib_depth:5,leslie:0.35,rotary:1,o_level:0.85,p_harm:10,p_decay:7,p_bright:0.55,
      p_hammer:0.5,p_inharm:0.0004,p_level:0.8,midi:57,vel:0.95,seed:1,reverb_mix:0.28,
      reverb_size:2.6,reverb_tone:0.5,chorus:0.3,chorus_rate:0.5,drive:0.12,bass:0,treble:2,volume:0.8};
    const p=Object.assign(base,o);
    P[0]=p.sample_rate;P[1]=p.blend;
    for(let i=0;i<8;i++)P[2+i]=p.draw[i];
    const keys=['tone','chiff','vib_rate','vib_depth','leslie','rotary','o_level','p_harm','p_decay',
      'p_bright','p_hammer','p_inharm','p_level','midi','vel','seed','reverb_mix','reverb_size',
      'reverb_tone','chorus','chorus_rate','drive','bass','treble','volume'];
    keys.forEach((k,i)=>P[10+i]=p[k]);
    return p;
  }

  console.log('\n== voice rendering (note A3 = 220 Hz) ==');
  setParams({blend:0,draw:[0,8,8,0,0,0,0,0]});
  ex.sonor_render(bufPtr,frames);
  let buf=Float32Array.from(out);
  console.log('ORGAN 8+4 ',JSON.stringify(stats(buf)),
    'f220='+mag(buf,220,4410,8192).toFixed(4),
    'f440='+mag(buf,440,4410,8192).toFixed(4),
    'f880='+mag(buf,880,4410,8192).toFixed(4));

  setParams({blend:0,draw:[0,0,0,0,0,0,8,0]});
  ex.sonor_render(bufPtr,frames);
  buf=Float32Array.from(out);
  console.log('ORGAN 1-2/3',JSON.stringify(stats(buf)),
    'f220='+mag(buf,220,4410,8192).toFixed(4),
    'f1760='+mag(buf,1760,4410,8192).toFixed(4));

  setParams({blend:1,p_harm:10,p_decay:4,p_bright:0.8,p_hammer:0.8});
  ex.sonor_render(bufPtr,frames);
  buf=Float32Array.from(out);
  console.log('PIANO     ',JSON.stringify(stats(buf)),
    'f220='+mag(buf,220,4410,8192).toFixed(4),
    'f440='+mag(buf,440,4410,8192).toFixed(4),
    'f660='+mag(buf,660,4410,8192).toFixed(4));

  setParams({blend:0,draw:[0,0,0,0,0,0,0,0]});
  ex.sonor_render(bufPtr,frames);
  buf=Float32Array.from(out);
  console.log('SILENT    ',JSON.stringify(stats(buf)));

  console.log('\n== analysis ==');
  /* synthetic C major chord (C4 E4 G4) + click track at 120 bpm */
  const h=ex.analyzer_new(4096);
  refresh(); /* the registry allocation grew the memory: re-create the views */
  console.log('analyzer handle:',h);
  const sr=44100, hop=1024;
  const chord=[261.63,329.63,392.0];
  const framesNeeded=sr*8;
  let detected=null;
  for(let pos=0;pos<framesNeeded;pos+=hop){
    const t=pos/sr;
    for(let i=0;i<N;i++){
      const tt=(pos+i)/sr;
      let v=0;
      chord.forEach((f,k)=>{v+=0.25*Math.sin(2*Math.PI*f*tt)*(k?1:1.0);});
      /* click track: 8th notes at 120 bpm */
      const beat=0.5; /* seconds per beat */
      const ph=(tt%beat)/beat;
      if(ph<0.004) v+=0.6*Math.sin(2*Math.PI*2000*tt)*(1-ph/0.004);
      input[i]=v;
    }
    ex.analyzer_analyze(h,inPtr,N,sr,hop);
    const rms=A[0],bpm=A[5],conf=A[6],root=A[7],scale=A[8],clar=A[9];
    detected={rms:+rms.toFixed(4),bpm:+bpm.toFixed(2),conf:+conf.toFixed(3),root:Ai[7],scale:Ai[8],clar:+clar.toFixed(3)};
  }
  console.log('C major + 120bpm click ->',JSON.stringify(detected));
  const names=['C','C#','D','D#','E','F','F#','G','G#','A','A#','B'];
  console.log('chroma:',Array.from(A.slice(10,22)).map(v=>v.toFixed(2)).join(' '));
  console.log('detected key:',names[detected.root],detected.scale?'minor':'major');

  /* a different key: F# minor-ish */
  const h2=ex.analyzer_new(4096);refresh();
  const chord2=[185.0,220.0,277.18]; /* F#3 A3 C#4 */
  for(let pos=0;pos<sr*4;pos+=hop){
    for(let i=0;i<N;i++){
      const tt=(pos+i)/sr;
      let v=0;
      chord2.forEach(f=>{v+=0.25*Math.sin(2*Math.PI*f*tt);});
      input[i]=v;
    }
    ex.analyzer_analyze(h2,inPtr,N,sr,hop);
  }
  console.log('F# major chord ->',names[Ai[7]],Ai[8]?'minor':'major','conf='+A[6].toFixed(3));
  console.log('  chroma:',Array.from(A.slice(10,22)).map((v,i)=>names[i]+':'+v.toFixed(2)).join(' '));

  /* tempo detection: 90 bpm click */
  const h3=ex.analyzer_new(2048);refresh();
  for(let pos=0;pos<sr*8;pos+=hop){
    for(let i=0;i<N;i++){
      const tt=(pos+i)/sr;
      const beat=60/90, ph=(tt%beat)/beat;
      input[i]=ph<0.004?0.8*Math.sin(2*Math.PI*1800*tt)*(1-ph/0.004):0.02*Math.sin(2*Math.PI*110*tt);
    }
    ex.analyzer_analyze(h3,inPtr,N,sr,hop);
  }
  console.log('90 bpm click -> bpm='+A[5].toFixed(2)+' clarity='+A[9].toFixed(3));
  console.log('\nOK');
})().catch(e=>{console.error('FAIL',e);process.exit(1);});
