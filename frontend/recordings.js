// ================= Session recordings (#31) =================
// Lists (Admin panel: every recording; personal settings: own and shared with me), the
// player, downloads and sharing. Terminal recordings are a header line followed by
// [seconds, "o" | "r", data] lines; the player replays them into a terminal of the
// recorded size.

const REC_IDLE_CAP = 2; // "Skip pauses": longer pauses are shortened to this many seconds

const recViews = {
  admin: { scope: 'all', prefix: 'admin-rec', items: [], content: '' },
  mine: { scope: 'mine', prefix: 'my-rec', items: [], content: '' },
};

panelLoaders['admin-panel'].recordings = () => loadRecordings('admin');
panelLoaders['settings-panel'].recordings = () => loadRecordings('mine');

function recDuration(ms) {
  const s = Math.max(0, Math.round((ms || 0) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

function recFind(id) {
  return recViews.admin.items.find((r) => r.id === id) || recViews.mine.items.find((r) => r.id === id);
}

async function loadRecordings(viewName) {
  const v = recViews[viewName];
  const params = new URLSearchParams({ scope: v.scope });
  if (v.content) params.set('content', v.content);
  const note = document.getElementById(`${v.prefix}-note`);
  if (v.content) {
    note.style.display = '';
    note.textContent = 'Searching inside recordings…';
  }
  let d;
  try {
    const res = await apiFetch(`api/recordings?${params}`);
    if (!res.ok) {
      showToast(await readError(res, 'Could not load recordings'), 'danger');
      return;
    }
    d = await res.json();
  } catch (e) {
    return;
  }
  v.items = d.recordings || [];
  v.sharingEnabled = !!d.sharing_enabled;
  if (v.content) {
    note.innerHTML = `Showing recordings containing <strong>${escapeHtml(v.content)}</strong>. <a href="#" data-rec-clear="${viewName}">Show all</a>`;
  } else {
    note.style.display = 'none';
  }
  if (viewName === 'admin') {
    const st = d.storage || {};
    const limit = st.max_total_gb ? `of ${st.max_total_gb} GB` : '(no size limit)';
    const keep = d.keep_days ? `kept for ${d.keep_days} days` : 'kept until the size limit is reached';
    document.getElementById('admin-rec-storage').innerHTML =
      `Using <strong>${formatBytes(st.total_bytes || 0, 1)}</strong> ${limit}, ${keep}. Folder: <code>${escapeHtml(st.folder || '')}</code>. Change these under <code>[recording]</code> in config.toml.`;
    const sel = document.getElementById('admin-rec-user');
    const current = sel.value;
    sel.innerHTML = '<option value="">All users</option>' +
      [...new Set(v.items.map((r) => r.username))].sort().map((u) => `<option value="${escapeHtml(u)}">${escapeHtml(u)}</option>`).join('');
    sel.value = current;
  } else {
    const keep = d.keep_days ? ` Recordings are deleted after ${d.keep_days} days.` : '';
    document.getElementById('my-rec-intro').textContent =
      `Sessions an administrator chose to record, and recordings others shared with you.${keep}`;
  }
  renderRecordings(viewName);
}

function renderRecordings(viewName) {
  const v = recViews[viewName];
  const text = document.getElementById(`${v.prefix}-filter`).value.trim().toLowerCase();
  const pick = document.getElementById(`${v.prefix}-user`).value;
  const rows = v.items.filter((r) => {
    if (viewName === 'admin' && pick && r.username !== pick) return false;
    if (viewName === 'mine' && pick === 'own' && r.relation !== 'own') return false;
    if (viewName === 'mine' && pick === 'shared' && r.relation !== 'shared') return false;
    return !text || `${r.username} ${r.connection_name} ${r.protocol}`.toLowerCase().includes(text);
  });
  document.getElementById(`${v.prefix}-count`).textContent = `${rows.length} of ${v.items.length}`;
  const body = document.getElementById(`${v.prefix}-body`);
  if (!rows.length) {
    body.innerHTML = `<tr><td colspan="7" class="text-muted">${v.items.length ? 'Nothing matches.' : 'No recordings yet.'}</td></tr>`;
    return;
  }
  body.innerHTML = rows.map((r) => {
    const started = new Date(r.started_at);
    const live = r.status === 'recording';
    const status = live
      ? ' <span class="pane-title-tag pane-rec-tag">LIVE</span>'
      : r.status === 'interrupted' ? ' <span class="text-muted text-sm" title="The server stopped while this was recording; the end may be missing">(cut short)</span>' : '';
    const who = viewName === 'admin'
      ? escapeHtml(r.username)
      : r.relation === 'own' ? '<span class="text-muted">You</span>' : `Shared by ${escapeHtml(r.username)}`;
    const shared = r.shared_with
      ? (r.shared_with.length ? r.shared_with.map((s) => escapeHtml(s.group_id ? `Group: ${s.name || '?'}` : s.name || '?') + (s.allow_download ? ' <span class="text-muted">(download)</span>' : '')).join(', ') : '—')
      : `<span class="text-muted">${r.can_download ? 'You can download' : 'Watch only'}</span>`;
    const id = escapeHtml(r.id);
    return `
      <tr>
        <td style="white-space:nowrap;">${started.toLocaleString()}</td>
        <td>${who}</td>
        <td><span style="display:inline-flex;align-items:center;gap:6px;">${connIconHtml(null, r.protocol, 'conn-icon-sm')}<strong>${escapeHtml(r.connection_name)}</strong>${status}</span></td>
        <td style="white-space:nowrap;">${live ? '—' : recDuration(r.duration_ms)}</td>
        <td style="white-space:nowrap;">${formatBytes(r.size_bytes || 0, 1)}</td>
        <td class="text-sm">${shared}</td>
        <td style="white-space:nowrap;">
          <button class="btn btn-accent btn-sm" data-rec-play="${id}">Play</button>
          ${r.can_download ? `<button class="btn btn-secondary btn-sm" data-rec-download="${id}">Download</button>` : ''}
          ${r.can_share ? `<button class="btn btn-secondary btn-sm" data-rec-share="${id}">Share</button>` : ''}
          ${r.can_delete && !live ? `<button class="btn btn-secondary btn-sm text-danger" data-rec-delete="${id}">Delete</button>` : ''}
        </td>
      </tr>`;
  }).join('');
}

function runContentSearch(viewName) {
  const v = recViews[viewName];
  v.content = document.getElementById(`${v.prefix}-content`).value.trim();
  loadRecordings(viewName);
}

document.addEventListener('DOMContentLoaded', () => {
  for (const name of Object.keys(recViews)) {
    const p = recViews[name].prefix;
    document.getElementById(`${p}-filter`).addEventListener('input', () => renderRecordings(name));
    document.getElementById(`${p}-user`).addEventListener('change', () => renderRecordings(name));
    document.getElementById(`${p}-content-btn`).addEventListener('click', () => runContentSearch(name));
    document.getElementById(`${p}-content`).addEventListener('keydown', (e) => {
      if (e.key === 'Enter') runContentSearch(name);
    });
  }
  setupRecordingPlayer();
  makeDraggable(document.getElementById('rec-share-box'), document.getElementById('rec-share-header'));
  document.getElementById('rec-share-save').addEventListener('click', saveRecordingShare);
  document.getElementById('rec-share-list').addEventListener('click', (e) => {
    const rm = e.target.closest('.assign-remove');
    if (rm) rm.closest('.assign-row').remove();
  });
});

document.addEventListener('click', async (e) => {
  const clear = e.target.closest('[data-rec-clear]');
  if (clear) {
    e.preventDefault();
    const v = recViews[clear.dataset.recClear];
    v.content = '';
    document.getElementById(`${v.prefix}-content`).value = '';
    loadRecordings(clear.dataset.recClear);
    return;
  }
  const play = e.target.closest('[data-rec-play]');
  if (play) return openRecordingPlayer(recFind(play.dataset.recPlay));
  const dl = e.target.closest('[data-rec-download]');
  if (dl) return downloadRecording(recFind(dl.dataset.recDownload));
  const share = e.target.closest('[data-rec-share]');
  if (share) return openRecordingShare(recFind(share.dataset.recShare));
  const del = e.target.closest('[data-rec-delete]');
  if (del) return deleteRecording(recFind(del.dataset.recDelete));
});

function reloadRecordingViews() {
  if (document.getElementById('admin-panel').style.display === 'flex') loadRecordings('admin');
  if (document.getElementById('settings-panel').style.display === 'flex') loadRecordings('mine');
}

async function downloadRecording(rec) {
  if (!rec) return;
  const res = await apiFetch(`api/recordings/${encodeURIComponent(rec.id)}/download`);
  if (!res.ok) {
    showToast(await readError(res, 'Could not download the recording'), 'danger');
    return;
  }
  const blob = await res.blob();
  const cd = res.headers.get('Content-Disposition') || '';
  const m = /filename\*=UTF-8''([^;]+)/.exec(cd);
  const name = m ? decodeURIComponent(m[1]) : 'recording.cast';
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
}

async function deleteRecording(rec) {
  if (!rec) return;
  const when = new Date(rec.started_at).toLocaleString();
  if (!(await uiConfirm(`Delete ${rec.username}'s recording of ${rec.connection_name} from ${when}? This cannot be undone.`, { title: 'Delete recording', okText: 'Delete', danger: true }))) return;
  const res = await apiFetch(`api/recordings/${encodeURIComponent(rec.id)}`, { method: 'DELETE' });
  if (!res.ok) {
    showToast(await readError(res, 'Could not delete the recording'), 'danger');
    return;
  }
  showToast('Recording deleted', 'success');
  reloadRecordingViews();
}

// ---- Sharing ----
let recShareTarget = null;
let recShareChoices = null;

async function openRecordingShare(rec) {
  if (!rec) return;
  const res = await apiFetch('api/recordings/share-targets');
  if (!res.ok) {
    showToast(await readError(res, 'Sharing recordings is not available'), 'warning');
    return;
  }
  recShareChoices = await res.json();
  recShareTarget = rec;
  document.getElementById('rec-share-title').textContent = `Share: ${rec.connection_name}, ${new Date(rec.started_at).toLocaleString()}`;
  const list = document.getElementById('rec-share-list');
  list.innerHTML = '';
  (rec.shared_with || []).forEach((s) => addRecordingShareRow(s));
  if (!list.children.length) addRecordingShareRow();
  document.getElementById('rec-share-box').style.transform = '';
  document.getElementById('rec-share').style.display = 'flex';
}

function closeRecordingShare() {
  document.getElementById('rec-share').style.display = 'none';
  recShareTarget = null;
}

function addRecordingShareRow(share) {
  const s = share || { allow_download: false };
  const target = s.user_id ? `u:${s.user_id}` : s.group_id ? `g:${s.group_id}` : '';
  const c = recShareChoices || { users: [], groups: [] };
  const userOpts = c.users
    .filter((u) => !recShareTarget || u.id !== recShareTarget.user_id)
    .map((u) => `<option value="u:${escapeHtml(u.id)}" ${target === 'u:' + u.id ? 'selected' : ''}>${escapeHtml(u.username)}${u.display_name && u.display_name !== u.username ? ` (${escapeHtml(u.display_name)})` : ''}</option>`)
    .join('');
  const groupOpts = c.groups
    .map((g) => `<option value="g:${escapeHtml(g.id)}" ${target === 'g:' + g.id ? 'selected' : ''}>${escapeHtml(g.name)}</option>`)
    .join('');
  const row = document.createElement('div');
  row.className = 'assign-row';
  row.innerHTML = `
    <select class="form-input assign-target">
      <option value="">Choose person or group…</option>
      <optgroup label="People">${userOpts}</optgroup>
      ${groupOpts ? `<optgroup label="Groups">${groupOpts}</optgroup>` : ''}
    </select>
    <label class="pw-check"><input type="checkbox" class="rec-share-dl" ${s.allow_download ? 'checked' : ''}/>Download</label>
    <button type="button" class="assign-remove" title="Remove">
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
    </button>`;
  document.getElementById('rec-share-list').appendChild(row);
}

async function saveRecordingShare() {
  if (!recShareTarget) return;
  const shares = [];
  for (const row of document.querySelectorAll('#rec-share-list .assign-row')) {
    const t = row.querySelector('.assign-target').value;
    if (!t) continue;
    shares.push({
      user_id: t.startsWith('u:') ? t.slice(2) : null,
      group_id: t.startsWith('g:') ? t.slice(2) : null,
      allow_download: row.querySelector('.rec-share-dl').checked,
    });
  }
  const res = await apiFetch(`api/recordings/${encodeURIComponent(recShareTarget.id)}/shares`, { method: 'PUT', body: JSON.stringify({ shares }) });
  if (!res.ok) {
    showToast(await readError(res, 'Could not share the recording'), 'danger');
    return;
  }
  showToast(shares.length ? 'Recording shared' : 'Recording is no longer shared', 'success');
  closeRecordingShare();
  reloadRecordingViews();
}

// ---- Player ----
const recPlayer = {
  rec: null,
  term: null,
  header: null,
  events: [], // { t (seconds), ts (with pauses shortened), code, data }
  duration: 0,
  durationSkip: 0,
  pos: 0, // next event to play
  clock: 0, // seconds on the active timeline
  playing: false,
  speed: 1,
  skipIdle: true,
  lastFrame: 0,
  raf: 0,
  text: '', // searchable text, lower case
  textOffsets: [], // where each event starts in `text`
  dragging: false,
};

const REC_PLAY_ICON = '<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><polygon points="6 4 20 12 6 20 6 4"/></svg>';
const REC_PAUSE_ICON = '<svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="4" width="5" height="16" rx="1"/><rect x="14" y="4" width="5" height="16" rx="1"/></svg>';
const REC_ANSI = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-_]|[\r\x00-\x08\x0b-\x1f\x7f]/g;

function setupRecordingPlayer() {
  makeDraggable(document.getElementById('rec-player-box'), document.getElementById('rec-player-header'));
  document.getElementById('rec-play-btn').addEventListener('click', toggleRecordingPlay);
  document.getElementById('rec-player-download').addEventListener('click', () => downloadRecording(recPlayer.rec));
  const seek = document.getElementById('rec-seek');
  seek.addEventListener('input', () => {
    recPlayer.dragging = true;
    recSeek((seek.value / 1000) * recActiveDuration());
  });
  seek.addEventListener('change', () => { recPlayer.dragging = false; });
  document.getElementById('rec-speed').addEventListener('change', (e) => {
    recPlayer.speed = Number(e.target.value) || 1;
  });
  document.getElementById('rec-skip-idle').addEventListener('change', (e) => {
    // Keep the same spot when switching timelines.
    const ev = recPlayer.events[Math.max(0, recPlayer.pos - 1)];
    recPlayer.skipIdle = e.target.checked;
    recPlayer.clock = ev ? recEventTime(ev) : 0;
    recUpdateControls();
  });
  document.getElementById('rec-find').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      recFindNext(e.target.value);
    }
    e.stopPropagation();
  });
  const overlay = document.getElementById('rec-player');
  overlay.addEventListener('keydown', (e) => {
    if (e.target.closest('input, select')) {
      if (e.key === 'Escape') e.target.blur();
      return;
    }
    if (e.key === 'Escape') { e.stopPropagation(); closeRecordingPlayer(); }
    else if (e.key === ' ') { e.preventDefault(); toggleRecordingPlay(); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); recSeek(recPlayer.clock + 5); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); recSeek(recPlayer.clock - 5); }
  });
}

