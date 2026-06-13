"use strict";

// Tauri 2: window.__TAURI__ is injected because withGlobalTauri=true
const { invoke }         = window.__TAURI__.core;
const { convertFileSrc } = window.__TAURI__.core;

const MAX_FAVICON_CONCURRENCY = 5; // intentional per-domain rate limiting
const APP_VERSION = '2.2.3-beta';

// ── Link checker ─────────────────────────────────────────────────────────
// Column config: id → CSS-var suffix, label, default width
const CHK_COLS = [
  { id: "url",  label: "URL",    cls: "cl-url",  var: "--cw-url",  w: 240, num: false },
  { id: "st",   label: "Статус", cls: "cl-st",   var: "--cw-st",   w: 58,  num: false },
  { id: "code", label: "HTTP",   cls: "cl-code", var: "--cw-code", w: 42,  num: true  },
  { id: "ms",   label: "Время",  cls: "cl-ms",   var: "--cw-ms",   w: 52,  num: true  },
];
let chkSort    = { col: null, asc: true };
let chkResults = [];   // all results for sort/re-render

function applyColWidths(log) {
  CHK_COLS.forEach(c => log.style.setProperty(c.var, c.w + "px"));
}

function buildChkHeader() {
  const head = document.getElementById("chk-log-head");
  const log  = document.getElementById("chk-log");
  head.innerHTML = "";
  applyColWidths(log);

  CHK_COLS.forEach((col, i) => {
    const cell = document.createElement("div");
    cell.className = `${col.cls} chk-col-head`;
    cell.dataset.col = col.id;

    const lbl  = document.createElement("span");
    lbl.textContent = col.label;
    const icon = document.createElement("span");
    icon.className = "chk-sort-icon";
    icon.id = `chk-si-${col.id}`;
    const handle = document.createElement("div");
    handle.className = "chk-resize-handle";

    cell.append(lbl, icon, handle);
    if (i === CHK_COLS.length - 1) cell.style.borderRight = "none";
    head.appendChild(cell);

    // Sort on click (not on handle)
    cell.addEventListener("click", (e) => {
      if (e.target === handle) return;
      if (chkSort.col === col.id) chkSort.asc = !chkSort.asc;
      else { chkSort.col = col.id; chkSort.asc = true; }
      renderChkTable();
    });

    // Resize
    handle.addEventListener("mousedown", (e) => {
      e.stopPropagation(); e.preventDefault();
      const startX = e.clientX, startW = col.w;
      function onMove(ev) {
        col.w = Math.max(36, startW + ev.clientX - startX);
        applyColWidths(log);
      }
      function onUp() {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
      }
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    });
  });
}

function makeChkRow(r) {
  const row = document.createElement("div");
  let cls, st;
  if (r.timed_out)      { cls = "cl-err";  st = "Timeout"; }
  else if (r.redirect)  { cls = "cl-warn"; st = "Redirect"; }
  else if (r.ok)        { cls = "cl-ok";   st = "OK"; }
  else                  { cls = "cl-err";  st = "Ошибка"; }
  const rowCls = (!r.ok || r.redirect) ? (r.redirect ? "cl-warn" : "cl-err") : "";
  row.className = "cl-row " + rowCls;
  const ms   = r.ms < 1000 ? `${r.ms}ms` : `${(r.ms/1000).toFixed(1)}s`;
  const code = r.status || (r.timed_out ? "T/O" : "—");
  const urlStr = r.url.replace(/^https?:\/\/(www\.)?/, "");
  row.innerHTML =
    `<div class="cl-url" title="${r.url}">${urlStr}</div>` +
    `<div class="cl-st ${cls}">${st}</div>` +
    `<div class="cl-code">${code}</div>` +
    `<div class="cl-ms">${ms}</div>`;
  return row;
}

function renderChkTable() {
  const body = document.getElementById("chk-log-body");
  const scroll = body.scrollTop;
  body.innerHTML = "";

  let rows = [...chkResults];
  if (chkSort.col) {
    const col = CHK_COLS.find(c => c.id === chkSort.col);
    rows.sort((a, b) => {
      let va, vb;
      if (col.id === "ms")   { va = a.ms    || 0; vb = b.ms    || 0; }
      else if (col.id === "code") { va = a.status || 0; vb = b.status || 0; }
      else if (col.id === "st")   { va = a.ok ? "OK" : "Ошибка"; vb = b.ok ? "OK" : "Ошибка"; }
      else { va = a.url || ""; vb = b.url || ""; }
      if (col.num || col.id === "ms" || col.id === "code") {
        return chkSort.asc ? va - vb : vb - va;
      }
      return chkSort.asc ? String(va).localeCompare(String(vb)) : String(vb).localeCompare(String(va));
    });
  }
  rows.forEach(r => body.appendChild(makeChkRow(r)));

  // Update sort icons
  CHK_COLS.forEach(c => {
    const icon = document.getElementById(`chk-si-${c.id}`);
    if (icon) icon.textContent = chkSort.col === c.id ? (chkSort.asc ? " ▲" : " ▼") : "";
  });

  body.scrollTop = chkSort.col ? 0 : scroll;
}
const checkerPanel   = document.getElementById("checker-panel");
const chkLogBody     = document.getElementById("chk-log-body");
const chkStatusText  = document.getElementById("chk-status-text");
const chkBarFill     = document.getElementById("chk-bar-fill");
const chkStartBtn    = document.getElementById("chk-start");
const chkPauseBtn    = document.getElementById("chk-pause");
const chkStopBtn     = document.getElementById("chk-stop");
const chkBody        = document.getElementById("chk-body");

const CC = {
  total:     document.getElementById("cc-total"),
  done:      document.getElementById("cc-done"),
  errors:    document.getElementById("cc-errors"),
  redirects: document.getElementById("cc-redirects"),
  timeouts:  document.getElementById("cc-timeouts"),
};

let _ck = {
  bookmarks: [], folderNode: null,
  running: false, paused: false, cancelled: false,
  done: 0, errors: 0, redirects: 0, timeouts: 0,
};

function getBookmarksUnder(folderId) {
  const result = [];
  const visit = (id) => {
    for (const n of allNodes) {
      if (n.parent !== id) continue;
      if (n.kind === "bookmark" && n.url) result.push(n);
      else if (n.kind === "folder") visit(n.id);
    }
  };
  visit(folderId);
  return result;
}

// Re-render tree preserving expanded state
// ── Tree open-state helpers ────────────────────────────────────────────────────

function saveOpenState() {
  return new Set(
    [...treeEl.querySelectorAll('.tree-item.open')]
      .map(el => parseInt(el.dataset.id, 10))
      .filter(n => !isNaN(n))
  );
}

function restoreOpenState(openIds) {
  openIds.forEach(id => {
    const item = treeEl.querySelector(`.tree-item[data-id="${id}"]`);
    if (!item) return;
    const ch = item.parentElement?.querySelector(':scope > .tree-children');
    if (ch) { ch.classList.add('open'); item.classList.add('open'); }
  });
}

// ── Inline folder rename ───────────────────────────────────────────────────────

function startInlineRename(folderId) {
  const item = treeEl.querySelector(`.tree-item[data-id="${folderId}"]`);
  if (!item) return;
  const labelEl = item.querySelector('.label');
  if (!labelEl) return;
  const orig = labelEl.textContent;

  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'tree-inline-rename';
  input.value = orig;
  labelEl.replaceWith(input);
  input.select();

  let committed = false;

  async function commit() {
    if (committed) return;
    committed = true;
    const newName = input.value.trim() || orig;
    const span = document.createElement('span');
    span.className = 'label';
    span.textContent = newName;
    input.replaceWith(span);
    if (newName !== orig) {
      try {
        await invoke('rename_node', { id: folderId, title: newName });
        const node = allNodes.find(n => n.id === folderId);
        if (node) node.title = newName;
        allFolders = allNodes.filter(n => n.kind === 'folder');
        if (activeFolderId === folderId)
          breadcrumb.textContent = buildBreadcrumbText(folderId);
      } catch(e) { console.error(e); }
    }
  }

  function cancel() {
    if (committed) return;
    committed = true;
    const span = document.createElement('span');
    span.className = 'label';
    span.textContent = orig;
    input.replaceWith(span);
  }

  input.addEventListener('blur', commit);
  input.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter')  { e.preventDefault(); input.blur(); }
    if (e.key === 'Escape') { cancel(); }
  });
}

// ── Create folder + select + inline rename ────────────────────────────────────

async function createFolderAndRename(parentId) {
  try {
    const openIds = saveOpenState();
    const newId = await invoke('create_folder', { parentId, title: 'Новая папка' });
    allNodes = await invoke('get_tree');
    allFolders = allNodes.filter(n => n.kind === 'folder');
    renderTree();
    restoreOpenState(openIds);

    // If subfolder — expand parent
    if (parentId != null) {
      const parentItem = treeEl.querySelector(`.tree-item[data-id="${parentId}"]`);
      const ch = parentItem?.parentElement?.querySelector(':scope > .tree-children');
      if (ch) { ch.classList.add('open'); parentItem?.classList.add('open'); }
    }

    await selectFolder(newId);
    treeEl.querySelector(`.tree-item[data-id="${newId}"]`)?.scrollIntoView({ block: 'nearest' });
    startInlineRename(newId);
  } catch(e) { console.error(e); }
}

async function refreshTree() {
  const openIds = saveOpenState();
  allNodes   = await invoke("get_tree");
  allFolders = allNodes.filter(n => n.kind === "folder");
  renderTree();
  restoreOpenState(openIds);
  if (activeFolderId != null) loadBookmarks(activeFolderId);
}

function openCheckerPanel(folderNode, bookmarkListOverride) {
  const bm = bookmarkListOverride ?? getBookmarksUnder(folderNode?.id ?? -1);
  _ck = { bookmarks: bm, folderNode,
          running: false, paused: false, cancelled: false,
          done: 0, errors: 0, redirects: 0, timeouts: 0 };
  CC.total.textContent     = _ck.bookmarks.length;
  CC.done.textContent      = "0";
  CC.errors.textContent    = "0";
  CC.redirects.textContent = "0";
  CC.timeouts.textContent  = "0";
  chkBarFill.style.width   = "0%";
  chkStatusText.textContent = "Готов к проверке";
  chkResults = [];
  chkSort    = { col: null, asc: true };
  buildChkHeader();
  chkLogBody.innerHTML = "";
  chkStartBtn.disabled   = false;
  chkPauseBtn.disabled   = true;
  chkStopBtn.disabled    = true;
  checkerPanel.classList.remove("hidden");
  chkBody.classList.remove("hidden");
}

function addCheckerRow(result, title) {
  chkResults.push({ ...result, title });
  chkLogBody.appendChild(makeChkRow(result));
  chkLogBody.scrollTop = chkLogBody.scrollHeight;
}

async function runChecker() {
  _ck.running = true; _ck.paused = false; _ck.cancelled = false;
  chkStartBtn.disabled  = true;
  chkPauseBtn.disabled  = false;
  chkStopBtn.disabled   = false;

  const total = _ck.bookmarks.length;
  const BATCH = 5;

  for (let i = _ck.done; i < total && !_ck.cancelled; i += BATCH) {
    // Pause: wait until unpaused
    while (_ck.paused && !_ck.cancelled) {
      await new Promise(r => setTimeout(r, 150));
    }
    if (_ck.cancelled) break;

    const batch = _ck.bookmarks.slice(i, i + BATCH);
    await Promise.all(batch.map(async (b) => {
      if (_ck.cancelled) return;
      chkStatusText.textContent = "Проверяется: " + b.url;

      const r = await invoke("check_url", { url: b.url }).catch(err => ({
        url: b.url, status: 0, ok: false, timed_out: false,
        redirect: null, ms: 0, err: String(err)
      }));

      _ck.done++;
      if (!r.ok)       _ck.errors++;
      if (r.redirect)  _ck.redirects++;
      if (r.timed_out) _ck.timeouts++;

      CC.done.textContent      = _ck.done;
      CC.errors.textContent    = _ck.errors;
      CC.redirects.textContent = _ck.redirects;
      CC.timeouts.textContent  = _ck.timeouts;
      chkBarFill.style.width   = Math.round(_ck.done / total * 100) + "%";

      if (!r.ok) {
        treeEl.querySelector(`.tree-item[data-id="${b.id}"]`)?.classList.add("broken");
      }
      addCheckerRow(r, b.title);
    }));
  }

  _ck.running = false;
  chkPauseBtn.disabled = true;
  chkStopBtn.disabled  = true;
  chkStartBtn.disabled = _ck.done >= total;
  chkStatusText.textContent = _ck.cancelled
    ? `Остановлено. Проверено: ${_ck.done}, ошибок: ${_ck.errors}`
    : `Завершено. Всего: ${total}, ошибок: ${_ck.errors}, redirect: ${_ck.redirects}`;
  chkBarFill.style.width = _ck.cancelled ? chkBarFill.style.width : "100%";
}

// Draggable panel
(function () {
  const titlebar = document.getElementById("chk-titlebar");
  let drag = { on: false, ox: 0, oy: 0 };
  titlebar.addEventListener("mousedown", (e) => {
    if (e.target.closest("button")) return;
    drag.on = true;
    const r = checkerPanel.getBoundingClientRect();
    // Switch from bottom/right to top/left
    checkerPanel.style.bottom = "auto";
    checkerPanel.style.right  = "auto";
    checkerPanel.style.left   = r.left + "px";
    checkerPanel.style.top    = r.top  + "px";
    drag.ox = e.clientX - r.left;
    drag.oy = e.clientY - r.top;
    e.preventDefault();
  });
  document.addEventListener("mousemove", (e) => {
    if (!drag.on) return;
    const pw = checkerPanel.offsetWidth, ph = checkerPanel.offsetHeight;
    checkerPanel.style.left = Math.max(0, Math.min(window.innerWidth  - pw, e.clientX - drag.ox)) + "px";
    checkerPanel.style.top  = Math.max(0, Math.min(window.innerHeight - ph, e.clientY - drag.oy)) + "px";
  });
  document.addEventListener("mouseup", () => { drag.on = false; });
})();

chkStartBtn.onclick   = () => { if (!_ck.running) runChecker(); };
chkPauseBtn.onclick   = () => {
  _ck.paused = !_ck.paused;
  chkPauseBtn.textContent = _ck.paused ? "Продолжить" : "Пауза";
  chkStatusText.textContent = _ck.paused ? "Пауза…" : chkStatusText.textContent;
};
chkStopBtn.onclick    = () => { _ck.cancelled = true; _ck.paused = false; };
document.getElementById("chk-close2").onclick  = () => checkerPanel.classList.add("hidden");
document.getElementById("chk-close-btn").onclick = () => checkerPanel.classList.add("hidden");
document.getElementById("chk-collapse").onclick = () => {
  chkBody.classList.toggle("hidden");
  document.getElementById("chk-collapse").textContent =
    chkBody.classList.contains("hidden") ? "□" : "─";
};

// ── Favicon panel ─────────────────────────────────────────────────────────

function showFaviconPanel(total) {
  _faviconTotal = total;
  _faviconDone  = 0;
  document.getElementById('fv-done').textContent  = '0';
  document.getElementById('fv-total').textContent = total;
  document.getElementById('fv-bar-fill').style.width = '0%';
  document.getElementById('fv-domain').textContent   = '';
  document.getElementById('favicon-panel').classList.remove('hidden');
  setStatus(`Загрузка favicon 0/${total}`, { sticky: true });
}

function _updateFaviconPanelProgress() {
  document.getElementById('fv-done').textContent = _faviconDone;
  const pct = _faviconTotal > 0 ? Math.round(_faviconDone / _faviconTotal * 100) : 0;
  document.getElementById('fv-bar-fill').style.width = pct + '%';
  setStatus(`Загрузка favicon ${_faviconDone}/${_faviconTotal}`, { sticky: true });
}

function hideFaviconPanel() {
  document.getElementById('favicon-panel').classList.add('hidden');
}

function _finishFaviconBatch() {
  document.getElementById('fv-domain').textContent = 'Готово';
  setStatus(`Загружено ${_faviconDone} favicon`);
  const openIds = saveOpenState();
  renderTree();
  restoreOpenState(openIds);
  if (activeFolderId != null) loadFolderContents(activeFolderId);
  setTimeout(hideFaviconPanel, 2000);
}

document.getElementById('fv-cancel-btn').addEventListener('click', () => {
  _faviconCancelled = true;
  _faviconQueue = [];
  hideFaviconPanel();
});

document.getElementById('fv-close-btn').addEventListener('click', () => {
  _faviconCancelled = true;
  _faviconQueue = [];
  hideFaviconPanel();
});

function showThumbPanel(total) {
  _thumbTotal = total;
  _thumbDone  = 0;
  document.getElementById('tp-done').textContent  = '0';
  document.getElementById('tp-total').textContent = total;
  document.getElementById('tp-bar-fill').style.width = '0%';
  document.getElementById('tp-label').textContent   = '';
  document.getElementById('thumb-panel').classList.remove('hidden');
  setStatus(`Загрузка рисунков 0/${total}`, { sticky: true });
}

function _updateThumbPanelProgress() {
  document.getElementById('tp-done').textContent = _thumbDone;
  const pct = _thumbTotal > 0 ? Math.round(_thumbDone / _thumbTotal * 100) : 0;
  document.getElementById('tp-bar-fill').style.width = pct + '%';
  setStatus(`Загрузка рисунков ${_thumbDone}/${_thumbTotal}`, { sticky: true });
}

function hideThumbPanel() {
  document.getElementById('thumb-panel').classList.add('hidden');
}

function _finishThumbBatch() {
  document.getElementById('tp-label').textContent = 'Готово';
  setStatus(`Обновлено ${_thumbDone} рисунков`);
  setTimeout(hideThumbPanel, 2000);
  if (activeFolderId != null) loadFolderContents(activeFolderId);
  if (activeBookmarkNode) {
    const fresh = allNodes.find(n => n.id === activeBookmarkNode.id);
    if (fresh?.thumb) {
      detailImgEl.src = convertFileSrc(thumbFilePath(fresh.thumb)) + '?t=' + Date.now();
      detailImgEl.style.display = '';
      detailNoImgEl.style.display = 'none';
    }
  }
}

document.getElementById('tp-cancel-btn').addEventListener('click', () => {
  _thumbCancelled = true;
  _thumbQueue = [];
  hideThumbPanel();
});

document.getElementById('tp-close-btn').addEventListener('click', () => {
  _thumbCancelled = true;
  _thumbQueue = [];
  hideThumbPanel();
});

makeDlgDraggable(document.getElementById('thumb-panel'), document.getElementById('tp-titlebar'));

// ── Duplicates finder (full utility) ─────────────────────────────────────────

const dupesOverlay   = document.getElementById("dupes-overlay");
const dupesSummaryEl = document.getElementById("dupes-summary");

makeDlgDraggable(document.getElementById("dupes-dlg"),
  document.querySelector("#dupes-dlg .props-drag-handle"));

let _dGroups      = [];
let _dGroupIdx    = -1;
let _dRowIdx      = -1;
let _dSortCol     = 't';
let _dSortDesc    = false;
let _dRows        = [];
let _dTableInited = false; // listeners added only once

function findDuplicates() {
  const map = new Map();
  allNodes.filter(n => n.kind === "bookmark" && n.url).forEach(n => {
    const key = n.url.toLowerCase().replace(/\/+$/, "");
    if (!map.has(key)) map.set(key, []);
    map.get(key).push(n);
  });
  return [...map.entries()]
    .filter(([, ns]) => ns.length > 1)
    .sort((a, b) => b[1].length - a[1].length)
    .map(([url, nodes]) => ({ url, nodes }));
}

function _dSummary() {
  if (!_dGroups.length) { dupesSummaryEl.textContent = "Дубликаты не найдены."; return; }
  const total = _dGroups.reduce((s, g) => s + g.nodes.length, 0);
  const extra = _dGroups.reduce((s, g) => s + g.nodes.length - 1, 0);
  dupesSummaryEl.textContent =
    `Групп: ${_dGroups.length}  ·  Копий: ${total}  ·  Лишних: ${extra}`;
}

function _dRenderGroups() {
  const el = document.getElementById('dupes-groups');
  el.innerHTML = '';
  if (!_dGroups.length) {
    el.innerHTML = '<div class="dup-empty">Нет дубликатов.</div>'; return;
  }
  _dGroups.forEach((g, i) => {
    const item = document.createElement('div');
    item.className = 'dupes-group-item' + (i === _dGroupIdx ? ' selected' : '');
    const short = g.url.replace(/^https?:\/\/(www\.)?/, '').replace(/\/+$/, '');
    item.innerHTML = `<span class="dgi-url" title="${g.url}">${short}</span>`
                   + `<span class="dgi-cnt">${g.nodes.length}</span>`;
    item.addEventListener('click', () => _dSelectGroup(i));
    el.appendChild(item);
  });
}

function _dSortedRows(nodes) {
  return [...nodes].sort((a, b) => {
    let va, vb;
    switch(_dSortCol) {
      case 'u': va = a.url    || ''; vb = b.url    || ''; break;
      case 'f': va = getFolderPath(a.parent) || ''; vb = getFolderPath(b.parent) || ''; break;
      case 'd': va = a.created|| ''; vb = b.created|| ''; break;
      default:  va = a.title  || ''; vb = b.title  || ''; break;
    }
    return _dSortDesc ? vb.localeCompare(va) : va.localeCompare(vb);
  });
}

function _dRenderTable(nodes) {
  _dRows    = _dSortedRows(nodes);
  _dRowIdx  = -1;
  const tbody = document.getElementById('dupes-tbody');
  tbody.innerHTML = '';
  if (!_dRows.length) {
    tbody.innerHTML = `<tr><td colspan="4" class="dupes-td" style="text-align:center;color:#888;padding:16px">Нет данных</td></tr>`;
    return;
  }
  _dRows.forEach((node, i) => {
    const tr = document.createElement('tr');
    tr.className = 'dupes-row';
    const folder = getFolderPath(node.parent) || '—';
    const date   = parseUADate(node.created)  || '—';
    tr.innerHTML = `<td class="dupes-td" title="${node.title}"><span>${node.title}</span></td>`
                 + `<td class="dupes-td dupes-td-url" title="${node.url}"><span>${node.url}</span></td>`
                 + `<td class="dupes-td" title="${folder}"><span>${folder}</span></td>`
                 + `<td class="dupes-td" title="${date}"><span>${date}</span></td>`;
    tr.addEventListener('click', () => {
      _dRowIdx = i;
      tbody.querySelectorAll('.dupes-row').forEach((r,ri) => r.classList.toggle('selected', ri===i));
      _dUpdateBtns();
      // Single click → navigate
      navigateToCard(node);
    });
    tr.addEventListener('dblclick', () => { if (node.url) openWithBrowser(node.url, getDefaultBrowserPath()); });
    tbody.appendChild(tr);
  });
}

function _dSelectGroup(idx) {
  _dGroupIdx = idx; _dRowIdx = -1;
  _dRenderGroups();
  const g = _dGroups[idx];
  document.getElementById('dupes-right-hdr').textContent =
    g ? `Дубликаты (${g.nodes.length}): ${g.url.replace(/^https?:\/\/(www\.)?/,'').substring(0,60)}` : 'Дубликаты';
  _dRenderTable(g?.nodes || []);
  _dUpdateBtns();
}

function _dUpdateBtns() {
  const hasRow   = _dRowIdx >= 0 && _dGroupIdx >= 0;
  const hasGroup = _dGroupIdx >= 0 && (_dGroups[_dGroupIdx]?.nodes.length ?? 0) > 1;
  document.getElementById('dupes-del-one').disabled  = !hasRow;
  document.getElementById('dupes-keep-one').disabled = !hasGroup;
}

// ── Sort & resize init ──────────────────────────────────────────────────────

