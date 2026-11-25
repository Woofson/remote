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
          ${r.can_download && !live ? (r.kind === 'graphics'
            ? `<button class="btn btn-secondary btn-sm" data-rec-video="${id}" title="Make a video file of this recording">Save as video</button>`
            : `<button class="btn btn-secondary btn-sm" data-rec-download="${id}">Download</button>`) : ''}
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
  const vid = e.target.closest('[data-rec-video]');
  if (vid) return openRecordingPlayer(recFind(vid.dataset.recVideo), { exportNow: true });
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
  document.getElementById('rec-player-download').addEventListener('click', () => {
    if (recPlayer.mode === 'graphics') exportRecordingVideo();
    else downloadRecording(recPlayer.rec);
  });
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
    if (recPlayer.mode === 'graphics') {
      // Desktop: the timeline stays real time; long pauses are just jumped over while playing.
      recPlayer.skipIdle = e.target.checked;
      return;
    }
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
  if (recPlayer.mode === 'graphics') return recPlayer.g ? recPlayer.g.duration : 0;
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

async function openRecordingPlayer(rec, opts = {}) {
  if (!rec) return;
  closeRecordingPlayer();
  if (rec.kind === 'graphics') return openGraphicsPlayer(rec, opts);
  recPlayer.mode = 'terminal';
  document.getElementById('rec-player-term').style.display = '';
  document.getElementById('rec-find').style.display = '';
  document.getElementById('rec-player-download').title = 'Download';
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
  if (p.exporting) p.exporting.cancelled = true;
  if (p.g) {
    p.g.token++;
    if (p.g.canvas) p.g.canvas.remove();
    p.g = null;
  }
  p.mode = null;
  const busy = document.getElementById('rec-player-busy');
  if (busy) busy.remove();
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
  if (p.mode === 'graphics') return recGraphicsSeek(t);
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
  if (!p.term && !p.g) return;
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
  if (p.mode === 'graphics') return recGraphicsFrame(now);
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

// ---- Desktop recordings (RDP, VNC) ----
// The server sends one piece at a time: from a seek point (screen size + whole picture) up
// to the next. Each piece is a list of compressed chunks; unpacked, it is a list of events
// [u32 time ms][u8 kind][u32 length][payload], kind 0 = picture update, kind 1 = size.

async function recInflate(packed) {
  const dv = new DataView(packed);
  const parts = [];
  let total = 0;
  let at = 0;
  while (at + 4 <= packed.byteLength) {
    const len = dv.getUint32(at, true);
    const piece = new Uint8Array(packed, at + 4, Math.min(len, packed.byteLength - at - 4));
    at += 4 + len;
    const out = await new Response(new Blob([piece]).stream().pipeThrough(new DecompressionStream('deflate-raw'))).arrayBuffer();
    parts.push(new Uint8Array(out));
    total += out.byteLength;
  }
  const all = new Uint8Array(total);
  let p = 0;
  for (const part of parts) { all.set(part, p); p += part.length; }
  return all.buffer;
}

function recParseEvents(buf) {
  const dv = new DataView(buf);
  const ev = [];
  let p = 0;
  while (p + 9 <= buf.byteLength) {
    const len = dv.getUint32(p + 5, true);
    if (p + 9 + len > buf.byteLength) break;
    ev.push({ t: dv.getUint32(p, true) / 1000, kind: dv.getUint8(p + 4), off: p + 9, len });
    p += 9 + len;
  }
  return { buf, ev };
}

async function recFetchSegment(recId, n) {
  const res = await apiFetch(`api/recordings/${encodeURIComponent(recId)}/segment/${n}`);
  if (!res.ok) throw new Error(await readError(res, 'Could not load this part of the recording'));
  return recParseEvents(await recInflate(await res.arrayBuffer()));
}

/** Draw one event on a canvas, the same way a live session is drawn. */
function recApplyEvent(canvas, ctx, buf, ev) {
  const dv = new DataView(buf, ev.off, ev.len);
  if (ev.kind === 1) {
    if (ev.len >= 4) {
      canvas.width = dv.getUint16(0, true);
      canvas.height = dv.getUint16(2, true);
      ctx.fillStyle = '#000';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
    }
    return;
  }
  const type = dv.getUint8(0);
  if (type === 0x01 && ev.len >= 9) {
    const w = dv.getUint16(5), h = dv.getUint16(7);
    if (w && h && ev.len >= 9 + w * h * 4) ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, ev.off + 9, w * h * 4), w, h), dv.getUint16(1), dv.getUint16(3));
  } else if (type === 0x03 && ev.len >= 3) {
    const count = dv.getUint16(1);
    let at = 3;
    for (let i = 0; i < count && at + 8 <= ev.len; i++) {
      const x = dv.getUint16(at), y = dv.getUint16(at + 2), w = dv.getUint16(at + 4), h = dv.getUint16(at + 6);
      at += 8;
      const n = w * h * 4;
      if (at + n > ev.len) break;
      if (w && h) ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, ev.off + at, n), w, h), x, y);
      at += n;
    }
  } else if (type === 0x02 && ev.len >= 13) {
    const w = dv.getUint16(5), h = dv.getUint16(7);
    if (w && h) ctx.drawImage(canvas, dv.getUint16(9), dv.getUint16(11), w, h, dv.getUint16(1), dv.getUint16(3), w, h);
  }
}