function recEventTime(ev) {
  return recPlayer.skipIdle ? ev.ts : ev.t;
}

function recActiveDuration() {
  return recPlayer.skipIdle ? recPlayer.durationSkip : recPlayer.duration;
}

function parseRecording(text) {
  const lines = text.split('\n');
  let header = {};
  try { header = JSON.parse(lines[0]); } catch (e) {}
  const events = [];
  let prev = 0;
  let shift = 0; // time removed by shortening pauses
  for (let i = 1; i < lines.length; i++) {
    if (!lines[i]) continue;
    let ev;
    try { ev = JSON.parse(lines[i]); } catch (e) { continue; }
    if (!Array.isArray(ev) || ev.length < 3) continue;
    const t = Number(ev[0]) || 0;
    if (t - prev > REC_IDLE_CAP) shift += t - prev - REC_IDLE_CAP;
    prev = t;
    events.push({ t, ts: t - shift, code: ev[1], data: String(ev[2]) });
  }
  return { header, events };
}

async function openRecordingPlayer(rec) {
  if (!rec) return;
  closeRecordingPlayer();
  if (rec.kind !== 'terminal') {
    showToast('Playing desktop recordings arrives in a later update', 'info');
    return;
  }
  const overlay = document.getElementById('rec-player');
  document.getElementById('rec-player-title').textContent =
    `${rec.connection_name} · ${rec.username} · ${new Date(rec.started_at).toLocaleString()}`;
  document.getElementById('rec-player-download').style.display = rec.can_download ? '' : 'none';
  document.getElementById('rec-player-box').style.transform = '';
  document.getElementById('rec-find').value = '';
  overlay.style.display = 'flex';
  const termEl = document.getElementById('rec-player-term');
  termEl.innerHTML = '<div class="text-muted" style="padding:16px;">Loading recording…</div>';

  const res = await apiFetch(`api/recordings/${encodeURIComponent(rec.id)}/data`);
  if (!res.ok) {
    termEl.innerHTML = `<div class="text-danger" style="padding:16px;">${escapeHtml(await readError(res, 'Could not open the recording'))}</div>`;
    return;
  }
  const { header, events } = parseRecording(await res.text());
  await loadTerminalFont();
  if (overlay.style.display !== 'flex') return; // closed while loading

  termEl.innerHTML = '';
  const term = new Terminal({
    cols: header.width || 80,
    rows: header.height || 24,
    fontSize: 13,
    fontFamily: TERMINAL_FONT_FAMILY,
    lineHeight: 1.2,
    scrollback: 5000,
    disableStdin: true,
    cursorBlink: false,
    convertEol: false,
    allowProposedApi: true,
    theme: terminalThemeFor(currentThemeDef()),
  });
  term.open(termEl);
  // The area around the terminal takes the terminal's own background.
  const bg = term.options.theme && term.options.theme.background;
  document.getElementById('rec-player-screen').style.background = bg || '';

  // Searchable text, with where each event starts.
  const offsets = [];
  let text = '';
  for (const ev of events) {
    offsets.push(text.length);
    if (ev.code === 'o') text += ev.data.replace(REC_ANSI, '').toLowerCase();
  }

  Object.assign(recPlayer, {
    rec,
    term,
    header,
    events,
    duration: Math.max(rec.duration_ms / 1000, events.length ? events[events.length - 1].t : 0),
    durationSkip: 0,
    pos: 0,
    clock: 0,
    playing: false,
    text,
    textOffsets: offsets,
  });
  // With pauses shortened, a quiet stretch at the end is shortened too.
  const last = events.length ? events[events.length - 1] : { t: 0, ts: 0 };
  recPlayer.durationSkip = last.ts + Math.min(REC_IDLE_CAP, Math.max(0, recPlayer.duration - last.t));
  recPlayer.speed = Number(document.getElementById('rec-speed').value) || 1;
  recPlayer.skipIdle = document.getElementById('rec-skip-idle').checked;
  recUpdateControls();
  document.getElementById('rec-player-box').focus();
  toggleRecordingPlay();
}