function _dInitTable() {
  const table = document.getElementById('dupes-table');
  // Sort headers
  document.querySelectorAll('#dupes-table .dupes-th').forEach(th => {
    th.style.position = 'sticky';
    th.addEventListener('click', e => {
      if (e.target.classList.contains('dupes-rsz')) return;
      const col = th.dataset.col;
      if (_dSortCol === col) _dSortDesc = !_dSortDesc;
      else { _dSortCol = col; _dSortDesc = false; }
      document.querySelectorAll('.dupes-th .th-lbl').forEach(l => {
        const c = l.closest('.dupes-th').dataset.col;
        const arr = c === _dSortCol ? (_dSortDesc ? ' ↓' : ' ↑') : '';
        l.textContent = l.textContent.replace(/ [↑↓]$/, '') + arr;
      });
      if (_dGroupIdx >= 0) _dRenderTable(_dGroups[_dGroupIdx].nodes);
    });
  });
  // Resize handles
  document.querySelectorAll('#dupes-table .dupes-rsz').forEach(handle => {
    const th = handle.closest('th');
    handle.addEventListener('mousedown', e => {
      e.preventDefault();
      const startX = e.clientX, startW = th.offsetWidth;
      const v = '--dw-' + th.dataset.col;
      const move = ev => table.style.setProperty(v, Math.max(60, startW + ev.clientX - startX) + 'px');
      const up   = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); };
      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
    });
  });
}

// ── Action buttons ──────────────────────────────────────────────────────────

document.getElementById('dupes-del-one').addEventListener('click', () => {
  if (_dRowIdx < 0 || _dGroupIdx < 0) return;
  const node = _dRows[_dRowIdx];
  const g    = _dGroups[_dGroupIdx];
  deleteConfirm(`Удалить «${node.title}»?`, async () => {
    await invoke('delete_node', { id: node.id });
    const ai = allNodes.findIndex(n => n.id === node.id);
    if (ai >= 0) allNodes.splice(ai, 1);
    const ni = g.nodes.findIndex(n => n.id === node.id);
    if (ni >= 0) g.nodes.splice(ni, 1);
    if (g.nodes.length < 2) {
      _dGroups.splice(_dGroupIdx, 1);
      _dGroupIdx = Math.min(_dGroupIdx, _dGroups.length - 1);
    }
    _dRowIdx = -1;
    _dSummary(); _dRenderGroups();
    if (_dGroupIdx >= 0) _dSelectGroup(_dGroupIdx);
    else _dRenderTable([]);
    _dUpdateBtns();
    refreshTree();
  });
});

document.getElementById('dupes-keep-one').addEventListener('click', () => {
  if (_dGroupIdx < 0) return;
  const g = _dGroups[_dGroupIdx];
  if (!g || g.nodes.length < 2) return;
  const toDelete = g.nodes.slice(1);
  deleteConfirm(`Удалить ${toDelete.length} дубликат(ов)?`, async () => {
    for (const node of toDelete) {
      await invoke('delete_node', { id: node.id });
      const ai = allNodes.findIndex(n => n.id === node.id);
      if (ai >= 0) allNodes.splice(ai, 1);
    }
    g.nodes = [g.nodes[0]];
    _dGroups.splice(_dGroupIdx, 1);
    _dGroupIdx = Math.min(_dGroupIdx, _dGroups.length - 1);
    _dRowIdx = -1;
    _dSummary(); _dRenderGroups();
    if (_dGroupIdx >= 0) _dSelectGroup(_dGroupIdx);
    else _dRenderTable([]);
    _dUpdateBtns();
    refreshTree();
  });
});


function closeDupesDialog() { dupesOverlay.classList.add("hidden"); }

document.getElementById("dupes-x").onclick         = closeDupesDialog;
document.getElementById("dupes-close-btn").onclick = closeDupesDialog;
dupesOverlay.addEventListener("click", e => { if (e.target === dupesOverlay) closeDupesDialog(); });

function openDupesDialog() {
  _dGroups   = findDuplicates();
  _dGroupIdx = -1;
  _dRowIdx   = -1;
  _dSortCol  = 't';
  _dSortDesc = false;

  _dSummary();
  if (!_dTableInited) { _dInitTable(); _dTableInited = true; }
  _dRenderGroups();
  // Reset sort arrows
  document.querySelectorAll('.dupes-th .th-lbl').forEach(l => {
    l.textContent = l.textContent.replace(/ [↑↓]$/, '');
  });
  if (_dGroups.length > 0) _dSelectGroup(0);
  else _dRenderTable([]);
  _dUpdateBtns();

  const dlg = document.getElementById("dupes-dlg");
  dlg.style.position = ""; dlg.style.left = ""; dlg.style.top = "";
  raiseOverlay(dupesOverlay);
}
dupesOverlay.addEventListener("keydown", (e) => { if (e.key === "Escape") closeDupesDialog(); });

// ── Delete folder ─────────────────────────────────────────────────────────
function collectSubtreeIds(folderId) {
  const ids = new Set();
  const visit = (id) => {
    ids.add(id);
    allNodes.filter(n => n.parent === id).forEach(n => visit(n.id));
  };
  visit(folderId);
  return ids;
}

function removeSubtreeFromState(ids) {
  for (let i = allNodes.length - 1; i >= 0; i--) {
    if (ids.has(allNodes[i].id)) allNodes.splice(i, 1);
  }
  allFolders = allNodes.filter(n => n.kind === "folder");
}

function deleteFolder(node) {
  const ids     = collectSubtreeIds(node.id);
  const total   = ids.size - 1;   // exclude the folder itself
  const detail  = total > 0 ? ` и всё содержимое (${total} эл.)` : "";
  deleteConfirm(`Удалить папку «${node.title}»${detail}?`, async () => {
    const nextEl = findNextAfterDelete(node.id);

    await invoke("delete_folder", { id: node.id }).catch(console.error);

    removeSubtreeFromState(ids);

    // Surgical: remove folder wrapper (takes tree-children with it)
    treeEl.querySelector(`.tree-item[data-id="${node.id}"]`)?.parentElement?.remove();

    // Clear grid/detail if they showed something deleted
    if (ids.has(activeFolderId)) {
      activeFolderId = null;
      gridEl.innerHTML = "";
      emptyHint.classList.remove("hidden");
    }
    if (ids.has(activeBookmarkNode?.id)) {
      clearSelection();
    }

    // Navigate to next/prev item
    if (nextEl) {
      const nextNode = allNodes.find(n => n.id === parseInt(nextEl.dataset.id));
      if (nextNode) {
        nextEl.scrollIntoView({ block: "nearest" });
        if (nextNode.kind === "folder") selectFolder(nextNode.id);
        else selectTreeBookmark(nextNode);
      }
    } else if (node.parent != null) {
      selectFolder(node.parent);
    }
  });
}

// ── Sort folder ───────────────────────────────────────────────────────────
async function sortFolder(folderNode, by, desc) {
  try {
    const newOrder = await invoke("sort_folder", { folderId: folderNode.id, by, desc });

    // Reorder tree children without re-rendering
    const folderItem = treeEl.querySelector(`.tree-item[data-id="${folderNode.id}"]`);
    const childrenEl = folderItem?.parentElement?.querySelector(":scope > .tree-children");
    if (childrenEl) {
      const orderMap = new Map(newOrder.map((id, idx) => [String(id), idx]));
      const wrappers = [...childrenEl.children];
      wrappers.sort((a, b) => {
        const ia = a.querySelector(".tree-item")?.dataset.id || "";
        const ib = b.querySelector(".tree-item")?.dataset.id || "";
        return (orderMap.get(ia) ?? 9999) - (orderMap.get(ib) ?? 9999);
      });
      wrappers.forEach(w => childrenEl.appendChild(w));
    }

    // Sync allNodes sort_idx from newOrder so loadFolderContents renders correct order
    newOrder.forEach((id, idx) => {
      const n = allNodes.find(n => n.id === id);
      if (n) n.sort_idx = idx;
    });

    // Reload grid if this folder is shown
    if (activeFolderId === folderNode.id) loadBookmarks(folderNode.id);
  } catch (err) { console.error("sort_folder:", err); }
}

// ── Folder context menu ───────────────────────────────────────────────────
// Opens a nested float submenu from an item already inside a float submenu.
function wireNestedFloat(trigger, buildFn) {
  let nestedEl = null;
  let nestedTimer = null;

  function openNested() {
    clearTimeout(nestedTimer);
    if (nestedEl) return;
    nestedEl = buildFn();
    nestedEl.style.cssText =
      "position:fixed;display:block;z-index:1002;background:#fff;" +
      "border:1px solid #ababab;box-shadow:2px 3px 8px rgba(0,0,0,.22);" +
      "padding:2px 0;min-width:170px;font-family:Segoe UI,system-ui,sans-serif;" +
      "font-size:12px;color:#000;user-select:none;";
    document.body.appendChild(nestedEl);

    const tr = trigger.getBoundingClientRect();
    const pr = trigger.closest("[style*='position:fixed']")?.getBoundingClientRect() || tr;
    let left = pr.right - 1, top = tr.top;
    const sw = nestedEl.offsetWidth, sh = nestedEl.offsetHeight;
    if (left + sw > window.innerWidth  - 4) left = pr.left - sw + 1;
    if (top  + sh > window.innerHeight - 4) top  = window.innerHeight - sh - 4;
    nestedEl.style.left = left + "px";
    nestedEl.style.top  = top  + "px";
    trigger.classList.add("sub-open");

    nestedEl.addEventListener("mouseenter", () => clearTimeout(nestedTimer));
    nestedEl.addEventListener("mouseleave", closeNested);
  }

  function closeNested() {
    nestedTimer = setTimeout(() => {
      nestedEl?.remove(); nestedEl = null;
      trigger.classList.remove("sub-open");
    }, 140);
  }

  trigger.addEventListener("mouseenter", openNested);
  trigger.addEventListener("mouseleave", (e) => {
    if (nestedEl?.contains(e.relatedTarget)) { clearTimeout(nestedTimer); return; }
    closeNested();
  });

  // Cleanup when parent float closes
  return () => { nestedEl?.remove(); nestedEl = null; };
}

function buildExportSubmenu(folderNode) {
  const sub = document.createElement("div");
  sub.className = "ctx-submenu";

  const doExport = (cmd, withImages) => {
    hideContextMenu();
    const args = { folderId: folderNode.id };
    if (withImages !== undefined) args.withImages = withImages;
    invoke(cmd, args).catch(err => { if (err !== "Отменено") console.error(err); });
  };

  sub.appendChild(ctxItem("import", "HTML файл",       null, () => doExport("export_folder_html")));
  sub.appendChild(ctxItem("props",  "Текстовый файл",  null, () => doExport("export_folder_txt")));
  sub.appendChild(ctxSep());

  // "Файл синхронизации" with nested submenu
  const syncEl = ctxItem("backup", "Файл синхронизации", null, null, false);
  syncEl.classList.add("ctx-has-sub");
  const syncArrow = document.createElement("span");
  syncArrow.className = "ctx-arrow";
  syncArrow.textContent = "▶";
  syncEl.appendChild(syncArrow);

  wireNestedFloat(syncEl, () => {
    const n = document.createElement("div");
    n.className = "ctx-submenu";
    n.appendChild(ctxItem("image",  "С рисунками",   null, () => doExport("export_folder_sync", true)));
    n.appendChild(ctxItem("delimg", "Без рисунков",  null, () => doExport("export_folder_sync", false)));
    return n;
  });
  sub.appendChild(syncEl);

  return sub;
}

// Generic: wires a float submenu from a main context menu trigger.
function wireMainContextFloat(trigger, buildSubFn) {
  const FLOAT_CSS =
    "position:fixed;display:block;z-index:1000;background:#fff;" +
    "border:1px solid #ababab;box-shadow:2px 3px 8px rgba(0,0,0,.22);" +
    "padding:2px 0;min-width:210px;" +
    "font-family:Segoe UI,system-ui,sans-serif;" +
    "font-size:12px;color:#000;user-select:none;";

  trigger.addEventListener("mouseenter", () => {
    clearTimeout(_subTimer);
    if (_subTrigger === trigger) return;
    if (_subEl) { _subEl.remove(); _subEl = null; }
    if (_subTrigger) _subTrigger.classList.remove("sub-open");
    _subTrigger = trigger;
    trigger.classList.add("sub-open");

    const sub = buildSubFn();
    sub.style.cssText = FLOAT_CSS;
    document.body.appendChild(sub);
    _subEl = sub;

    const mr = ctxMenuEl.getBoundingClientRect();
    const tr = trigger.getBoundingClientRect();
    const sw = sub.offsetWidth, sh = sub.offsetHeight;
    let left = mr.right - 1, top = tr.top;
    if (left + sw > window.innerWidth  - 4) left = mr.left - sw + 1;
    if (top  + sh > window.innerHeight - 4) top  = window.innerHeight - sh - 4;
    sub.style.left = left + "px";
    sub.style.top  = top  + "px";

    sub.addEventListener("mouseenter", () => clearTimeout(_subTimer));
    sub.addEventListener("mouseleave", scheduleSubClose);
  });
  trigger.addEventListener("mouseleave", (e) => {
    if (_subEl?.contains(e.relatedTarget)) { clearTimeout(_subTimer); return; }
    scheduleSubClose();
  });
}

// Tracks last sort per folder: Map<folderId, {by, desc}>
const _sortState = new Map();

function buildSortSubmenu(folderNode) {
  const sub = document.createElement("div");
  sub.className = "ctx-submenu";
  const cur = _sortState.get(folderNode.id);

  const S = (label, by) => {
    const active = cur?.by === by;
    const desc   = active ? !cur.desc : false; // toggle on repeat click
    const arrow  = active ? (cur.desc ? " ▼" : " ▲") : "";
    return ctxItem(null, label + arrow, null, () => {
      _sortState.set(folderNode.id, { by, desc });
      hideContextMenu();
      sortFolder(folderNode, by, desc);
    });
  };

  sub.appendChild(S("По имени",         "title"));
  sub.appendChild(ctxSep());
  sub.appendChild(S("По дате добавления", "created"));
  sub.appendChild(ctxSep());
  sub.appendChild(S("По URL",           "url"));
  return sub;
}

function buildFolderImportSubmenu(folderNode) {
  const sub = document.createElement("div");
  sub.className = "ctx-submenu";

  const addItem = (icon, label, action) => {
    sub.appendChild(ctxItem(icon, label, null, () => {
      hideContextMenu();
      invokeFolderImport(action, folderNode.id);
    }));
  };

  addItem("browser", "Из браузера...",     "import-from-browser");
  sub.appendChild(ctxSep());
  addItem("import",  "Из файла HTML",      "import-html");
  addItem("import",  "Из файла TXT",       "import-txt-lines");
  sub.appendChild(ctxSep());
  addItem("backup",  "Файл синхронизации", "import-sync");
  addItem("folder",  "Из ua.dat...",       "import-folder");

  return sub;
}

async function invokeFolderImport(action, parentId) {
  try {
    let count = 0;
    switch (action) {
      case 'import-from-browser':
        openBrowserImportDialog();
        return;
      case 'import-html':
        count = await invoke('import_html', { parentId });
        break;
      case 'import-txt-lines':
        count = await invoke('import_txt_lines', { parentId });
        break;
      case 'import-sync':
        count = await invoke('import_sync', { parentId });
        break;
      case 'import-folder':
        count = await invoke('import_uadat_pick', { parentId });
        break;
    }
    if (count > 0) {
      allNodes = await invoke('get_tree');
      allFolders = allNodes.filter(n => n.kind === 'folder');
      renderTree();
      selectFolder(parentId);
    }
  } catch(e) {
    if (e !== 'Отменено') console.error('invokeFolderImport:', e);
  }
}

function addSubTrigger(label, icon) {
  const el = ctxItem(icon, label, null, null, false);
  el.classList.add("ctx-has-sub");
  const arrow = document.createElement("span");
  arrow.className = "ctx-arrow";
  arrow.textContent = "▶";
  el.appendChild(arrow);
  return el;
}

function showFolderContextMenu(e, folderNode) {
  e.preventDefault();
  e.stopPropagation();
  closeSubFloat();
  ctxMenuEl.innerHTML = "";

  // ── Folder management ──────────────────────────────────────────────────
  ctxMenuEl.appendChild(ctxItem("new", "Новая папка", null, () => {
    hideContextMenu();
    createFolderAndRename(folderNode.id);
  }));
  ctxMenuEl.appendChild(ctxItem("edit", "Переименовать", "F2", () => {
    hideContextMenu();
    startInlineRename(folderNode.id);
  }));
  ctxMenuEl.appendChild(ctxItem("trash", "Удалить", "Del", () => {
    hideContextMenu();
    deleteFolder(folderNode);
  }));
  ctxMenuEl.appendChild(ctxSep());

  // ── Transfer ───────────────────────────────────────────────────────────
  const impEl = addSubTrigger("Импорт в папку", "import");
  wireMainContextFloat(impEl, () => buildFolderImportSubmenu(folderNode));
  ctxMenuEl.appendChild(impEl);

  const expEl = addSubTrigger("Экспорт папки", "backup");
  wireMainContextFloat(expEl, () => buildExportSubmenu(folderNode));
  ctxMenuEl.appendChild(expEl);
  ctxMenuEl.appendChild(ctxSep());

  // ── Operations ─────────────────────────────────────────────────────────
  ctxMenuEl.appendChild(ctxItem("verify", "Проверить ссылки", null, () => {
    hideContextMenu();
    openCheckerPanel(folderNode);
  }));
  ctxMenuEl.appendChild(ctxItem("favicon", "Обновить favicon'ы", null, () => {
    hideContextMenu();
    startFaviconBatch(folderNode, true);
  }));
  ctxMenuEl.appendChild(ctxItem("refresh", "Обновить рисунки", null, () => {
    hideContextMenu();
    startThumbBatch(folderNode);
  }));
  ctxMenuEl.appendChild(ctxSep());

  // ── Sort ───────────────────────────────────────────────────────────────
  const sortEl = addSubTrigger("Сортировка", "sort");
  wireMainContextFloat(sortEl, () => buildSortSubmenu(folderNode));
  ctxMenuEl.appendChild(sortEl);
  ctxMenuEl.appendChild(ctxSep());

  // ── Properties ─────────────────────────────────────────────────────────
  ctxMenuEl.appendChild(ctxItem("props", "Свойства", "F4", () => {
    hideContextMenu();
    openFolderPropsDialog(folderNode);
  }));

  ctxMenuEl.querySelectorAll(".ctx-item:not(.ctx-has-sub)").forEach(it => {
    it.addEventListener("mouseenter", () => { clearTimeout(_subTimer); closeSubFloat(); });
  });

  ctxMenuEl.classList.remove("hidden");
  const mw = ctxMenuEl.offsetWidth, mh = ctxMenuEl.offsetHeight;
  ctxMenuEl.style.left = Math.min(e.clientX, window.innerWidth  - mw - 4) + "px";
  ctxMenuEl.style.top  = Math.min(e.clientY, window.innerHeight - mh - 4) + "px";
}

// ── Thumb operations ─────────────────────────────────────────────────────
let _isRefreshing = false;
const detailLoadingOverlay = document.getElementById("detail-loading-overlay");

async function refreshThumb(node) {
  if (!node.url || _isRefreshing) return;
  _isRefreshing = true;

  // Switch to detail view so the overlay is visible
  if (activeBookmarkNode?.id !== node.id) {
    showDetailView(node);
    await new Promise(r => setTimeout(r, 40));
  }

  detailLoadingOverlay.classList.add("visible");

  try {
    const newPath = await invoke("refresh_thumb", {
      id: node.id,
      url: node.url,
      width:   appSettings.thumbWidth   || 1280,
      height:  appSettings.thumbHeight  || 800,
      timeout: appSettings.thumbTimeout || 15,
    });

    if (activeBookmarkNode?.id === node.id) activeBookmarkNode.thumb = newPath;
    _applyThumbToCard(node.id, node.title, newPath);

    // Update detail image
    detailImgEl.src             = convertFileSrc(thumbFilePath(newPath)) + "?t=" + Date.now();
    detailImgEl.style.display   = "";
    detailNoImgEl.style.display = "none";
    detailImgEl.onerror = () => {};
  } catch (err) {
    console.error("refresh_thumb:", err);
  } finally {
    detailLoadingOverlay.classList.remove("visible");
    _isRefreshing = false;
  }
}

function _applyThumbToCard(id, title, newPath) {
  const n = allNodes.find(n => n.id === id);
  if (n) n.thumb = newPath;
  const card = gridEl.querySelector(`.card[data-id="${id}"]`);
  if (!card) return;
  card.dataset.thumb = newPath;
  const thumbDiv = card.querySelector('.card-thumb');
  if (!thumbDiv) return;
  thumbDiv.innerHTML = '';
  const img = document.createElement('img');
  img.src = convertFileSrc(thumbFilePath(newPath));
  img.onerror = () => { img.remove(); thumbDiv.appendChild(makeNoImg(title)); };
  thumbDiv.appendChild(img);
}

async function clearThumb(node) {
  await invoke("clear_thumb", { id: node.id }).catch(console.error);
  const n = allNodes.find(n => n.id === node.id);
  if (n) n.thumb = null;

  // Update grid card
  const card = gridEl.querySelector(`.card[data-id="${node.id}"]`);
  if (card) {
    const t = card.querySelector(".card-thumb");
    if (t) { t.innerHTML = ""; t.appendChild(makeNoImg(node.title)); }
  }

  // Update detail view
  if (activeBookmarkNode?.id === node.id) {
    activeBookmarkNode.thumb = null;
    detailImgEl.style.display  = "none";
    detailNoImgEl.style.display = "";
    setNoImgPlaceholder({ ...node, thumb: null });
  }
}

// ── Confirm dialog ────────────────────────────────────────────────────────
const confirmOverlay = document.getElementById("confirm-overlay");
const confirmMsg     = document.getElementById("confirm-msg");
let   _confirmCb     = null;

// Bring any overlay to the DOM front (correct stacking when multiple overlays are open)
function raiseOverlay(el) {
  document.body.appendChild(el);
  el.classList.remove("hidden");
}

function showConfirm(msg, onOk) {
  confirmMsg.textContent = msg;
  _confirmCb = onOk;
  raiseOverlay(confirmOverlay); // confirm must be above ALL other dialogs
  setTimeout(() => document.getElementById("confirm-ok")?.focus(), 20);
}
function deleteConfirm(msg, onOk) {
  if (!appSettings.confirmDelete) { onOk(); return; }
  showConfirm(msg, onOk);
}
function closeConfirm() { confirmOverlay.classList.add("hidden"); _confirmCb = null; }

document.getElementById("confirm-x").onclick      = closeConfirm;
document.getElementById("confirm-cancel").onclick  = closeConfirm;
document.getElementById("confirm-ok").onclick      = () => { const cb = _confirmCb; closeConfirm(); cb?.(); };
confirmOverlay.addEventListener("click", (e) => { if (e.target === confirmOverlay) closeConfirm(); });

// ── Delete bookmark ───────────────────────────────────────────────────────
// Returns all currently visible (not inside closed branch) tree items.
function getVisibleTreeItems() {
  return [...treeEl.querySelectorAll(".tree-item")].filter(el => {
    let p = el.parentElement;
    while (p && p !== treeEl) {
      if (p.classList.contains("tree-children") && !p.classList.contains("open")) return false;
      p = p.parentElement;
    }
    return true;
  });
}

// Finds the best item to select after deleting deletedId.
function findNextAfterDelete(deletedId) {
  const items = getVisibleTreeItems();
  const idx   = items.findIndex(el => el.dataset.id === String(deletedId));
  if (idx < 0) return null;
  if (idx + 1 < items.length) return items[idx + 1];  // next down
  if (idx - 1 >= 0)           return items[idx - 1];  // previous up
  return null;
}

// Updates the count badge on a folder after one bookmark is removed.
function decrementFolderBadge(parentId) {
  if (parentId == null) return;
  const pNode = allNodes.find(n => n.id === parentId && n.kind === "folder");
  if (!pNode || pNode.count <= 0) return;
  pNode.count--;
  const folderItem = treeEl.querySelector(`.tree-item[data-id="${parentId}"]`);
  const badge = folderItem?.querySelector(".tree-count");
  if (!badge) return;
  if (pNode.count > 0) badge.textContent = pNode.count;
  else badge.remove();
}