async function openGraphicsPlayer(rec, opts) {
  const p = recPlayer;
  p.mode = 'graphics';
  const overlay = document.getElementById('rec-player');
  document.getElementById('rec-player-title').textContent =
    `${rec.connection_name} · ${rec.username} · ${new Date(rec.started_at).toLocaleString()}`;
  const dl = document.getElementById('rec-player-download');
  dl.style.display = rec.can_download ? '' : 'none';
  dl.title = 'Save as video';
  document.getElementById('rec-find').style.display = 'none';
  document.getElementById('rec-player-box').style.transform = '';
  document.getElementById('rec-player-screen').style.background = '#000';
  const termEl = document.getElementById('rec-player-term');
  termEl.innerHTML = '<div class="text-muted" style="padding:16px;">Loading recording…</div>';
  overlay.style.display = 'flex';

  const res = await apiFetch(`api/recordings/${encodeURIComponent(rec.id)}/index`);
  if (!res.ok) {
    termEl.innerHTML = `<div class="text-danger" style="padding:16px;">${escapeHtml(await readError(res, 'Could not open the recording'))}</div>`;
    return;
  }
  const idx = await res.json();
  if (p.mode !== 'graphics' || overlay.style.display !== 'flex') return;
  if (!idx.segments.length) {
    termEl.innerHTML = '<div class="text-muted" style="padding:16px;">This recording has no picture yet.</div>';
    return;
  }
  termEl.innerHTML = '';
  termEl.style.display = 'none';
  const canvas = document.createElement('canvas');
  canvas.className = 'rec-player-canvas';
  canvas.width = 1280;
  canvas.height = 720;
  document.getElementById('rec-player-screen').appendChild(canvas);
  p.rec = rec;
  p.g = {
    segments: idx.segments.map((s) => s.t_ms / 1000),
    duration: Math.max(idx.duration_ms / 1000, idx.segments[idx.segments.length - 1].t_ms / 1000),
    cache: new Map(), // piece number -> Promise of parsed events
    canvas,
    ctx: canvas.getContext('2d'),
    seg: -1,
    data: null,
    pos: 0,
    token: 0,
    waiting: false,
  };
  p.clock = 0;
  p.playing = false;
  p.speed = Number(document.getElementById('rec-speed').value) || 1;
  p.skipIdle = document.getElementById('rec-skip-idle').checked;
  await recGraphicsSeek(0);
  document.getElementById('rec-player-box').focus();
  if (opts.exportNow) exportRecordingVideo();
  else toggleRecordingPlay();
}

/** A piece, from the cache or the server; keeps at most a few in memory. */
function recGraphicsPiece(n) {
  const g = recPlayer.g;
  if (!g.cache.has(n)) {
    g.cache.set(n, recFetchSegment(recPlayer.rec.id, n));
    for (const k of [...g.cache.keys()]) {
      if (k < n - 1 || k > n + 2) g.cache.delete(k);
    }
  }
  return g.cache.get(n);
}

