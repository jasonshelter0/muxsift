/* Browser-only interaction prototype. No media is uploaded or processed. */
'use strict';
const theme = document.body.dataset.theme;
const isGlass = theme === 'glass';
const icons = {
  mark: '<path d="m4 18 5-12 3 7 3-7 5 12M7 15h10"/>',
  grid: '<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>',
  book: '<path d="M12 5v16M3 3l9 2 9-2v16l-9 2-9-2z"/>',
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="16" cy="17" r="3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  arrow: '<path d="M4 12h16m-6-6 6 6-6 6"/>',
  link: '<path d="m10 14 4-4m-6 6-1 1a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0m2 0 1-1a4 4 0 0 1 6 6l-4 4a4 4 0 0 1-6 0"/>',
  upload: '<path d="M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6"/>',
  file: '<path d="M5 3h9l5 5v13H5zM14 3v6h5M8 13h8M8 17h6"/>',
  play: '<path d="m8 4 12 8-12 8z"/>',
  pause: '<path d="M8 5v14M16 5v14"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  search: '<circle cx="10" cy="10" r="6"/><path d="m15 15 5 5"/>',
  download: '<path d="M12 3v13m-5-5 5 5 5-5M4 17v4h16v-4"/>',
  copy: '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
  close: '<path d="m6 6 12 12M6 18 18 6"/>',
  spark: '<path d="m12 2 3 7 7 3-7 3-3 7-3-7-7-3 7-3z"/>',
  folder: '<path d="M3 6h7l2 3h9v12H3z"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="2"/><path d="m3 17 6-6 4 4 3-3 5 5"/>',
  audio: '<path d="M4 10v4M8 6v12M12 3v18M16 7v10M20 10v4"/>',
};
const icon = name => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.file}</svg>`;
const esc = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const defaultSettings = {asr:'buzz',model:'large-v3-turbo',format:'notes'};
let settings = {...defaultSettings};
try { settings = {...settings, ...JSON.parse(localStorage.getItem(`muxsift-demo-${theme}`) || '{}')}; } catch (_) { /* storage is optional */ }
let sourceTab = 'url', mode = settings.format, view = 'tasks', previewTab = 'notes', search = '', pickedFiles = [], sequence = 4;
let tasks = seedTasks(), selected = 1, toastTimer;
function seedTasks() {
  return [
    {id:1,title:'从零理解 Transformer 与注意力机制',source:'YouTube',length:'42:18',status:'done',progress:100,kind:'video',mode:'notes',sample:true},
    {id:2,title:'设计系统：从组件到产品',source:'本地视频',length:'18:32',status:'queued',progress:0,kind:'video',mode:'notes',sample:true},
    {id:3,title:'与创造者对话 · 第 12 期',source:'本地音频',length:'56:04',status:'done',progress:100,kind:'audio',mode:'notes',sample:true},
  ];
}
document.querySelector('#app').innerHTML = `
<a class="skip-link" href="#main">跳转到工作区</a>
<div class="ambient ambient-one"></div><div class="ambient ambient-two"></div>
<div class="app-shell">
  <aside class="sidebar">
    <a href="index.html" class="brand"><span class="brand-mark">${icon('mark')}</span><span>MuxSift<span class="brand-sub">MEDIA TO MEANING</span></span></a>
    <div class="workspace-label">个人工作空间 <span>↗</span></div>
    <nav aria-label="主导航">
      <button class="nav-item active" aria-label="工作台" data-action="view" data-view="tasks">${icon('grid')}<span>工作台</span><span class="nav-count" id="nav-count">3</span></button>
      <button class="nav-item" aria-label="我的笔记" data-action="view" data-view="library">${icon('book')}<span>我的笔记</span></button>
      <button class="nav-item" aria-label="偏好设置" data-action="settings">${icon('settings')}<span>偏好设置</span></button>
    </nav>
    <div class="sidebar-note"><span class="tiny-label">${isGlass ? 'MAKE ROOM FOR IDEAS' : 'WATCH LESS.\nKEEP MORE.'}</span><p>好内容，<br>值得被留下。</p><div class="note-orbit">${icon('spark')}</div></div>
    <div class="sidebar-bottom"><span class="connection-dot"></span>本地优先 <span class="version">v0.2.0</span></div>
  </aside>
  <main id="main">
    <header class="topbar"><div class="breadcrumb">工作空间 <span>/</span> <strong id="view-title">工作台</strong></div><div class="top-actions"><a class="style-link" aria-label="切换到另一个设计候选版" title="切换到另一个设计候选版" href="${isGlass?'neo-brutalism':'liquid-glass'}.html">${isGlass?'01 / LIQUID GLASS':'02 / NEO-BRUTALISM'} ${icon('arrow')}</a><button class="avatar" data-action="settings" aria-label="打开个人偏好设置">J</button></div></header>
    <div class="demo-notice"><span class="demo-dot"></span>交互原型 · 示例内容 · 文件不上传 <button data-action="reset">重置体验 ↻</button></div>
    <section class="hero"><div><div class="eyebrow">${isGlass?'A LITTLE LESS NOISE. A LOT MORE CLARITY.':'YOUR MEDIA. YOUR KNOWLEDGE.'}</div><h1>${isGlass?'从媒体，到洞见。':'把内容留下。<br><span>把时间省下。</span>'}</h1><p>视频、播客、课程。把值得记住的，整理成属于你的知识。</p></div><div class="hero-art" aria-hidden="true"><div class="art-layer art-back"></div><div class="art-layer art-front">${icon('spark')}<span>IDEAS<br>WORTH<br>KEEPING.</span></div><span class="art-caption">${isGlass?'LESS SCROLL. MORE SOUL.':'100% YOURS.'}</span></div></section>
    <div class="workspace">
      <div class="left-stack">
        <section class="panel composer" aria-labelledby="composer-heading">
          <div class="section-header"><h2 id="composer-heading"><span class="section-number">01</span> 开始一份新笔记</h2><span class="small-label">给灵感一个入口</span></div>
          <div class="source-tabs" role="tablist" aria-label="媒体来源"><button role="tab" aria-selected="true" class="active" data-action="source" data-source="url">${icon('link')}粘贴链接</button><button role="tab" aria-selected="false" data-action="source" data-source="local">${icon('folder')}本地文件</button></div>
          <div id="url-source"><label class="sr-only" for="url-input">视频网址</label><div class="url-field">${icon('link')}<input id="url-input" type="url" placeholder="粘贴 YouTube、Bilibili 或其他视频链接" autocomplete="off"></div><div class="field-foot"><span>支持 Bilibili · YouTube · yt-dlp 来源</span><button data-action="sample">试试示例链接 ↗</button></div></div>
          <div id="local-source" hidden><div class="drop-zone" id="drop-zone">${icon('upload')}<strong id="file-label">把文件拖到这里，或选择文件</strong><span>视频、音频或 SRT 字幕 · 支持多选与文件夹</span><div><button class="text-button" data-action="files">选择文件</button><button class="text-button" data-action="folder">选择文件夹</button></div></div><input id="file-input" type="file" accept="video/*,audio/*,.srt,.mkv,.webm,.flac,.m4a" multiple hidden><input id="folder-input" type="file" webkitdirectory multiple hidden></div>
          <p class="input-error" id="input-error" role="alert" hidden></p>
          <div class="output-label">你想得到什么？</div>
          <div class="output-options" role="group" aria-label="输出类型"><button data-action="mode" data-mode="notes">${icon('book')}<span>学习笔记<small>字幕 + AI 整理</small></span></button><button data-action="mode" data-mode="srt">${icon('file')}<span>纯净字幕<small>保留时间轴</small></span></button><button data-action="mode" data-mode="frames">${icon('image')}<span>关键画面<small>实验性提取</small></span></button></div>
          <div class="composer-footer"><button class="engine-label" data-action="settings"><span class="connection-dot"></span><span id="engine-label">Buzz · 本地转写</span>${icon('settings')}</button><button class="primary" data-action="create">开始模拟 ${icon('arrow')}</button></div>
        </section>
        <section class="panel tasks-panel" aria-labelledby="tasks-heading"><div class="section-header"><h2 id="tasks-heading"><span class="section-number">02</span> 处理队列 <span id="task-count" class="count">3</span></h2><button class="icon-button" data-action="start-all" aria-label="开始全部等待任务" title="开始全部等待任务">${icon('play')}</button></div><div class="task-toolbar"><div class="filters"><button data-action="filter" data-filter="all" class="active">全部</button><button data-action="filter" data-filter="active">进行中</button><button data-action="filter" data-filter="done">已完成</button></div><label class="search">${icon('search')}<input id="search" placeholder="搜索任务" aria-label="搜索任务"></label></div><div id="task-list"></div><div class="queue-foot">${icon('clock')} 完整阶段可复用，灵感不必从头再来。</div></section>
      </div>
      <section class="panel inspector" aria-labelledby="preview-heading"><div class="section-header"><h2 id="preview-heading"><span class="section-number">03</span> 内容预览</h2><span class="live-label"><span></span> 示例</span></div><div class="preview-tabs" role="tablist" aria-label="结果预览"><button role="tab" aria-selected="true" class="active" data-action="preview" data-preview="notes">笔记</button><button role="tab" aria-selected="false" data-action="preview" data-preview="frames">关键帧</button><button role="tab" aria-selected="false" data-action="preview" data-preview="timeline">时间线</button></div><div id="preview-content"></div><div class="preview-footer"><span>你的内容，随时带走。</span><button class="icon-button" data-action="copy" title="复制示例结果" aria-label="复制示例结果">${icon('copy')}</button><button class="export-button" data-action="export">${icon('download')} 导出示例</button></div></section>
    </div>
    <footer class="page-footer"><span>MUXSIFT / MAKE IT MEANINGFUL.</span><a href="index.html">比较两个设计方向 ${icon('arrow')}</a></footer>
  </main>