function deleteBookmark(node) {
  deleteConfirm(`Удалить «${node.title}»?`, async () => {
    // Capture next item BEFORE DOM changes
    const nextEl = findNextAfterDelete(node.id);

    await invoke("delete_node", { id: node.id }).catch(console.error);

    // Update in-memory state
    const idx = allNodes.findIndex(n => n.id === node.id);
    if (idx >= 0) allNodes.splice(idx, 1);
    allFolders = allNodes.filter(n => n.kind === "folder");

    // Surgical DOM update — no renderTree(), no collapse
    decrementFolderBadge(node.parent);
    treeEl.querySelector(`.tree-item[data-id="${node.id}"]`)?.parentElement?.remove();
    gridEl.querySelector(`.card[data-id="${node.id}"]`)?.remove();

    // If grid now empty, show hint
    if (activeFolderId === node.parent && gridEl.children.length === 0) {
      emptyHint.classList.remove("hidden");
    }

    // If detail view was showing this node, clear it
    if (activeBookmarkNode?.id === node.id) {
      activeBookmarkNode = null;
      hideDetailView();
    }

    // Navigate to next/prev item
    if (nextEl) {
      const nextNode = allNodes.find(n => n.id === parseInt(nextEl.dataset.id));
      if (!nextNode) return;
      nextEl.scrollIntoView({ block: "nearest" });
      if (nextNode.kind === "folder") selectFolder(nextNode.id);
      else selectTreeBookmark(nextNode);
    } else if (node.parent != null) {
      // Nothing left — go to parent folder
      selectFolder(node.parent);
    }
  });
}

// ── Draggable dialog helper ────────────────────────────────────────────────
function makeDlgDraggable(dlgEl, handleEl) {
  let drag = { on: false, ox: 0, oy: 0 };
  handleEl.addEventListener("mousedown", (e) => {
    if (e.target.closest("button")) return;
    const r = dlgEl.getBoundingClientRect();
    dlgEl.style.position = "fixed";
    dlgEl.style.margin   = "0";
    dlgEl.style.bottom   = "auto";
    dlgEl.style.right    = "auto";
    dlgEl.style.left     = r.left + "px";
    dlgEl.style.top      = r.top  + "px";
    drag = { on: true, ox: e.clientX - r.left, oy: e.clientY - r.top };
    e.preventDefault();
  });
  document.addEventListener("mousemove", (e) => {
    if (!drag.on) return;
    const w = dlgEl.offsetWidth, h = dlgEl.offsetHeight;
    dlgEl.style.left = Math.max(0, Math.min(window.innerWidth  - w, e.clientX - drag.ox)) + "px";
    dlgEl.style.top  = Math.max(0, Math.min(window.innerHeight - h, e.clientY - drag.oy)) + "px";
  });
  document.addEventListener("mouseup", () => { drag.on = false; });
}

function parseUADate(s) {
  if (!s || s.length < 12) return "—";
  return `${s.slice(0,2)}.${s.slice(2,4)}.20${s.slice(4,6)} ${s.slice(6,8)}:${s.slice(8,10)}:${s.slice(10,12)}`;
}

// ── Link properties dialog ─────────────────────────────────────────────────
const propsOverlay = document.getElementById("props-overlay");
const propsTitle   = document.getElementById("props-title");
const propsUrl     = document.getElementById("props-url");
const propsNote    = document.getElementById("props-note");
let   propsNode    = null;

makeDlgDraggable(document.getElementById("props-dlg"), document.querySelector("#props-dlg .props-drag-handle"));

function openPropsDialog(node) {
  propsNode        = node;
  propsTitle.value = node.title || "";
  propsUrl.value   = node.url   || "";
  propsNote.value  = node.note  || "";
  const thumbEl = document.getElementById("props-thumb");
  thumbEl.textContent = node.thumb || "—";
  thumbEl.title       = node.thumb || "";
  // Reset drag position
  const dlg = document.getElementById("props-dlg");
  dlg.style.position = ""; dlg.style.left = ""; dlg.style.top = ""; dlg.style.margin = "";
  raiseOverlay(propsOverlay);
  setTimeout(() => propsTitle.focus(), 30);
}

function closePropsDialog() { propsOverlay.classList.add("hidden"); propsNode = null; }

async function savePropsDialog() {
  if (!propsNode) return false;
  const title = propsTitle.value.trim();
  const url   = propsUrl.value.trim();
  const note  = propsNote.value;
  if (!title) { propsTitle.focus(); return false; }
  await invoke("update_bookmark", { id: propsNode.id, title, url, note }).catch(console.error);
  const n = allNodes.find(n => n.id === propsNode.id);
  if (n) Object.assign(n, { title, url, note });
  const ti = treeEl.querySelector(`.tree-item[data-id="${propsNode.id}"] .label`);
  if (ti) ti.textContent = title;
  const card = gridEl.querySelector(`.card[data-id="${propsNode.id}"]`);
  if (card) {
    const ct = card.querySelector(".row-name"), cu = card.querySelector(".row-addr");
    if (ct) ct.textContent = title;
    if (cu) { cu.textContent = url; cu.title = url; }
    card.dataset.url = url;
  }
  if (activeBookmarkNode?.id === propsNode.id) {
    Object.assign(activeBookmarkNode, { title, url, note });
    // Immediately refresh detail panel — no restart or re-select needed
    detailUrlEl.textContent = url;
    detailUrlEl.title = url;
    detailNoteEl.textContent = note;
    breadcrumb.textContent = (propsNode.parent != null
      ? buildBreadcrumbText(propsNode.parent) + "  /  " : "") + title;
  }
  return true;
}

document.getElementById("props-x").onclick      = closePropsDialog;
document.getElementById("props-cancel").onclick  = closePropsDialog;
document.getElementById("props-ok").onclick      = async () => { if (await savePropsDialog()) closePropsDialog(); };
// No click-outside-to-close — user could lose typed data
propsOverlay.addEventListener("keydown", (e) => {
  if (e.key === "Enter")  document.getElementById("props-ok").click();
  if (e.key === "Escape") closePropsDialog();
});

// ── Folder properties dialog ───────────────────────────────────────────────
const fpropsOverlay = document.getElementById("fprops-overlay");
const fpropsTitle   = document.getElementById("fprops-title");
let   fpropsNode    = null;

makeDlgDraggable(document.getElementById("fprops-dlg"), document.querySelector("#fprops-dlg .props-drag-handle"));

function getFolderStats(folderId) {
  let links = 0, folders = 0;
  const visit = (id) => {
    allNodes.filter(n => n.parent === id).forEach(n => {
      if (n.kind === "bookmark") links++;
      else { folders++; visit(n.id); }
    });
  };
  visit(folderId);
  return { links, folders };
}

function openFolderPropsDialog(node) {
  fpropsNode        = node;
  fpropsTitle.value = node.title || "";
  const stats = getFolderStats(node.id);
  document.getElementById("fprops-links").textContent   = stats.links;
  document.getElementById("fprops-folders").textContent = stats.folders;
  document.getElementById("fprops-path").textContent    = getFolderPath(node.id) || node.title;
  const dlg = document.getElementById("fprops-dlg");
  dlg.style.position = ""; dlg.style.left = ""; dlg.style.top = ""; dlg.style.margin = "";
  raiseOverlay(fpropsOverlay);
  setTimeout(() => fpropsTitle.focus(), 30);
}

function closeFolderPropsDialog() { fpropsOverlay.classList.add("hidden"); fpropsNode = null; }

async function saveFolderPropsDialog() {
  if (!fpropsNode) return false;
  const title = fpropsTitle.value.trim();
  if (!title) { fpropsTitle.focus(); return false; }
  await invoke("rename_node", { id: fpropsNode.id, title }).catch(console.error);
  const n = allNodes.find(n => n.id === fpropsNode.id);
  if (n) n.title = title;
  allFolders = allNodes.filter(n => n.kind === "folder");
  const ti = treeEl.querySelector(`.tree-item[data-id="${fpropsNode.id}"] .label`);
  if (ti) ti.textContent = title;
  return true;
}

document.getElementById("fprops-x").onclick      = closeFolderPropsDialog;
document.getElementById("fprops-cancel").onclick  = closeFolderPropsDialog;
document.getElementById("fprops-ok").onclick      = async () => { if (await saveFolderPropsDialog()) closeFolderPropsDialog(); };
// No click-outside-to-close — user could lose typed data
fpropsOverlay.addEventListener("keydown", (e) => {
  if (e.key === "Enter")  document.getElementById("fprops-ok").click();
  if (e.key === "Escape") closeFolderPropsDialog();
});

// ── DB Properties dialog ──────────────────────────────────────────────────
const dbpropsOverlay = document.getElementById('dbprops-overlay');

function openDbPropertiesDialog() {
  invoke('get_db_properties')
    .then(props => {
      document.getElementById('dbp-path').textContent      = props.path;
      document.getElementById('dbp-size').textContent      = formatBytes(props.size_bytes);
      document.getElementById('dbp-folders').textContent   = props.folder_count;
      document.getElementById('dbp-bookmarks').textContent = props.bookmark_count;
      const dlg = document.getElementById('dbprops-dlg');
      dlg.style.position = ''; dlg.style.left = ''; dlg.style.top = ''; dlg.style.margin = '';
      raiseOverlay(dbpropsOverlay);
      dbpropsOverlay.classList.remove('hidden');
    })
    .catch(console.error);
}

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

document.getElementById('dbprops-x').onclick    = () => dbpropsOverlay.classList.add('hidden');
document.getElementById('dbp-ok-btn').onclick   = () => dbpropsOverlay.classList.add('hidden');
document.getElementById('dbp-clear-btn').onclick = () => {
  dbpropsOverlay.classList.add('hidden');
  handleMenuAction('clear-db');
};

makeDlgDraggable(
  document.getElementById('dbprops-dlg'),
  document.querySelector('#dbprops-dlg .props-drag-handle')
);

// ── Active link helper ────────────────────────────────────────────────────
function getActiveLink() {
  return activeBookmarkNode
    || allNodes.find(n => String(n.id) === gridEl.querySelector(".card.selected")?.dataset.id);
}

// ── Browser list ─────────────────────────────────────────────────────────
let browsers        = [{ name: "Браузер по умолчанию", path: "default" }];
let defaultBrowserPath = "default";

// File-based portable storage for browser list
async function loadBrowsersConfig() {
  try {
    const json = await invoke('load_browsers_config');
    if (json) {
      const data = JSON.parse(json);
      if (Array.isArray(data.browsers) && data.browsers.length) browsers = data.browsers;
      if (data.default) defaultBrowserPath = data.default;
      return;
    }
  } catch(e) { /* file not yet created */ }
  // Migrate from localStorage if exists
  try {
    const s = localStorage.getItem("ua_browsers");
    if (s) {
      const parsed = JSON.parse(s);
      if (Array.isArray(parsed) && parsed.length) {
        browsers = parsed;
        defaultBrowserPath = localStorage.getItem("ua_default_browser") || "default";
        await saveBrowsersConfig();
        localStorage.removeItem("ua_browsers");
        localStorage.removeItem("ua_default_browser");
      }
    }
  } catch(e) {}
}

async function saveBrowsersConfig() {
  try {
    await invoke('save_browsers_config', {
      json: JSON.stringify({ browsers, default: defaultBrowserPath })
    });
  } catch(e) { console.error('saveBrowsersConfig:', e); }
}

function saveBrowsers() { saveBrowsersConfig(); }
function getDefaultBrowserPath() { return defaultBrowserPath; }
function setDefaultBrowserPath(p) { defaultBrowserPath = p; saveBrowsersConfig(); }

function openWithBrowser(url, path) {
  invoke("open_url_with", { url, browser: path || "default" }).catch(console.error);
}

// ── Open With picker ─────────────────────────────────────────────────────
(function () {
  const overlay = document.getElementById("openwith-overlay");
  const list    = document.getElementById("openwith-list");
  let   _owUrl  = "", _owPath = "default";

  window.showOpenWithDialog = function (url) {
    if (!url) return;
    if (browsers.length <= 1) { openWithBrowser(url, "default"); return; }
    _owUrl = url; _owPath = browsers[0]?.path || "default";
    list.innerHTML = "";
    browsers.forEach((b, i) => {
      const el = document.createElement("div");
      el.className = "dlg-browser-entry" + (i === 0 ? " selected" : "");
      el.textContent = b.name;
      el.addEventListener("click", () => {
        list.querySelectorAll(".dlg-browser-entry").forEach(e => e.classList.remove("selected"));
        el.classList.add("selected"); _owPath = b.path;
      });
      el.addEventListener("dblclick", () => { openWithBrowser(_owUrl, b.path); overlay.classList.add("hidden"); });
      list.appendChild(el);
    });
    raiseOverlay(overlay);
  };

  document.getElementById("openwith-x").onclick      = () => overlay.classList.add("hidden");
  document.getElementById("openwith-cancel").onclick  = () => overlay.classList.add("hidden");
  document.getElementById("openwith-ok").onclick      = () => { openWithBrowser(_owUrl, _owPath); overlay.classList.add("hidden"); };
  overlay.addEventListener("click", (e) => { if (e.target === overlay) overlay.classList.add("hidden"); });
})();

// ── Context menu ─────────────────────────────────────────────────────────
let ctxTarget = null;
const ctxMenuEl = document.getElementById("ctx-menu");

function ctxItem(icon, label, shortcut, action, disabled) {
  const el  = document.createElement("div");
  el.className = "ctx-item" + (disabled ? " ctx-disabled" : "");

  const ic = document.createElement("span");
  ic.className = "ctx-icon";
  ic.innerHTML = ICONS[icon] || "";

  const tx = document.createElement("span");
  tx.className = "ctx-text";
  tx.textContent = label;

  el.append(ic, tx);

  if (shortcut) {
    const sc = document.createElement("span");
    sc.className = "ctx-shortcut";
    sc.textContent = shortcut;
    el.appendChild(sc);
  }

  if (!disabled && action) {
    el.addEventListener("click", () => { hideContextMenu(); action(); });
  }
  return el;
}

function ctxSep() {
  const el = document.createElement("div");
  el.className = "ctx-sep";
  return el;
}

// ── Floating submenu (appended to body, position:fixed) ──────────────────
let _subTimer   = null;
let _subTrigger = null;   // the .ctx-has-sub element currently active
let _subEl      = null;   // the floating submenu div in document.body

function closeSubFloat() {
  clearTimeout(_subTimer);
  if (_subEl)      { _subEl.remove(); _subEl = null; }
  if (_subTrigger) { _subTrigger.classList.remove("sub-open"); _subTrigger = null; }
}

function scheduleSubClose() {
  _subTimer = setTimeout(closeSubFloat, 160);
}

function openSubFloat(trigger, node) {
  clearTimeout(_subTimer);
  if (_subTrigger === trigger) return;   // already open for this trigger

  // Close previous
  if (_subEl) { _subEl.remove(); _subEl = null; }
  if (_subTrigger) _subTrigger.classList.remove("sub-open");

  _subTrigger = trigger;
  trigger.classList.add("sub-open");

  // Build submenu items
  const sub = document.createElement("div");
  sub.className = "ctx-submenu";
  browsers.forEach(b => {
    const it = ctxItem("openwith", b.name, null,
      () => { hideContextMenu(); openWithBrowser(node.url, b.path); });
    sub.appendChild(it);
  });
  sub.appendChild(ctxSep());
  sub.appendChild(ctxItem("gear", "Список браузеров...", null,
    () => { hideContextMenu(); openBrowsersDialog(); }));

  // Fixed positioning — no parent stacking context issues
  sub.style.position = "fixed";
  sub.style.display  = "block";
  sub.style.zIndex   = "1000";
  document.body.appendChild(sub);
  _subEl = sub;

  // Calculate position from menu and trigger rects
  const menuR = ctxMenuEl.getBoundingClientRect();
  const trigR = trigger.getBoundingClientRect();
  const sw    = sub.offsetWidth;
  const sh    = sub.offsetHeight;

  let left = menuR.right - 1;           // flush against menu right edge
  let top  = trigR.top;

  if (left + sw > window.innerWidth  - 4) left = menuR.left - sw + 1;
  if (top  + sh > window.innerHeight - 4) top  = window.innerHeight - sh - 4;

  sub.style.left = left + "px";
  sub.style.top  = top  + "px";

  // Keep submenu alive while mouse is over it
  sub.addEventListener("mouseenter", () => clearTimeout(_subTimer));
  sub.addEventListener("mouseleave", scheduleSubClose);
}

function wireSubFloat(trigger, node) {
  trigger.addEventListener("mouseenter", () => openSubFloat(trigger, node));
  trigger.addEventListener("mouseleave", (e) => {
    // If moving directly into the floating submenu — don't close
    if (_subEl && _subEl.contains(e.relatedTarget)) {
      clearTimeout(_subTimer);
      return;
    }
    scheduleSubClose();
  });
}

function showContextMenu(e, node) {
  e.preventDefault();
  e.stopPropagation();
  ctxTarget = node;
  closeSubFloat();
  ctxMenuEl.innerHTML = "";

  const hasThumb = !!node.thumb;

  ctxMenuEl.appendChild(
    ctxItem("link", "Открыть", null,
      () => openWithBrowser(node.url, getDefaultBrowserPath()))
  );

  // "Открыть с помощью" — NO submenu child; float created on hover
  const owEl = ctxItem("openwith", "Открыть с помощью", null, null, false);
  owEl.classList.add("ctx-has-sub");
  const arrow = document.createElement("span");
  arrow.className = "ctx-arrow";
  arrow.textContent = "▶";
  owEl.appendChild(arrow);
  wireSubFloat(owEl, node);     // wire float submenu
  ctxMenuEl.appendChild(owEl);

  ctxMenuEl.appendChild(ctxSep());

  ctxMenuEl.appendChild(
    ctxItem("image",   "Открыть рисунок",  "F12",
      () => invoke("open_file", { path: thumbFilePath(node.thumb) }), !hasThumb)
  );
  ctxMenuEl.appendChild(
    ctxItem("refresh", "Обновить рисунок", null,
      () => refreshThumb(node), !node.url)
  );
  ctxMenuEl.appendChild(
    ctxItem("delimg",  "Удалить рисунок",  null,
      () => clearThumb(node), !hasThumb)
  );
  ctxMenuEl.appendChild(ctxSep());
  ctxMenuEl.appendChild(
    ctxItem("favicon", "Загрузить favicon", null,
      () => { hideContextMenu(); loadSingleFavicon(node); },
      !node.url)
  );

  ctxMenuEl.appendChild(ctxSep());

  ctxMenuEl.appendChild(ctxItem("trash",  "Удалить ссылку", "Del",
    () => deleteBookmark(node)));

  ctxMenuEl.appendChild(ctxSep());

  ctxMenuEl.appendChild(
    ctxItem("copy", "Копировать URL", "Ctrl+C",
      () => navigator.clipboard.writeText(node.url || "").catch(() => {}))
  );
  ctxMenuEl.appendChild(ctxItem("props", "Свойства", "F4",
    () => { hideContextMenu(); openPropsDialog(node); }));

  // Other items: close float submenu on hover
  ctxMenuEl.querySelectorAll(".ctx-item:not(.ctx-has-sub)").forEach(it => {
    it.addEventListener("mouseenter", () => {
      clearTimeout(_subTimer);
      closeSubFloat();
    });
  });

  // Position: avoid viewport overflow
  ctxMenuEl.classList.remove("hidden");
  const mw = ctxMenuEl.offsetWidth, mh = ctxMenuEl.offsetHeight;
  ctxMenuEl.style.left = Math.min(e.clientX, window.innerWidth  - mw - 4) + "px";
  ctxMenuEl.style.top  = Math.min(e.clientY, window.innerHeight - mh - 4) + "px";
}

function hideContextMenu() {
  closeSubFloat();
  ctxMenuEl.classList.add("hidden");
}

// Capture phase: fires before ANY element's stopPropagation
document.addEventListener("mousedown", (e) => {
  if (ctxMenuEl.classList.contains("hidden")) return;
  if (ctxMenuEl.contains(e.target))  return;  // click inside main menu
  if (_subEl?.contains(e.target))    return;  // click inside float submenu
  hideContextMenu();
}, true);

// Close on Escape (already handled elsewhere, but add safety)
// Close on window losing focus (e.g. Alt+Tab)
window.addEventListener("blur", () => {
  if (!ctxMenuEl.classList.contains("hidden")) hideContextMenu();
});

// ── Browsers dialog ───────────────────────────────────────────────────────
let dlgBrowsers = [];
let dlgSelIdx   = -1;

const browsersOverlay  = document.getElementById("browsers-overlay");
const dlgBrowserList   = document.getElementById("dlg-browser-list");
const dlgDefaultSel    = document.getElementById("dlg-default-sel");
const dlgDelBtn        = document.getElementById("dlg-del");
const dlgPropsBtn      = document.getElementById("dlg-props");

// ── Browser import dialog ─────────────────────────────────────────────────────

function browserIconSvg(id) {
  const norm = (id || '').toLowerCase();
  let cfg;
  if (norm.startsWith('opera') || norm.includes('opera'))
    cfg = norm.includes('gx') ? { bg: '#FF1B2D', letter: 'G' } : { bg: '#FF1B2D', letter: 'O' };
  else cfg = {
    chrome:   { bg: '#4285F4', letter: 'C' },
    edge:     { bg: '#0078D4', letter: 'E' },
    firefox:  { bg: '#FF7139', letter: 'F' },
    brave:    { bg: '#FB542B', letter: 'B' },
    vivaldi:  { bg: '#EF3939', letter: 'V' },
    chromium: { bg: '#4285F4', letter: '⬡' },
    waterfox: { bg: '#00AEF0', letter: 'W' },
    librewolf:{ bg: '#00ACAC', letter: 'L' },
  }[norm] || { bg: '#888', letter: '?' };
  return `<svg width="18" height="18" viewBox="0 0 18 18"><circle cx="9" cy="9" r="9" fill="${cfg.bg}"/><text x="9" y="13.5" text-anchor="middle" fill="white" font-family="Arial,sans-serif" font-weight="700" font-size="11">${cfg.letter}</text></svg>`;
}

// ── Browser import dialog (full) ─────────────────────────────────────────────

const biOverlay    = document.getElementById('browser-import-overlay');
const biList       = document.getElementById('bi-list');
const biStatus     = document.getElementById('bi-status');
const biSelPathEl  = document.getElementById('bi-sel-path');
const biOkBtn      = document.getElementById('bi-ok');

let biDetected  = [];
let biSelIdx    = -1;
let biManual    = null; // { path, kind, name } — portable/manual selection

function biReset() {
  biSelIdx = -1; biManual = null;
  biSelPathEl.textContent = '';
  biStatus.textContent    = '';
  biOkBtn.disabled        = true;
  biList.querySelectorAll('.dlg-browser-entry').forEach(e => e.classList.remove('selected'));
}

document.getElementById('bi-x').addEventListener('click', () => biOverlay.classList.add('hidden'));
document.getElementById('bi-cancel').addEventListener('click', () => biOverlay.classList.add('hidden'));
biOkBtn.addEventListener('click', doBrowserImport);

// "Файл закладок…" — pick Bookmarks / places.sqlite directly
document.getElementById('bi-pick-file').addEventListener('click', async () => {
  const path = await invoke('pick_bookmarks_file').catch(() => null);
  if (!path) return;
  biReset();
  const fname  = path.split(/[\\/]/).pop();
  const isFF   = fname.toLowerCase().endsWith('.sqlite');
  const name   = guessNameFromBookmarksPath(path);
  biManual     = { path, kind: isFF ? 'firefox' : 'chromium', name };
  biSelPathEl.textContent = path;
  biOkBtn.disabled = false;
});

// "Папку профиля…" — pick profile folder, auto-detect bookmarks inside
document.getElementById('bi-pick-folder').addEventListener('click', async () => {
  const folder = await invoke('pick_profile_folder').catch(() => null);
  if (!folder) return;
  biReset();
  const found = await invoke('find_bookmarks_in_folder', { folder }).catch(() => null);
  if (!found) {
    biSelPathEl.textContent = '';
    biStatus.textContent = 'Файл закладок не найден в выбранной папке.';
    return;
  }
  const name = guessNameFromBookmarksPath(folder);
  biManual   = { path: found.path, kind: found.kind, name };
  biSelPathEl.textContent = found.path;
  biOkBtn.disabled = false;
});