async function recGraphicsSeek(t) {
  const p = recPlayer;
  const g = p.g;
  if (!g) return;
  t = Math.max(0, Math.min(g.duration, t));
  let n = 0;
  while (n + 1 < g.segments.length && g.segments[n + 1] <= t) n++;
  const token = ++g.token;
  p.clock = t;
  recUpdateControls();
  let data;
  try {
    data = await recGraphicsPiece(n);
  } catch (e) {
    showToast(e.message, 'danger');
    return;
  }
  if (token !== g.token || recPlayer.g !== g) return; // a newer jump won
  g.seg = n;
  g.data = data;
  g.pos = 0;
  while (g.pos < data.ev.length && data.ev[g.pos].t <= p.clock) {
    recApplyEvent(g.canvas, g.ctx, data.buf, data.ev[g.pos++]);
  }
  if (n + 1 < g.segments.length) recGraphicsPiece(n + 1).catch(() => {});
  recUpdateControls();
}

/** Time of the next event (in this piece or the next), or null at the end. */
function recGraphicsNextTime() {
  const g = recPlayer.g;
  if (g.data && g.pos < g.data.ev.length) return g.data.ev[g.pos].t;
  return g.seg + 1 < g.segments.length ? g.segments[g.seg + 1] : null;
}

function recGraphicsFrame(now) {
  const p = recPlayer;
  const g = p.g;
  if (!p.playing || !g) return;
  const dt = ((now - p.lastFrame) / 1000) * p.speed;
  p.lastFrame = now;
  if (!g.waiting && g.data) {
    p.clock += dt;
    // "Skip pauses": jump over long quiet stretches.
    const next = recGraphicsNextTime();
    if (p.skipIdle && next !== null && next - p.clock > REC_IDLE_CAP) p.clock = next - REC_IDLE_CAP;
    while (g.data && g.pos < g.data.ev.length && g.data.ev[g.pos].t <= p.clock) {
      recApplyEvent(g.canvas, g.ctx, g.data.buf, g.data.ev[g.pos++]);
    }
    // End of this piece: carry on with the next one once it is loaded.
    if (g.data && g.pos >= g.data.ev.length && g.seg + 1 < g.segments.length && p.clock >= g.segments[g.seg + 1]) {
      const n = g.seg + 1;
      const token = g.token;
      g.waiting = true;
      recGraphicsPiece(n).then((data) => {
        if (recPlayer.g !== g || token !== g.token) return;
        g.seg = n;
        g.data = data;
        g.pos = 0;
        g.waiting = false;
        p.lastFrame = performance.now();
        if (n + 1 < g.segments.length) recGraphicsPiece(n + 1).catch(() => {});
      }).catch((e) => {
        g.waiting = false;
        p.playing = false;
        showToast(e.message, 'danger');
        recUpdateControls();
      });
    }
  }
  if (p.clock >= g.duration) {
    p.clock = g.duration;
    p.playing = false;
    recUpdateControls();
    return;
  }
  recUpdateControls();
  p.raf = requestAnimationFrame(recGraphicsFrame);
}

// ---- Save as video ----
// The browser's own video encoder turns the recording into a .webm file as fast as the
// computer allows. A frame is only made when the picture changed (at most 10 per second),
// so quiet stretches cost nothing. "Skip pauses" also shortens pauses in the video.

const REC_VIDEO_FPS = 10;