</div>
<dialog id="settings-dialog" aria-labelledby="settings-title"><form id="settings-form"><div class="section-header"><h2 id="settings-title">按你的方式整理</h2><button type="button" class="icon-button" data-action="close-settings" aria-label="关闭设置">${icon('close')}</button></div><p>偏好仅保存在当前浏览器，两个候选版各自独立。</p><label>转写方式<select name="asr"><option value="buzz">Buzz · 本地转写</option><option value="bcut">Bcut · 云端转写</option></select></label><div class="setting-hint" id="asr-hint"></div><label>Buzz 模型<select name="model"><option value="large-v3-turbo">large-v3-turbo</option><option value="medium">medium</option><option value="small">small</option></select></label><label>默认输出<select name="format"><option value="notes">学习笔记 · SRT + Markdown</option><option value="srt">纯净字幕 · SRT</option><option value="frames">关键画面 · 实验性</option></select></label><div class="settings-note">本原型使用示例数据，不会启动下载、ASR 或 AI 请求，也不需要 API key。</div><button type="submit" class="primary">保存偏好 ${icon('check')}</button></form></dialog>
<div id="toast" class="toast" role="status" hidden></div>`;

let filter = 'all';
function toast(message) { const el=document.querySelector('#toast'); el.textContent=message; el.hidden=false; clearTimeout(toastTimer); toastTimer=setTimeout(()=>el.hidden=true,3600); }
function selectedTask() {return tasks.find(t=>t.id===selected) || tasks[0];}
function stages(t) {if(t.isSrt)return t.mode==='srt'?['读取字幕','整理字幕','校验时间轴','导出字幕']:['读取字幕','校对文本','整理结构','生成笔记'];if(t.mode==='frames')return ['准备媒体','采样画面','检测变化','生成索引'];return [t.source.startsWith('本地')?'准备媒体':'获取媒体','提取音频','语音转写',t.mode==='srt'?'生成字幕':'AI 整理'];}
function stage(t) {return stages(t)[Math.min(3,Math.floor(t.progress/25))];}
function stateLabel(t) {return ({done:'已完成',queued:'等待开始',paused:'已暂停',cancelled:'已取消'})[t.status] || stage(t);}
function renderTasks() {
  document.querySelector('#nav-count').textContent=tasks.length;
  document.querySelector('#task-count').textContent=view==='library'?tasks.filter(t=>t.status==='done').length:tasks.length;
  const visible=tasks.filter(t=>(view!=='library'||t.status==='done') && (filter!=='done'||t.status==='done') && (filter!=='active'||['running','paused','queued'].includes(t.status)) && t.title.toLowerCase().includes(search.toLowerCase()));
  document.querySelector('#task-list').innerHTML=visible.length?visible.map(t=>`<article class="task-row ${t.id===selected?'selected':''}"><button class="task-main" data-action="select" data-id="${t.id}" aria-pressed="${t.id===selected}"><span class="task-thumb thumb-${t.kind}">${icon(t.kind==='audio'?'audio':'play')}</span><span class="task-info"><strong>${esc(t.title)}</strong><small>${esc(t.source)} <span>·</span> ${esc(t.length)} <span>·</span> ${t.mode==='notes'?'Markdown':t.mode==='srt'?'SRT':'Frames'}</small><span class="task-state ${t.status}">${t.status==='done'?icon('check'):''}${stateLabel(t)}${t.status==='running'?` · ${t.progress}%`:''}</span>${['running','paused'].includes(t.status)?`<span class="progress"><i style="width:${t.progress}%"></i></span>`:''}</span></button><button class="icon-button row-action" data-action="${t.status==='done'?'select':t.status==='running'?'pause':'run'}" data-id="${t.id}" aria-label="${t.status==='done'?'查看':t.status==='running'?'暂停':'开始或继续'} ${esc(t.title)}">${icon(t.status==='done'?'arrow':t.status==='running'?'pause':'play')}</button></article>`).join(''):'<div class="empty-state">没有匹配的任务。换个关键词，或添加一个新来源。</div>';
}
function renderPreview() {
  const t=selectedTask(); const complete=t.status==='done';
  document.querySelector('[data-preview="notes"]').textContent=t.mode==='srt'?'字幕':'笔记';
  let body='';
  if(!complete){body=`<div class="waiting-preview"><div class="waiting-symbol">${icon('spark')}</div><span class="tiny-label">A LITTLE MAGIC IN PROGRESS</span><h3>${t.status==='running'?'正在把内容串起来…':stateLabel(t)}</h3><p>${esc(t.title)}</p><div class="pipeline">${stages(t).map((s,i)=>`<div class="${t.progress>=(i+1)*25?'finished':t.progress>=i*25?'current':''}"><span>${t.progress>=(i+1)*25?'✓':i+1}</span>${s}</div>`).join('')}</div><p class="fine-print">模拟进度 · 不执行真实媒体处理</p><button class="secondary" data-action="${t.status==='running'?'pause':'run'}" data-id="${t.id}">${t.status==='running'?'暂停模拟':'继续模拟'}</button><button class="text-button" data-action="cancel" data-id="${t.id}">取消任务</button></div>`;}
  else if(previewTab==='notes'&&t.mode==='srt'){body=`<div class="note-paper"><div class="paper-meta">SRT 字幕 / 示例</div><h3>${esc(t.title)}</h3><p class="fine-print">仅用于演示字幕排版，不来自所选媒体。</p><pre class="subtitle-text">${esc(sampleSrt())}</pre></div>`;}
  else if(previewTab==='notes'){body=`<div class="note-paper"><div class="paper-meta"><span>学习笔记 / 示例</span><span>约 3 分钟阅读</span></div><h3>${esc(t.title)}</h3><div class="note-tags"><span>机器学习</span><span>概念梳理</span><span>3 个知识点</span></div><p class="note-intro">这是一份展示排版与交互的示例笔记。新增任务也使用同一示例，实际内容将在接入后端后生成。</p><h4><span class="time-tag">00:00</span> 从序列到关系</h4><p>注意力机制让模型在处理一个位置时，参考序列中其他位置的信息。理解重点在于：如何判断哪些信息更值得关注。</p><div class="quote">“不是记住每一个词，而是理解词与词之间的联系。”<small>示例说明 · 非视频原文引述</small></div><h4><span class="time-tag">08:24</span> 理解 Q、K、V</h4><p>Query 发起查询，Key 用于匹配，Value 提供信息。匹配得到的权重，决定不同信息对输出的贡献。</p><div class="mini-diagram"><span>Query</span><b>×</b><span>Key</span><b>→</b><span class="diagram-result">Attention</span></div><h4><span class="time-tag">19:36</span> 值得继续思考</h4><ul><li>多头注意力如何捕捉不同关系？</li><li>为什么需要位置信息？</li></ul></div>`;}
  else if(previewTab==='frames'){body=`<div class="frames-view"><p class="fine-print">3 张示意课件 · 用于演示关键帧审阅</p>${['Attention is a relationship','Query × Key → Weights','Multiple heads. More perspectives.'].map((title,i)=>`<button class="frame-card" data-action="frame" data-index="${i}"><div class="slide slide-${i}"><small>THE TRANSFORMER / ${String(i+1).padStart(2,'0')}</small><strong>${title}</strong><div class="slide-nodes"><span>Q</span><i>→</i><span>K</span><i>→</i><span>V</span></div></div><span class="frame-caption"><b>${['00:00','08:24','19:36'][i]}</b><span>画面变化 · 示例</span>${icon('arrow')}</span></button>`).join('')}</div>`;}
  else {body=`<div class="timeline-view"><span class="tiny-label">CHAPTERS / 示例章节</span>${[['00:00','为什么需要注意力','从序列处理，转向信息之间的关系。'],['08:24','Q、K、V 的协作','用查询、匹配与聚合组织信息。'],['19:36','多头注意力','在不同的表示空间观察关系。'],['35:10','回顾与延伸','整理核心概念，留下进一步的问题。']].map(([time,title,description])=>`<button class="timeline-item" data-action="chapter" data-time="${time}"><span class="timeline-dot"></span><time>${time}</time><strong>${title}</strong><p>${description}</p></button>`).join('')}</div>`;}
  document.querySelector('#preview-content').innerHTML=body;
  document.querySelector('[data-action="export"]').disabled=!complete;
  document.querySelector('[data-action="copy"]').disabled=!complete;
}
function renderModes() {document.querySelectorAll('[data-action="mode"]').forEach(el=>{el.classList.toggle('active',el.dataset.mode===mode);el.setAttribute('aria-pressed',el.dataset.mode===mode);});document.querySelector('#engine-label').textContent=settings.asr==='buzz'?'Buzz · 本地转写':'Bcut · 云端转写';}
function setPreview(tab){previewTab=tab;document.querySelectorAll('[data-action="preview"]').forEach(el=>{el.classList.toggle('active',el.dataset.preview===tab);el.setAttribute('aria-selected',el.dataset.preview===tab);});renderPreview();}
function error(message){const el=document.querySelector('#input-error');el.textContent=message;el.hidden=!message;}
function acceptFiles(files){const accepted=Array.from(files).filter(f=>/\.(mp4|mkv|mov|webm|avi|m4v|mp3|wav|m4a|flac|ogg|aac|opus|srt)$/i.test(f.name));pickedFiles=accepted;document.querySelector('#file-label').textContent=accepted.length?`已选择 ${accepted.length} 个文件 · ${accepted[0].name}`:'未找到支持的媒体或字幕文件';error(accepted.length?'':'请选择视频、音频或 SRT 字幕文件。');}
function run(id){const t=tasks.find(t=>t.id===id);if(!t||t.status==='done')return;t.status='running';selected=t.id;renderTasks();renderPreview();}
function createTasks(){error('');let inputs=[];
  if(sourceTab==='url'){let url;try{url=new URL(document.querySelector('#url-input').value.trim());if(!['http:','https:'].includes(url.protocol))throw Error();}catch(_){error('请填写完整的 http:// 或 https:// 视频链接。');document.querySelector('#url-input').focus();return;}inputs=[{title:`${url.hostname} · 新的视频笔记`,source:url.hostname,kind:'video'}];}
  else {if(!pickedFiles.length){error('请先选择或拖入媒体文件。');return;}inputs=pickedFiles.map(f=>({title:f.webkitRelativePath||f.name,source:'本地文件',kind:/\.(mp3|wav|m4a|flac|ogg|aac|opus)$/i.test(f.name)?'audio':'video',isSrt:/\.srt$/i.test(f.name)}));}
  if(mode==='frames'&&inputs.some(item=>item.isSrt||item.kind==='audio')){error('关键画面需要视频文件。音频或 SRT 请使用学习笔记／纯净字幕模式。');return;}
  const created=inputs.map(item=>({...item,id:sequence++,status:'running',progress:0,length:'演示',mode,sample:true}));tasks.unshift(...created);selected=created[0].id;view='tasks';filter='all';search='';document.querySelector('#search').value='';updateView();renderTasks();setPreview(mode==='frames'?'frames':'notes');toast(`已加入 ${created.length} 个模拟任务；未上传任何文件。`);
}
function updateView(){document.querySelectorAll('[data-action="view"]').forEach(el=>el.classList.toggle('active',el.dataset.view===view));document.querySelector('#view-title').textContent=view==='library'?'我的笔记':'工作台';document.querySelector('.composer').hidden=view==='library';document.querySelectorAll('[data-action="filter"]').forEach(el=>el.classList.toggle('active',el.dataset.filter===filter));renderTasks();}
function markdown(){return `# ${selectedTask().title}\n\n> MuxSift GUI 原型示例。以下内容不来自所选媒体，未进行真实 ASR 或 AI 处理。\n\n## 从序列到关系 · 00:00\n\n注意力机制让模型在处理一个位置时，参考序列中其他位置的信息。\n\n## 理解 Q、K、V · 08:24\n\nQuery 发起查询，Key 用于匹配，Value 提供信息。匹配得到的权重决定不同信息对输出的贡献。\n\n## 值得继续思考 · 19:36\n\n- 多头注意力如何捕捉不同关系？\n- 为什么需要位置信息？\n`;}
function sampleSrt(){return '1\n00:00:00,000 --> 00:00:04,000\n[原型示例，非媒体转写结果]\n\n2\n00:00:04,000 --> 00:00:09,000\n注意力机制关注信息之间的关系。\n\n3\n00:00:09,000 --> 00:00:14,000\nQuery 发起查询，Key 用于匹配。\n';}
function exportData(){const t=selectedTask();if(t.mode==='srt')return {text:sampleSrt(),ext:'srt',type:'text/plain'};if(t.mode==='frames')return {text:JSON.stringify({demo:true,notice:'示例关键帧，不来自媒体分析',frames:[{time:0,title:'Attention',reason:'demo'},{time:504,title:'Q / K / V',reason:'demo'},{time:1176,title:'Multi-head attention',reason:'demo'}]},null,2),ext:'json',type:'application/json'};return {text:markdown(),ext:'md',type:'text/markdown'};}
function openSettings(){const form=document.querySelector('#settings-form');for(const key of Object.keys(defaultSettings))form.elements[key].value=settings[key];asrHint();document.querySelector('#settings-dialog').showModal();}
function asrHint(){document.querySelector('#asr-hint').textContent=document.querySelector('#settings-form').elements.asr.value==='buzz'?'在正式应用中，Buzz 在本地运行，需要模型环境。':'在正式应用中，Bcut 会上传音频进行云端转写。';}
document.addEventListener('click',async event=>{const b=event.target.closest('[data-action]');if(!b)return;const id=Number(b.dataset.id);const t=tasks.find(t=>t.id===id);
  switch(b.dataset.action){
    case 'source':sourceTab=b.dataset.source;document.querySelectorAll('[data-action="source"]').forEach(el=>{el.classList.toggle('active',el.dataset.source===sourceTab);el.setAttribute('aria-selected',el.dataset.source===sourceTab);});document.querySelector('#url-source').hidden=sourceTab!=='url';document.querySelector('#local-source').hidden=sourceTab!=='local';error('');break;
    case 'sample':document.querySelector('#url-input').value='https://www.youtube.com/watch?v=demo-transformer';toast('已填入示例链接，点击“开始模拟”体验流程。');break;
    case 'files':document.querySelector('#file-input').click();break;
    case 'folder':document.querySelector('#folder-input').click();break;
    case 'mode':mode=b.dataset.mode;renderModes();break;
    case 'create':createTasks();break;
    case 'select':selected=id;renderTasks();renderPreview();break;
    case 'run':run(id);break;
    case 'pause':if(t)t.status='paused';renderTasks();renderPreview();break;
    case 'cancel':if(t)t.status='cancelled';renderTasks();renderPreview();toast('模拟任务已取消，可从当前进度继续。');break;
    case 'start-all':tasks.filter(t=>t.status==='queued'||t.status==='paused').forEach(t=>run(t.id));break;
    case 'filter':filter=b.dataset.filter;updateView();break;
    case 'preview':setPreview(b.dataset.preview);break;
    case 'view':view=b.dataset.view;filter='all';updateView();break;
    case 'settings':openSettings();break;
    case 'close-settings':document.querySelector('#settings-dialog').close();break;
    case 'frame':setPreview('notes');toast(`已定位到示例章节 ${['00:00','08:24','19:36'][Number(b.dataset.index)]}。`);document.querySelectorAll('.time-tag')[Number(b.dataset.index)]?.scrollIntoView({block:'nearest',behavior:'smooth'});break;
    case 'chapter':setPreview('notes');toast(`示例章节：${b.dataset.time}。接入播放器后可跳转原视频。`);break;
    case 'copy':try{await navigator.clipboard.writeText(exportData().text);toast('示例内容已复制。');}catch(_){toast('当前浏览器不允许复制，请使用“导出示例”。');}break;
    case 'export':{const data=exportData();const url=URL.createObjectURL(new Blob([data.text],{type:data.type+';charset=utf-8'}));const a=document.createElement('a');a.href=url;a.download='muxsift-demo.'+data.ext;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);toast('已导出示例 '+data.ext.toUpperCase()+'。');break;}
    case 'reset':tasks=seedTasks();selected=1;sequence=4;search='';filter='all';view='tasks';document.querySelector('#search').value='';updateView();setPreview('notes');toast('已恢复三个示例任务。');break;
  }
});
document.querySelector('#url-input').addEventListener('keydown',e=>{if(e.key==='Enter')createTasks();});
document.querySelector('#search').addEventListener('input',e=>{search=e.target.value;renderTasks();});
for(const id of ['file-input','folder-input'])document.querySelector(`#${id}`).addEventListener('change',e=>acceptFiles(e.target.files));
const drop=document.querySelector('#drop-zone');
drop.addEventListener('dragover',e=>{e.preventDefault();drop.classList.add('dragging');});
drop.addEventListener('dragleave',()=>drop.classList.remove('dragging'));
drop.addEventListener('drop',e=>{e.preventDefault();drop.classList.remove('dragging');acceptFiles(e.dataTransfer.files);});
document.querySelector('#settings-form').elements.asr.addEventListener('change',asrHint);
document.querySelector('#settings-form').addEventListener('submit',e=>{e.preventDefault();settings=Object.fromEntries(new FormData(e.target));mode=settings.format;let saved=true;try{localStorage.setItem(`muxsift-demo-${theme}`,JSON.stringify(settings));}catch(_){saved=false;}renderModes();document.querySelector('#settings-dialog').close();toast(saved?'偏好已保存在当前浏览器。':'当前浏览器禁止存储，偏好仅在本次打开期间生效。');});
setInterval(()=>{let changed=false;for(const t of tasks){if(t.status!=='running')continue;changed=true;t.progress=Math.min(100,t.progress+5);if(t.progress===100){t.status='done';toast(`“${t.title}”模拟完成，可查看示例结果。`);}}if(changed){renderTasks();renderPreview();}},650);
renderModes();renderTasks();renderPreview();