function guessNameFromBookmarksPath(path) {
  // Walk path segments to find a recognizable browser name
  const parts = path.replace(/\\/g, '/').split('/');
  const known = {
    'opera stable': 'Opera', 'opera gx stable': 'Opera GX',
    'opera developer': 'Opera Developer', 'opera beta': 'Opera Beta',
    'opera neon': 'Opera Neon',
    'google': 'Chrome', 'chrome': 'Chrome',
    'microsoft': 'Edge', 'edge': 'Edge',
    'mozilla firefox': 'Firefox', 'waterfox': 'Waterfox',
    'librewolf': 'LibreWolf', 'brave-browser': 'Brave', 'vivaldi': 'Vivaldi',
  };
  for (let i = parts.length - 1; i >= 0; i--) {
    const lc = parts[i].toLowerCase();
    if (known[lc]) return known[lc];
    // Check partial match
    for (const [k, v] of Object.entries(known)) {
      if (lc.includes(k) || k.includes(lc)) return v;
    }
  }
  // Fallback: last non-generic directory name
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i];
    if (p && !['default', 'user data', 'bookmarks', 'profiles'].includes(p.toLowerCase())) {
      return p;
    }
  }
  return 'Браузер';
}

async function openBrowserImportDialog() {
  biReset();
  biList.innerHTML = '<div style="padding:8px;color:#888;font-size:11px">Поиск браузеров…</div>';
  raiseOverlay(biOverlay);

  try { biDetected = await invoke('detect_browsers'); }
  catch(e) { biDetected = []; }

  biList.innerHTML = '';
  if (!biDetected.length) {
    biList.innerHTML = '<div style="padding:8px;color:#888;font-size:11px">Установленные браузеры не найдены.<br>Используйте кнопки ниже.</div>';
    return;
  }

  biDetected.forEach((b, i) => {
    const el = document.createElement('div');
    el.className = 'dlg-browser-entry bi-entry';
    el.innerHTML = `<span class="bi-icon">${browserIconSvg(b.id)}</span><span style="flex:1">${b.name}</span>`;
    el.title = b.bookmarks_path || '';
    el.addEventListener('click', () => {
      biList.querySelectorAll('.dlg-browser-entry').forEach(e => e.classList.remove('selected'));
      el.classList.add('selected');
      biSelIdx = i; biManual = null;
      biSelPathEl.textContent = '';
      biStatus.textContent = '';
      biOkBtn.disabled = false;
    });
    el.addEventListener('dblclick', () => { biSelIdx = i; biManual = null; doBrowserImport(); });
    biList.appendChild(el);
  });
}

async function doBrowserImport() {
  biOkBtn.disabled = true;

  if (biManual) {
    // Portable / manual file mode
    biStatus.textContent = `Импортирую из ${biManual.name}…`;
    try {
      const r = await invoke('import_from_bookmarks_file', {
        path: biManual.path, name: biManual.name
      });
      biStatus.textContent = `Импортировано: ${r.links} ссылок, ${r.folders} папок.`;
      refreshTree();
      setTimeout(() => biOverlay.classList.add('hidden'), 2500);
    } catch(e) {
      biStatus.textContent = `Ошибка: ${e}`;
      biOkBtn.disabled = false;
    }
    return;
  }

  if (biSelIdx < 0 || biSelIdx >= biDetected.length) { biOkBtn.disabled = false; return; }
  const b = biDetected[biSelIdx];
  biStatus.textContent = `Импортирую из ${b.name}…`;
  try {
    const r = await invoke('import_from_browser', { browserId: b.id });
    biStatus.textContent = `Импортировано: ${r.links} ссылок, ${r.folders} папок.`;
    refreshTree();
    setTimeout(() => biOverlay.classList.add('hidden'), 2500);
  } catch(e) {
    biStatus.textContent = `Ошибка: ${e}`;
    biOkBtn.disabled = false;
  }
}

function openBrowsersDialog() {
  dlgBrowsers = browsers.map(b => ({...b}));
  dlgSelIdx = -1;
  renderDlgList();
  renderDlgDefault();
  updateDlgBtns();
  raiseOverlay(browsersOverlay);
}

function closeBrowsersDialog(save) {
  if (save) {
    browsers = dlgBrowsers;
    defaultBrowserPath = dlgDefaultSel.value;
    saveBrowsersConfig();
  }
  browsersOverlay.classList.add("hidden");
}

function renderDlgList() {
  dlgBrowserList.innerHTML = "";
  dlgBrowsers.forEach((b, i) => {
    const el = document.createElement("div");
    el.className = "dlg-browser-entry" + (i === dlgSelIdx ? " selected" : "");
    el.textContent = b.name;
    el.addEventListener("click", () => { dlgSelIdx = i; renderDlgList(); updateDlgBtns(); });
    el.addEventListener("dblclick", () => openAddBrowserDialog(i));
    dlgBrowserList.appendChild(el);
  });
}

function renderDlgDefault() {
  dlgDefaultSel.innerHTML = "";
  dlgBrowsers.forEach(b => {
    const opt = document.createElement("option");
    opt.value = b.path; opt.textContent = b.name;
    dlgDefaultSel.appendChild(opt);
  });
  dlgDefaultSel.value = getDefaultBrowserPath();
}

function updateDlgBtns() {
  const isDefaultEntry = dlgSelIdx >= 0 && dlgBrowsers[dlgSelIdx]?.path === "default";
  dlgDelBtn.disabled   = dlgSelIdx < 0 || isDefaultEntry;
  dlgPropsBtn.disabled = dlgSelIdx < 0;
}

document.getElementById("dlg-x").onclick      = () => closeBrowsersDialog(false);
document.getElementById("dlg-cancel").onclick  = () => closeBrowsersDialog(false);
document.getElementById("dlg-ok").onclick      = () => closeBrowsersDialog(true);
document.getElementById("dlg-add").onclick     = () => openAddBrowserDialog(-1);
document.getElementById("dlg-del").onclick     = () => {
  if (dlgSelIdx < 0) return;
  dlgBrowsers.splice(dlgSelIdx, 1);
  dlgSelIdx = -1;
  renderDlgList(); renderDlgDefault(); updateDlgBtns();
};
document.getElementById("dlg-props").onclick   = () => { if (dlgSelIdx >= 0) openAddBrowserDialog(dlgSelIdx); };

// ── "Portable…" button ───────────────────────────────────────────────────────
document.getElementById("dlg-portable").addEventListener("click", async () => {
  const path = await invoke("pick_browser_file").catch(() => null);
  if (!path) return;
  openAddBrowserDialog(-1, path, guessNameFromExePath(path));
});

// ── "Обнаружить" button ──────────────────────────────────────────────────────
const dlgDetectStatus = document.getElementById("dlg-detect-status");
document.getElementById("dlg-detect").addEventListener("click", async () => {
  dlgDetectStatus.textContent = "Поиск…";
  let detected = [];
  try { detected = await invoke("detect_browser_exes"); } catch(e) { console.error(e); }

  let added = 0;
  const addedNames = [];
  for (const b of detected) {
    const already = dlgBrowsers.some(x =>
      x.path.toLowerCase() === b.path.toLowerCase() || x.name === b.name);
    if (!already) {
      dlgBrowsers.push({ name: b.name, path: b.path });
      addedNames.push(b.name);
      added++;
    }
  }
  if (added > 0) {
    dlgSelIdx = dlgBrowsers.length - 1;
    renderDlgList(); renderDlgDefault(); updateDlgBtns();
    dlgDetectStatus.textContent = `Добавлено: ${addedNames.join(", ")}`;
  } else if (detected.length === 0) {
    dlgDetectStatus.textContent = "Браузеры не обнаружены.";
  } else {
    dlgDetectStatus.textContent = "Все найденные браузеры уже в списке.";
  }
  setTimeout(() => { dlgDetectStatus.textContent = ""; }, 3000);
});

// ── Helper ──────────────────────────────────────────────────────────────────
function guessNameFromExePath(path) {
  const exe = (path.split(/[\\/]/).pop() || "").replace(/\.exe$/i, "").toLowerCase();
  const known = {
    chrome: "Google Chrome", chromium: "Chromium",
    firefox: "Mozilla Firefox", waterfox: "Waterfox", librewolf: "LibreWolf",
    msedge: "Microsoft Edge", edge: "Microsoft Edge",
    opera: "Opera", operagx: "Opera GX", launcher: "Opera",
    brave: "Brave", vivaldi: "Vivaldi",
    iexplore: "Internet Explorer",
    palemoon: "Pale Moon", basilisk: "Basilisk", seamonkey: "SeaMonkey",
  };
  return known[exe] || (exe.charAt(0).toUpperCase() + exe.slice(1));
}

// ── Add/edit browser sub-dialog ───────────────────────────────────────────
const addBrowserOverlay = document.getElementById("add-browser-overlay");
const addBrowserName    = document.getElementById("add-browser-name");
const addBrowserPath    = document.getElementById("add-browser-path");
let   editBrowserIdx    = -1;

function openAddBrowserDialog(idx, presetPath = "", presetName = "") {
  editBrowserIdx = idx;
  addBrowserName.value = idx >= 0 ? dlgBrowsers[idx].name : presetName;
  addBrowserPath.value = idx >= 0 ? dlgBrowsers[idx].path : presetPath;
  document.body.appendChild(addBrowserOverlay); // bring to DOM front
  raiseOverlay(addBrowserOverlay);
  setTimeout(() => addBrowserName.focus(), 30);
}

// "..." — native file picker via Rust/rfd
document.getElementById("add-browser-browse").addEventListener("click", async () => {
  const path = await invoke("pick_browser_file").catch(() => null);
  if (!path) return;
  addBrowserPath.value = path;
  // Auto-fill name from filename if empty
  if (!addBrowserName.value.trim()) {
    const fname = path.split(/[\\/]/).pop().replace(/\.exe$/i, "");
    addBrowserName.value = fname.charAt(0).toUpperCase() + fname.slice(1);
  }
  addBrowserName.focus();
});

function closeAddBrowserDialog() { addBrowserOverlay.classList.add("hidden"); }

document.getElementById("add-browser-x").onclick      = closeAddBrowserDialog;
document.getElementById("add-browser-cancel").onclick = closeAddBrowserDialog;
document.getElementById("add-browser-ok").onclick = () => {
  const name = addBrowserName.value.trim();
  const path = addBrowserPath.value.trim();
  if (!name || !path) return;
  if (editBrowserIdx >= 0) {
    dlgBrowsers[editBrowserIdx] = { name, path };
  } else {
    dlgBrowsers.push({ name, path });
    dlgSelIdx = dlgBrowsers.length - 1;
  }
  closeAddBrowserDialog();
  renderDlgList(); renderDlgDefault(); updateDlgBtns();
};

// Close dialogs on overlay click
browsersOverlay.addEventListener("click",  (e) => { if (e.target === browsersOverlay)  closeBrowsersDialog(false); });
addBrowserOverlay.addEventListener("click",(e) => { if (e.target === addBrowserOverlay) closeAddBrowserDialog(); });

// ── Menu ──────────────────────────────────────────────────────────────────

const ICONS = {
  folder:    `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M0.7 3h4.6l1.2 1.6H13.3V11.4H0.7V3z"/></svg>`,
  subfolder: `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M0.7 2.2h3.7l1 1.3H10.5v2.6"/><path d="M0.7 2.2v6.3"/><path d="M3.5 5.8h3.6l1 1.3H13.3V12.2H3.5V5.8z"/></svg>`,
  link:   `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><path d="M5.5 8.5a3 3 0 004.2 0l2-2a3 3 0 00-4.2-4.2L6.4 3.4"/><path d="M8.5 5.5a3 3 0 00-4.2 0l-2 2a3 3 0 004.2 4.2l1.1-1.1"/></svg>`,
  gear:   `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><circle cx="7" cy="7" r="1.8"/><path d="M7 1.5v1.2M7 11.3v1.2M1.5 7h1.2M11.3 7h1.2M3.3 3.3l.85.85M9.85 9.85l.85.85M3.3 10.7l.85-.85M9.85 4.15l.85-.85"/></svg>`,
  db:     `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><ellipse cx="7" cy="4" rx="4.5" ry="1.8"/><path d="M2.5 4v3c0 1 2 1.8 4.5 1.8S11.5 8 11.5 7V4"/><path d="M2.5 7v3c0 1 2 1.8 4.5 1.8S11.5 11 11.5 10V7"/></svg>`,
  trash:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4h10M5 4V2.5h4V4M3.5 4l1 7.5h5l1-7.5"/></svg>`,
  backup: `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M7 2v7M4.5 6.5L7 9l2.5-2.5"/><path d="M2 11h10"/></svg>`,
  import: `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 8.5v2.5H3V8.5M7 2v7M4.5 6L7 8.5 9.5 6"/></svg>`,
  open:   `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M7 2.5L11 7 7 11.5M3 7h8"/></svg>`,
  props:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="2" y="2" width="10" height="10" rx="1.2"/><path d="M5 5h4M5 7h4M5 9h2.5"/></svg>`,
  check:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 7.5l3 3L11.5 4"/></svg>`,
  search: `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><circle cx="5.8" cy="5.8" r="3.5"/><path d="M9 9l3.5 3.5"/></svg>`,
  dupes:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="4.5" width="8" height="7.5" rx="1"/><path d="M5 4.5V2H13v7.5h-2"/></svg>`,
  sort:    `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d="M2 4h10M4 7h6M6 10h2"/></svg>`,
  image:   `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="2.5" width="12" height="9" rx="1"/><path d="M1 9l3.5-3 3 3 2-2 4.5 4.5"/><circle cx="4.5" cy="5.5" r="1.1" fill="currentColor" stroke="none"/></svg>`,
  refresh: `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5 7A4.5 4.5 0 013 5.5M3 2.5v3h3"/></svg>`,
  delimg:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="3" width="9" height="7" rx="1"/><path d="M10 5.5l3-3M13 5.5l-3-3"/></svg>`,
  copy:    `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="4.5" y="4.5" width="8" height="8" rx="1"/><path d="M4.5 4.5V2H1.5v8.5H4.5"/></svg>`,
  openwith:`<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><path d="M6 2H2a1 1 0 00-1 1v9a1 1 0 001 1h10a1 1 0 001-1V8"/><path d="M9.5 1.5h3V5M9 5.5l3.5-4"/></svg>`,
  verify:  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><circle cx="7" cy="7" r="5.5"/><path d="M4.5 7l2 2 3-3"/></svg>`,
  browser:      `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="2.5" width="12" height="9" rx="1"/><path d="M1 5.5h12"/><circle cx="3.5" cy="4" r=".6" fill="currentColor" stroke="none"/><circle cx="5.5" cy="4" r=".6" fill="currentColor" stroke="none"/></svg>`,
  edit:          `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.5 2.5l2 2L5 11H3V9l6.5-6.5z"/><path d="M8.5 3.5l2 2"/></svg>`,
  'expand-all':  `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="1" width="5" height="5" rx=".8"/><rect x="1" y="8" width="5" height="5" rx=".8"/><path d="M8 3.5h5M8 10.5h5M10.5 1.5v4M10.5 8.5v4"/></svg>`,
  'collapse-all':`<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><rect x="1" y="1" width="5" height="5" rx=".8"/><rect x="1" y="8" width="5" height="5" rx=".8"/><path d="M8 3.5h5M8 10.5h5"/></svg>`,
  'move-up':     `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M7 11V3M3.5 6.5L7 3l3.5 3.5"/></svg>`,
  'move-down':   `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M7 3v8M3.5 7.5L7 11l3.5-3.5"/></svg>`,
};

// todo:true = disabled (not yet implemented)
const MENU_DATA = [
  {
    id: 'file', label: 'Файл',
    items: [
      { label: 'Создать базу...',  icon: 'db',     action: 'new-db'    },
      { label: 'Открыть базу...',  icon: 'db',     action: 'open-db'   },
      { label: 'Последние базы',   icon: 'db',     sub: [], _dynamic: true },
      { label: 'Закрыть базу',     icon: 'db',     action: 'close-db'  },
      '---',
      { label: 'Создать резервную копию...',             icon: 'backup', action: 'backup-without' },
      { label: 'Создать резервную копию с рисунками...', icon: 'backup', action: 'backup-with'    },
      '---',
      { label: 'Свойства базы',  icon: 'props',  action: 'db-properties' },
      '---',
      { label: 'Настройки',      icon: 'gear',   action: 'settings'   },
    ]
  },
  {
    id: 'links', label: 'Ссылки',
    items: [
      { label: 'Новая ссылка',                   icon: 'new',      action: 'new-link'                },
      { label: 'Редактировать',                  icon: 'edit',     shortcut: 'F2',  action: 'properties' },
      { label: 'Удалить ссылку',                 icon: 'trash',    shortcut: 'Del', action: 'delete-link' },
      '---',
      { label: 'Открыть',                        icon: 'open',     shortcut: 'Enter', action: 'open-link' },
      { label: 'Открыть в переносном браузере',  icon: 'openwith', action: 'open-portable'           },
      { label: 'Открыть с помощью...',           icon: 'openwith', action: 'open-with'               },
      '---',
      { label: 'Проверить ссылки',               icon: 'verify',   action: 'check-all-links'         },
      { label: 'Найти дубликаты',                icon: 'dupes',    action: 'find-dupes'              },
      '---',
      { label: "Обновить favicon'ы",             icon: 'favicon',  action: 'refresh-favicons-folder' },
      { label: 'Обновить рисунки',               icon: 'refresh',  action: 'refresh-thumbs-folder'  },
      '---',
      { label: 'Копировать URL',                 icon: 'copy',     shortcut: 'Ctrl+C', action: 'copy-url' },
      { label: 'Свойства',                       icon: 'props',    shortcut: 'F4',  action: 'properties'  },
    ]
  },
  {
    id: 'transfer', label: 'Перенос',
    items: [
      { label: 'Импорт', icon: 'import', sub: [
        { label: 'Из браузера...',          icon: 'browser', action: 'import-from-browser' },
        { label: 'Из другой базы...',       icon: 'db',      action: 'import-from-db'      },
        '---',
        { label: 'Из файла HTML',           icon: 'import',  action: 'import-html'         },
        { label: 'Из файла TXT',            icon: 'import',  action: 'import-txt-lines'    },
        '---',
        { label: 'Файл синхронизации',      icon: 'backup',  action: 'import-sync'         },
        { label: 'Из ua.dat...',            icon: 'folder',  action: 'import-folder'        },
      ]},
      { label: 'Экспорт', icon: 'backup', sub: [
        { label: 'HTML файл',                   icon: 'import',  action: 'export-html'          },
        { label: 'Текстовый файл',              icon: 'import',  action: 'export-txt'           },
        '---',
        { label: 'Синхронизация с рисунками',   icon: 'backup',  action: 'export-sync-with'    },
        { label: 'Синхронизация без рисунков',  icon: 'backup',  action: 'export-sync-without' },
      ]},
      '---',
      { label: 'Браузеры', icon: 'browser', action: 'manage-browsers' },
    ]
  },
  {
    id: 'search', label: 'Поиск',
    items: [
      { label: 'Найти', icon: 'search', shortcut: 'Ctrl+F', action: 'find' },
    ]
  },
  {
    id: 'view', label: 'Вид',
    items: [
      { label: 'Развернуть все папки', icon: 'expand-all', action: 'toggle-expand-all' },
      '---',
      { label: 'Настроить toolbar…',   icon: 'gear',       action: 'customize-toolbar' },
    ]
  },
];

// ── Command Registry — единый источник для всех команд ───────────────────────
// Чтобы добавить новую команду: добавить запись сюда — она автоматически
// появится в toolbar customization без других изменений.
const CMD_REGISTRY = [
  // Создание
  { id:'new-link',            label:'Новая ссылка',              icon:'link',        group:'Создание',     shortcut:'Ctrl+Shift+N', action:'new-link' },
  { id:'new-folder',          label:'Новая папка',               icon:'folder',      group:'Создание',     shortcut:'Ctrl+N',       action:'new-folder' },
  { id:'new-subfolder',       label:'Новая подпапка',            icon:'subfolder',   group:'Создание',                              action:'new-subfolder' },
  // Правка
  { id:'delete-link',         label:'Удалить ссылку',            icon:'trash',       group:'Правка',       shortcut:'Del',          action:'delete-link' },
  { id:'properties',          label:'Свойства ссылки',           icon:'props',       group:'Правка',       shortcut:'F4',           action:'properties' },
  { id:'copy-url',            label:'Копировать URL',            icon:'copy',        group:'Правка',       shortcut:'Ctrl+C',       action:'copy-url' },
  { id:'open-link',           label:'Открыть в браузере',        icon:'open',        group:'Правка',       shortcut:'Enter',        action:'open-link' },
  { id:'open-with',           label:'Открыть с помощью…',        icon:'openwith',    group:'Правка',                                action:'open-with' },
  { id:'refresh-thumb',       label:'Обновить рисунок',          icon:'refresh',     group:'Правка',                                action:'refresh-thumb' },
  { id:'clear-thumb',         label:'Удалить рисунок',           icon:'delimg',      group:'Правка',                                action:'clear-thumb' },
  // Навигация
  { id:'toggle-expand-all',   label:'Развернуть/Свернуть все',   icon:'expand-all',  group:'Навигация',                             action:'toggle-expand-all' },
  { id:'move-up',             label:'Переместить вверх',         icon:'move-up',     group:'Навигация',                             action:'move-up' },
  { id:'move-down',           label:'Переместить вниз',          icon:'move-down',   group:'Навигация',                             action:'move-down' },
  // Поиск
  { id:'find',                label:'Поиск',                     icon:'search',      group:'Поиск',        shortcut:'Ctrl+F',       action:'find' },
  { id:'find-dupes',          label:'Поиск дубликатов',          icon:'dupes',       group:'Поиск',                                 action:'find-dupes' },
  // Проверка
  { id:'check-all-links',     label:'Проверить все ссылки',      icon:'verify',      group:'Проверка',                              action:'check-all-links' },
  // Импорт
  { id:'import-from-browser', label:'Импорт из браузера',        icon:'import',      group:'Импорт',                                action:'import-from-browser' },
  { id:'import-html',         label:'Импорт из HTML',            icon:'import',      group:'Импорт',                                action:'import-html' },
  { id:'import-txt-lines',    label:'Импорт URL из TXT',         icon:'import',      group:'Импорт',                                action:'import-txt-lines' },
  { id:'import-sync',         label:'Импорт синхронизации',      icon:'backup',      group:'Импорт',                                action:'import-sync' },
  // Экспорт
  { id:'export-html',         label:'Экспорт в HTML',            icon:'backup',      group:'Экспорт',                               action:'export-html' },
  { id:'export-txt',          label:'Экспорт в TXT',             icon:'backup',      group:'Экспорт',                               action:'export-txt' },
  { id:'export-sync-without', label:'Синхронизация',             icon:'backup',      group:'Экспорт',                               action:'export-sync-without' },
  // Backup
  { id:'backup-without',      label:'Backup (без рисунков)',      icon:'backup',      group:'Backup',                                action:'backup-without' },
  { id:'backup-with',         label:'Backup (с рисунками)',       icon:'backup',      group:'Backup',                                action:'backup-with' },
  // Сортировка
  { id:'sort-title-asc',      label:'По имени ↑',                icon:'sort',        group:'Сортировка',                            action:'sort-all-title-asc' },
  { id:'sort-title-desc',     label:'По имени ↓',                icon:'sort',        group:'Сортировка',                            action:'sort-all-title-desc' },
  { id:'sort-url-asc',        label:'По URL ↑',                  icon:'sort',        group:'Сортировка',                            action:'sort-all-url-asc' },
  { id:'sort-url-desc',       label:'По URL ↓',                  icon:'sort',        group:'Сортировка',                            action:'sort-all-url-desc' },
  { id:'sort-created-asc',    label:'По дате ↑',                 icon:'sort',        group:'Сортировка',                            action:'sort-all-created-asc' },
  { id:'sort-created-desc',   label:'По дате ↓',                 icon:'sort',        group:'Сортировка',                            action:'sort-all-created-desc' },
];

// Alias for backward-compat
const TOOLBAR_DEFS = CMD_REGISTRY;