/** A minimal WebM (Matroska) file writer for one video track. */
class RecWebmWriter {
  constructor(codecId, width, height) {
    this.parts = [];
    this.pos = 0; // bytes written after the start of the Segment's content
    this.cues = [];
    this.cluster = null;
    const ebml = RecWebmWriter.el(0x1a45dfa3, RecWebmWriter.cat(
      RecWebmWriter.uint(0x4286, 1), RecWebmWriter.uint(0x42f7, 1), RecWebmWriter.uint(0x42f2, 4),
      RecWebmWriter.uint(0x42f3, 8), RecWebmWriter.str(0x4282, 'webm'), RecWebmWriter.uint(0x4287, 2),
      RecWebmWriter.uint(0x4285, 2)));
    // Segment with an 8-byte size, filled in at the end.
    this.segHead = RecWebmWriter.cat(RecWebmWriter.id(0x18538067), new Uint8Array([0x01, 0, 0, 0, 0, 0, 0, 0]));
    this.parts.push(ebml, this.segHead);
    // Seek head with fixed-width positions (Cues' is filled in at the end), then Info and Tracks.
    this.seekIndex = this.parts.length;
    this.parts.push(this.seekHead(0, 0, 0));
    const seekLen = this.parts[this.seekIndex].length;
    this.infoPos = seekLen;
    this.infoIndex = this.parts.length;
    this.parts.push(this.info(0));
    this.tracksPos = this.infoPos + this.parts[this.infoIndex].length;
    const tracks = RecWebmWriter.el(0x1654ae6b, RecWebmWriter.el(0xae, RecWebmWriter.cat(
      RecWebmWriter.uint(0xd7, 1), RecWebmWriter.uint(0x73c5, 1), RecWebmWriter.uint(0x83, 1),
      RecWebmWriter.str(0x86, codecId),
      RecWebmWriter.el(0xe0, RecWebmWriter.cat(RecWebmWriter.uint(0xb0, width), RecWebmWriter.uint(0xba, height))))));
    this.parts.push(tracks);
    this.pos = this.tracksPos + tracks.length;
  }

  static cat(...arrays) {
    const out = new Uint8Array(arrays.reduce((n, a) => n + a.length, 0));
    let p = 0;
    for (const a of arrays) { out.set(a, p); p += a.length; }
    return out;
  }

  static id(id) {
    const b = [];
    for (let v = id; v > 0; v = Math.floor(v / 256)) b.unshift(v & 0xff);
    return new Uint8Array(b);
  }

  static size(n) {
    let len = 1;
    while (len < 8 && n >= 2 ** (7 * len) - 1) len++;
    const out = new Uint8Array(len);
    let v = n;
    for (let i = len - 1; i >= 0; i--) { out[i] = v & 0xff; v = Math.floor(v / 256); }
    out[0] |= 1 << (8 - len);
    return out;
  }

  static el(id, payload) {
    return RecWebmWriter.cat(RecWebmWriter.id(id), RecWebmWriter.size(payload.length), payload);
  }

  static uint(id, v, width = 0) {
    const b = [];
    for (let x = v; x > 0; x = Math.floor(x / 256)) b.unshift(x & 0xff);
    while (b.length < Math.max(1, width)) b.unshift(0);
    return RecWebmWriter.el(id, new Uint8Array(b));
  }

  static str(id, s) {
    return RecWebmWriter.el(id, new TextEncoder().encode(s));
  }

  static float(id, v) {
    const b = new Uint8Array(8);
    new DataView(b.buffer).setFloat64(0, v);
    return RecWebmWriter.el(id, b);
  }

  seekHead(infoPos, tracksPos, cuesPos) {
    const seek = (id, pos) => RecWebmWriter.el(0x4dbb, RecWebmWriter.cat(RecWebmWriter.el(0x53ab, RecWebmWriter.id(id)), RecWebmWriter.uint(0x53ac, pos, 8)));
    return RecWebmWriter.el(0x114d9b74, RecWebmWriter.cat(seek(0x1549a966, infoPos), seek(0x1654ae6b, tracksPos), seek(0x1c53bb6b, cuesPos)));
  }

  info(durationMs) {
    return RecWebmWriter.el(0x1549a966, RecWebmWriter.cat(
      RecWebmWriter.uint(0x2ad7b1, 1000000), RecWebmWriter.str(0x4d80, 'Remote'), RecWebmWriter.str(0x5741, 'Remote'),
      RecWebmWriter.float(0x4489, durationMs)));
  }