function closeRecordingPlayer() {
  const p = recPlayer;
  p.playing = false;
  cancelAnimationFrame(p.raf);
  if (p.term) {
    try { p.term.dispose(); } catch (e) {}
  }
  Object.assign(p, { rec: null, term: null, events: [], text: '', textOffsets: [], pos: 0, clock: 0 });
  document.getElementById('rec-player').style.display = 'none';
}

/** Write events up to the clock (all at once, so seeking is quick). */
function recCatchUp() {
  const p = recPlayer;
  let buf = '';
  while (p.pos < p.events.length && recEventTime(p.events[p.pos]) <= p.clock) {
    const ev = p.events[p.pos++];
    if (ev.code === 'o') {
      buf += ev.data;
    } else if (ev.code === 'r') {
      const m = /^(\d+)x(\d+)$/.exec(ev.data);
      if (m) {
        if (buf) { p.term.write(buf); buf = ''; }
        try { p.term.resize(Number(m[1]), Number(m[2])); } catch (e) {}
      }
    }
  }
  if (buf) p.term.write(buf);
}

function recSeek(t) {
  const p = recPlayer;
  if (!p.term) return;
  t = Math.max(0, Math.min(recActiveDuration(), t));
  if (t < p.clock) {
    p.term.reset();
    try { p.term.resize(p.header.width || 80, p.header.height || 24); } catch (e) {}
    p.pos = 0;
  }
  p.clock = t;
  recCatchUp();
  recUpdateControls();
}