const DEFAULT_TOOLBAR = [
  'new-link', 'new-folder', '|',
  'delete-link', 'properties', '|',
  'find', '|',
  'check-all-links', '|',
  'toggle-expand-all',
];

let toolbarConfig = [...DEFAULT_TOOLBAR];

async function loadToolbarConfig() {
  try {
    const json = await invoke('load_toolbar_config');
    if (json) {
      const arr = JSON.parse(json);
      if (Array.isArray(arr) && arr.length) toolbarConfig = arr;
    }
  } catch(e) {}
}

async function saveToolbarConfig() {
  try { await invoke('save_toolbar_config', { json: JSON.stringify(toolbarConfig) }); }
  catch(e) { console.error(e); }
}

const toolbarEl = document.getElementById('toolbar');

function tbIconHtml(iconKey) {
  const raw = ICONS[iconKey] || '';
  return raw.replace(/^<svg[^>]*>/, '<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">');
}

function buildToolbar() {
  toolbarEl.innerHTML = '';
  for (const item of toolbarConfig) {
    if (item === '|') {
      const s = document.createElement('div'); s.className = 'tb-sep'; toolbarEl.appendChild(s);
      continue;
    }
    const cmd = CMD_REGISTRY.find(c => c.id === item);
    if (!cmd) continue;
    const btn = document.createElement('button');
    btn.className = 'tb-btn';
    btn.dataset.tbCmd = cmd.id;
    const hint = cmd.shortcut ? `${cmd.label} (${cmd.shortcut})` : cmd.label;
    btn.title = hint;
    btn.innerHTML = tbIconHtml(cmd.icon);
    btn.addEventListener('click', () => handleToolbarAction(cmd.id));
    toolbarEl.appendChild(btn);
  }
}

// Dispatch: looks up cmd in registry → fires its action via handleMenuAction
function handleToolbarAction(id) {
  const cmd = CMD_REGISTRY.find(c => c.id === id);
  if (!cmd) return;
  // Special local actions (not going through handleMenuAction)
  if (id === 'toggle-expand-all') {
    const anyOpen = !!treeEl.querySelector('.tree-children.open');
    treeEl.querySelectorAll('.tree-children').forEach(el => {
      el.classList.toggle('open', !anyOpen);
      el.previousElementSibling?.classList.toggle('open', !anyOpen);
    });
    _syncExpandToggleUI();
    return;
  }
  if (id === 'move-up')   { tbMoveItem(-1); return; }
  if (id === 'move-down') { tbMoveItem(+1); return; }
  handleMenuAction(cmd.action || id);
}

async function _applySortOrder(siblings, node) {
  await Promise.all(siblings.map((n, si) => invoke('set_sort_idx', { id: n.id, sortIdx: si })));
  siblings.forEach((n, si) => { n.sort_idx = si; });
  const openIds = saveOpenState();
  renderTree();
  restoreOpenState(openIds);
  _activateTreeItem(node);
  if (node.kind === 'folder' && activeFolderId === node.parent) {
    await loadFolderContents(node.parent);
  } else if (node.kind === 'bookmark') {
    await loadBookmarks(node.parent);
    const card = gridEl.querySelector(`.card[data-id="${node.id}"]`);
    if (card) gridSelectRow(card);
  }
}

function tbMoveItem(dir) {
  const node = activeBookmarkNode ||
    (activeFolderId != null ? allNodes.find(n => n.id === activeFolderId) : null);
  if (!node) return;
  const siblings = allNodes
    .filter(n => n.kind === node.kind && n.parent === node.parent)
    .sort((a, b) => (a.sort_idx ?? 0) - (b.sort_idx ?? 0) || a.id - b.id);
  const idx = siblings.findIndex(n => n.id === node.id);
  if (idx < 0 || idx + dir < 0 || idx + dir >= siblings.length) return;
  [siblings[idx], siblings[idx + dir]] = [siblings[idx + dir], siblings[idx]];
  _applySortOrder(siblings, node).catch(console.error);
}

// ── Open database dialog ──────────────────────────────────────────────────────

(function() {
  const overlay = document.getElementById('open-db-overlay');
  const close   = () => overlay.classList.add('hidden');

  document.getElementById('open-db-x').onclick      = close;
  document.getElementById('open-db-cancel').onclick  = close;
  overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

  document.getElementById('open-db-ok').onclick = async () => {
    const errEl = document.getElementById('open-db-err');
    if (errEl) errEl.textContent = '';
    try {
      await invoke('open_db');
      close();
      await showApp();
    } catch(e) {
      if (e === 'Отменено') return;
      if (errEl) errEl.textContent = e;
      else console.error('open_db:', e);
    }
  };
})();

// ── Import from another DB dialog ─────────────────────────────────────────────

(function() {
  const overlay       = document.getElementById('import-db-overlay');
  const destSelect    = document.getElementById('import-db-dest');
  const newFolderRow  = document.getElementById('import-db-new-folder-row');
  const newFolderName = document.getElementById('import-db-new-folder-name');
  let _analysis = null;

  function close() { overlay.classList.add('hidden'); _analysis = null; }

  document.getElementById('import-db-x').onclick      = close;
  document.getElementById('import-db-cancel').onclick  = close;
  overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

  destSelect.addEventListener('change', () => {
    const isNew = destSelect.value === 'new';
    newFolderRow.classList.toggle('hidden', !isNew);
    if (isNew) newFolderName.focus();
  });

  document.getElementById('import-db-ok').onclick = async () => {
    if (!_analysis) return;
    const okBtn = document.getElementById('import-db-ok');
    okBtn.disabled = true;
    try {
      let destParent = null;
      const sel = destSelect.value;
      if (sel === 'new') {
        const name = newFolderName.value.trim();
        if (!name) { newFolderName.focus(); okBtn.disabled = false; return; }
        destParent = await invoke('create_folder', { parentId: null, title: name });
      } else if (sel !== 'root') {
        destParent = parseInt(sel);
      }
      const count = await invoke('execute_import_db', {
        path: _analysis.source_path,
        destParent,
      });
      close();
      if (count > 0) {
        await refreshTree();
        setStatus(`Импортировано ${count} ссылок`);
      } else {
        setStatus('Нет новых ссылок для импорта');
      }
    } catch(e) {
      if (e !== 'Отменено') alert('Ошибка импорта: ' + e);
    } finally {
      okBtn.disabled = false;
    }
  };

  window.openImportDbDialog = function(analysis) {
    _analysis = analysis;
    document.getElementById('import-db-path').textContent    = analysis.source_path;
    document.getElementById('import-db-folders').textContent = analysis.total_folders;
    document.getElementById('import-db-total').textContent   = analysis.total_bookmarks;
    document.getElementById('import-db-new').textContent     = analysis.new_count;
    document.getElementById('import-db-dupes').textContent   = analysis.duplicate_count;

    destSelect.innerHTML = '';
    const addOpt = (val, label) => {
      const o = document.createElement('option');
      o.value = val; o.textContent = label;
      destSelect.appendChild(o);
    };
    addOpt('root', 'Корень');
    allNodes.filter(n => n.kind === 'folder' && n.parent === null)
            .forEach(f => addOpt(String(f.id), f.title));
    addOpt('new', '— Создать новую папку...');

    newFolderRow.classList.add('hidden');
    newFolderName.value = '';
    raiseOverlay(overlay);
  };
})();

// ── New item dialog (folder / bookmark) ───────────────────────────────────────

(function() {
  const overlay      = document.getElementById('new-item-overlay');
  const titleEl      = document.getElementById('new-item-dlg-title');
  const nameInput    = document.getElementById('new-item-name');
  const urlInput     = document.getElementById('new-item-url');
  const noteInput    = document.getElementById('new-item-note');
  const urlRow       = document.getElementById('new-item-url-row');
  const noteRow      = document.getElementById('new-item-note-row');
  const folderSelect = document.getElementById('new-item-folder');
  const okBtn        = document.getElementById('new-item-ok');
  let _mode = 'folder';

  function _populateFolderSelect(selectedId) {
    folderSelect.innerHTML = '';
    const addFolderOpts = (nodes, parentId, depth) => {
      const children = nodes.filter(n => n.kind === 'folder' && n.parent === parentId);
      for (const f of children) {
        const opt = document.createElement('option');
        opt.value = f.id;
        opt.textContent = ' '.repeat(depth * 2) + f.title;
        if (f.id === selectedId) opt.selected = true;
        folderSelect.appendChild(opt);
        addFolderOpts(nodes, f.id, depth + 1);
      }
    };
    addFolderOpts(allNodes, null, 0);
  }

  function openNewItemDlg(mode, prefill = {}) {
    _mode = mode;
    titleEl.textContent = 'Новая ссылка';
    nameInput.value = prefill.title || '';
    urlInput.value  = prefill.url   || '';
    noteInput.value = '';
    urlRow.classList.remove('hidden');
    noteRow.classList.remove('hidden');
    _populateFolderSelect(activeFolderId);
    raiseOverlay(overlay);
    setTimeout(() => (prefill.url ? nameInput.focus() : urlInput.focus()), 30);
  }

  window.doNewFolder = () => createFolderAndRename(null);
  window.doNewLink   = () => openNewItemDlg('link');
  // Новая подпапка → вложенная в текущую выбранную папку, с inline rename
  window.doNewSubfolder = () => { if (activeFolderId != null) createFolderAndRename(activeFolderId); };

  const warnEl = document.getElementById('new-item-warn');

  async function submit() {
    const pid = Number(folderSelect.value) || activeFolderId;
    if (pid == null) return;
    const url = urlInput.value.trim();
    if (!url) return;

    // Duplicate URL check
    if (appSettings.noDuplicateUrls) {
      const dup = allNodes.find(n => n.kind === 'bookmark' && n.url === url);
      if (dup) {
        warnEl.textContent = `Уже существует: «${dup.title}»`;
        warnEl.classList.remove('hidden');
        urlInput.focus();
        return;
      }
    }
    warnEl.classList.add('hidden');

    const name = nameInput.value.trim() || url;
    const note = noteInput ? noteInput.value : '';
    overlay.classList.add('hidden');
    try {
      const newId = await invoke('create_bookmark', { parentId: pid, title: name, url, note });

      // Refresh in-memory state (tree badge counts update too)
      const openIds = saveOpenState();
      allNodes   = await invoke('get_tree');
      allFolders = allNodes.filter(n => n.kind === 'folder');
      renderTree();
      restoreOpenState(openIds);

      // Reload right panel
      await loadFolderContents(pid);

      // Select the new item immediately
      const newNode = allNodes.find(n => n.id === newId);
      if (newNode) {
        const card = gridEl.querySelector(`.card[data-id="${newId}"]`);
        if (card) { gridSelectRow(card); card.scrollIntoView({ block: 'nearest' }); }
        navigateToCard(newNode);
      }
    } catch(e) { console.error(e); }
  }

  document.getElementById('new-item-x').onclick      = () => overlay.classList.add('hidden');
  document.getElementById('new-item-cancel').onclick = () => overlay.classList.add('hidden');
  okBtn.onclick = submit;
  urlInput.addEventListener('input', () => warnEl.classList.add('hidden'));
  // Enter submits (except in textarea), Escape closes — NO click-outside-to-close
  [nameInput, urlInput].forEach(inp => inp.addEventListener('keydown', e => {
    if (e.key === 'Enter') submit();
    if (e.key === 'Escape') overlay.classList.add('hidden');
  }));
  noteInput?.addEventListener('keydown', e => {
    if (e.key === 'Escape') overlay.classList.add('hidden');
    // Enter in textarea = newline (default) — intentional
  });
})();

// ── Toolbar customize dialog ──────────────────────────────────────────────────

(function() {
  const overlay  = document.getElementById('tbc-overlay');
  const availEl  = document.getElementById('tbc-avail');
  const activeEl = document.getElementById('tbc-active');
  const addBtn   = document.getElementById('tbc-add-btn');
  const remBtn   = document.getElementById('tbc-rem-btn');
  const upBtn    = document.getElementById('tbc-up');
  const downBtn  = document.getElementById('tbc-down');

  let tbcItems  = [];  // current toolbar config copy
  let availSelId = null;   // selected cmd id in left panel
  let activeSel  = -1;    // selected index in right panel
  let dragSrcIdx = -1;    // DnD source index in right panel
  let dragSrcIsAvail = false; // DnD from left panel

  // ── Left panel: grouped by CMD_REGISTRY.group ──────────────────────────────
  function renderAvail() {
    const used = new Set(tbcItems.filter(x => x !== '|'));
    const groups = {};
    for (const cmd of CMD_REGISTRY) {
      if (used.has(cmd.id)) continue;
      const g = cmd.group || 'Другое';
      (groups[g] = groups[g] || []).push(cmd);
    }
    availEl.innerHTML = '';
    for (const [grp, cmds] of Object.entries(groups)) {
      const hdr = document.createElement('div');
      hdr.className = 'tbc-group-hdr';
      hdr.textContent = grp;
      availEl.appendChild(hdr);
      for (const cmd of cmds) {
        const el = document.createElement('div');
        el.className = 'dlg-browser-entry tbc-avail-item' + (availSelId === cmd.id ? ' selected' : '');
        el.innerHTML = `<span class="tbc-item-icon">${tbIconHtml(cmd.icon)}</span>`
                     + `<span class="tbc-item-label">${cmd.label}</span>`
                     + (cmd.shortcut ? `<span class="tbc-item-sc">${cmd.shortcut}</span>` : '');
        el.dataset.id = cmd.id;
        el.draggable = true;
        el.addEventListener('click',   () => { availSelId = cmd.id; activeSel = -1; render(); });
        el.addEventListener('dblclick',() => { availSelId = cmd.id; doAdd(); });
        el.addEventListener('dragstart', e => {
          dragSrcIsAvail = true; dragSrcIdx = -1;
          e.dataTransfer.effectAllowed = 'copy';
          e.dataTransfer.setData('text/tbc-avail', cmd.id);
        });
        el.addEventListener('dragend', () => { dragSrcIsAvail = false; });
        availEl.appendChild(el);
      }
    }
  }

  // ── Right panel: current toolbar items ────────────────────────────────────
  function renderActive() {
    activeEl.innerHTML = '';
    tbcItems.forEach((item, i) => {
      const el = document.createElement('div');
      el.className = 'dlg-browser-entry tbc-act-item' + (activeSel === i ? ' selected' : '');
      if (item === '|') {
        el.className += ' tbc-sep-item';
        el.textContent = '──────────';
      } else {
        const cmd = CMD_REGISTRY.find(c => c.id === item);
        el.innerHTML = cmd
          ? `<span class="tbc-item-icon">${tbIconHtml(cmd.icon)}</span><span class="tbc-item-label">${cmd.label}</span>`
          : `<span class="tbc-item-label">${item}</span>`;
      }
      el.draggable = true;
      el.addEventListener('click', () => { activeSel = i; availSelId = null; render(); });

      // DnD — drag within active list
      el.addEventListener('dragstart', e => {
        dragSrcIdx = i; dragSrcIsAvail = false;
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/tbc-idx', String(i));
        el.classList.add('tbc-dragging');
      });
      el.addEventListener('dragend', () => {
        el.classList.remove('tbc-dragging');
        activeEl.querySelectorAll('.tbc-drop-before,.tbc-drop-after').forEach(x => x.classList.remove('tbc-drop-before','tbc-drop-after'));
      });
      el.addEventListener('dragover', e => {
        e.preventDefault();
        activeEl.querySelectorAll('.tbc-drop-before,.tbc-drop-after').forEach(x => x.classList.remove('tbc-drop-before','tbc-drop-after'));
        const rect = el.getBoundingClientRect();
        el.classList.add(e.clientY < rect.top + rect.height / 2 ? 'tbc-drop-before' : 'tbc-drop-after');
      });
      el.addEventListener('dragleave', () => el.classList.remove('tbc-drop-before','tbc-drop-after'));
      el.addEventListener('drop', e => {
        e.preventDefault();
        el.classList.remove('tbc-drop-before','tbc-drop-after');
        const rect = el.getBoundingClientRect();
        const before = e.clientY < rect.top + rect.height / 2;

        if (dragSrcIsAvail) {
          // Drop from left panel → insert
          const cmdId = e.dataTransfer.getData('text/tbc-avail');
          if (!cmdId || tbcItems.includes(cmdId)) return;
          const at = before ? i : i + 1;
          tbcItems.splice(at, 0, cmdId);
          activeSel = at;
        } else {
          // Reorder within right panel
          const src = parseInt(e.dataTransfer.getData('text/tbc-idx'));
          if (isNaN(src) || src === i) return;
          let dest = before ? i : i + 1;
          if (src < dest) dest--;
          const [moved] = tbcItems.splice(src, 1);
          tbcItems.splice(dest, 0, moved);
          activeSel = dest;
        }
        dragSrcIsAvail = false; dragSrcIdx = -1;
        render();
      });
      activeEl.appendChild(el);
    });

    // Drop on empty area at bottom of list (from left panel)
    activeEl.addEventListener('dragover', e => {
      if (!dragSrcIsAvail) return;
      e.preventDefault();
    });
    activeEl.addEventListener('drop', e => {
      if (!dragSrcIsAvail) return;
      const cmdId = e.dataTransfer.getData('text/tbc-avail');
      if (!cmdId || tbcItems.includes(cmdId)) return;
      // Only if dropped on the container (not on an item)
      if (e.target !== activeEl) return;
      e.preventDefault();
      tbcItems.push(cmdId);
      activeSel = tbcItems.length - 1;
      dragSrcIsAvail = false;
      render();
    });
  }

  function render() { renderAvail(); renderActive(); updateBtns(); }

  function updateBtns() {
    const canAdd = availSelId != null && !tbcItems.includes(availSelId);
    addBtn.disabled  = !canAdd;
    remBtn.disabled  = activeSel < 0;
    upBtn.disabled   = activeSel <= 0;
    downBtn.disabled = activeSel < 0 || activeSel >= tbcItems.length - 1;
  }

  function doAdd() {
    if (!availSelId) return;
    if (tbcItems.includes(availSelId)) return;
    const at = activeSel >= 0 ? activeSel + 1 : tbcItems.length;
    tbcItems.splice(at, 0, availSelId);
    activeSel = at; availSelId = null;
    render();
  }

  function doRemove() {
    if (activeSel < 0) return;
    tbcItems.splice(activeSel, 1);
    if (activeSel >= tbcItems.length) activeSel = tbcItems.length - 1;
    render();
  }

  function doMove(dir) {
    const n = activeSel + dir;
    if (n < 0 || n >= tbcItems.length) return;
    [tbcItems[activeSel], tbcItems[n]] = [tbcItems[n], tbcItems[activeSel]];
    activeSel = n;
    render();
  }

  window.openToolbarCustomizeDialog = function() {
    tbcItems = [...toolbarConfig];
    availSelId = null; activeSel = -1;
    render();
    makeDlgDraggable(document.getElementById('tbc-dlg'), document.querySelector('#tbc-dlg .dlg-title'));
    raiseOverlay(overlay);
  };

  document.getElementById('tbc-x').onclick      = () => overlay.classList.add('hidden');
  document.getElementById('tbc-cancel').onclick = () => overlay.classList.add('hidden');
  document.getElementById('tbc-add-sep').onclick = () => {
    const at = activeSel >= 0 ? activeSel + 1 : tbcItems.length;
    tbcItems.splice(at, 0, '|'); activeSel = at; render();
  };
  addBtn.onclick  = doAdd;
  remBtn.onclick  = doRemove;
  upBtn.onclick   = () => doMove(-1);
  downBtn.onclick = () => doMove(+1);
  document.getElementById('tbc-reset').onclick = () => {
    tbcItems = [...DEFAULT_TOOLBAR]; activeSel = -1; availSelId = null; render();
  };
  document.getElementById('tbc-ok').onclick = () => {
    toolbarConfig = tbcItems; buildToolbar(); saveToolbarConfig();
    overlay.classList.add('hidden');
  };
  overlay.addEventListener('click', e => { if (e.target === overlay) overlay.classList.add('hidden'); });
})();

function _populateRecentDbs(drop) {
  const recentEntry = drop.querySelector('.menu-entry.has-sub[data-recent-dbs]');
  if (!recentEntry) return;
  const subEl = recentEntry.querySelector('.menu-sub');
  if (!subEl) return;
  invoke('get_recent_dbs').then(paths => {
    subEl.innerHTML = '';
    if (paths.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'menu-entry disabled';
      empty.innerHTML = '<span class="entry-icon"></span><span class="entry-label">(пусто)</span>';
      subEl.appendChild(empty);
      return;
    }
    for (const p of paths) {
      const name = p.split(/[\\/]/).pop();
      const el = document.createElement('div');
      el.className = 'menu-entry';
      el.innerHTML = `<span class="entry-icon">${ICONS['db'] || ''}</span><span class="entry-label" title="${p}">${name}</span>`;
      el.addEventListener('click', (ev) => {
        ev.stopPropagation();
        closeAllMenus();
        invoke('switch_db', { newPath: p })
          .then(() => showApp())
          .catch(console.error);
      });
      subEl.appendChild(el);
    }
  }).catch(() => {});
}

function buildMenubar() {
  const bar = document.getElementById('menubar');

  for (const menu of MENU_DATA) {
    const group = document.createElement('div');
    group.className = 'menu-group';
    group.dataset.id = menu.id;

    const lbl = document.createElement('div');
    lbl.className = 'menu-label';
    lbl.textContent = menu.label;

    const drop = document.createElement('div');
    drop.className = 'menu-dropdown';

    for (const item of menu.items) {
      if (item === '---') {
        const sep = document.createElement('div');
        sep.className = 'menu-sep';
        drop.appendChild(sep);
        continue;
      }
      const entry = document.createElement('div');
      const hasSub = Array.isArray(item.sub);
      entry.className = 'menu-entry' + (item.todo ? ' disabled' : '') + (hasSub ? ' has-sub' : '');

      const icon = document.createElement('span');
      icon.className = 'entry-icon';
      icon.innerHTML = ICONS[item.icon] || '';

      const lbl2 = document.createElement('span');
      lbl2.className = 'entry-label';
      lbl2.textContent = item.label;

      entry.append(icon, lbl2);

      if (item.shortcut) {
        const sc = document.createElement('span');
        sc.className = 'entry-shortcut';
        sc.textContent = item.shortcut;
        entry.appendChild(sc);
      }

      // Nested submenu
      if (hasSub) {
        const subEl = document.createElement('div');
        subEl.className = 'menu-sub';
        for (const si of item.sub) {
          if (si === '---') {
            const s = document.createElement('div'); s.className = 'menu-sep';
            subEl.appendChild(s); continue;
          }
          const se = document.createElement('div');
          se.className = 'menu-entry' + (si.todo ? ' disabled' : '');
          const ic = document.createElement('span'); ic.className = 'entry-icon';
          ic.innerHTML = ICONS[si.icon] || '';
          const lb = document.createElement('span'); lb.className = 'entry-label';
          lb.textContent = si.label;
          se.append(ic, lb);
          if (!si.todo) {
            se.addEventListener('click', (e) => { e.stopPropagation(); handleMenuAction(si.action); });
          }
          subEl.appendChild(se);
        }
        entry.appendChild(subEl);
        if (item._dynamic) entry.dataset.recentDbs = '1';
      } else if (!item.todo) {
        entry.addEventListener('click', (e) => {
          e.stopPropagation();
          handleMenuAction(item.action);
        });
      }

      drop.appendChild(entry);
    }

    group.append(lbl, drop);
    bar.appendChild(group);

    lbl.addEventListener('click', (e) => {
      e.stopPropagation();
      const isOpen = group.classList.contains('open');
      closeAllMenus();
      if (!isOpen) {
        group.classList.add('open');
        if (menu.id === 'view') _syncExpandToggleUI();
        if (menu.id === 'file') _populateRecentDbs(drop);
      }
    });

    lbl.addEventListener('mouseenter', () => {
      if (document.querySelector('.menu-group.open')) {
        closeAllMenus();
        group.classList.add('open');
        if (menu.id === 'file') _populateRecentDbs(drop);
      }
    });
  }

  document.addEventListener('click', closeAllMenus);
  window.addEventListener('blur', closeAllMenus);
  document.addEventListener('blur', closeAllMenus, true); // capture phase
  // Tauri window blur event (title bar click on Windows)
  try {
    window.__TAURI__?.event?.listen('tauri://blur', closeAllMenus);
  } catch(e) {}
}