  closeCluster() {
    if (!this.cluster) return;
    const c = RecWebmWriter.el(0x1f43b675, RecWebmWriter.cat(RecWebmWriter.uint(0xe7, this.cluster.time), ...this.cluster.blocks));
    if (this.cluster.key) this.cues.push({ time: this.cluster.time, pos: this.pos });
    this.parts.push(c);
    this.pos += c.length;
    this.cluster = null;
  }

  /** One encoded frame (EncodedVideoChunk). */
  add(chunk) {
    const ms = Math.round(chunk.timestamp / 1000);
    const key = chunk.type === 'key';
    if (!this.cluster || key || ms - this.cluster.time > 30000) {
      this.closeCluster();
      this.cluster = { time: ms, key, blocks: [] };
    }
    const data = new Uint8Array(chunk.byteLength);
    chunk.copyTo(data);
    const head = new Uint8Array(4);
    head[0] = 0x81; // track 1
    new DataView(head.buffer).setInt16(1, ms - this.cluster.time);
    head[3] = key ? 0x80 : 0;
    this.cluster.blocks.push(RecWebmWriter.el(0xa3, RecWebmWriter.cat(head, data)));
  }

  finish(durationMs) {
    this.closeCluster();
    const cuesPos = this.pos;
    const cues = RecWebmWriter.el(0x1c53bb6b, RecWebmWriter.cat(...this.cues.map((c) => RecWebmWriter.el(0xbb, RecWebmWriter.cat(
      RecWebmWriter.uint(0xb3, c.time),
      RecWebmWriter.el(0xb7, RecWebmWriter.cat(RecWebmWriter.uint(0xf7, 1), RecWebmWriter.uint(0xf1, c.pos))))))));
    this.parts.push(cues);
    this.pos += cues.length;
    this.parts[this.seekIndex] = this.seekHead(this.infoPos, this.tracksPos, cuesPos);
    this.parts[this.infoIndex] = this.info(durationMs);
    // Segment size: everything after its header.
    let v = this.pos;
    for (let i = 7; i >= 1; i--) { this.segHead[4 + i] = v & 0xff; v = Math.floor(v / 256); }
    return new Blob(this.parts, { type: 'video/webm' });
  }
}

function recBusy(text, onCancel) {
  let el = document.getElementById('rec-player-busy');
  if (!el) {
    el = document.createElement('div');
    el.id = 'rec-player-busy';
    el.className = 'rec-player-busy';
    el.innerHTML = '<div class="rec-busy-card"><span class="rec-busy-text"></span><div class="rec-busy-bar"><div></div></div><button type="button" class="btn btn-secondary btn-sm">Cancel</button></div>';
    document.getElementById('rec-player-screen').appendChild(el);
  }
  el.querySelector('.rec-busy-text').textContent = text;
  el.querySelector('button').onclick = onCancel;
  return el;
}