function toggleRecordingPlay() {
  const p = recPlayer;
  if (!p.term) return;
  if (p.playing) {
    p.playing = false;
    cancelAnimationFrame(p.raf);
  } else {
    if (p.clock >= recActiveDuration()) recSeek(0);
    p.playing = true;
    p.lastFrame = performance.now();
    p.raf = requestAnimationFrame(recFrame);
  }
  recUpdateControls();
}

function recFrame(now) {
  const p = recPlayer;
  if (!p.playing || !p.term) return;
  p.clock += ((now - p.lastFrame) / 1000) * p.speed;
  p.lastFrame = now;
  if (p.clock >= recActiveDuration()) {
    p.clock = recActiveDuration();
    recCatchUp();
    p.playing = false;
    recUpdateControls();
    return;
  }
  recCatchUp();
  recUpdateControls();
  p.raf = requestAnimationFrame(recFrame);
}

function recUpdateControls() {
  const p = recPlayer;
  const total = recActiveDuration();
  document.getElementById('rec-play-btn').innerHTML = p.playing ? REC_PAUSE_ICON : REC_PLAY_ICON;
  document.getElementById('rec-time').textContent = `${recDuration(p.clock * 1000)} / ${recDuration(total * 1000)}`;
  if (!p.dragging) document.getElementById('rec-seek').value = total ? Math.round((p.clock / total) * 1000) : 0;
}

/** Jump to the next place the text appears (after the current position, wrapping around). */
function recFindNext(query) {
  const p = recPlayer;
  const q = (query || '').trim().toLowerCase();
  if (!q || !p.term) return;
  const from = p.pos < p.textOffsets.length ? p.textOffsets[p.pos] : p.text.length;
  let at = p.text.indexOf(q, from);
  if (at < 0) at = p.text.indexOf(q);
  if (at < 0) {
    showToast(`"${query.trim()}" does not appear in this recording`, 'info');
    return;
  }
  // The event holding the end of the match: play up to and including it.
  const end = at + q.length - 1;
  let lo = 0;
  let hi = p.textOffsets.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (p.textOffsets[mid] <= end) lo = mid; else hi = mid - 1;
  }
  if (p.playing) toggleRecordingPlay();
  recSeek(recEventTime(p.events[lo]));
}