function closeAllMenus() {
  document.querySelectorAll('.menu-group.open').forEach(g => g.classList.remove('open'));
}

function _syncExpandToggleUI() {
  const anyOpen = !!treeEl.querySelector('.tree-children.open');
  const label = anyOpen ? 'Свернуть все папки' : 'Развернуть все папки';
  const icon  = anyOpen ? 'collapse-all' : 'expand-all';
  // Menu label + icon
  document.querySelectorAll('.menu-entry .entry-label').forEach(el => {
    if (el.textContent === 'Развернуть все папки' || el.textContent === 'Свернуть все папки') {
      el.textContent = label;
      const iconEl = el.closest('.menu-entry')?.querySelector('.entry-icon');
      if (iconEl) iconEl.innerHTML = ICONS[icon] || '';
    }
  });
  // Toolbar button
  const btn = toolbarEl.querySelector('[data-tb-cmd="toggle-expand-all"]');
  if (btn) { btn.innerHTML = tbIconHtml(icon); btn.title = label; }
}

function handleMenuAction(action) {
  closeAllMenus();
  switch (action) {
    // ── Export (whole DB via root folder) ──
    case 'export-html': {
      const fid = allFolders.find(f => f.parent == null)?.id ?? activeFolderId;
      if (fid != null) invoke("export_folder_html", { folderId: fid }).catch(console.error);
      break;
    }
    case 'export-txt': {
      const fid = allFolders.find(f => f.parent == null)?.id ?? activeFolderId;
      if (fid != null) invoke("export_folder_txt", { folderId: fid }).catch(console.error);
      break;
    }
    case 'export-sync-with': {
      const fid = allFolders.find(f => f.parent == null)?.id ?? activeFolderId;
      if (fid != null) invoke("export_folder_sync", { folderId: fid, withImages: true }).catch(console.error);
      break;
    }
    case 'export-sync-without': {
      const fid = allFolders.find(f => f.parent == null)?.id ?? activeFolderId;
      if (fid != null) invoke("export_folder_sync", { folderId: fid, withImages: false }).catch(console.error);
      break;
    }
    // ── Backup ──
    case 'backup-without': invoke("backup_db").catch(console.error); break;
    case 'backup-with':    invoke("backup_db_with_data").catch(console.error); break;
    // ── Sort all ──
    case 'sort-all-title-asc':    invoke("sort_all_bookmarks", { by: "title",   desc: false }).then(refreshTree).catch(console.error); break;
    case 'sort-all-title-desc':   invoke("sort_all_bookmarks", { by: "title",   desc: true  }).then(refreshTree).catch(console.error); break;
    case 'sort-all-url-asc':      invoke("sort_all_bookmarks", { by: "url",     desc: false }).then(refreshTree).catch(console.error); break;
    case 'sort-all-url-desc':     invoke("sort_all_bookmarks", { by: "url",     desc: true  }).then(refreshTree).catch(console.error); break;
    case 'sort-all-created-asc':  invoke("sort_all_bookmarks", { by: "created", desc: false }).then(refreshTree).catch(console.error); break;
    case 'sort-all-created-desc': invoke("sort_all_bookmarks", { by: "created", desc: true  }).then(refreshTree).catch(console.error); break;
    // ── Check all ──
    case 'check-all-links':
      openCheckerPanel(null, allNodes.filter(n => n.kind === "bookmark" && n.url));
      break;
    case 'find-dupes':
      openDupesDialog();
      break;

    // ── Import ──
    case 'import-browser':
      openBrowsersDialog();
      break;
    case 'import-from-browser':
      openBrowserImportDialog();
      break;
    case 'import-txt-lines':
      invoke("import_txt_lines")
        .then(n => { if (n > 0) { refreshTree(); } })
        .catch(e => { if (e !== 'Отменено') console.error('import_txt_lines:', e); });
      break;
    case 'import-html':
      invoke("import_html")
        .then(n => { if (n > 0) { refreshTree(); } })
        .catch(e => { if (e !== 'Отменено') console.error('import_html:', e); });
      break;
    case 'import-txt':
      invoke("import_txt")
        .then(n => { if (n > 0) { refreshTree(); } })
        .catch(e => { if (e !== 'Отменено') console.error('import_txt:', e); });
      break;
    case 'import-sync':
      invoke("import_sync")
        .then(n => { if (n > 0) { refreshTree(); } })
        .catch(e => { if (e !== 'Отменено') console.error('import_sync:', e); });
      break;
    case 'import-folder':
      invoke("import_uadat_pick")
        .then(n => { if (n > 0) { refreshTree(); } })
        .catch(e => { if (e !== 'Отменено') console.error('import_folder:', e); });
      break;

    case 'open-link': {
      const t = getActiveLink();
      if (t?.url) openWithBrowser(t.url, getDefaultBrowserPath());
      break;
    }
    case 'open-with': {
      const t = getActiveLink();
      if (t?.url) showOpenWithDialog(t.url);
      break;
    }
    case 'properties': {
      const t = getActiveLink();
      if (t) openPropsDialog(t);
      break;
    }
    case 'copy-url': {
      const url = getActiveLink()?.url;
      if (url) navigator.clipboard.writeText(url).catch(() => {});
      break;
    }
    case 'delete-link': {
      const t = activeBookmarkNode
        || allNodes.find(n => String(n.id) === gridEl.querySelector(".card.selected")?.dataset.id);
      if (t) deleteBookmark(t);
      break;
    }
    case 'find':
      openSearchDialog();
      break;
    case 'new-folder':
      doNewFolder();
      break;
    case 'new-link':
      doNewLink();
      break;
    case 'new-subfolder':
      doNewSubfolder();
      break;
    case 'toggle-expand-all': {
      const anyOpen = !!treeEl.querySelector('.tree-children.open');
      treeEl.querySelectorAll('.tree-children').forEach(el => {
        el.classList.toggle('open', !anyOpen);
        el.previousElementSibling?.classList.toggle('open', !anyOpen);
      });
      _syncExpandToggleUI();
      break;
    }
    case 'settings':
      openSettingsDialog();
      break;

    // ── Database ──
    case 'open-db':
      raiseOverlay(document.getElementById('open-db-overlay'));
      break;
    case 'new-db':
      invoke('create_new_db')
        .then(() => showApp())
        .catch(e => { if (e !== 'Отменено') console.error('create_new_db:', e); });
      break;
    case 'clear-db':
      deleteConfirm('Очистить базу данных?\nВсе закладки, папки и скриншоты будут удалены.', async () => {
        await Promise.all([invoke('clear_db'), invoke('clear_screenshots')]).catch(console.error);
        allNodes = []; allFolders = [];
        activeFolderId = null;
        exitSearchMode();          // clear search state + UI
        hideDetailView();          // hide detail panel, reset activeBookmarkNode
        renderTree();              // empty tree — also calls updateStatusLeft
        gridEl.innerHTML = '';
        emptyHint.classList.add('hidden');
        breadcrumb.textContent = '';
        _sbInFolderCount = null;
        updateStatusLeft();
      });
      break;
    case 'customize-toolbar':
      openToolbarCustomizeDialog();
      break;
    case 'close-db':
      invoke('close_db').catch(console.error);
      showImportScreen();
      break;

    case 'db-properties':
      openDbPropertiesDialog();
      break;

    case 'manage-browsers':
      openBrowsersDialog();
      break;

    case 'import-from-db':
      invoke('analyze_import_db')
        .then(a => openImportDbDialog(a))
        .catch(e => { if (e !== 'Отменено') alert('Ошибка: ' + e); });
      break;

    case 'refresh-favicons-folder': {
      const folder = allNodes.find(n => n.id === activeFolderId);
      if (folder) startFaviconBatch(folder, false);
      break;
    }

    case 'refresh-thumbs-folder': {
      const folder = allNodes.find(n => n.id === activeFolderId);
      if (folder) startThumbBatch(folder);
      break;
    }

    case 'open-portable': {
      const t = getActiveLink();
      if (!t?.url) break;
      invoke('load_browsers_config')
        .then(config => {
          const browsers = (config?.browsers) || [];
          if (browsers.length > 0) {
            invoke('open_url_with', { url: t.url, browser: browsers[0].path }).catch(console.error);
          } else {
            handleMenuAction('open-with');
          }
        })
        .catch(() => handleMenuAction('open-with'));
      break;
    }
  }
}

document.addEventListener('keydown', e => {
  if (e.ctrlKey && e.key === 'f') {
    e.preventDefault(); handleMenuAction('find'); return;
  }
  if (e.ctrlKey && e.shiftKey && e.key === 'N') {
    e.preventDefault(); handleMenuAction('new-link'); return;
  }
  if (e.ctrlKey && !e.shiftKey && e.key === 'n') {
    e.preventDefault(); handleMenuAction('new-folder'); return;
  }

  // Ctrl+C — copy URL of selected card or active bookmark
  if (e.ctrlKey && e.key === 'c' && !window.getSelection()?.toString()) {
    const selected = gridEl.querySelector(".card.selected");
    const url = selected?.dataset.url || ctxTarget?.url || activeBookmarkNode?.url;
    if (url) { e.preventDefault(); navigator.clipboard.writeText(url).catch(() => {}); }
    return;
  }

  // Navigation inside search results
  const inSearch = !searchResultsEl.classList.contains("hidden") && searchResults.length > 0;
  if (inSearch) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveResult(activeResultIdx + 1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveResult(activeResultIdx - 1);
      return;
    }
    if (e.key === 'Enter' && activeResultIdx >= 0) {
      e.preventDefault();
      openSearchResult(activeResultIdx);
      return;
    }
  }

  // Enter — open URL (when not in input/search)
  if ((e.key === 'Enter' || (e.ctrlKey && e.key === 'Enter')) && !e.target.matches('input,textarea')) {
    const url = activeBookmarkNode?.url
      || allNodes.find(n => String(n.id) === gridEl.querySelector(".card.selected")?.dataset.id)?.url;
    if (url) { e.preventDefault(); openWithBrowser(url, getDefaultBrowserPath()); }
  }

  // Del — delete focused tree item (folder or bookmark) or selected card
  if (e.key === 'Delete' && !e.target.matches('input,textarea')) {
    const focused = document.activeElement;
    if (focused?.classList.contains("tree-item")) {
      const node = allNodes.find(n => n.id === parseInt(focused.dataset.id));
      if (node) {
        e.preventDefault();
        if (node.kind === "folder") deleteFolder(node);
        else deleteBookmark(node);
      }
      return;
    }
    const target = activeBookmarkNode
      || allNodes.find(n => String(n.id) === gridEl.querySelector(".card.selected")?.dataset.id);
    if (target) { e.preventDefault(); deleteBookmark(target); }
  }

  // F4 — properties of focused/active item
  if (e.key === 'F2') {
    const focused = document.activeElement;
    if (focused?.classList.contains("tree-item")) {
      const node = allNodes.find(n => n.id === parseInt(focused.dataset.id, 10));
      if (node?.kind === 'folder') { e.preventDefault(); startInlineRename(node.id); }
    }
    return;
  }

  if (e.key === 'F4') {
    const focused = document.activeElement;
    if (focused?.classList.contains("tree-item")) {
      const node = allNodes.find(n => n.id === parseInt(focused.dataset.id));
      if (node) {
        e.preventDefault();
        if (node.kind === "folder") openFolderPropsDialog(node);
        else openPropsDialog(node);
      }
      return;
    }
    const target = activeBookmarkNode
      || allNodes.find(n => String(n.id) === gridEl.querySelector(".card.selected")?.dataset.id);
    if (target) { e.preventDefault(); openPropsDialog(target); }
  }

  if (e.key === 'Escape') {
    closeAllMenus();
    // Close topmost visible overlay dialog
    const openOverlays = [...document.querySelectorAll('.dlg-overlay:not(.hidden)')];
    if (openOverlays.length > 0) {
      const top = openOverlays[openOverlays.length - 1];
      const closeBtn = top.querySelector('.dlg-close, [id$="-cancel"], [id$="-close-btn"]');
      closeBtn?.click();
      return;
    }
    if (searchEl.value) {
      clearSearch();
    } else if (activeBookmarkNode?.parent != null) {
      selectFolder(activeBookmarkNode.parent);
    }
  }
});

// ── State ─────────────────────────────────────────────────────────────────
let allNodes          = [];
let allFolders        = [];
let activeFolderId    = null;
let activeBookmarkNode = null;
let dataDir           = ""; // absolute path to Data/ dir, set at startup

// ── Favicon queue state ───────────────────────────────────────────────────
let _faviconQueue     = [];   // Array<{id, url, domain, sameIds: number[]}>
let _faviconActive    = 0;    // current in-flight invoke count
let _faviconCancelled = false;
let _faviconTotal     = 0;
let _faviconDone      = 0;

// ── Thumb batch state ─────────────────────────────────────────────────────
const MAX_THUMB_CONCURRENCY = 1;
let _thumbQueue     = [];   // Array<{id, url, title}>
let _thumbActive    = 0;
let _thumbCancelled = false;
let _thumbTotal     = 0;
let _thumbDone      = 0;

// ── App settings ──────────────────────────────────────────────────────────────

let appSettings = {
  // Общие
  showToolbar:   true,
  listColWidth:  42,   // % width of "Название" column
  sidebarWidth:  230,  // px
  accordionTree: true,
  confirmDelete: true,
  noDuplicateUrls: false,
  uiFontSize:    13,
  // Рисунок
  thumbWidth:    1280,
  thumbHeight:   800,
  thumbTimeout:  15,
  // Расширение браузера
  extensionToken: '',
};

async function loadAppSettings() {
  try {
    const json = await invoke('load_settings');
    if (json) {
      Object.assign(appSettings, JSON.parse(json));
    }
  } catch(e) {}
  applySettings(false);
}

async function saveAppSettings() {
  try { await invoke('save_settings', { json: JSON.stringify(appSettings) }); }
  catch(e) { console.error(e); }
}

function applySettings(save = true) {
  if (typeof toolbarEl !== 'undefined') {
    if (appSettings.showToolbar) toolbarEl.classList.remove('hidden');
    else toolbarEl.classList.add('hidden');
  }
  applyColWidth(appSettings.listColWidth ?? 42, false);
  applySidebarWidth(appSettings.sidebarWidth ?? 230, false);
  document.documentElement.style.setProperty('--ui-font', (appSettings.uiFontSize || 13) + 'px');
  if (save) saveAppSettings();
}

function applySidebarWidth(px, persist = true) {
  appSettings.sidebarWidth = px;
  document.documentElement.style.setProperty('--sidebar-w', px + 'px');
  if (persist) saveAppSettings();
}

// ── Sidebar splitter ──────────────────────────────────────────────────────────
(function initSplitter() {
  const splitter = document.getElementById('splitter');
  const sidebar  = document.getElementById('sidebar');
  if (!splitter || !sidebar) return;

  splitter.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebar.offsetWidth;

    splitter.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    const onMove = (ev) => {
      const newW = Math.max(120, Math.min(600, startW + ev.clientX - startX));
      document.documentElement.style.setProperty('--sidebar-w', newW + 'px');
    };

    const onUp = () => {
      splitter.classList.remove('dragging');
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      applySidebarWidth(sidebar.offsetWidth, true);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });
})();

// ── Column resizer ────────────────────────────────────────────────────────────
function applyColWidth(pct, persist = true) {
  appSettings.listColWidth = pct;
  document.documentElement.style.setProperty('--col-name-w', pct + '%');
  if (persist) saveAppSettings();
}

(function initColResizer() {
  const resizer = document.getElementById('col-resizer');
  if (!resizer) return;

  resizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const header = document.getElementById('list-header');
    const startX  = e.clientX;
    const startPct = appSettings.listColWidth ?? 42;

    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    const onMove = (e) => {
      const dx   = e.clientX - startX;
      const total = header.offsetWidth - 18 - 5 - 16; // subtract dot + resizer + scrollbar est.
      const newPct = Math.max(15, Math.min(75, startPct + (dx / total * 100)));
      applyColWidth(newPct, false);
    };

    const onUp = () => {
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      applyColWidth(appSettings.listColWidth, true); // persist final value
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });
})();

// ── Settings dialog ────────────────────────────────────────────────────────────