async function exportRecordingVideo() {
  const p = recPlayer;
  const g = p.g;
  const rec = p.rec;
  if (!g || !rec || p.exporting) return;
  if (!('VideoEncoder' in window) || !('VideoFrame' in window)) {
    showToast('This browser cannot make videos. Use a current Chrome, Edge or Firefox.', 'warning');
    return;
  }
  if (p.playing) toggleRecordingPlay();
  const job = { cancelled: false };
  p.exporting = job;
  const busy = recBusy('Preparing the video…', () => { job.cancelled = true; });
  const bar = busy.querySelector('.rec-busy-bar div');
  try {
    const ok = await apiFetch(`api/recordings/${encodeURIComponent(rec.id)}/export`, { method: 'POST' });
    if (!ok.ok) throw new Error(await readError(ok, 'Saving as video is not allowed'));

    // Video size: the first screen size, rounded down to even numbers.
    const first = await recGraphicsPiece(0);
    const sizeEv = first.ev.find((e) => e.kind === 1);
    const sdv = sizeEv ? new DataView(first.buf, sizeEv.off, 4) : null;
    const W = Math.max(2, (sdv ? sdv.getUint16(0, true) : 1280) & ~1);
    const H = Math.max(2, (sdv ? sdv.getUint16(2, true) : 720) & ~1);

    let codec = null;
    for (const [c, id] of [['vp09.00.10.08', 'V_VP9'], ['vp8', 'V_VP8']]) {
      const cfg = { codec: c, width: W, height: H, bitrate: 2500000, framerate: REC_VIDEO_FPS };
      try {
        if ((await VideoEncoder.isConfigSupported(cfg)).supported) { codec = { cfg, id }; break; }
      } catch (e) {}
    }
    if (!codec) throw new Error('This browser cannot make WebM videos. Use a current Chrome, Edge or Firefox.');

    const writer = new RecWebmWriter(codec.id, W, H);
    let failure = null;
    const encoder = new VideoEncoder({ output: (chunk) => writer.add(chunk), error: (e) => { failure = e; } });
    encoder.configure(codec.cfg);

    const work = document.createElement('canvas');
    work.width = W;
    work.height = H;
    const wctx = work.getContext('2d');
    const out = document.createElement('canvas');
    out.width = W;
    out.height = H;
    const octx = out.getContext('2d');

    const step = 1000 / REC_VIDEO_FPS;
    let shift = 0; // ms removed by shortening pauses
    let prevT = 0;
    let stateMs = 0; // video time of the picture as it is now
    let lastFrame = -Infinity;
    let lastKey = -Infinity;
    let dirty = true;
    const emit = async (when) => {
      const ms = Math.max(Math.round(when), lastFrame + 1); // whole, strictly increasing milliseconds
      octx.drawImage(work, 0, 0, W, H);
      const frame = new VideoFrame(out, { timestamp: Math.round(ms * 1000) });
      const keyFrame = ms - lastKey >= 10000;
      if (keyFrame) lastKey = ms;
      encoder.encode(frame, { keyFrame });
      frame.close();
      lastFrame = ms;
      dirty = false;
      while (encoder.encodeQueueSize > 4) await new Promise((r) => setTimeout(r, 4));
      if (failure) throw failure;
    };

    const total = g.duration * 1000;
    for (let n = 0; n < g.segments.length; n++) {
      if (job.cancelled) break;
      const piece = n === 0 ? first : await recGraphicsPiece(n);
      for (const ev of piece.ev) {
        if (job.cancelled) break;
        const raw = ev.t * 1000;
        if (p.skipIdle && raw - prevT > REC_IDLE_CAP * 1000) shift += raw - prevT - REC_IDLE_CAP * 1000;
        prevT = raw;
        const vt = raw - shift;
        // The picture before this change is shown up to now: give it a frame if it changed.
        if (dirty && vt - lastFrame >= step) await emit(Math.max(stateMs, lastFrame + 1, 0));
        // `work` follows the remote screen's own size; frames are scaled to the video size.
        recApplyEvent(work, wctx, piece.buf, ev);
        stateMs = vt;
        dirty = true;
      }
      const pct = Math.min(99, Math.round((prevT / total) * 100));
      busy.querySelector('.rec-busy-text').textContent = `Making the video… ${pct}%`;
      bar.style.width = `${pct}%`;
    }
    if (job.cancelled) {
      encoder.close();
      showToast('Video cancelled', 'info');
      return;
    }
    if (dirty) await emit(Math.max(stateMs, lastFrame + 1));
    const endMs = Math.max(total - shift, lastFrame + 1);
    await emit(endMs); // hold the last picture to the end
    await encoder.flush();
    encoder.close();
    if (failure) throw failure;
    const blob = writer.finish(endMs + step);
    const stamp = new Date(rec.started_at).toISOString().slice(0, 16).replace(/[-:]/g, '').replace('T', '-');
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${rec.connection_name}-${rec.username}-${stamp}.webm`.replace(/[\\/:*?"<>|]/g, '_');
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 60000);
    showToast(`Video saved (${formatBytes(blob.size, 1)})`, 'success');
  } catch (e) {
    showToast(e.message || 'Could not make the video', 'danger');
  } finally {
    if (p.exporting === job) p.exporting = null;
    const el = document.getElementById('rec-player-busy');
    if (el) el.remove();
  }
}