(function() {
  const overlay = document.getElementById('settings-overlay');

  // ── Tab switching ──
  const tabs   = document.querySelectorAll('#settings-tabs .stab');
  const panels = document.querySelectorAll('.stab-panel');
  tabs.forEach(tab => tab.addEventListener('click', () => {
    tabs.forEach(t => t.classList.remove('active'));
    panels.forEach(p => p.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById('stab-' + tab.dataset.tab)?.classList.add('active');
  }));

  // ── Font size slider ──
  const fontSlider = document.getElementById('s-font-size');
  const fontVal    = document.getElementById('s-font-size-val');
  fontSlider.addEventListener('input', () => { fontVal.textContent = fontSlider.value; });

  // ── Open dialog: populate fields ──
  window.openSettingsDialog = function() {
    // Show actual DB path
    invoke('get_db_path').then(p => {
      const el = document.getElementById('s-db-path');
      if (el) el.textContent = p;
    }).catch(() => {});
    // Reset to first tab
    tabs.forEach(t => t.classList.remove('active'));
    panels.forEach(p => p.classList.remove('active'));
    tabs[0]?.classList.add('active');
    document.getElementById('stab-general')?.classList.add('active');

    // Общие
    document.getElementById('s-show-toolbar').checked = appSettings.showToolbar;
    document.getElementById('s-accordion').checked   = appSettings.accordionTree;
    document.getElementById('s-confirm-del').checked  = appSettings.confirmDelete;
    document.getElementById('s-no-dupes').checked     = appSettings.noDuplicateUrls;
    fontSlider.value = appSettings.uiFontSize || 13;
    fontVal.textContent = fontSlider.value;
    // Рисунок
    document.getElementById('s-thumb-w').value       = appSettings.thumbWidth;
    document.getElementById('s-thumb-h').value       = appSettings.thumbHeight;
    document.getElementById('s-thumb-timeout').value = appSettings.thumbTimeout;
    document.getElementById('s-thumb-defaults').onclick = () => {
      document.getElementById('s-thumb-w').value       = 1280;
      document.getElementById('s-thumb-h').value       = 800;
      document.getElementById('s-thumb-timeout').value = 15;
    };

    raiseOverlay(overlay);
  };

  // ── Save ──
  document.getElementById('settings-x').onclick      = () => overlay.classList.add('hidden');
  document.getElementById('settings-cancel').onclick = () => overlay.classList.add('hidden');
  document.getElementById('settings-ok').onclick = () => {
    // Общие
    appSettings.showToolbar     = document.getElementById('s-show-toolbar').checked;
    appSettings.accordionTree   = document.getElementById('s-accordion').checked;
    appSettings.confirmDelete   = document.getElementById('s-confirm-del').checked;
    appSettings.noDuplicateUrls = document.getElementById('s-no-dupes').checked;
    appSettings.uiFontSize      = parseInt(fontSlider.value) || 13;
    // Рисунок
    appSettings.thumbWidth   = parseInt(document.getElementById('s-thumb-w').value)   || 1280;
    appSettings.thumbHeight  = parseInt(document.getElementById('s-thumb-h').value)   || 800;
    appSettings.thumbTimeout = parseInt(document.getElementById('s-thumb-timeout').value) || 30;

    applySettings(true);
    overlay.classList.add('hidden');
  };
  overlay.addEventListener('click', e => { if (e.target === overlay) overlay.classList.add('hidden'); });
})();


// ── DOM refs ──────────────────────────────────────────────────────────────
const importScreen = document.getElementById("import-screen");
const app          = document.getElementById("app");
const treeEl       = document.getElementById("tree");
const gridEl       = document.getElementById("grid");
const rootZone     = document.getElementById('tree-root-drop');
const breadcrumb   = document.getElementById("breadcrumb");
const emptyHint    = document.getElementById("empty-hint");
const searchEl        = document.getElementById("search");
const searchClearBtn  = document.getElementById("search-clear");
const searchResultsEl = document.getElementById("search-results");
const searchbarEl     = document.getElementById("searchbar");
const detailViewEl    = document.getElementById("detail-view");
const detailThumbEl   = document.getElementById("detail-thumb-wrap");
const detailImgEl     = document.getElementById("detail-img");
const detailNoImgEl   = document.getElementById("detail-no-img");
const detailUrlEl     = document.getElementById("detail-url");
const detailNoteEl    = document.getElementById("detail-note");
const detailOpenBtn   = document.getElementById("detail-open-btn");

// ── Init ──────────────────────────────────────────────────────────────────
async function init() {
  await Promise.all([
    loadBrowsersConfig(),
    loadToolbarConfig(),
    loadAppSettings(),
    invoke('get_data_dir').then(d => { dataDir = d; }).catch(() => {}),
  ]);
  buildToolbar();
  await showApp();

  // Handle urlalbum:// deep link if app was launched via protocol
  invoke('get_pending_url').then(pending => {
    if (!pending) return;
    if (activeFolderId == null) {
      alert('Сначала откройте или создайте базу данных (Файл → Создать базу).');
      return;
    }
    openNewItemDlg('link', { url: pending.url, title: pending.title });
  }).catch(() => {});
}

// ── Welcome / first-run screen ────────────────────────────────────────────────

const welcomeActs   = document.getElementById("welcome-actions");

async function showImportScreen() {
  welcomeActs.classList.remove("hidden");
  importScreen.classList.remove("hidden");
  app.classList.add("hidden");
  toolbarEl.classList.add("hidden");
  document.getElementById('statusbar').classList.add('hidden');
  currentDbName = ''; _sbInFolderCount = null; _sbSearchCount = null;
  clearStatus();
}

// "Создать новую базу" — save dialog then open empty app
document.getElementById("wb-new").addEventListener("click", async () => {
  try {
    await invoke("create_new_db");
    await showApp();
  } catch(e) {
    if (e !== "Отменено") console.error("create_new_db:", e);
  }
});

// "Открыть существующую базу" — file picker then reload
document.getElementById("wb-open").addEventListener("click", async () => {
  try {
    await invoke("open_db");
    await showApp();
  } catch(e) {
    if (e !== "Отменено") console.error("open_db:", e);
  }
});

// "Импортировать ua.dat" — file picker then import
document.getElementById("wb-import").addEventListener("click", async () => {
  try {
    await invoke("import_uadat_pick");
    await showApp();
  } catch(e) {
    if (e !== "Отменено") console.error("import_uadat_pick:", e);
  }
});


// ── Statusbar state ───────────────────────────────────────────────────────
let _sbTimer         = null;   // auto-clear timer for temp messages
let currentDbName    = '';     // basename of current DB file
let _sbInFolderCount = null;   // item count in active folder; null = no folder selected
let _sbSearchCount   = null;   // search result count; null = not in search mode

// ── Window title ──────────────────────────────────────────────────────────
async function updateWindowTitle() {
  try {
    const p = await invoke('get_db_path');
    currentDbName = p ? p.replace(/\\/g, '/').split('/').pop() : '';
    const title = `URL Album ${APP_VERSION}`;
    document.title = title;
    await invoke('set_window_title', { title });
    rootZone.textContent = currentDbName || '↑ Корень';
  } catch(_) { currentDbName = ''; }
}

// ── Statusbar API ─────────────────────────────────────────────────────────

// sticky=false → auto-clears after 3s; sticky=true → stays until replaced/cleared
function setStatus(text, { sticky = false } = {}) {
  const el = document.getElementById('sb-right');
  clearTimeout(_sbTimer); _sbTimer = null;
  el.textContent = text;
  el.className   = sticky ? 'sb-sticky' : 'sb-temp';
  if (!sticky && text) {
    _sbTimer = setTimeout(() => { el.textContent = ''; el.className = ''; }, 3000);
  }
}

function clearStatus() {
  clearTimeout(_sbTimer); _sbTimer = null;
  const el = document.getElementById('sb-right');
  el.textContent = ''; el.className = '';
}

function updateStatusLeft() {
  const nFolders = allFolders.length;
  const nLinks   = allNodes.filter(n => n.kind === 'bookmark').length;
  const parts    = [`Папок: ${nFolders}`, `Ссылок: ${nLinks}`];
  if      (_sbSearchCount   !== null) parts.push(`Найдено: ${_sbSearchCount}`);
  else if (_sbInFolderCount !== null) parts.push(`В папке: ${_sbInFolderCount}`);
  document.getElementById('sb-left').textContent = parts.join(' · ');
}

// ── Main app ──────────────────────────────────────────────────────────────
async function showApp() {
  app.classList.remove("hidden");
  if (appSettings.showToolbar) toolbarEl.classList.remove("hidden");
  importScreen.classList.add("hidden");

  await updateWindowTitle();

  // Full UI reset — clears artifacts from any previously open database
  activeFolderId = null;
  activeBookmarkNode = null;
  hideInfoBar();
  gridEl.innerHTML = '';
  emptyHint.classList.remove('hidden');
  breadcrumb.textContent = '';

  allNodes   = await invoke("get_tree");
  allFolders = allNodes.filter(n => n.kind === "folder");

  _sbInFolderCount = null;
  _sbSearchCount   = null;
  document.getElementById('statusbar').classList.remove('hidden');

  dataDir = await invoke('get_data_dir').catch(() => dataDir);

  renderTree();
  updateStatusLeft();

  // Highlight first top-level folder without expanding it
  const roots    = allFolders.filter(f => f.parent === null);
  const topLevel = roots.length === 1
    ? allFolders.filter(f => f.parent === roots[0].id)
    : roots;
  if (topLevel.length > 0) selectFolder(topLevel[0].id, false, true);
}

// ── Drag & Drop ───────────────────────────────────────────────────────────────

let _dragNode        = null;  // { id, kind, parent }
let _dragExpandTimer = null;
let _scrollRafId     = null;
let _scrollDir       = 0;     // -1 up / 0 stop / +1 down

function _isDragValid(targetFolderId) {
  if (!_dragNode) return false;
  if (_dragNode.id === targetFolderId) return false;
  if (_dragNode.kind === 'folder') {
    // No circular refs: walk up from target, reject if we hit dragNode
    let cur = allNodes.find(n => n.id === targetFolderId);
    while (cur) {
      if (cur.id === _dragNode.id) return false;
      if (cur.parent == null) break;
      cur = allNodes.find(n => n.id === cur.parent);
    }
  }
  return true;
}

async function _doDrop(targetFolderId) {
  if (!_isDragValid(targetFolderId) || !_dragNode) return;
  const openIds = saveOpenState();
  try {
    await invoke('move_node', { id: _dragNode.id, newParent: targetFolderId });
    allNodes   = await invoke('get_tree');
    allFolders = allNodes.filter(n => n.kind === 'folder');
    renderTree();
    restoreOpenState(openIds);
    if (targetFolderId !== null) {
      const ti = treeEl.querySelector(`.tree-item[data-id="${targetFolderId}"]`);
      const ch = ti?.parentElement?.querySelector(':scope > .tree-children');
      if (ti && ch) { ch.classList.add('open'); ti.classList.add('open'); }
    }
    if (activeFolderId != null) await loadFolderContents(activeFolderId);
  } catch(e) { console.error('move_node:', e); }
}

async function _doSort(targetNodeId, insertBefore) {
  if (!_dragNode) return;
  const node = allNodes.find(n => n.id === _dragNode.id);
  if (!node) return;
  const siblings = allNodes
    .filter(n => n.kind === node.kind && n.parent === node.parent)
    .sort((a, b) => (a.sort_idx ?? 0) - (b.sort_idx ?? 0) || a.id - b.id);
  const srcIdx = siblings.findIndex(n => n.id === node.id);
  if (srcIdx < 0) return;
  siblings.splice(srcIdx, 1);
  let destIdx = siblings.findIndex(n => n.id === targetNodeId);
  if (destIdx < 0) return;
  if (!insertBefore) destIdx++;
  siblings.splice(destIdx, 0, node);
  await _applySortOrder(siblings, node);
}

// ── DnD via event delegation on containers ────────────────────────────────

function _clearDragOver() {
  treeEl.querySelectorAll('.drag-over').forEach(el => el.classList.remove('drag-over'));
  gridEl.querySelectorAll('.drag-over').forEach(el => el.classList.remove('drag-over'));
  gridEl.querySelectorAll('.sort-before,.sort-after').forEach(el => el.classList.remove('sort-before','sort-after'));
  document.getElementById('tree-root-drop').classList.remove('drag-over');
  clearTimeout(_dragExpandTimer); _dragExpandTimer = null;
}

function _stopAutoScroll() {
  _scrollDir = 0;
  if (_scrollRafId) { cancelAnimationFrame(_scrollRafId); _scrollRafId = null; }
}

function _initDragDrop() {
  // ── Root drop zone ────────────────────────────────────────────────────────
  rootZone.addEventListener('dragover', (e) => {
    if (!_dragNode) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    rootZone.classList.add('drag-over');
  });
  rootZone.addEventListener('dragleave', () => rootZone.classList.remove('drag-over'));
  rootZone.addEventListener('drop', async (e) => {
    e.preventDefault();
    rootZone.classList.remove('drag-over');
    _stopAutoScroll();
    await _doDrop(null);
  });

  // ── Tree drop target ──────────────────────────────────────────────────────
  treeEl.addEventListener('dragover', (e) => {
    if (!_dragNode) return;

    // ── Auto-scroll ───────────────────────────────────────────────────────
    const ZONE = 40;
    const rect = treeEl.getBoundingClientRect();
    const newDir = e.clientY < rect.top + ZONE ? -1 : e.clientY > rect.bottom - ZONE ? 1 : 0;
    if (newDir !== _scrollDir) {
      _scrollDir = newDir;
      if (_scrollDir !== 0 && !_scrollRafId) {
        const scroll = () => {
          if (_scrollDir === 0) { _scrollRafId = null; return; }
          treeEl.scrollTop += _scrollDir * 8;
          _scrollRafId = requestAnimationFrame(scroll);
        };
        _scrollRafId = requestAnimationFrame(scroll);
      }
    }

    const targetEl = e.target.closest('.tree-item');
    if (!targetEl) return;
    const isFolderDrag = _dragNode.kind === 'folder';
    const isTargetFolder = !targetEl.classList.contains('link');

    if (isFolderDrag && isTargetFolder) {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      const r = targetEl.getBoundingClientRect();
      const rel = (e.clientY - r.top) / r.height;
      const isOpen = targetEl.classList.contains('open');
      const wantClass = rel < 0.25 ? 'sort-before'
        : (!isOpen && rel > 0.75) ? 'sort-after'
        : 'drag-over';
      if (!targetEl.classList.contains(wantClass)) {
        _clearDragOver();
        targetEl.classList.add(wantClass);
        if (wantClass === 'drag-over') {
          const childrenEl = targetEl.parentElement?.querySelector(':scope > .tree-children');
          if (childrenEl && !childrenEl.classList.contains('open')) {
            _dragExpandTimer = setTimeout(() => {
              childrenEl.classList.add('open');
              targetEl.classList.add('open');
            }, 650);
          }
        }
      }
    } else if (!isFolderDrag && isTargetFolder) {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      if (!targetEl.classList.contains('drag-over')) {
        _clearDragOver();
        targetEl.classList.add('drag-over');
        const childrenEl = targetEl.parentElement?.querySelector(':scope > .tree-children');
        if (childrenEl && !childrenEl.classList.contains('open')) {
          _dragExpandTimer = setTimeout(() => {
            childrenEl.classList.add('open');
            targetEl.classList.add('open');
          }, 650);
        }
      }
    }
  });

  treeEl.addEventListener('dragleave', (e) => {
    if (!treeEl.contains(e.relatedTarget)) _clearDragOver();
  });

  treeEl.addEventListener('drop', async (e) => {
    e.preventDefault();
    _clearDragOver();
    _stopAutoScroll();
    const targetEl = e.target.closest('.tree-item');
    if (!targetEl) return;
    const targetId = Number(targetEl.dataset.id);
    const targetNode = allNodes.find(n => n.id === targetId);
    if (!targetNode) return;
    const isFolderDrag = _dragNode?.kind === 'folder';
    const isTargetFolder = !targetEl.classList.contains('link');
    if (isFolderDrag && isTargetFolder) {
      const r = targetEl.getBoundingClientRect();
      const rel = (e.clientY - r.top) / r.height;
      const isOpen = targetEl.classList.contains('open');
      const zone = rel < 0.25 ? 'before' : (!isOpen && rel > 0.75) ? 'after' : 'into';
      if (zone === 'into') {
        await _doDrop(targetId);
      } else {
        if (targetNode.parent !== _dragNode.parent) return;
        await _doSort(targetId, zone === 'before');
      }
    } else if (isTargetFolder) {
      await _doDrop(targetId);
    }
  });

  // ── Grid drop target ──────────────────────────────────────────────────────
  gridEl.addEventListener('dragover', (e) => {
    if (!_dragNode) return;
    const card = e.target.closest('.card');
    if (!card) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    if (!card.classList.contains('card-folder') && _dragNode.kind === 'bookmark') {
      const rect = card.getBoundingClientRect();
      const wantClass = e.clientY < rect.top + rect.height / 2 ? 'sort-before' : 'sort-after';
      if (!card.classList.contains(wantClass)) {
        _clearDragOver();
        card.classList.add(wantClass);
      }
    } else if (card.classList.contains('card-folder')) {
      if (_dragNode.kind === 'folder') {
        const rect = card.getBoundingClientRect();
        const rel = (e.clientY - rect.top) / rect.height;
        const wantClass = rel < 0.25 ? 'sort-before' : rel > 0.75 ? 'sort-after' : 'drag-over';
        if (!card.classList.contains(wantClass)) {
          _clearDragOver();
          card.classList.add(wantClass);
        }
      } else {
        if (!card.classList.contains('drag-over')) {
          _clearDragOver();
          card.classList.add('drag-over');
        }
      }
    }
  });

  gridEl.addEventListener('dragleave', (e) => {
    if (!gridEl.contains(e.relatedTarget)) _clearDragOver();
  });

  gridEl.addEventListener('drop', async (e) => {
    e.preventDefault();
    _clearDragOver();
    _stopAutoScroll();
    const card = e.target.closest('.card');
    if (!card) return;
    if (!card.classList.contains('card-folder') && _dragNode?.kind === 'bookmark') {
      const targetId = Number(card.dataset.id);
      const targetNode = allNodes.find(n => n.id === targetId);
      if (!targetNode || targetNode.parent !== _dragNode.parent) return;
      const rect = card.getBoundingClientRect();
      const before = e.clientY < rect.top + rect.height / 2;
      await _doSort(targetId, before);
    } else if (card.classList.contains('card-folder')) {
      if (_dragNode?.kind === 'folder') {
        const targetId = Number(card.dataset.id);
        const targetNode = allNodes.find(n => n.id === targetId);
        const rect = card.getBoundingClientRect();
        const rel = (e.clientY - rect.top) / rect.height;
        if (rel < 0.25 || rel > 0.75) {
          if (!targetNode || targetNode.parent !== _dragNode.parent) return;
          await _doSort(targetId, rel < 0.25);
        } else {
          await _doDrop(targetId);
        }
      } else {
        await _doDrop(Number(card.dataset.id));
      }
    }
  });
}

// kept for compatibility — no-op now that we use delegation
function _makeFolderDropTarget(_el, _folderId, _childrenEl) {}

// ── Tree ──────────────────────────────────────────────────────────────────

function buildTree() {
  const map = new Map();
  for (const n of allNodes) { n.children = []; map.set(n.id, n); }

  const roots = [];
  for (const n of allNodes) {
    if (n.parent == null) roots.push(map.get(n.id));
    else map.get(n.parent)?.children.push(map.get(n.id));
  }

  // Folders always above bookmarks within each level
  const foldersFirst = (a, b) => {
    if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1;
    return (a.sort_idx ?? 0) - (b.sort_idx ?? 0) || a.id - b.id;
  };
  roots.sort(foldersFirst);
  for (const node of map.values()) node.children.sort(foldersFirst);

  return roots;
}

function renderTree() {
  const roots = buildTree();
  treeEl.innerHTML = "";
  for (const node of roots) {
    treeEl.appendChild(createTreeNode(node, 0));
  }
  updateStatusLeft();
}

function createTreeNode(node, depth) {
  const wrap = document.createElement("div");

  const item = document.createElement("div");
  item.style.paddingLeft = `${12 + depth * 14}px`;
  item.dataset.id   = node.id;
  item.dataset.kind = node.kind;
  if (node.url) item.dataset.url = node.url;
  item.tabIndex = -1;

  // ── Drag source (all tree items) ──────────────────────────────────────────
  item.draggable = true;
  item.addEventListener('dragstart', (e) => {
    _dragNode = { id: node.id, kind: node.kind, parent: node.parent };
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', String(node.id));
    item.classList.add('dragging');
    document.body.classList.add('is-dragging');
    rootZone.textContent = '↑ Корень';
  });
  item.addEventListener('dragend', () => {
    _dragNode = null;
    item.classList.remove('dragging');
    document.body.classList.remove('is-dragging');
    rootZone.textContent = currentDbName || '↑ Корень';
    _clearDragOver();
    _stopAutoScroll();
  });

  wrap.appendChild(item);

  if (node.kind === "folder") {
    item.className = "tree-item";

    const arrow = document.createElement("span");
    arrow.className = "arrow";
    if (node.children.length > 0) arrow.dataset.hasChildren = "1";

    const folderIco = document.createElement("span");
    folderIco.className = "folder-icon";
    folderIco.appendChild(_makeFolderSvg(false));
    folderIco.appendChild(_makeFolderSvg(true));

    const label = document.createElement("span");
    label.className = "label";
    label.textContent = node.title;

    item.append(arrow, folderIco, label);

    if (node.count > 0) {
      const badge = document.createElement("span");
      badge.className = "tree-count";
      badge.textContent = node.count;
      item.appendChild(badge);
    }

    let childrenEl = null;
    if (node.children.length > 0) {
      childrenEl = document.createElement("div");
      childrenEl.className = "tree-children";
      for (const child of node.children) {
        childrenEl.appendChild(createTreeNode(child, depth + 1));
      }
      wrap.appendChild(childrenEl);
    }

    // ── Drop target (folders only) ──────────────────────────────────────────
    _makeFolderDropTarget(item, node.id, childrenEl);

    // Click on [+]/[-] box — toggle open/close only
    arrow.addEventListener("click", (e) => {
      e.stopPropagation();
      if (!childrenEl) return;
      const opening = !childrenEl.classList.contains("open");
      childrenEl.classList.toggle("open", opening);
      item.classList.toggle("open", opening);
    });

    // Click on folder label area — highlight + show contents, no toggle
    item.addEventListener("click", (e) => {
      if (e.target === arrow) return; // handled above
      e.stopPropagation();
      item.focus();
      // noTreeExpand=true: only highlight + show contents, never close open folders
      selectFolder(node.id, false, true);
    });

    item.addEventListener("dblclick", (e) => {
      e.stopPropagation();
      if (!childrenEl) return;
      const opening = !childrenEl.classList.contains("open");
      childrenEl.classList.toggle("open", opening);
      item.classList.toggle("open", opening);
    });

    item.addEventListener("contextmenu", (e) => {
      e.stopPropagation();
      // Highlight the folder on right-click
      document.querySelectorAll(".tree-item.active").forEach(el => el.classList.remove("active"));
      item.classList.add("active");
      showFolderContextMenu(e, node);
    });

  } else {
    // Bookmark leaf
    item.className = "tree-item link";

    const icon = document.createElement("span");
    icon.className = "tree-link-icon";
    if (node.favicon && dataDir) {
      setFaviconOnEl(icon, convertFileSrc(faviconFilePath(node.favicon)));
    } else {
      icon.textContent = "●";
    }

    const label = document.createElement("span");
    label.className = "label";
    label.textContent = node.title;

    item.append(icon, label);

    item.addEventListener("click", (e) => {
      e.stopPropagation();
      item.focus();
      if (e.detail >= 2) { if (node.url) openWithBrowser(node.url, getDefaultBrowserPath()); }
      else selectTreeBookmark(node);
    });

    item.addEventListener("contextmenu", (e) => {
      selectTreeBookmark(node);
      showContextMenu(e, node);
    });
  }

  item.addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (node.kind === "bookmark") openDetailView(node);
    else item.click();
  });

  return wrap;
}

// ── Search dialog (Win32-style) ───────────────────────────────────────────
const searchDlgOverlay = document.getElementById("search-dlg-overlay");
const searchDlgInput   = document.getElementById("search-dlg-input");
const schTitle         = document.getElementById("sch-title");
const schUrl           = document.getElementById("sch-url");
const schNote          = document.getElementById("sch-note");

function openSearchDialog() {
  raiseOverlay(searchDlgOverlay);
  setTimeout(() => searchDlgInput.select() || searchDlgInput.focus(), 30);
}

function closeSearchDialog() {
  searchDlgOverlay.classList.add("hidden");
}

function runSearchFromDialog() {
  const q = searchDlgInput.value.trim();
  if (!q) return;
  closeSearchDialog();
  doSearch(q, {
    title: schTitle.checked,
    url:   schUrl.checked,
    note:  schNote.checked,
  });
}

document.getElementById("search-dlg-x").onclick     = closeSearchDialog;
document.getElementById("search-dlg-close").onclick = closeSearchDialog;
document.getElementById("search-dlg-find").onclick  = runSearchFromDialog;
searchDlgOverlay.addEventListener("click", (e) => {
  if (e.target === searchDlgOverlay) closeSearchDialog();
});
searchDlgInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") { e.preventDefault(); runSearchFromDialog(); }
  if (e.key === "Escape") { e.preventDefault(); closeSearchDialog(); }
});

// ── Global search ─────────────────────────────────────────────────────────
let searchTimer   = null;
let searchResults = [];
let activeResultIdx = -1;

function getFolderPath(folderId) {
  const parts = [];
  let cur = folderId;
  const singleRoot = allFolders.filter(f => f.parent == null).length === 1;
  while (cur != null) {
    const node = allFolders.find(f => f.id === cur);
    if (!node) break;
    if (node.parent == null && singleRoot) break; // skip root wrapper
    parts.unshift(node.title);
    cur = node.parent;
  }
  return parts.join(" / ");
}

function enterSearchMode() {
  hideDetailView();
  gridEl.classList.add("hidden");
  emptyHint.classList.add("hidden");
  searchResultsEl.classList.remove("hidden");
}

// Only clears search UI, does NOT reload current folder.
function clearSearchUI() {
  if (searchResultsEl.classList.contains("hidden")) return;
  searchResultsEl.classList.add("hidden");
  searchResultsEl.innerHTML = "";
  gridEl.classList.remove("hidden");
  searchResults    = [];
  activeResultIdx  = -1;
  _sbSearchCount   = null;
  updateStatusLeft();
}

// Clears search UI and reloads current folder.
function exitSearchMode() {
  searchEl.value = "";
  searchClearBtn.classList.remove("visible");
  clearTimeout(searchTimer);
  clearSearchUI();
  if (activeFolderId != null) loadBookmarks(activeFolderId);
}

// Expand all ancestor folders in the tree so folderId becomes visible.
// Returns Set of ancestor folder IDs for a given folderId
function getAncestorIds(folderId) {
  const ids = new Set();
  let cur = allFolders.find(f => f.id === folderId)?.parent;
  while (cur != null) {
    ids.add(cur);
    cur = allFolders.find(f => f.id === cur)?.parent;
  }
  return ids;
}

// Single-branch accordion: close every open branch not on the path to activeFolderId
function collapseSiblingBranches(activeFolderId) {
  const keep = getAncestorIds(activeFolderId);
  keep.add(activeFolderId);
  treeEl.querySelectorAll('.tree-children.open').forEach(childrenEl => {
    const parentItem = childrenEl.previousElementSibling;
    if (!parentItem) return;
    const pid = parseInt(parentItem.dataset.id, 10);
    if (!isNaN(pid) && !keep.has(pid)) {
      childrenEl.classList.remove('open');
      parentItem.classList.remove('open');
    }
  });
}

function expandTreePath(folderId) {
  const ancestors = [];
  let cur = allFolders.find(f => f.id === folderId)?.parent;
  while (cur != null) {
    ancestors.unshift(cur);
    cur = allFolders.find(f => f.id === cur)?.parent;
  }
  for (const id of ancestors) {
    const item = treeEl.querySelector(`.tree-item[data-id="${id}"]`);
    if (!item) continue;
    const children = item.parentElement.querySelector(":scope > .tree-children");
    if (children) { children.classList.add("open"); item.classList.add("open"); }
  }
  if (appSettings.accordionTree) collapseSiblingBranches(folderId);
}

function highlightCard(bookmarkId) {
  gridEl.querySelectorAll(".card.highlighted").forEach(c => c.classList.remove("highlighted"));
  const card = gridEl.querySelector(`.card[data-id="${bookmarkId}"]`);
  if (card) {
    card.classList.add("highlighted");
    card.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
}

function navigateToResult(result) {
  if (result.kind === 'folder') {
    exitSearchMode();
    selectFolder(result.id);
    return;
  }
  if (result.parent == null) return;
  const node = allNodes.find(n => n.id === result.id) || result;
  searchEl.value = "";
  searchClearBtn.classList.remove("visible");
  clearTimeout(searchTimer);
  searchResults = [];
  activeResultIdx = -1;
  if (node.parent != null) {
    expandTreePath(node.parent);
    // expandTreePath only opens ancestors of node.parent, not node.parent itself —
    // open it explicitly so the bookmark leaf is visible in the tree
    const parentItem = treeEl.querySelector(`.tree-item[data-id="${node.parent}"]`);
    if (parentItem) {
      const ch = parentItem.parentElement?.querySelector(":scope > .tree-children");
      if (ch) { ch.classList.add("open"); parentItem.classList.add("open"); }
    }
  }
  _activateTreeItem(node);
  requestAnimationFrame(() => {
    treeEl.querySelector(`.tree-item[data-id="${node.id}"]`)
      ?.scrollIntoView({ block: "nearest" });
  });
  openDetailView(node);
}

async function doSearch(q, fields = { title: true, url: true, note: true }) {
  if (!q) { exitSearchMode(); return; }
  enterSearchMode();
  try {
    searchResults = await invoke("search_bookmarks", {
      query:   q,
      byTitle: fields.title !== false,
      byUrl:   fields.url   !== false,
      byNote:  fields.note  !== false,
    });
    renderSearchResults(q);
  } catch (err) { console.error(err); }
}

function renderSearchResults(q) {
  searchResultsEl.innerHTML = "";
  activeResultIdx = -1;

  _sbSearchCount   = searchResults.length;
  _sbInFolderCount = null;
  updateStatusLeft();

  if (searchResults.length === 0) {
    const el = document.createElement("div");
    el.className = "search-empty";
    el.textContent = "Ничего не найдено.";
    searchResultsEl.appendChild(el);
    return;
  }

  const cnt = document.createElement("div");
  cnt.className = "result-count";
  cnt.textContent = `Найдено: ${searchResults.length}`;
  searchResultsEl.appendChild(cnt);

  searchResults.forEach((r, i) => {
    const isFolder = r.kind === 'folder';

    const item = document.createElement("div");
    item.className = "result-item" + (isFolder ? " result-item-folder" : "");

    const top = document.createElement("div");
    top.className = "result-top";

    const title = document.createElement("span");
    title.className = "result-title";
    title.textContent = (isFolder ? "▶ " : "") + r.title;

    const path = document.createElement("span");
    path.className = "result-path";
    // For folders show path to the folder itself; for bookmarks show parent folder path
    path.textContent = isFolder ? getFolderPath(r.id) : getFolderPath(r.parent);

    top.append(title, path);
    item.appendChild(top);

    if (!isFolder && r.url) {
      const url = document.createElement("div");
      url.className = "result-url";
      url.textContent = r.url;
      url.title = r.url;
      item.appendChild(url);
    }

    item.addEventListener("click", (e) => {
      if (!isFolder && e.detail >= 2) openSearchResult(i);
      else navigateToResult(r);
    });
    item.addEventListener("mouseenter", () => setActiveResult(i, false));
    searchResultsEl.appendChild(item);
  });
}

function setActiveResult(idx, scroll = true) {
  searchResultsEl.querySelectorAll(".result-item.active")
    .forEach(el => el.classList.remove("active"));
  activeResultIdx = Math.max(0, Math.min(idx, searchResults.length - 1));
  const items = searchResultsEl.querySelectorAll(".result-item");
  if (items[activeResultIdx]) {
    items[activeResultIdx].classList.add("active");
    if (scroll) items[activeResultIdx].scrollIntoView({ block: "nearest" });
  }
}

function openSearchResult(idx) {
  const r = searchResults[idx];
  if (r?.url) invoke("open_url", { url: r.url });
}

function clearSearch() {
  searchEl.value = "";
  searchClearBtn.classList.remove("visible");
  exitSearchMode();
  searchEl.focus();
}

searchEl.addEventListener("input", () => {
  const q = searchEl.value.trim();
  searchClearBtn.classList.toggle("visible", q.length > 0);
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => doSearch(q), 200);
});

searchClearBtn.addEventListener("click", clearSearch);

// ── Detail view ───────────────────────────────────────────────────────────
function showDetailView(node) {
  activeBookmarkNode = node;
  searchbarEl.classList.add("hidden");
  gridEl.classList.add("hidden");
  emptyHint.classList.add("hidden");
  searchResultsEl.classList.add("hidden");
  detailViewEl.classList.remove("hidden");

  const url = node.url || "";

  detailUrlEl.textContent = url;
  detailUrlEl.title = url;

  const detailFavEl = document.getElementById('detail-favicon');
  if (detailFavEl) {
    if (node.favicon && dataDir) {
      detailFavEl.src = convertFileSrc(faviconFilePath(node.favicon));
      detailFavEl.classList.remove('hidden');
      detailFavEl.onerror = () => detailFavEl.classList.add('hidden');
    } else {
      detailFavEl.classList.add('hidden');
      detailFavEl.src = '';
    }
  }

  detailNoteEl.textContent = node.note || "";

  // Viewer: show real thumbnail, or subtle domain placeholder
  detailThumbEl.style.display = "";
  if (node.thumb) {
    detailImgEl.src = convertFileSrc(thumbFilePath(node.thumb));
    detailImgEl.style.display = "";
    detailNoImgEl.style.display = "none";
    detailImgEl.onerror = () => {
      detailImgEl.style.display = "none";
      detailNoImgEl.style.display = "";
      setNoImgPlaceholder(node);
    };
  } else {
    detailImgEl.style.display = "none";
    detailNoImgEl.style.display = "";
    setNoImgPlaceholder(node);
  }

  const open = () => { if (url) invoke("open_url", { url }); };
  detailOpenBtn.onclick      = open;
  detailUrlEl.onclick        = open;
  detailThumbEl.ondblclick   = open;
  detailViewEl.oncontextmenu = (e) => showContextMenu(e, node);
}

function setNoImgPlaceholder(node) {
  try {
    detailNoImgEl.textContent = new URL(
      (node.url || "").startsWith("http") ? node.url : "http://" + node.url
    ).hostname.replace(/^www\./, "").slice(0, 2).toUpperCase();
  } catch { detailNoImgEl.textContent = (node.title || "?").slice(0, 2).toUpperCase(); }
}

// ── Info bar (single-click: selection + compact info) ────────────────────────
const infoBarEl   = document.getElementById('info-bar');
const infoBarUrl  = document.getElementById('info-bar-url');
const infoBarNote = document.getElementById('info-bar-note');

function showInfoBar(node) {
  activeBookmarkNode = node;
  const url = node.url || '';
  infoBarUrl.textContent = url;
  infoBarUrl.title = url;
  infoBarNote.textContent = node.note || '';
  infoBarNote.style.display = node.note ? '' : 'none';
  infoBarEl.classList.remove('hidden');
  infoBarUrl.onclick = () => { if (url) invoke('open_url', { url }); };
}

function hideInfoBar() {
  infoBarEl?.classList.add('hidden');
}

// Full viewer (double-click): hides grid, shows detail panel
function openDetailView(node) {
  hideInfoBar();
  gridEl.classList.add('hidden');
  showDetailView(node);
}

function hideDetailView() {
  detailViewEl.classList.add("hidden");
  searchbarEl.classList.remove("hidden");

  // If viewer was opened from tree (folder not shown), reload the parent folder
  const node = activeBookmarkNode;
  if (node?.parent != null && activeFolderId !== node.parent) {
    activeFolderId = node.parent;
    loadFolderContents(node.parent);
  }

  gridEl.classList.remove("hidden");
  // Keep info bar showing for the still-selected item
  if (node) showInfoBar(node);
  else hideInfoBar();
}

function clearSelection() {
  activeBookmarkNode = null;
  hideInfoBar();
  detailViewEl.classList.add("hidden");
  gridEl.classList.remove("hidden");
}

// Shared: update tree active highlight + breadcrumb, no side effects
function _activateTreeItem(node) {
  treeEl.querySelectorAll(".tree-item.active").forEach(el => el.classList.remove("active"));
  const treeItem = treeEl.querySelector(`.tree-item[data-id="${node.id}"]`);
  if (treeItem) { treeItem.classList.add("active"); treeItem.focus(); }
  breadcrumb.textContent = node.parent != null
    ? buildBreadcrumbText(node.parent) + "  /  " + node.title
    : node.title;
}

// Tree panel click on a bookmark → open full detail viewer (grid hides)
function selectTreeBookmark(node) {
  hideContextMenu();
  _activateTreeItem(node);
  openDetailView(node);
}

// ── Tree keyboard navigation ───────────────────────────────────────────────
treeEl.addEventListener("keydown", (e) => {
  const focused = document.activeElement;
  if (!focused?.classList.contains("tree-item")) return;

  const items = getVisibleTreeItems();
  const idx   = items.indexOf(focused);

  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const next = e.key === "ArrowDown"
      ? items[Math.min(idx + 1, items.length - 1)]
      : items[Math.max(idx - 1, 0)];
    if (!next || next === focused) return;
    next.focus();
    next.scrollIntoView({ block: "nearest" });
    const id = parseInt(next.dataset.id, 10);
    if (next.dataset.kind === "folder") selectFolder(id, false);
    else {
      const node = allNodes.find(n => n.id === id);
      if (node) selectTreeBookmark(node);
    }
  } else if (e.key === "ArrowRight" && focused.dataset.kind === "folder") {
    const ch = focused.parentElement.querySelector(":scope > .tree-children");
    if (ch && !ch.classList.contains("open")) { ch.classList.add("open"); focused.classList.add("open"); }
  } else if (e.key === "ArrowLeft" && focused.dataset.kind === "folder") {
    const ch = focused.parentElement.querySelector(":scope > .tree-children");
    if (ch?.classList.contains("open")) { ch.classList.remove("open"); focused.classList.remove("open"); }
  }
});

// expand=true      → force-open the folder (navigation from right panel / programmatic)
// expand=false     → don't touch open state (tree click already toggled it)
// noTreeExpand=true → skip expandTreePath entirely (single-click: only highlight, never close others)
function selectFolder(folderId, expand = true, noTreeExpand = false) {
  hideContextMenu();
  searchEl.value = "";
  searchClearBtn.classList.remove("visible");
  clearTimeout(searchTimer);
  clearSearchUI();
  clearSelection();
  activeFolderId = folderId;

  // Update active style
  document.querySelectorAll(".tree-item.active")
    .forEach(el => el.classList.remove("active"));

  // Expand ancestors (and possibly collapse siblings via accordion) —
  // skipped on plain single-click so open folders are never closed unexpectedly
  if (!noTreeExpand) expandTreePath(folderId);

  const folderTreeItem = treeEl.querySelector(`.tree-item[data-id="${folderId}"]`);
  if (folderTreeItem) {
    if (expand) {
      // Force-open the folder itself (navigating from outside the tree)
      const ch = folderTreeItem.parentElement?.querySelector(":scope > .tree-children");
      if (ch) { ch.classList.add("open"); folderTreeItem.classList.add("open"); }
    }
    folderTreeItem.classList.add("active");
    folderTreeItem.scrollIntoView({ block: "nearest" });
  }

  // Breadcrumb
  const folder = allFolders.find(f => f.id === folderId);
  breadcrumb.textContent = folder ? buildBreadcrumbText(folderId) : "";

  return loadFolderContents(folderId);
}

function buildBreadcrumbText(id) {
  const parts = [];
  let current = id;
  while (current != null) {
    const node = allFolders.find(f => f.id === current);
    if (!node) break;
    // Skip root wrapper
    if (node.parent == null && allFolders.filter(f => f.parent == null).length === 1) break;
    parts.unshift(node.title);
    current = node.parent;
  }
  return parts.join("  /  ");
}

// ── Folder contents (subfolders + bookmarks) ──────────────────────────────
async function loadFolderContents(folderId) {
  gridEl.innerHTML = "";
  emptyHint.classList.add("hidden");

  const subfolders = allNodes
    .filter(n => n.parent === folderId && n.kind === 'folder')
    .sort((a, b) => (a.sort_idx ?? 0) - (b.sort_idx ?? 0) || a.id - b.id);
  const bookmarks  = allNodes
    .filter(n => n.parent === folderId && n.kind === 'bookmark')
    .sort((a, b) => (a.sort_idx ?? 0) - (b.sort_idx ?? 0) || a.id - b.id);

  _sbInFolderCount = subfolders.length + bookmarks.length;
  _sbSearchCount   = null;
  updateStatusLeft();

  if (subfolders.length === 0 && bookmarks.length === 0) {
    emptyHint.classList.remove("hidden");
    return;
  }

  for (const f of subfolders) gridEl.appendChild(createFolderRow(f));
  for (const b of bookmarks)  gridEl.appendChild(createCard(b));
}

// Kept for callers that reload after sort/move — delegates to loadFolderContents
function loadBookmarks(folderId) { return loadFolderContents(folderId); }

function createFolderRow(node) {
  const row = document.createElement("div");
  row.className = "card card-folder";
  row.dataset.id   = node.id;
  row.dataset.kind = "folder";

  // Drag source (move the folder itself)
  row.draggable = true;
  row.addEventListener('dragstart', (e) => {
    _dragNode = { id: node.id, kind: 'folder', parent: node.parent ?? activeFolderId };
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', String(node.id));
    row.classList.add('dragging');
    document.body.classList.add('is-dragging');
    rootZone.textContent = '↑ Корень';
  });
  row.addEventListener('dragend', () => {
    _dragNode = null;
    row.classList.remove('dragging');
    document.body.classList.remove('is-dragging');
    rootZone.textContent = currentDbName || '↑ Корень';
    _clearDragOver();
    _stopAutoScroll();
  });

  // Drop target (drop onto this folder to move items into it)
  _makeFolderDropTarget(row, node.id, null);

  const dot = document.createElement("span");
  dot.className = "row-dot row-dot-folder";
  dot.textContent = "▶";

  const name = document.createElement("span");
  name.className = "row-name";
  name.textContent = node.title;

  const sep = document.createElement("span");
  sep.className = "row-sep";

  const addr = document.createElement("span");
  addr.className = "row-addr row-addr-folder";
  addr.textContent = node.count > 0 ? node.count + " ссылок" : "";

  row.append(dot, name, sep, addr);
  return row;
}

function createCard(b) {
  const card = document.createElement("div");
  card.className = "card";
  card.dataset.id    = b.id;
  card.dataset.url   = b.url;
  card.dataset.thumb = b.thumb || "";
  if (b.note) card.title = b.note;

  // Drag source
  card.draggable = true;
  card.addEventListener('dragstart', (e) => {
    _dragNode = { id: b.id, kind: 'bookmark', parent: b.parent ?? activeFolderId };
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', String(b.id));
    card.classList.add('dragging');
    document.body.classList.add('is-dragging');
    rootZone.textContent = '↑ Корень';
  });
  card.addEventListener('dragend', () => {
    _dragNode = null;
    card.classList.remove('dragging');
    document.body.classList.remove('is-dragging');
    rootZone.textContent = currentDbName || '↑ Корень';
    _clearDragOver();
    _stopAutoScroll();
  });

  const dot = document.createElement("span");
  dot.className = "row-dot";
  if (b.favicon && dataDir) {
    setFaviconOnEl(dot, convertFileSrc(faviconFilePath(b.favicon)));
  } else {
    dot.textContent = "●";
  }

  const name = document.createElement("span");
  name.className = "row-name";
  name.textContent = b.title;

  const sep = document.createElement("span");
  sep.className = "row-sep";

  const addr = document.createElement("span");
  addr.className = "row-addr";
  addr.textContent = b.url;
  addr.title = b.url;

  card.append(dot, name, sep, addr);
  return card;
}

function _makeFolderSvg(open) {
  const img = document.createElement('img');
  img.src = open ? 'icons/folder-open.png' : 'icons/folder-closed.png';
  img.classList.add(open ? 'fsvg-open' : 'fsvg-closed');
  return img;
}

function extractDomain(url) {
  try {
    const u = new URL(url.startsWith('http') ? url : 'https://' + url);
    return u.hostname.replace(/^www\./, '').toLowerCase();
  } catch { return null; }
}

function setFaviconOnEl(el, src, fallback = '●') {
  const img = document.createElement('img');
  img.src = src;
  img.className = 'favicon-icon';
  img.onerror = () => { img.remove(); if (!el.firstChild) el.textContent = fallback; };
  el.innerHTML = '';
  el.appendChild(img);
}

// ── Favicon queue engine ──────────────────────────────────────────────────

function updateFaviconInDOM(nodeId, filePath) {
  const src = convertFileSrc(filePath);

  // Grid row
  const card = gridEl.querySelector(`.card[data-id="${nodeId}"]`);
  if (card) {
    const dot = card.querySelector('.row-dot');
    if (dot) setFaviconOnEl(dot, src);
  }

  // Tree leaf
  const treeItem = treeEl.querySelector(`.tree-item[data-id="${nodeId}"]`);
  if (treeItem) {
    const icon = treeItem.querySelector('.tree-link-icon');
    if (icon) setFaviconOnEl(icon, src);
  }

  // Detail view (if this node is active)
  if (activeBookmarkNode?.id === nodeId) {
    const detailFav = document.getElementById('detail-favicon');
    if (detailFav) {
      detailFav.src = src;
      detailFav.classList.remove('hidden');
      detailFav.onerror = () => detailFav.classList.add('hidden');
    }
  }
}

function faviconFilePath(filename) {
  // Normalize to forward slashes so convertFileSrc works on all platforms
  return dataDir.replace(/\\/g, '/') + '/favicons/' + filename;
}
function thumbFilePath(filename) {
  return dataDir.replace(/\\/g, '/') + '/' + filename;
}

function applyFaviconToDOM(item, filename) {
  const filePath = faviconFilePath(filename);

  const primary = allNodes.find(n => n.id === item.id);
  if (primary) primary.favicon = filename;

  for (const sid of item.sameIds) {
    const sn = allNodes.find(n => n.id === sid);
    if (sn) {
      sn.favicon = filename;
      // Persist to DB (no HTTP — primary already fetched the file)
      invoke('update_node_favicon', { id: sid, filename }).catch(() => {});
    }
    updateFaviconInDOM(sid, filePath);
  }

  updateFaviconInDOM(item.id, filePath);
}

function _runFaviconWorker() {
  if (_faviconCancelled || _faviconQueue.length === 0 || _faviconActive >= MAX_FAVICON_CONCURRENCY) return;
  _faviconActive++;
  const item = _faviconQueue.shift();

  const domainEl = document.getElementById('fv-domain');
  if (domainEl) domainEl.textContent = item.domain;

  invoke('fetch_favicon', { id: item.id, url: item.url })
    .then(filename => {
      if (filename) applyFaviconToDOM(item, filename);
    })
    .catch(() => {})
    .finally(() => {
      _faviconDone++;
      _faviconActive--;
      _updateFaviconPanelProgress();
      if (_faviconQueue.length === 0 && _faviconActive === 0) {
        _finishFaviconBatch();
      } else {
        _runFaviconWorker();
      }
    });
}

function startFaviconWorkers() {
  for (let i = 0; i < MAX_FAVICON_CONCURRENCY; i++) _runFaviconWorker();
}

function collectBookmarksRecursive(folderId) {
  const result = [];
  const queue  = [folderId];
  while (queue.length > 0) {
    const id = queue.shift();
    for (const n of allNodes) {
      if (n.parent !== id) continue;
      if (n.kind === 'bookmark' && n.url) result.push(n);
      else if (n.kind === 'folder') queue.push(n.id);
    }
  }
  return result;
}

function buildFaviconQueue(bookmarks) {
  const domainMap = new Map(); // domain -> index in queue
  const queue = [];
  for (const node of bookmarks) {
    const domain = extractDomain(node.url);
    if (!domain) continue;
    if (domainMap.has(domain)) {
      queue[domainMap.get(domain)].sameIds.push(node.id);
    } else {
      domainMap.set(domain, queue.length);
      queue.push({ id: node.id, url: node.url, domain, sameIds: [] });
    }
  }
  return queue;
}

async function loadSingleFavicon(node) {
  if (!node.url) return;
  try {
    const filename = await invoke('fetch_favicon', { id: node.id, url: node.url });
    if (filename) {
      const n = allNodes.find(n => n.id === node.id);
      if (n) n.favicon = filename;
      updateFaviconInDOM(node.id, faviconFilePath(filename));
      // Also reload right panel if this bookmark's folder is currently displayed
      if (activeFolderId === node.parent) await loadFolderContents(activeFolderId);
    }
  } catch(e) {
    console.error('loadSingleFavicon:', e);
  }
}

function startFaviconBatch(folderNode, recursive = true) {
  _faviconCancelled = false;
  _faviconQueue     = [];
  _faviconActive    = 0;

  const bookmarks = recursive
    ? collectBookmarksRecursive(folderNode.id)
    : allNodes.filter(n => n.parent === folderNode.id && n.kind === 'bookmark' && n.url);

  if (bookmarks.length === 0) return;

  _faviconQueue = buildFaviconQueue(bookmarks);
  if (_faviconQueue.length === 0) return;

  showFaviconPanel(_faviconQueue.length);
  startFaviconWorkers();
}

function _runThumbWorker() {
  if (_thumbCancelled || _thumbQueue.length === 0 || _thumbActive >= MAX_THUMB_CONCURRENCY) return;
  _thumbActive++;
  const item = _thumbQueue.shift();

  document.getElementById('tp-label').textContent = item.title || item.url;

  invoke('refresh_thumb', {
    id:      item.id,
    url:     item.url,
    width:   appSettings.thumbWidth   || 1280,
    height:  appSettings.thumbHeight  || 800,
    timeout: appSettings.thumbTimeout || 15,
  })
    .then(newPath => {
      if (!newPath) return;
      _applyThumbToCard(item.id, item.title, newPath);
    })
    .catch(err => {
      setStatus('Ошибка рисунка: ' + (err?.message ?? String(err)));
    })
    .finally(() => {
      _thumbDone++;
      _thumbActive--;
      _updateThumbPanelProgress();
      if (_thumbQueue.length === 0 && _thumbActive === 0) {
        _finishThumbBatch();
      } else {
        _runThumbWorker();
      }
    });
}

function startThumbBatch(folderNode) {
  _thumbCancelled = false;
  _thumbQueue     = [];
  _thumbActive    = 0;

  const bookmarks = allNodes.filter(
    n => n.parent === folderNode.id && n.kind === 'bookmark' && n.url
  );
  if (bookmarks.length === 0) return;

  _thumbQueue = bookmarks.map(n => ({ id: n.id, url: n.url, title: n.title }));
  showThumbPanel(_thumbQueue.length);
  for (let i = 0; i < MAX_THUMB_CONCURRENCY; i++) _runThumbWorker();
}

// ─────────────────────────────────────────────────────────────────────────────

function makeNoImg(title) {
  const el = document.createElement("div");
  el.className = "no-img";
  // Show first 2 chars of domain as placeholder
  try {
    el.textContent = new URL(title.startsWith("http") ? title : "http://" + title)
      .hostname.replace(/^www\./, "").slice(0, 2).toUpperCase();
  } catch {
    el.textContent = (title || "?").slice(0, 2).toUpperCase();
  }
  return el;
}

// ── Grid interaction (navigation-first desktop UX) ────────────────────────
function nodeFromCard(card) {
  const found = allNodes.find(n => String(n.id) === String(card.dataset.id));
  return found
    ? { ...found, thumb: found.thumb || card.dataset.thumb || null }
    : { url: card.dataset.url, id: card.dataset.id, title: "",
        thumb: card.dataset.thumb || null };
}

// Grid single-click: sync tree highlight + show info bar, keep grid visible
function navigateToCard(node) {
  if (node.parent != null) expandTreePath(node.parent);
  _activateTreeItem(node);
  showInfoBar(node);
  requestAnimationFrame(() => {
    treeEl.querySelector(`.tree-item[data-id="${node.id}"]`)
      ?.scrollIntoView({ block: "nearest" });
  });
}

// Close all folders, open only the path to the node's parent, highlight node
function focusTreeLink(node) {
  // Close all open folders
  treeEl.querySelectorAll('.tree-children.open').forEach(el => {
    el.classList.remove('open');
    el.previousElementSibling?.classList.remove('open');
  });
  // Build ancestor path from root to parent
  const ancestors = [];
  let cur = node.parent;
  while (cur != null) {
    ancestors.unshift(cur);
    cur = allFolders.find(f => f.id === cur)?.parent ?? null;
  }
  // Open each ancestor folder
  for (const id of ancestors) {
    const item = treeEl.querySelector(`.tree-item[data-id="${id}"]`);
    const ch   = item?.parentElement?.querySelector(':scope > .tree-children');
    if (item && ch) { ch.classList.add('open'); item.classList.add('open'); }
  }
  // Highlight the link and scroll to it
  _activateTreeItem(node);
}

// ── Grid interaction ──────────────────────────────────────────────────────
function gridSelectRow(card) {
  gridEl.querySelectorAll(".card.selected").forEach(c => c.classList.remove("selected"));
  card.classList.add("selected");
}

gridEl.addEventListener("click", (e) => {
  if (e.detail >= 2) return;
  const card = e.target.closest(".card");
  if (!card) return;
  gridSelectRow(card);

  if (card.dataset.kind === "folder") {
    const node = allNodes.find(n => n.id === parseInt(card.dataset.id));
    if (node) selectFolder(node.id);
  } else {
    const node = nodeFromCard(card);
    focusTreeLink(node);
    openDetailView(node);
  }
});

gridEl.addEventListener("dblclick", (e) => {
  const card = e.target.closest(".card");
  if (!card) return;
  if (card.dataset.kind === "folder") {
    const node = allNodes.find(n => n.id === parseInt(card.dataset.id));
    if (node) selectFolder(node.id);
  } else {
    const node = nodeFromCard(card);
    if (node.url) openWithBrowser(node.url, getDefaultBrowserPath());
  }
});

gridEl.addEventListener("contextmenu", (e) => {
  const card = e.target.closest(".card");
  if (!card) return;
  gridSelectRow(card);
  if (card.dataset.kind === "folder") {
    const node = allNodes.find(n => n.id === parseInt(card.dataset.id));
    if (node) showFolderContextMenu(e, node);
    return;
  }
  if (!card.dataset.url) return;
  const found = allNodes.find(n => String(n.id) === String(card.dataset.id));
  const node  = found
    ? { ...found, thumb: found.thumb || card.dataset.thumb || null }
    : { url: card.dataset.url, id: card.dataset.id, title: "",
        thumb: card.dataset.thumb || null };
  showContextMenu(e, node);
});

// ── Browser extension events ──────────────────────────────────────────────
window.__TAURI__.event.listen('bookmark-added', () => {
    refreshTree();
});

window.__TAURI__.event.listen('thumb-updated', (e) => {
    const { id, path } = e.payload;
    const node = allNodes.find(n => n.id === id);
    if (node) _applyThumbToCard(id, node.title || '', path);
});

// ── Start ─────────────────────────────────────────────────────────────────
buildMenubar();
_initDragDrop();
init();
