/**
 * 🐕 Remote — High-Performance Remote Gateway Client
 * Part of the Woofson Suite (CommanderDog, NoteDog, DotDog)
 */

// ================= Terminal Theme (Woofson Golden Amber) =================
const TERMINAL_THEME = {
  background: '#090a0d',
  foreground: '#f4f4f5',
  cursor: '#f59e0b',
  cursorAccent: '#121214',
  selectionBackground: 'rgba(245, 158, 11, 0.35)',
  selectionForeground: '#ffffff',
  selectionInactiveBackground: 'rgba(245, 158, 11, 0.20)',
  black: '#18181b',
  red: '#ef4444',
  green: '#10b981',
  yellow: '#f59e0b',
  blue: '#3b82f6',
  magenta: '#d946ef',
  cyan: '#06b6d4',
  white: '#e4e4e7',
  brightBlack: '#52525b',
  brightRed: '#f87171',
  brightGreen: '#34d399',
  brightYellow: '#fbbf24',
  brightBlue: '#60a5fa',
  brightMagenta: '#e879f9',
  brightCyan: '#22d3ee',
  brightWhite: '#fafafa',
};

// ================= Global State =================
const state = {
  currentUser: null,
  jwtToken: localStorage.getItem('remote_token') || localStorage.getItem('remotedog_token') || '',
  activePaneIndex: 1,
  paneLayout: 1,
  connections: [],
  filesPath: '',
  panes: {
    1: { id: 1, socket: null, conn: null, type: null, canvas: null, ctx: null, terminal: null, fitAddon: null, resizeObserver: null },
    2: { id: 2, socket: null, conn: null, type: null, canvas: null, ctx: null, terminal: null, fitAddon: null, resizeObserver: null },
    3: { id: 3, socket: null, conn: null, type: null, canvas: null, ctx: null, terminal: null, fitAddon: null, resizeObserver: null },
    4: { id: 4, socket: null, conn: null, type: null, canvas: null, ctx: null, terminal: null, fitAddon: null, resizeObserver: null },
  },
};

// ================= Keyboard Layouts & Hardware Scancodes =================
const KEYBOARD_LAYOUTS = [
  { id: 'no', code: 'no', name: 'Norwegian', flag: '🇳🇴', label: 'NO' },
  { id: 'en_us', code: 'en_us', name: 'English (US)', flag: '🇺🇸', label: 'US' },
  { id: 'en_gb', code: 'en_gb', name: 'English (UK)', flag: '🇬🇧', label: 'GB' },
  { id: 'sv', code: 'sv', name: 'Swedish', flag: '🇸🇪', label: 'SE' },
  { id: 'da', code: 'da', name: 'Danish', flag: '🇩🇰', label: 'DK' },
  { id: 'fi', code: 'fi', name: 'Finnish', flag: '🇫🇮', label: 'FI' },
  { id: 'de', code: 'de', name: 'German', flag: '🇩🇪', label: 'DE' },
  { id: 'fr', code: 'fr', name: 'French', flag: '🇫🇷', label: 'FR' },
  { id: 'es', code: 'es', name: 'Spanish', flag: '🇪🇸', label: 'ES' },
  { id: 'it', code: 'it', name: 'Italian', flag: '🇮🇹', label: 'IT' },
  { id: 'auto', code: 'auto', name: 'Auto-Detect', flag: '🌐', label: 'Auto' },
];

const CODE_TO_SCANCODE = {
  // Row 1 (Esc & Function Keys)
  Escape: [0x01, false],
  F1: [0x3b, false],
  F2: [0x3c, false],
  F3: [0x3d, false],
  F4: [0x3e, false],
  F5: [0x3f, false],
  F6: [0x40, false],
  F7: [0x41, false],
  F8: [0x42, false],
  F9: [0x43, false],
  F10: [0x44, false],
  F11: [0x57, false],
  F12: [0x58, false],

  // Row 2 (Numbers)
  Backquote: [0x29, false],     // § | on NO / ` ~ on US
  Digit1: [0x02, false],
  Digit2: [0x03, false],
  Digit3: [0x04, false],
  Digit4: [0x05, false],
  Digit5: [0x06, false],
  Digit6: [0x07, false],
  Digit7: [0x08, false],
  Digit8: [0x09, false],
  Digit9: [0x0a, false],
  Digit0: [0x0b, false],
  Minus: [0x0c, false],         // + ? on NO / - _ on US
  Equal: [0x0d, false],         // \ ` on NO / = + on US
  Backspace: [0x0e, false],

  // Row 3 (QWERTY top)
  Tab: [0x0f, false],
  KeyQ: [0x10, false],
  KeyW: [0x11, false],
  KeyE: [0x12, false],
  KeyR: [0x13, false],
  KeyT: [0x14, false],
  KeyY: [0x15, false],
  KeyU: [0x16, false],
  KeyI: [0x17, false],
  KeyO: [0x18, false],
  KeyP: [0x19, false],
  BracketLeft: [0x1a, false],   // Å on NO / [ { on US
  BracketRight: [0x1b, false],  // ¨ ^ ~ on NO / ] } on US

  // Row 4 (Home row)
  CapsLock: [0x3a, false],
  KeyA: [0x1e, false],
  KeyS: [0x1f, false],
  KeyD: [0x20, false],
  KeyF: [0x21, false],
  KeyG: [0x22, false],
  KeyH: [0x23, false],
  KeyJ: [0x24, false],
  KeyK: [0x25, false],
  KeyL: [0x26, false],
  Semicolon: [0x27, false],     // Ø on NO / ; : on US
  Quote: [0x28, false],         // Æ on NO / ' " on US
  Backslash: [0x2b, false],     // ' * on NO / \ | on US
  Enter: [0x1c, false],

  // Row 5 (Bottom row)
  ShiftLeft: [0x2a, false],
  IntlBackslash: [0x56, false], // < > | on ISO / European / NO keyboards
  KeyZ: [0x2c, false],
  KeyX: [0x2d, false],
  KeyC: [0x2e, false],
  KeyV: [0x2f, false],
  KeyB: [0x30, false],
  KeyN: [0x31, false],
  KeyM: [0x32, false],
  Comma: [0x33, false],
  Period: [0x34, false],
  Slash: [0x35, false],         // - _ on NO / / ? on US
  ShiftRight: [0x36, false],

  // Bottom Row / Controls
  ControlLeft: [0x1d, false],
  MetaLeft: [0x5b, true],       // Windows / Cmd Left
  AltLeft: [0x38, false],
  Space: [0x39, false],
  AltRight: [0x38, true],       // AltGr
  AltGraph: [0x38, true],       // AltGr alias
  MetaRight: [0x5c, true],
  ContextMenu: [0x5d, true],
  ControlRight: [0x1d, true],

  // Navigation & Editing (Extended E0)
  Insert: [0x52, true],
  Delete: [0x53, true],
  Home: [0x47, true],
  End: [0x4f, true],
  PageUp: [0x49, true],
  PageDown: [0x51, true],
  ArrowUp: [0x48, true],
  ArrowDown: [0x50, true],
  ArrowLeft: [0x4b, true],
  ArrowRight: [0x4d, true],
  PrintScreen: [0x37, true],
  ScrollLock: [0x46, false],
  Pause: [0x45, false],
  NumLock: [0x45, false],

  // Numpad
  Numpad0: [0x52, false],
  Numpad1: [0x4f, false],
  Numpad2: [0x50, false],
  Numpad3: [0x51, false],
  Numpad4: [0x4b, false],
  Numpad5: [0x4c, false],
  Numpad6: [0x4d, false],
  Numpad7: [0x47, false],
  Numpad8: [0x48, false],
  Numpad9: [0x49, false],
  NumpadDecimal: [0x53, false],
  NumpadDivide: [0x35, true],
  NumpadMultiply: [0x37, false],
  NumpadSubtract: [0x4a, false],
  NumpadAdd: [0x4e, false],
  NumpadEnter: [0x1c, true],
};

// ================= API Helper =================
async function apiFetch(url, options = {}) {
  const headers = Object.assign({}, options.headers || {});
  if (state.jwtToken && !headers['Authorization']) {
    headers['Authorization'] = `Bearer ${state.jwtToken}`;
  }
  if (!headers['Content-Type'] && options.body && typeof options.body === 'string') {
    headers['Content-Type'] = 'application/json';
  }
  const res = await fetch(url, Object.assign({}, options, { headers }));
  if (res.status === 403) {
    res.clone().json().then((d) => {
      if (d && d.code === 'password_change_required') showPasswordRequired();
    }).catch(() => {});
  }
  return res;
}

// ================= Initialization & Lifecycle =================
document.addEventListener('DOMContentLoaded', async () => {
  loadAppVersion();
  loadPasswordPolicy();
  setupPasswordDialogs();
  setupKeyboardShortcuts();
  setupGlobalDragAndDrop();
  initPaneStyles();
  initAllPaneKbdBadges();
  await checkAuth();
});

// Version badges read the version from the server (Cargo.toml is the single source of truth).
async function loadAppVersion() {
  try {
    const res = await fetch('/api/version');
    if (!res.ok) return;
    const { version } = await res.json();
    document.querySelectorAll('.app-version').forEach((el) => {
      el.textContent = `v${version}`;
    });
  } catch (e) {}
}

// ================= Authentication =================
async function checkAuth() {
  if (!state.jwtToken) {
    showLoginModal();
    return;
  }

  try {
    const res = await fetch('/api/auth/me', {
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });

    if (res.ok) {
      state.currentUser = await res.json();
      if (state.currentUser.must_change_password) {
        showPasswordRequired();
      } else {
        onAuthenticated();
      }
    } else {
      showLoginModal();
    }
  } catch (err) {
    console.error('Auth check error:', err);
    showLoginModal();
  }
}

function renderAvatarElement(el, avatar, fallbackText) {
  if (!el) return;
  const isImage = avatar && (avatar.startsWith('data:image') || avatar.startsWith('http://') || avatar.startsWith('https://') || avatar.startsWith('/'));
  if (isImage) {
    el.innerHTML = `<img src="${avatar}" alt="Avatar" style="width: 100%; height: 100%; object-fit: cover; border-radius: inherit; display: block;">`;
  } else {
    el.innerHTML = '';
    el.textContent = (fallbackText || 'A').toUpperCase();
  }
}

function handleAvatarFileUpload(event) {
  const file = event.target.files?.[0];
  if (!file) return;

  if (!file.type.startsWith('image/')) {
    showToast('Please select a valid image file (JPEG, PNG, WebP)', 'warning');
    return;
  }

  const reader = new FileReader();
  reader.onload = function(e) {
    const img = new Image();
    img.onload = function() {
      const canvas = document.createElement('canvas');
      const targetSize = 160;
      canvas.width = targetSize;
      canvas.height = targetSize;
      const ctx = canvas.getContext('2d');

      const minDim = Math.min(img.width, img.height);
      const startX = (img.width - minDim) / 2;
      const startY = (img.height - minDim) / 2;

      ctx.drawImage(img, startX, startY, minDim, minDim, 0, 0, targetSize, targetSize);

      let dataUri = canvas.toDataURL('image/webp', 0.85);
      if (!dataUri.startsWith('data:image/webp')) {
        dataUri = canvas.toDataURL('image/jpeg', 0.85);
      }

      const avatarInput = document.getElementById('profile-avatar-data');
      const avatarPreview = document.getElementById('profile-modal-avatar');
      if (avatarInput) avatarInput.value = dataUri;
      if (avatarPreview) renderAvatarElement(avatarPreview, dataUri);
      showToast('Profile photo ready! Click "Save Changes" to apply.', 'info');
    };
    img.src = e.target.result;
  };
  reader.readAsDataURL(file);
}

function resetAvatarToDefault() {
  const avatarInput = document.getElementById('profile-avatar-data');
  const avatarPreview = document.getElementById('profile-modal-avatar');
  if (avatarInput) avatarInput.value = '';
  const initial = (state.currentUser?.username?.[0] || 'A').toUpperCase();
  if (avatarPreview) renderAvatarElement(avatarPreview, '', initial);
  showToast('Photo cleared. Click "Save Changes" to apply.', 'info');
}

function updateHeaderProfile(user) {
  if (!user) user = state.currentUser;
  if (!user) return;
  const uname = user.display_name || user.username || 'admin';
  const initial = (user.username?.[0] || 'A').toUpperCase();
  const roleStr = (user.role || 'operator').toUpperCase();
  const avatar = user.avatar_data || '';

  const navName = document.getElementById('nav-user-name');
  if (navName) navName.textContent = uname;
  const navAvatar = document.getElementById('nav-user-avatar');
  if (navAvatar) renderAvatarElement(navAvatar, avatar, initial);
  const navRole = document.getElementById('nav-user-role');
  if (navRole) navRole.textContent = roleStr;

  const menuName = document.getElementById('menu-user-name');
  if (menuName) menuName.textContent = uname;
  const menuEmail = document.getElementById('menu-user-email');
  if (menuEmail) menuEmail.textContent = user.email || `${user.username}@remote.local`;
  const menuAvatar = document.getElementById('menu-avatar-large');
  if (menuAvatar) renderAvatarElement(menuAvatar, avatar, initial);

  const lockName = document.getElementById('lock-screen-name');
  if (lockName) lockName.textContent = uname;
  const lockAvatar = document.getElementById('lock-screen-avatar');
  if (lockAvatar) renderAvatarElement(lockAvatar, avatar, initial);

  const adminItem = document.getElementById('admin-nav-item');
  if (adminItem) {
    adminItem.style.display = user.role === 'admin' ? 'flex' : 'none';
  }
}

function onAuthenticated() {
  const modal = document.getElementById('login-modal');
  if (modal) {
    modal.classList.remove('active');
    modal.style.display = 'none';
  }
  updateHeaderProfile();
  loadConnections();
  if (sessionStorage.getItem('remote_session_locked') === 'true') {
    lockSession();
  } else {
    showToast(`Welcome back, ${state.currentUser.display_name || state.currentUser.username}!`);
  }
}

function showLoginModal() {
  const modal = document.getElementById('login-modal');
  modal.classList.add('active');
  modal.style.display = 'flex';
  fetchAuthProviders();
  setTimeout(() => {
    const userInput = document.getElementById('login-username');
    if (userInput) userInput.focus();
  }, 100);
}

async function fetchAuthProviders() {
  try {
    const res = await fetch('/api/auth/providers');
    if (res.ok) {
      const data = await res.json();
      if (data.oidc && data.oidc.enabled) {
        document.getElementById('oidc-login-section').style.display = 'block';
        document.getElementById('oidc-provider-btn-label').textContent = `Sign in with ${data.oidc.provider_name || 'Authentik'}`;
      } else {
        document.getElementById('oidc-login-section').style.display = 'none';
      }
    }
  } catch (e) {}
}

async function handleLoginSubmit() {
  const username = document.getElementById('login-username').value.trim();
  const password = document.getElementById('login-password').value;
  const errEl = document.getElementById('login-error');
  errEl.style.display = 'none';

  if (!username) {
    errEl.textContent = 'Please enter your username';
    errEl.style.display = 'block';
    return;
  }

  try {
    const res = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password }),
    });

    const data = await res.json();
    if (res.ok) {
      setSessionToken(data.token);
      state.currentUser = data.user;
      if (data.must_change_password) {
        state.currentUser.must_change_password = true;
        state.loginPassword = password;
        showPasswordRequired();
      } else {
        onAuthenticated();
      }
    } else {
      errEl.textContent = data.error || 'Invalid username or password';
      errEl.style.display = 'block';
    }
  } catch (err) {
    errEl.textContent = 'Server connection error. Please try again.';
    errEl.style.display = 'block';
  }
}

const handleLogin = (e) => {
  if (e) e.preventDefault();
  handleLoginSubmit();
};

function loginWithOidc() {
  window.location.href = '/api/auth/oidc/login';
}

function logout() {
  for (let i = 1; i <= 4; i++) {
    disconnectPane(i);
  }
  state.jwtToken = '';
  state.currentUser = null;
  sessionStorage.removeItem('remote_session_locked');
  localStorage.removeItem('remote_token');
  localStorage.removeItem('remotedog_token');
  document.cookie = 'remote_token=; path=/; max-age=0; SameSite=Lax';
  document.cookie = 'remotedog_token=; path=/; max-age=0; SameSite=Lax';
  window.location.reload();
}

function lockSession() {
  const menu = document.getElementById('profile-dropdown-menu');
  if (menu) {
    menu.classList.remove('active');
    menu.style.display = 'none';
  }
  const lockScreen = document.getElementById('session-lock-screen');
  if (lockScreen) {
    sessionStorage.setItem('remote_session_locked', 'true');
    const user = state.currentUser;
    const uname = user?.display_name || user?.username || 'admin';
    const initial = (user?.username?.[0] || 'A').toUpperCase();
    const avatar = user?.avatar_data || '';

    const nameEl = document.getElementById('lock-screen-name');
    const avatarEl = document.getElementById('lock-screen-avatar');
    if (nameEl) nameEl.textContent = uname;
    if (avatarEl) renderAvatarElement(avatarEl, avatar, initial);

    lockScreen.classList.add('active');
    lockScreen.style.display = 'flex';
    const pwdInput = document.getElementById('lock-password');
    if (pwdInput) {
      pwdInput.value = '';
      setTimeout(() => pwdInput.focus(), 50);
    }
  }
  const errEl = document.getElementById('lock-error');
  if (errEl) errEl.style.display = 'none';
}

async function submitUnlockSession() {
  const pwdInput = document.getElementById('lock-password');
  const password = pwdInput ? pwdInput.value : '';
  const errEl = document.getElementById('lock-error');
  if (errEl) errEl.style.display = 'none';

  if (!password) {
    if (errEl) {
      errEl.textContent = 'Please enter your password to unlock.';
      errEl.style.display = 'block';
    }
    return;
  }

  try {
    const res = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        username: state.currentUser?.username || '',
        password: password,
      }),
    });

    if (res.ok) {
      sessionStorage.removeItem('remote_session_locked');
      const lockScreen = document.getElementById('session-lock-screen');
      if (lockScreen) {
        lockScreen.classList.remove('active');
        lockScreen.style.display = 'none';
      }
      if (pwdInput) pwdInput.value = '';
      showToast('🔓 Session unlocked!', 'success');
      
      // Restore focus to active pane
      const p = state.panes[state.activePaneIndex];
      if (p) {
        if (p.terminal) p.terminal.focus();
        else if (p.canvas) p.canvas.focus();
      }
    } else {
      const data = await res.json();
      if (errEl) {
        errEl.textContent = data.error || 'Incorrect password.';
        errEl.style.display = 'block';
      }
      if (pwdInput) {
        pwdInput.focus();
        pwdInput.select();
      }
    }
  } catch (err) {
    if (errEl) {
      errEl.textContent = 'Failed to verify password with server.';
      errEl.style.display = 'block';
    }
  }
}

function toggleProfileMenu(e) {
  if (e) e.stopPropagation();
  const menu = document.getElementById('profile-dropdown-menu');
  if (menu) {
    const isVisible = menu.classList.contains('active') || menu.style.display === 'block';
    if (isVisible) {
      menu.classList.remove('active');
      menu.style.display = 'none';
    } else {
      menu.classList.add('active');
      menu.style.display = 'block';
    }
  }
}

document.addEventListener('click', (e) => {
  if (!e.target.closest('.profile-dropdown-wrapper')) {
    const menu = document.getElementById('profile-dropdown-menu');
    if (menu) {
      menu.classList.remove('active');
      menu.style.display = 'none';
    }
  }
});

// ================= Full Window & Fullscreen Toggle System =================
function toggleFullWindow(paneIndex) {
  const app = document.getElementById('app');
  if (!app) return;

  if (paneIndex) {
    const pane = document.getElementById(`pane-${paneIndex}`);
    if (!pane) return;
    const isMax = pane.classList.contains('maximized-pane');
    // Clear maximized on all panes first
    for (let i = 1; i <= 4; i++) {
      document.getElementById(`pane-${i}`)?.classList.remove('maximized-pane');
    }
    if (isMax) {
      app.classList.remove('full-window-mode');
      showToast('Restored standard layout');
    } else {
      activatePane(paneIndex);
      pane.classList.add('maximized-pane');
      app.classList.add('full-window-mode');
      showToast(`Pane ${paneIndex} Full Window (F10 / Alt+W to restore)`);
    }
  } else {
    // Global Full Window toggle
    const isFull = app.classList.contains('full-window-mode');
    if (isFull) {
      app.classList.remove('full-window-mode');
      for (let i = 1; i <= 4; i++) {
        document.getElementById(`pane-${i}`)?.classList.remove('maximized-pane');
      }
      showToast('Exited Full Window mode');
    } else {
      app.classList.add('full-window-mode');
      showToast('Full Window mode active (F10 / Alt+W to exit)');
    }
  }

  const btnHeader = document.getElementById('btn-header-fullwindow');
  if (btnHeader) {
    btnHeader.classList.toggle('active', app.classList.contains('full-window-mode'));
  }

  // Trigger resize on visible terminals and canvases
  setTimeout(() => {
    window.dispatchEvent(new Event('resize'));
    for (let i = 1; i <= 4; i++) {
      const p = state.panes[i];
      if (p && p.fitAddon && p.terminal) {
        try { p.fitAddon.fit(); } catch (e) {}
      }
    }
  }, 100);
}

function togglePaneFullWindow(paneIndex) {
  toggleFullWindow(paneIndex);
}

function toggleFullscreen() {
  if (!document.fullscreenElement) {
    document.documentElement.requestFullscreen().catch(() => {
      toggleFullWindow();
    });
  } else {
    document.exitFullscreen().catch(() => {});
  }
}

function togglePaneFullscreen(paneIndex) {
  activatePane(paneIndex);
  const pane = document.getElementById(`pane-${paneIndex}`);
  if (!document.fullscreenElement) {
    if (pane && pane.requestFullscreen) {
      pane.requestFullscreen().catch(() => {
        document.documentElement.requestFullscreen().catch(() => {
          toggleFullWindow(paneIndex);
        });
      });
    } else {
      toggleFullscreen();
    }
  } else {
    document.exitFullscreen().catch(() => {});
  }
}

document.addEventListener('fullscreenchange', () => {
  const app = document.getElementById('app');
  const btnFullscreen = document.getElementById('btn-header-fullscreen');
  const isFs = !!document.fullscreenElement;
  if (app) {
    app.classList.toggle('fullscreen-mode', isFs);
  }
  if (btnFullscreen) {
    btnFullscreen.classList.toggle('active', isFs);
  }
  setTimeout(() => {
    window.dispatchEvent(new Event('resize'));
    for (let i = 1; i <= 4; i++) {
      const p = state.panes[i];
      if (p && p.fitAddon && p.terminal) {
        try { p.fitAddon.fit(); } catch (e) {}
      }
    }
  }, 100);
});

function togglePaneScaling(paneIndex) {
  const pane = state.panes[paneIndex];
  if (!pane || !pane.canvas) {
    showToast('Display scaling applies to active graphics sessions (RDP/VNC)');
    return;
  }
  const canvas = pane.canvas;
  const isOriginal = canvas.classList.contains('original-size');
  if (isOriginal) {
    canvas.classList.remove('original-size');
    canvas.classList.add('scaled-fit');
    showToast(`Pane ${paneIndex}: Scaled to fit window`);
  } else {
    canvas.classList.remove('scaled-fit');
    canvas.classList.add('original-size');
    showToast(`Pane ${paneIndex}: Native 1:1 resolution (No scaling)`);
  }
}

// ================= Multi-Pane Layout Controls =================
function setPaneLayout(layout) {
  state.paneLayout = layout;
  const grid = document.getElementById('viewport-grid');
  if (grid) {
    grid.className = `viewport-grid layout-${layout}`;
  }

  // Update layout button styles
  ['1', '2v', '2h', '3', '4'].forEach(id => {
    const btn = document.getElementById(`layout-${id}`);
    if (btn) btn.classList.toggle('active', String(layout) === id || (layout === 2 && id === '2v'));
  });

  const maxPanes = (layout === 1) ? 1 : (layout === 2 || layout === '2v' || layout === '2h') ? 2 : (layout === 3) ? 3 : 4;

  // Show/Hide Panes WITHOUT disconnecting background sessions
  for (let i = 1; i <= 4; i++) {
    const paneEl = document.getElementById(`pane-${i}`);
    if (paneEl) {
      const shouldShow = i <= maxPanes;
      paneEl.style.display = shouldShow ? 'flex' : 'none';
      if (shouldShow) {
        const pane = state.panes[i];
        if (pane && pane.fitAddon && pane.terminal) {
          setTimeout(() => {
            try {
              pane.fitAddon.fit();
              if (pane.socket && pane.socket.readyState === WebSocket.OPEN) {
                pane.socket.send(JSON.stringify({
                  type: 'resize',
                  cols: pane.terminal.cols,
                  rows: pane.terminal.rows,
                }));
              }
            } catch (e) {}
          }, 60);
        }
      }
    }
  }

  if (state.activePaneIndex > maxPanes) {
    activatePane(1);
  }
}

// ================= Help, About & Profile Modals =================
function openHelpModal() {
  const menu = document.getElementById('profile-dropdown-menu');
  if (menu) { menu.classList.remove('active'); menu.style.display = 'none'; }
  const originTag = document.getElementById('help-origin-tag');
  if (originTag) {
    originTag.textContent = window.location.origin;
  }
  const m = document.getElementById('help-modal');
  if (m) { m.classList.add('active'); m.style.display = 'flex'; }
}

function closeHelpModal() {
  const m = document.getElementById('help-modal');
  if (m) { m.classList.remove('active'); m.style.display = 'none'; }
}

function copyCurrentOriginToClipboard() {
  const origin = window.location.origin;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(origin).then(() => {
      showToast(`Copied ${origin} to clipboard!`, 'success');
    }).catch(() => {
      uiCopyDialog('Copy origin URL', 'Copy this origin URL for chrome://flags:', origin);
    });
  } else {
    uiCopyDialog('Copy origin URL', 'Copy this origin URL for chrome://flags:', origin);
  }
}

function openAboutModal() {
  const menu = document.getElementById('profile-dropdown-menu');
  if (menu) { menu.classList.remove('active'); menu.style.display = 'none'; }
  const m = document.getElementById('about-modal');
  if (m) { m.classList.add('active'); m.style.display = 'flex'; }
}

function closeAboutModal() {
  const m = document.getElementById('about-modal');
  if (m) { m.classList.remove('active'); m.style.display = 'none'; }
}

function openUserProfileModal() {
  const menu = document.getElementById('profile-dropdown-menu');
  if (menu) { menu.classList.remove('active'); menu.style.display = 'none'; }
  if (state.currentUser) {
    const editUsername = document.getElementById('profile-edit-username');
    if (editUsername) editUsername.textContent = state.currentUser.username || 'user';
    const editRoleBadge = document.getElementById('profile-edit-role-badge');
    if (editRoleBadge) editRoleBadge.textContent = (state.currentUser.role || 'operator').toUpperCase();
    const editAuthType = document.getElementById('profile-edit-auth-type');
    if (editAuthType) editAuthType.textContent = state.currentUser.auth_provider === 'oidc' ? 'Authentik / OIDC SSO Account' : 'Local Database Account';

    const uInput = document.getElementById('profile-username');
    if (uInput) uInput.value = state.currentUser.username || '';
    const dInput = document.getElementById('profile-display-name');
    if (dInput) dInput.value = state.currentUser.display_name || '';
    const eInput = document.getElementById('profile-email');
    if (eInput) eInput.value = state.currentUser.email || '';
    ['profile-current-password', 'profile-new-password', 'profile-new-password-confirm'].forEach((id) => {
      const el = document.getElementById(id);
      if (el) el.value = '';
    });
    const pwSection = document.getElementById('profile-password-section');
    if (pwSection) {
      const canChange = state.currentUser.can_change_password ?? state.currentUser.auth_provider === 'local';
      pwSection.style.display = canChange ? '' : 'none';
    }
    const avatarInput = document.getElementById('profile-avatar-data');
    if (avatarInput) avatarInput.value = state.currentUser.avatar_data || '';
    const aPreview = document.getElementById('profile-modal-avatar');
    const initial = (state.currentUser.username?.[0] || 'A').toUpperCase();
    if (aPreview) renderAvatarElement(aPreview, state.currentUser.avatar_data, initial);
    const msgEl = document.getElementById('profile-modal-msg');
    if (msgEl) msgEl.style.display = 'none';
  }
  const m = document.getElementById('user-profile-modal');
  if (m) { m.classList.add('active'); m.style.display = 'flex'; }
}

function closeUserProfileModal() {
  const m = document.getElementById('user-profile-modal');
  if (m) { m.classList.remove('active'); m.style.display = 'none'; }
}

async function handleUpdateProfile(e) {
  e.preventDefault();
  const displayName = document.getElementById('profile-display-name').value.trim();
  const email = document.getElementById('profile-email').value.trim();
  const currentPassword = document.getElementById('profile-current-password').value;
  const newPassword = document.getElementById('profile-new-password').value;
  const newPasswordAgain = document.getElementById('profile-new-password-confirm').value;
  const avatarData = document.getElementById('profile-avatar-data').value;
  const msgEl = document.getElementById('profile-modal-msg');

  const changingPassword = !!(currentPassword || newPassword || newPasswordAgain);
  if (changingPassword) {
    const problem = !currentPassword
      ? 'Enter your current password'
      : passwordProblem(newPassword, newPasswordAgain, state.currentUser.username, currentPassword);
    if (problem) {
      showToast(problem, 'danger');
      return;
    }
  }

  try {
    const payload = {
      display_name: displayName || null,
      email: email || null,
      avatar_data: avatarData || null,
    };
    const res = await apiFetch(`/api/users/${state.currentUser.id}`, {
      method: 'PUT',
      body: JSON.stringify(payload),
    });
    if (res.ok && changingPassword) {
      const result = await changeOwnPassword(currentPassword, newPassword);
      if (!result.ok) {
        showToast(result.error, 'danger');
        return;
      }
      showToast('Password changed. Other logins have been ended.', 'success');
    }
    if (res.ok) {
      const data = await res.json();
      if (data.user) {
        state.currentUser = Object.assign({}, state.currentUser, data.user);
      } else {
        state.currentUser.display_name = displayName || null;
        state.currentUser.email = email || null;
        state.currentUser.avatar_data = avatarData || null;
      }
      updateHeaderProfile();
      msgEl.textContent = 'Profile updated successfully!';
      msgEl.style.display = 'block';
      showToast(`Profile updated! Nickname set to "${state.currentUser.display_name || state.currentUser.username}"`, 'success');
      setTimeout(() => { closeUserProfileModal(); msgEl.style.display = 'none'; }, 1000);
    } else {
      const d = await res.json();
      showToast(d.error || 'Failed to update profile', 'danger');
    }
  } catch (err) {
    showToast('Error updating profile', 'danger');
  }
}

function activatePane(index) {
  state.activePaneIndex = index;
  for (let i = 1; i <= 4; i++) {
    const p = document.getElementById(`pane-${i}`);
    if (p) p.classList.toggle('active', i === index);
  }
  const activeInfo = document.getElementById('active-pane-info');
  if (activeInfo) {
    activeInfo.textContent = `Active Pane: [ ${index} ]`;
  }

  const activePane = state.panes[index];
  const sftpSection = document.getElementById('sftp-explorer-section');
  if (sftpSection) {
    sftpSection.style.display = (activePane && activePane.conn && activePane.conn.protocol === 'ssh') ? 'block' : 'none';
  }
}

// ================= Connection Management =================
async function loadConnections() {
  try {
    const res = await fetch('/api/connections', {
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });
    if (res.ok) {
      state.connections = await res.json();
      renderConnectionsTable();
    }
  } catch (err) {
    console.error('Failed to load connections:', err);
  }
}

function openConnectionsModal() {
  document.getElementById('connections-modal').style.display = 'flex';
  renderConnectionsTable();
}

function closeConnectionsModal() {
  document.getElementById('connections-modal').style.display = 'none';
}

function filterConnectionList(filter) {
  state.connectionFilter = filter;
  ['all', 'global', 'personal'].forEach(f => {
    const btn = document.getElementById(`filter-conn-${f}`);
    if (btn) {
      btn.className = (f === filter) ? 'btn btn-sm active' : 'btn btn-sm btn-secondary';
    }
  });
  renderConnectionsTable();
}

function onSearchConnections(query) {
  state.connectionSearch = (query || '').toLowerCase().trim();
  renderConnectionsTable();
}

function renderConnectionsTable() {
  const tbody = document.getElementById('connections-table-body');
  if (!tbody) return;

  const currentFilter = state.connectionFilter || 'all';
  const query = state.connectionSearch || '';

  const filtered = state.connections.filter(c => {
    if (currentFilter === 'global' && !c.is_global) return false;
    if (currentFilter === 'personal' && c.is_global) return false;
    if (query) {
      const matchName = c.name && c.name.toLowerCase().includes(query);
      const matchHost = c.host && c.host.toLowerCase().includes(query);
      const matchTags = c.tags && c.tags.toLowerCase().includes(query);
      const matchProto = c.protocol && c.protocol.toLowerCase().includes(query);
      if (!matchName && !matchHost && !matchTags && !matchProto) return false;
    }
    return true;
  });

  if (filtered.length === 0) {
    tbody.innerHTML = `<tr><td colspan="6" class="text-center text-muted p-3">No matching connections found.</td></tr>`;
    return;
  }

  tbody.innerHTML = filtered
    .map(c => {
      const scopeBadge = c.is_global
        ? `<span style="color: #f59e0b; background: rgba(245,158,11,0.15); border: 1px solid rgba(245,158,11,0.3); padding: 2px 6px; border-radius: 4px; font-size: 10px; font-weight: 700;">🌐 GLOBAL</span>`
        : `<span style="color: #38bdf8; background: rgba(56,189,248,0.15); border: 1px solid rgba(56,189,248,0.3); padding: 2px 6px; border-radius: 4px; font-size: 10px; font-weight: 700;">🔒 PERSONAL</span>`;

      const modeBadge = c.view_only
        ? `<span style="color: #f59e0b; font-size: 10px; font-weight: 600; margin-right: 6px;" title="View-Only / Observer Mode">👁️ View</span>`
        : `<span style="color: #10b981; font-size: 10px; font-weight: 600; margin-right: 6px;" title="Full Interactive Mode">⚡ Active</span>`;

      const clipBadge = c.allow_clipboard === 'disabled'
        ? `<span style="color: #ef4444; font-size: 10px; margin-right: 6px;" title="Clipboard Blocked">🚫 Clip</span>`
        : c.allow_clipboard === 'host_to_remote'
        ? `<span style="color: #38bdf8; font-size: 10px; margin-right: 6px;" title="Paste Only">📥 Paste</span>`
        : `<span style="color: #a1a1aa; font-size: 10px; margin-right: 6px;" title="Bidirectional Clipboard">📋 Clip</span>`;

      const transBadge = c.allow_transfer === 'disabled'
        ? `<span style="color: #ef4444; font-size: 10px;" title="Transfers Blocked">🚫 Files</span>`
        : `<span style="color: #a1a1aa; font-size: 10px;" title="File Transfers Allowed">📁 Files</span>`;

      return `
        <tr>
          <td>${scopeBadge}</td>
          <td>
            <strong>${escapeHtml(c.name)}</strong>
            ${c.tags ? `<span class="badge-count" style="margin-left: 6px;">${escapeHtml(c.tags)}</span>` : ''}
            ${connectionAccessLine(c)}
          </td>
          <td><span class="pane-protocol-badge">${c.protocol.toUpperCase()}</span></td>
          <td><code>${c.protocol === 'local_pty' ? 'Local System' : `${escapeHtml(c.host)}:${c.port}`}</code></td>
          <td>
            <div style="display: flex; align-items: center;">
              ${modeBadge}
              ${clipBadge}
              ${transBadge}
            </div>
          </td>
          <td>
            <div style="display:flex;gap:4px;">
              <button class="btn btn-primary btn-sm" onclick="connectToTarget('${c.id}')">Connect [ ${state.activePaneIndex} ]</button>
              ${c.user_permissions && c.user_permissions.can_edit ? `
                <button class="btn btn-secondary btn-sm" onclick="openEditConnectionModal('${c.id}')">Edit</button>
                <button class="btn btn-secondary btn-sm text-danger" onclick="deleteConnection('${c.id}')">Delete</button>
              ` : ''}
            </div>
          </td>
        </tr>
      `;
    })
    .join('');
}

function openAddConnectionModal() {
  document.getElementById('connection-edit-title').textContent = 'New Connection Target';
  document.getElementById('conn-id').value = '';
  document.getElementById('conn-name').value = '';
  document.getElementById('conn-protocol').value = 'ssh';
  document.getElementById('conn-host').value = '';
  document.getElementById('conn-port').value = '22';
  document.getElementById('conn-username').value = '';
  document.getElementById('conn-password').value = '';
  document.getElementById('conn-private-key').value = '';
  const certEl = document.getElementById('conn-ssh-certificate');
  if (certEl) certEl.value = '';
  document.getElementById('conn-tags').value = '';

  const isAdm = state.currentUser && state.currentUser.role === 'admin';
  const scopeEl = document.getElementById('conn-is-global');
  if (scopeEl) {
    scopeEl.value = isAdm ? 'true' : 'false';
  }
  const scopeGroup = document.getElementById('group-conn-scope');
  if (scopeGroup) {
    scopeGroup.style.display = isAdm ? 'block' : 'none';
  }
  prepareAssignments(null);

  document.getElementById('conn-allow-clipboard').value = 'bidirectional';
  document.getElementById('conn-allow-transfer').value = 'full';
  document.getElementById('conn-view-only').value = 'false';

  const ignoreCertEl = document.getElementById('conn-rdp-ignore-cert');
  if (ignoreCertEl) ignoreCertEl.value = 'true';
  const domainEl = document.getElementById('conn-rdp-domain');
  if (domainEl) domainEl.value = '';
  const kbdEl = document.getElementById('conn-rdp-keyboard-layout');
  if (kbdEl) kbdEl.value = localStorage.getItem('rd_default_kbd_layout') || 'no';

  const perfPresetEl = document.getElementById('conn-rdp-perf-preset');
  if (perfPresetEl) perfPresetEl.value = 'high_speed';
  const colorDepthEl = document.getElementById('conn-rdp-color-depth');
  if (colorDepthEl) colorDepthEl.value = '32';
  onRdpPresetChanged();

  onProtocolChanged();
  document.getElementById('connection-edit-modal').style.display = 'flex';
}

function openEditConnectionModal(id) {
  const c = state.connections.find(item => item.id === id);
  if (!c) return;

  document.getElementById('connection-edit-title').textContent = `Edit Target — ${c.name}`;
  document.getElementById('conn-id').value = c.id;
  document.getElementById('conn-name').value = c.name;
  document.getElementById('conn-protocol').value = c.protocol;
  document.getElementById('conn-host').value = c.host;
  document.getElementById('conn-port').value = c.port;
  document.getElementById('conn-username').value = c.username || '';
  document.getElementById('conn-password').value = '';
  document.getElementById('conn-private-key').value = '';
  document.getElementById('conn-tags').value = c.tags || '';

  let settings = {};
  try {
    if (c.settings_json) settings = JSON.parse(c.settings_json);
  } catch (err) {}

  const editCertEl = document.getElementById('conn-ssh-certificate');
  if (editCertEl) editCertEl.value = settings.ssh_certificate || settings.certificate || '';

  const isAdm = state.currentUser && state.currentUser.role === 'admin';
  const scopeEl = document.getElementById('conn-is-global');
  if (scopeEl) {
    scopeEl.value = c.is_global ? 'true' : 'false';
  }
  prepareAssignments(c);
  const scopeGroup = document.getElementById('group-conn-scope');
  if (scopeGroup) {
    scopeGroup.style.display = isAdm ? 'block' : 'none';
  }

  document.getElementById('conn-allow-clipboard').value = c.allow_clipboard || 'bidirectional';
  document.getElementById('conn-allow-transfer').value = c.allow_transfer || 'full';
  document.getElementById('conn-view-only').value = c.view_only ? 'true' : 'false';

  const ignoreCertEl = document.getElementById('conn-rdp-ignore-cert');
  if (ignoreCertEl) ignoreCertEl.value = (settings.ignore_cert !== false) ? 'true' : 'false';
  const domainEl = document.getElementById('conn-rdp-domain');
  if (domainEl) domainEl.value = settings.domain || '';
  const editKbdEl = document.getElementById('conn-rdp-keyboard-layout');
  if (editKbdEl) editKbdEl.value = settings.keyboard_layout || 'no';

  const perfPresetEl = document.getElementById('conn-rdp-perf-preset');
  if (perfPresetEl) perfPresetEl.value = settings.perf_preset || 'high_speed';
  const colorDepthEl = document.getElementById('conn-rdp-color-depth');
  if (colorDepthEl) colorDepthEl.value = settings.color_depth ? String(settings.color_depth) : '32';

  const disableWallpaper = document.getElementById('conn-rdp-disable-wallpaper');
  if (disableWallpaper) disableWallpaper.checked = settings.disable_wallpaper !== false;
  const disableWindowDrag = document.getElementById('conn-rdp-disable-window-drag');
  if (disableWindowDrag) disableWindowDrag.checked = settings.disable_window_drag !== false;
  const disableMenuAnim = document.getElementById('conn-rdp-disable-menu-anim');
  if (disableMenuAnim) disableMenuAnim.checked = settings.disable_menu_anim !== false;
  const disableThemes = document.getElementById('conn-rdp-disable-themes');
  if (disableThemes) disableThemes.checked = !!settings.disable_themes;
  const fontSmoothing = document.getElementById('conn-rdp-font-smoothing');
  if (fontSmoothing) fontSmoothing.checked = settings.font_smoothing !== false;
  const driveRedir = document.getElementById('conn-rdp-drive-redirection');
  if (driveRedir) driveRedir.checked = !!settings.enable_drive_redirection;
  const audio = document.getElementById('conn-rdp-audio');
  if (audio) audio.checked = !!settings.enable_audio;

  onProtocolChanged();
  document.getElementById('connection-edit-modal').style.display = 'flex';
}

function closeConnectionEditModal() {
  document.getElementById('connection-edit-modal').style.display = 'none';
}

function onRdpPresetChanged() {
  const presetEl = document.getElementById('conn-rdp-perf-preset');
  if (!presetEl) return;
  const preset = presetEl.value;
  const disableWallpaper = document.getElementById('conn-rdp-disable-wallpaper');
  const disableWindowDrag = document.getElementById('conn-rdp-disable-window-drag');
  const disableMenuAnim = document.getElementById('conn-rdp-disable-menu-anim');
  const disableThemes = document.getElementById('conn-rdp-disable-themes');
  const fontSmoothing = document.getElementById('conn-rdp-font-smoothing');
  const audio = document.getElementById('conn-rdp-audio');

  if (preset === 'high_speed') {
    if (disableWallpaper) disableWallpaper.checked = true;
    if (disableWindowDrag) disableWindowDrag.checked = true;
    if (disableMenuAnim) disableMenuAnim.checked = true;
    if (disableThemes) disableThemes.checked = true;
    if (fontSmoothing) fontSmoothing.checked = false;
    if (audio) audio.checked = false;
  } else if (preset === 'balanced') {
    if (disableWallpaper) disableWallpaper.checked = true;
    if (disableWindowDrag) disableWindowDrag.checked = true;
    if (disableMenuAnim) disableMenuAnim.checked = true;
    if (disableThemes) disableThemes.checked = false;
    if (fontSmoothing) fontSmoothing.checked = true;
    if (audio) audio.checked = false;
  } else if (preset === 'high_quality') {
    if (disableWallpaper) disableWallpaper.checked = false;
    if (disableWindowDrag) disableWindowDrag.checked = false;
    if (disableMenuAnim) disableMenuAnim.checked = false;
    if (disableThemes) disableThemes.checked = false;
    if (fontSmoothing) fontSmoothing.checked = true;
    if (audio) audio.checked = true;
  }
}

function onProtocolChanged() {
  const proto = document.getElementById('conn-protocol').value;
  const isLocal = proto === 'local_pty';
  const isSsh = proto === 'ssh';
  const isTelnet = proto === 'telnet';
  const isRdp = proto === 'rdp';

  document.getElementById('group-host').style.display = isLocal ? 'none' : 'block';
  document.getElementById('group-port').style.display = isLocal ? 'none' : 'block';
  document.getElementById('group-auth-user').style.display = isLocal ? 'none' : 'grid';
  document.getElementById('group-ssh-key').style.display = isSsh ? 'block' : 'none';
  const sshCertGroup = document.getElementById('group-ssh-cert');
  if (sshCertGroup) sshCertGroup.style.display = isSsh ? 'block' : 'none';

  const rdpGroup = document.getElementById('group-rdp-settings');
  if (rdpGroup) rdpGroup.style.display = isRdp ? 'flex' : 'none';

  if (proto === 'vnc') document.getElementById('conn-port').value = '5900';
  if (proto === 'rdp') document.getElementById('conn-port').value = '3389';
  if (proto === 'ssh') document.getElementById('conn-port').value = '22';
  if (proto === 'telnet') document.getElementById('conn-port').value = '23';
}

async function handleSaveConnection(e) {
  e.preventDefault();
  const id = document.getElementById('conn-id').value;
  const isGlobalVal = document.getElementById('conn-is-global') ? document.getElementById('conn-is-global').value === 'true' : true;

  const connSettings = {
    ssh_certificate: document.getElementById('conn-ssh-certificate') ? document.getElementById('conn-ssh-certificate').value.trim() || null : null,
    ignore_cert: document.getElementById('conn-rdp-ignore-cert') ? document.getElementById('conn-rdp-ignore-cert').value === 'true' : true,
    domain: document.getElementById('conn-rdp-domain') ? document.getElementById('conn-rdp-domain').value.trim() || null : null,
    keyboard_layout: document.getElementById('conn-rdp-keyboard-layout') ? document.getElementById('conn-rdp-keyboard-layout').value : 'no',
    perf_preset: document.getElementById('conn-rdp-perf-preset') ? document.getElementById('conn-rdp-perf-preset').value : 'high_speed',
    color_depth: document.getElementById('conn-rdp-color-depth') ? parseInt(document.getElementById('conn-rdp-color-depth').value, 10) : 32,
    disable_wallpaper: document.getElementById('conn-rdp-disable-wallpaper') ? document.getElementById('conn-rdp-disable-wallpaper').checked : true,
    disable_window_drag: document.getElementById('conn-rdp-disable-window-drag') ? document.getElementById('conn-rdp-disable-window-drag').checked : true,
    disable_menu_anim: document.getElementById('conn-rdp-disable-menu-anim') ? document.getElementById('conn-rdp-disable-menu-anim').checked : true,
    disable_themes: document.getElementById('conn-rdp-disable-themes') ? document.getElementById('conn-rdp-disable-themes').checked : false,
    font_smoothing: document.getElementById('conn-rdp-font-smoothing') ? document.getElementById('conn-rdp-font-smoothing').checked : true,
    enable_drive_redirection: document.getElementById('conn-rdp-drive-redirection') ? document.getElementById('conn-rdp-drive-redirection').checked : true,
    enable_audio: document.getElementById('conn-rdp-audio') ? document.getElementById('conn-rdp-audio').checked : false,
  };

  const payload = {
    name: document.getElementById('conn-name').value.trim(),
    protocol: document.getElementById('conn-protocol').value,
    host: document.getElementById('conn-host').value.trim() || 'localhost',
    port: parseInt(document.getElementById('conn-port').value, 10) || 22,
    username: document.getElementById('conn-username').value.trim() || null,
    password: document.getElementById('conn-password').value || null,
    private_key: document.getElementById('conn-private-key').value || null,
    tags: document.getElementById('conn-tags').value.trim() || null,
    settings_json: JSON.stringify(connSettings),
    is_global: isGlobalVal,
    allow_clipboard: document.getElementById('conn-allow-clipboard').value,
    allow_transfer: document.getElementById('conn-allow-transfer').value,
    view_only: document.getElementById('conn-view-only').value === 'true',
  };
  const isAdminUser = state.currentUser && state.currentUser.role === 'admin';
  if (isAdminUser) {
    const owner = document.getElementById('conn-owner').value;
    if (owner) payload.owner_id = owner;
  }

  const url = id ? `/api/connections/${id}` : '/api/connections';
  const method = id ? 'PUT' : 'POST';

  try {
    const res = await fetch(url, {
      method,
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${state.jwtToken}`,
      },
      body: JSON.stringify(payload),
    });

    if (res.ok) {
      const saved = await res.json().catch(() => ({}));
      const connId = id || saved.id;
      if (isAdminUser && connId) {
        const ok = await saveAssignments(connId);
        if (!ok) return;
      }
      showToast(id ? 'Connection updated!' : 'Connection created!', 'success');
      closeConnectionEditModal();
      await loadConnections();
    } else {
      const err = await res.json();
      showToast(`Error saving connection: ${err.error}`, 'danger');
    }
  } catch (err) {
    showToast(`Failed to save connection: ${err}`, 'danger');
  }
}

async function deleteConnection(id) {
  if (!(await uiConfirm('Are you sure you want to delete this connection?', { title: 'Delete connection', okText: 'Delete', danger: true }))) return;
  try {
    const res = await fetch(`/api/connections/${id}`, {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });
    if (res.ok) {
      showToast('Connection deleted');
      await loadConnections();
    }
  } catch (err) {
    showToast(`Failed to delete connection: ${err}`, 'danger');
  }
}

// ================= Remote Gateway WebSocket Tunnel & Protocol Handlers =================
function connectToTarget(connectionId) {
  const conn = state.connections.find(c => c.id === connectionId);
  if (!conn) return;

  closeConnectionsModal();
  const paneIndex = state.activePaneIndex;
  const pane = state.panes[paneIndex];

  disconnectPane(paneIndex);

  const titleEl = document.getElementById(`pane-${paneIndex}-title`);
  const protoBadge = document.getElementById(`pane-${paneIndex}-proto`);
  const bodyEl = document.getElementById(`pane-${paneIndex}-body`);
  const statsEl = document.getElementById(`pane-${paneIndex}-stats`);

  const viewOnlyTag = conn.view_only
    ? ` <span style="font-size: 9px; padding: 2px 5px; background: rgba(245,158,11,0.2); border: 1px solid rgba(245,158,11,0.4); color: var(--woofson-accent); border-radius: 3px; font-weight: 700; margin-left: 6px;">VIEW-ONLY</span>`
    : '';

  titleEl.innerHTML = `${escapeHtml(conn.name)}${viewOnlyTag}`;
  protoBadge.textContent = conn.protocol.toUpperCase();
  protoBadge.style.display = 'inline-block';
  statsEl.style.display = 'inline-block';
  statsEl.textContent = 'Connecting...';

  bodyEl.innerHTML = '';
  pane.conn = conn;
  pane.type = conn.protocol;
  pane.lastError = null;

  const rect = bodyEl.getBoundingClientRect();
  const initW = Math.min(3840, Math.max(640, Math.round(rect.width) || 1920));
  const initH = Math.min(2160, Math.max(480, Math.round(rect.height) || 1080));
  const cols = Math.max(20, Math.floor(initW / 9)) || 120;
  const rows = Math.max(5, Math.floor(initH / 18)) || 32;

  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const wsUrl = `${protocol}//${location.host}/ws/tunnel/${conn.id}?token=${encodeURIComponent(state.jwtToken)}&width=${initW}&height=${initH}&cols=${cols}&rows=${rows}`;

  const ws = new WebSocket(wsUrl);
  ws.binaryType = 'arraybuffer';
  pane.socket = ws;

  const startTime = Date.now();

  ws.onopen = () => {
    const latency = Date.now() - startTime;
    statsEl.textContent = `${latency} ms`;
    showToast(`Connected to ${conn.name} [Pane ${paneIndex}]`);

    if (conn.protocol === 'ssh' && conn.allow_transfer !== 'disabled' && !(conn.user_permissions && !conn.user_permissions.can_transfer)) {
      if (conn.has_password || conn.has_private_key) {
        loadSftpDirectory(paneIndex, '.');
      } else {
        showSftpNote('File browsing needs a saved password or key for this connection.');
      }
    }
  };

  ws.onerror = (e) => {
    statsEl.textContent = 'Error';
    showToast(`Connection error on ${conn.name}`, 'danger');
  };

  ws.onclose = () => {
    if (pane.authPending) {
      pane.authPending = false;
      closeUiDialog(null);
    }
    statsEl.textContent = 'Disconnected';
    statsEl.style.color = 'var(--woofson-danger, #ef4444)';
    showToast(`Disconnected from ${conn.name}`);

    // Free canvas/terminal resources and clean up pane state
    if (pane.resizeObserver) {
      pane.resizeObserver.disconnect();
      pane.resizeObserver = null;
    }
    if (pane.terminal) {
      try {
        pane.terminal.dispose();
      } catch (e) {}
      pane.terminal = null;
      pane.fitAddon = null;
    }
    if (pane.canvas) {
      pane.canvas = null;
      pane.ctx = null;
    }
    pane.socket = null;

    const errBanner = pane.lastError
      ? `<div style="max-width: 480px; margin-top: 6px; padding: 8px 12px; background: rgba(239, 68, 68, 0.12); border: 1px solid rgba(239, 68, 68, 0.35); border-radius: 6px; color: #fca5a5; font-size: 11px; line-height: 1.4; font-family: var(--font-mono, monospace); word-break: break-word;">${escapeHtml(pane.lastError)}</div>`
      : '';

    // Display a clean Reconnect banner
    bodyEl.innerHTML = `
      <div class="pane-empty-state" style="padding: 24px; text-align: center; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; height: 100%;">
        <div class="empty-icon" style="opacity: 0.8;">
          <img src="assets/Remoteiconsmall.png" alt="Remote" class="empty-brand-img" style="filter: grayscale(30%); width: 44px; height: 44px;" />
        </div>
        <div style="display: flex; flex-direction: column; align-items: center;">
          <h3 style="margin: 0 0 4px 0; font-size: 14px; font-weight: 700; color: var(--woofson-text);">Session Disconnected</h3>
          <p style="margin: 0; font-size: 11px; color: var(--woofson-text-muted);">Disconnected from <strong style="color: var(--woofson-accent);">${escapeHtml(conn.name)}</strong> (${escapeHtml(conn.host)}:${conn.port})</p>
          ${errBanner}
        </div>
        <div style="display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; margin-top: 4px;">
          <button class="btn btn-primary" onclick="connectToTarget('${conn.id}')" style="display: flex; align-items: center; gap: 6px; font-weight: 600; padding: 6px 14px; font-size: 12px;">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/></svg>
            Reconnect to ${escapeHtml(conn.name)}
          </button>
          <button class="btn btn-secondary" onclick="openConnectionsModal()" style="display: flex; align-items: center; gap: 6px; padding: 6px 12px; font-size: 12px;">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7"></rect><rect x="14" y="3" width="7" height="7"></rect><rect x="14" y="14" width="7" height="7"></rect><rect x="3" y="14" width="7" height="7"></rect></svg>
            Switch Target
          </button>
          <button class="btn btn-secondary" onclick="disconnectPane(${paneIndex})" style="display: flex; align-items: center; gap: 6px; padding: 6px 10px; font-size: 12px; opacity: 0.8;">
            ✕ Clear
          </button>
        </div>
      </div>
    `;
  };

  // Branch by Protocol
  if (conn.protocol === 'vnc' || conn.protocol === 'rdp') {
    setupGraphicsProtocol(pane, ws, bodyEl);
  } else {
    setupTerminalProtocol(pane, ws, bodyEl);
  }
}

function setupTerminalProtocol(pane, ws, bodyEl) {
  const container = document.createElement('div');
  container.className = 'pane-terminal-container';
  bodyEl.appendChild(container);

  // Initialize xterm.js
  const term = new Terminal({
    cursorBlink: true,
    cursorStyle: 'block',
    fontSize: 13,
    fontFamily: '"JetBrainsMono Nerd Font", "JetBrains Mono", "Symbols Nerd Font Mono", "Fira Code", monospace',
    lineHeight: 1.2,
    letterSpacing: 0,
    allowProposedApi: true,
    scrollback: 10000,
    theme: TERMINAL_THEME,
    convertEol: true,
  });

  // Fit Addon
  const FitAddonClass = (window.FitAddon && window.FitAddon.FitAddon) || window.FitAddon;
  let fitAddon = null;
  if (FitAddonClass) {
    fitAddon = new FitAddonClass();
    term.loadAddon(fitAddon);
  }

  // Web Links Addon
  const WebLinksAddonClass = (window.WebLinksAddon && window.WebLinksAddon.WebLinksAddon) || window.WebLinksAddon;
  if (WebLinksAddonClass) {
    term.loadAddon(new WebLinksAddonClass());
  }

  term.open(container);
  pane.terminal = term;
  pane.fitAddon = fitAddon;

  // Fit initial size
  if (fitAddon) {
    try {
      fitAddon.fit();
    } catch (e) {}
  }

  // Send window resize event to WebSocket
  function sendTerminalResize(cols, rows) {
    if (ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({
      type: 'resize',
      cols: cols,
      rows: rows,
    }));
  }

  // Handle incoming data from WebSocket
  ws.onmessage = (e) => {
    if (typeof e.data === 'string') {
      try {
        const msg = JSON.parse(e.data);
        if (msg.type === 'error') {
          pane.lastError = msg.message;
          term.write(`\r\n\x1b[31m[Remote Error]: ${msg.message}\x1b[0m\r\n`);
          return;
        } else if (msg.type === 'auth_required') {
          askSessionPassword(pane, ws, msg);
          return;
        } else if (msg.type === 'clipboard_sync') {
          if (!pane.conn || (pane.conn.allow_clipboard !== 'disabled' && pane.conn.allow_clipboard !== 'host_to_remote')) {
            onRemoteClipboardSync(msg.text);
          }
          return;
        }
      } catch (err) {
        // Plain text data
      }
      term.write(e.data);
    } else if (e.data instanceof ArrayBuffer) {
      term.write(new Uint8Array(e.data));
    } else if (e.data instanceof Blob) {
      e.data.arrayBuffer().then(buf => term.write(new Uint8Array(buf)));
    }
  };

  // Clipboard paste into terminal
  container.addEventListener('paste', (e) => {
    e.preventDefault();
    if (pane.conn && (pane.conn.view_only || pane.conn.allow_clipboard === 'disabled' || pane.conn.allow_clipboard === 'remote_to_host')) {
      showToast('Clipboard paste blocked by connection policy', 'warning');
      return;
    }
    const text = (e.clipboardData || window.clipboardData)?.getData('text');
    if (text && ws.readyState === WebSocket.OPEN) {
      ws.send(text);
    }
  });

  // On user input in terminal -> send to WebSocket
  term.onData((data) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    if (pane.conn && pane.conn.view_only) return;
    ws.send(data);
  });

  term.onResize(({ cols, rows }) => {
    sendTerminalResize(cols, rows);
  });

  term.onSelectionChange(() => {
    const sel = term.getSelection();
    if (sel && sel.trim().length > 0) {
      const autoClip = document.getElementById('chk-auto-clip')?.checked;
      const bufEl = document.getElementById('clipboard-buffer');
      if (bufEl) bufEl.value = sel;
      if (autoClip && navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(sel).catch(() => {});
      }
    }
  });

  // ResizeObserver for dynamic terminal resizing
  let resizeDebounceTimer = null;
  const resizeObserver = new ResizeObserver((entries) => {
    clearTimeout(resizeDebounceTimer);
    resizeDebounceTimer = setTimeout(() => {
      if (fitAddon && term.element && term.element.parentElement) {
        if (entries && entries.length > 0) {
          const cr = entries[0].contentRect;
          if (cr.width < 50 || cr.height < 50) return;
        }
        try {
          fitAddon.fit();
          sendTerminalResize(term.cols, term.rows);
        } catch (e) {}
      }
    }, 60);
  });

  resizeObserver.observe(bodyEl);
  pane.resizeObserver = resizeObserver;

  // Initial resize notification and focus after render
  setTimeout(() => {
    if (fitAddon) {
      try {
        fitAddon.fit();
        sendTerminalResize(term.cols, term.rows);
      } catch (e) {}
    }
    term.focus();
  }, 100);
}

function setupGraphicsProtocol(pane, ws, bodyEl) {
  const canvas = document.createElement('canvas');
  canvas.className = 'pane-canvas';
  canvas.width = 1280;
  canvas.height = 720;
  canvas.tabIndex = 0;
  bodyEl.appendChild(canvas);

  const ctx = canvas.getContext('2d');
  pane.canvas = canvas;
  pane.ctx = ctx;

  // Dynamic Resolution Engine: Automatically adjust remote desktop resolution when panel resizes
  let resizeDebounceTimer = null;
  const resizeObserver = new ResizeObserver((entries) => {
    if (ws.readyState !== WebSocket.OPEN) return;
    if (pane.conn && pane.conn.protocol !== 'rdp' && pane.conn.protocol !== 'vnc') return;

    for (const entry of entries) {
      const cr = entry.contentRect;
      // Guard against collapsed / background panes (less than 50px)
      if (cr.width < 50 || cr.height < 50) continue;

      const newW = Math.min(3840, Math.max(640, Math.round(cr.width)));
      const newH = Math.min(2160, Math.max(480, Math.round(cr.height)));

      // If current canvas size differs by more than 32px, request dynamic resolution change
      if (Math.abs(canvas.width - newW) > 32 || Math.abs(canvas.height - newH) > 32) {
        clearTimeout(resizeDebounceTimer);
        resizeDebounceTimer = setTimeout(() => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({
              type: 'resize',
              width: newW,
              height: newH,
            }));
          }
        }, 500); // 500ms debounce
      }
    }
  });
  resizeObserver.observe(bodyEl);
  pane.resizeObserver = resizeObserver;

  // Background black fill
  ctx.fillStyle = '#000000';
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  let mouseMask = 0;

  ws.onmessage = (e) => {
    if (typeof e.data === 'string') {
      try {
        const msg = JSON.parse(e.data);
        if (msg.type === 'init') {
          canvas.width = msg.width;
          canvas.height = msg.height;
          ctx.fillStyle = '#000000';
          ctx.fillRect(0, 0, canvas.width, canvas.height);
        } else if (msg.type === 'clipboard_sync') {
          if (!pane.conn || pane.conn.allow_clipboard !== 'disabled' && pane.conn.allow_clipboard !== 'host_to_remote') {
            onRemoteClipboardSync(msg.text);
          }
        } else if (msg.type === 'error') {
          pane.lastError = msg.message;
          showToast(msg.message, 'danger');
        }
      } catch (err) {}
    } else {
      // Binary Tile Packet: [type: 0x01, x: u16, y: u16, w: u16, h: u16, rgba...]
      const dv = new DataView(e.data);
      const type = dv.getUint8(0);

      if (type === 0x01) {
        // Frame Tile: [0x01, x: u16, y: u16, w: u16, h: u16, rgba...]
        const x = dv.getUint16(1, false);
        const y = dv.getUint16(3, false);
        const w = dv.getUint16(5, false);
        const h = dv.getUint16(7, false);

        const pixelBytes = new Uint8ClampedArray(e.data, 9, w * h * 4);
        const imgData = new ImageData(pixelBytes, w, h);
        ctx.putImageData(imgData, x, y);
      } else if (type === 0x03) {
        // Atomic Batch Tiles: [0x03, tile_count: u16, (x: u16, y: u16, w: u16, h: u16, rgba...)*count]
        const tileCount = dv.getUint16(1, false);
        let offset = 3;
        for (let i = 0; i < tileCount; i++) {
          if (offset + 8 > e.data.byteLength) break;
          const x = dv.getUint16(offset, false);
          const y = dv.getUint16(offset + 2, false);
          const w = dv.getUint16(offset + 4, false);
          const h = dv.getUint16(offset + 6, false);
          offset += 8;

          const byteLen = w * h * 4;
          if (offset + byteLen > e.data.byteLength) break;
          const pixelBytes = new Uint8ClampedArray(e.data, offset, byteLen);
          const imgData = new ImageData(pixelBytes, w, h);
          ctx.putImageData(imgData, x, y);
          offset += byteLen;
        }
      } else if (type === 0x02) {
        // CopyRect
        const x = dv.getUint16(1, false);
        const y = dv.getUint16(3, false);
        const w = dv.getUint16(5, false);
        const h = dv.getUint16(7, false);
        const srcX = dv.getUint16(9, false);
        const srcY = dv.getUint16(11, false);

        ctx.drawImage(canvas, srcX, srcY, w, h, x, y, w, h);
      }
    }
  };

  // Mouse Inputs
  let lastPointerPos = { x: -1, y: -1, mask: -1 };
  let pointerThrottleTimer = null;

  function sendPointer(mask, x, y, force = false) {
    if (ws.readyState !== WebSocket.OPEN) return;
    if (pane.conn && pane.conn.view_only) return;
    const clampedX = Math.max(0, Math.min(canvas.width, x));
    const clampedY = Math.max(0, Math.min(canvas.height, y));
    if (!force && lastPointerPos.x === clampedX && lastPointerPos.y === clampedY && lastPointerPos.mask === mask) return;
    lastPointerPos = { x: clampedX, y: clampedY, mask };

    const buf = new ArrayBuffer(6);
    const dv = new DataView(buf);
    dv.setUint8(0, 0x02); // POINTER_EVENT
    dv.setUint8(1, mask);
    dv.setUint16(2, clampedX, false);
    dv.setUint16(4, clampedY, false);
    ws.send(buf);
  }

  function getCanvasCoords(evt) {
    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const scaleY = canvas.height / rect.height;
    return {
      x: Math.round((evt.clientX - rect.left) * scaleX),
      y: Math.round((evt.clientY - rect.top) * scaleY),
    };
  }

  let pendingPointer = null;
  let pointerRaf = null;

  canvas.addEventListener('mousemove', (e) => {
    const pos = getCanvasCoords(e);
    pendingPointer = pos;
    if (!pointerRaf) {
      pointerRaf = requestAnimationFrame(() => {
        if (pendingPointer) {
          sendPointer(mouseMask, pendingPointer.x, pendingPointer.y);
        }
        pointerRaf = null;
      });
    }
  });

  canvas.addEventListener('mousedown', (e) => {
    canvas.focus();
    if (e.button === 0) mouseMask |= 1; // Left
    if (e.button === 1) mouseMask |= 2; // Middle
    if (e.button === 2) mouseMask |= 4; // Right
    const pos = getCanvasCoords(e);
    sendPointer(mouseMask, pos.x, pos.y, true);
    e.preventDefault();
  });

  canvas.addEventListener('mouseup', (e) => {
    if (e.button === 0) mouseMask &= ~1;
    if (e.button === 1) mouseMask &= ~2;
    if (e.button === 2) mouseMask &= ~4;
    const pos = getCanvasCoords(e);
    sendPointer(mouseMask, pos.x, pos.y, true);
    e.preventDefault();
  });

  canvas.addEventListener('contextmenu', (e) => e.preventDefault());

  canvas.addEventListener('wheel', (e) => {
    const pos = getCanvasCoords(e);
    const wheelMask = e.deltaY < 0 ? (mouseMask | 8) : (mouseMask | 16);
    sendPointer(wheelMask, pos.x, pos.y, true);
    sendPointer(mouseMask, pos.x, pos.y, true); // release wheel
    e.preventDefault();
  }, { passive: false });

  // Key Inputs: Direct PS/2 Hardware Scancodes (with E0 extended flag) + Unicode fallback
  function sendKey(down, e) {
    if (ws.readyState !== WebSocket.OPEN) return;
    if (pane.conn && pane.conn.view_only) return;

    let scancode = 0;
    let extended = 0;
    if (e.code && CODE_TO_SCANCODE[e.code]) {
      const [sc, ext] = CODE_TO_SCANCODE[e.code];
      scancode = sc;
      extended = ext ? 1 : 0;
    }

    const unicode = (e.key && e.key.length === 1) ? e.key.charCodeAt(0) : 0;

    const buf = new ArrayBuffer(6);
    const dv = new DataView(buf);
    dv.setUint8(0, 0x04); // KEY_EVENT
    dv.setUint8(1, down ? 1 : 0);
    dv.setUint8(2, scancode);
    dv.setUint8(3, extended);
    dv.setUint16(4, unicode, false);
    ws.send(buf);
  }

  let lastPasteTimestamp = 0;
  function pushPastedText(text) {
    if (!text) return;
    const now = Date.now();
    if (now - lastPasteTimestamp < 350) return; // Deduplicate paste events within 350ms
    lastPasteTimestamp = now;
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: 'clipboard_push',
        text: text,
      }));
      showToast(`📋 Pasted into remote session (${text.length} chars)`);
    }
  }

  canvas.addEventListener('keydown', (e) => {
    // If user presses Ctrl+V / Cmd+V (Paste from local device to remote session)
    if ((e.ctrlKey || e.metaKey) && (e.key === 'v' || e.key === 'V' || e.code === 'KeyV')) {
      e.preventDefault();
      e.stopPropagation();
      if (pane.conn && (pane.conn.view_only || pane.conn.allow_clipboard === 'disabled' || pane.conn.allow_clipboard === 'remote_to_host')) {
        showToast('Clipboard paste blocked by connection policy', 'warning');
        return;
      }
      if (navigator.clipboard && navigator.clipboard.readText) {
        navigator.clipboard.readText().then(text => {
          pushPastedText(text);
        }).catch(() => {
          // If readText() is blocked on HTTP, let native paste event fire below
        });
      }
      return;
    }

    sendKey(true, e);
    // Prevent default browser shortcuts except developer tools
    if (e.key !== 'F12' && !(e.ctrlKey && e.shiftKey && e.key === 'I')) {
      e.preventDefault();
    }
  });

  canvas.addEventListener('keyup', (e) => {
    if ((e.ctrlKey || e.metaKey) && (e.key === 'v' || e.key === 'V' || e.code === 'KeyV')) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    sendKey(false, e);
    if (e.key !== 'F12' && !(e.ctrlKey && e.shiftKey && e.key === 'I')) {
      e.preventDefault();
    }
  });

  // Native paste event on canvas: captures Ctrl+V and forwards to remote session
  function handleCanvasPaste(e) {
    e.preventDefault();
    e.stopPropagation();
    if (pane.conn && (pane.conn.view_only || pane.conn.allow_clipboard === 'disabled' || pane.conn.allow_clipboard === 'remote_to_host')) {
      showToast('Clipboard paste blocked by connection policy', 'warning');
      return;
    }
    const text = (e.clipboardData || window.clipboardData)?.getData('text');
    if (text) {
      pushPastedText(text);
    }
  }

  canvas.addEventListener('paste', handleCanvasPaste);

  canvas.focus();
}

function disconnectPane(paneIndex) {
  const pane = state.panes[paneIndex];
  const prevConn = pane.conn;
  if (pane.resizeObserver) {
    pane.resizeObserver.disconnect();
    pane.resizeObserver = null;
  }
  if (pane.terminal) {
    try {
      pane.terminal.dispose();
    } catch (e) {}
    pane.terminal = null;
    pane.fitAddon = null;
  }
  if (pane.canvas) {
    pane.canvas = null;
    pane.ctx = null;
  }
  if (pane.socket) {
    pane.socket.onclose = null;
    pane.socket.close();
    pane.socket = null;
  }
  pane.conn = null;

  document.getElementById(`pane-${paneIndex}-title`).textContent = 'Ready for Session';
  document.getElementById(`pane-${paneIndex}-proto`).style.display = 'none';
  document.getElementById(`pane-${paneIndex}-stats`).style.display = 'none';

  const reconnectBtn = prevConn ? `
    <button class="btn btn-secondary" onclick="connectToTarget('${prevConn.id}')" style="display: flex; align-items: center; gap: 6px; font-size: 11px; margin-top: 4px; padding: 5px 10px;">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/></svg>
      Reconnect ${escapeHtml(prevConn.name)}
    </button>
  ` : '';

  document.getElementById(`pane-${paneIndex}-body`).innerHTML = `
    <div class="pane-empty-state">
      <div class="empty-icon"><img src="assets/Remoteiconsmall.png" alt="Remote" class="empty-brand-img" /></div>
      <h3>Ready for Session</h3>
      <div style="display: flex; flex-direction: column; align-items: center; gap: 8px;">
        <button class="btn btn-primary" onclick="openConnectionsModal()">Connect Target</button>
        ${reconnectBtn}
      </div>
    </div>
  `;
}

// ================= Bi-Directional Clipboard Engine =================
function toggleClipboardDrawer() {
  const drawer = document.getElementById('clipboard-drawer');
  drawer.style.display = drawer.style.display === 'none' ? 'flex' : 'none';
}

function onRemoteClipboardSync(text) {
  const bufEl = document.getElementById('clipboard-buffer');
  if (bufEl) bufEl.value = text;
  const autoClip = document.getElementById('chk-auto-clip')?.checked;

  if (autoClip && navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(() => {
      showToast('📋 Remote clipboard copied to your device!');
    }).catch(() => {
      showToast('📋 Remote clipboard received in Live Drawer');
    });
  } else {
    showToast('📋 Remote clipboard updated in Live Drawer');
  }
}

function pushClipboardToRemote() {
  const text = document.getElementById('clipboard-buffer').value;
  const pane = state.panes[state.activePaneIndex];

  if (!pane || !pane.socket || pane.socket.readyState !== WebSocket.OPEN) {
    showToast('No active remote connection on this pane', 'danger');
    return;
  }

  pane.socket.send(JSON.stringify({
    type: 'clipboard_push',
    text: text,
  }));

  showToast(`📋 Sent ${text.length} characters to remote host!`);
}

async function copyLocalClipboardToBuffer() {
  try {
    if (navigator.clipboard && navigator.clipboard.readText) {
      const text = await navigator.clipboard.readText();
      document.getElementById('clipboard-buffer').value = text;
      showToast('📋 Pasted from local device clipboard!');
      return;
    }
  } catch (err) {}

  // Fallback for non-secure HTTP contexts where readText() is blocked
  const textarea = document.getElementById('clipboard-buffer');
  if (textarea) {
    textarea.focus();
    textarea.select();
  }
  showToast('In HTTP mode: press Ctrl+V into the drawer, then click "Send to Remote Host"', 'info');
}

async function copyBufferToLocalDevice() {
  const text = document.getElementById('clipboard-buffer').value;
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      await navigator.clipboard.writeText(text);
      showToast('📋 Copied buffer to your device clipboard!');
      return;
    }
  } catch (err) {}

  // Fallback using textarea selection and document.execCommand('copy')
  try {
    const textarea = document.getElementById('clipboard-buffer');
    if (textarea) {
      textarea.focus();
      textarea.select();
      const successful = document.execCommand('copy');
      if (successful) {
        showToast('📋 Copied buffer to your device clipboard!');
        return;
      }
    }
  } catch (e) {}

  showToast('Press Ctrl+C in the drawer buffer to copy text', 'warning');
}

// ================= Files (shared folder) & SFTP Transfers =================
const FOLDER_ICON = '<svg class="icon" viewBox="0 0 24 24"><path fill="currentColor" d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"/></svg>';
const FILE_ICON = '<svg class="icon" viewBox="0 0 24 24"><path fill="currentColor" d="M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8l-6-6zm-1 7V3.5L18.5 9H13z"/></svg>';

function toggleTransferDrawer() {
  const drawer = document.getElementById('transfer-drawer');
  const opening = drawer.style.display === 'none';
  drawer.style.display = opening ? 'flex' : 'none';
  if (opening) loadFiles(state.filesPath);
}

function openPaneFiles(paneIndex) {
  activatePane(paneIndex);
  toggleTransferDrawer();
}

function triggerFileInput() {
  document.getElementById('file-input').click();
}

async function handleFileInputChange(e) {
  const files = e.target.files;
  if (!files || files.length === 0) return;
  await uploadFiles(files);
  e.target.value = '';
}

function setupGlobalDragAndDrop() {
  for (let i = 1; i <= 4; i++) {
    const paneEl = document.getElementById(`pane-${i}`);
    const dropzone = document.getElementById(`pane-${i}-dropzone`);
    if (!paneEl || !dropzone) continue;

    paneEl.addEventListener('dragover', (e) => {
      e.preventDefault();
      dropzone.classList.add('active');
    });

    paneEl.addEventListener('dragleave', (e) => {
      e.preventDefault();
      if (!paneEl.contains(e.relatedTarget)) {
        dropzone.classList.remove('active');
      }
    });

    paneEl.addEventListener('drop', async (e) => {
      e.preventDefault();
      dropzone.classList.remove('active');
      const files = e.dataTransfer.files;
      if (files.length > 0) {
        activatePane(i);
        await uploadFiles(files);
      }
    });
  }

  const drawerZone = document.getElementById('drawer-dropzone');
  if (drawerZone) {
    drawerZone.addEventListener('dragover', (e) => e.preventDefault());
    drawerZone.addEventListener('drop', async (e) => {
      e.preventDefault();
      if (e.dataTransfer.files.length > 0) await uploadFiles(e.dataTransfer.files);
    });
  }

  const listEl = document.getElementById('files-list');
  if (listEl) listEl.addEventListener('click', onFilesListClick);
  const crumbEl = document.getElementById('files-breadcrumb');
  if (crumbEl) {
    crumbEl.addEventListener('click', (e) => {
      const el = e.target.closest('[data-path]');
      if (el) loadFiles(el.dataset.path);
    });
  }
}

async function readError(res, fallback) {
  try {
    const data = await res.json();
    return data.error || fallback;
  } catch (e) {
    return fallback;
  }
}

async function loadFiles(path) {
  const listEl = document.getElementById('files-list');
  try {
    const res = await apiFetch(`/api/files/list?path=${encodeURIComponent(path || '')}`);
    if (!res.ok) {
      // Folder vanished (e.g. deleted from inside Windows): fall back to the top level.
      if (path) return loadFiles('');
      listEl.innerHTML = `<div class="text-muted text-sm p-3 text-center">${escapeHtml(await readError(res, 'Could not load files'))}</div>`;
      return;
    }
    const data = await res.json();
    state.filesPath = data.path;
    renderFiles(data.entries);
  } catch (err) {
    listEl.innerHTML = `<div class="text-muted text-sm p-3 text-center">Could not load files</div>`;
  }
}

function renderFiles(entries) {
  const crumbEl = document.getElementById('files-breadcrumb');
  const listEl = document.getElementById('files-list');

  const parts = state.filesPath ? state.filesPath.split('/') : [];
  let crumbs = `<span class="files-crumb" data-path="">Files</span>`;
  parts.forEach((part, i) => {
    const p = parts.slice(0, i + 1).join('/');
    crumbs += `<span class="files-crumb-sep">/</span><span class="files-crumb" data-path="${escapeHtml(p)}">${escapeHtml(part)}</span>`;
  });
  crumbEl.innerHTML = crumbs;

  if (!entries || entries.length === 0) {
    listEl.innerHTML = `<div class="text-muted text-sm p-3 text-center">This folder is empty.</div>`;
    return;
  }

  const pane = state.panes[state.activePaneIndex];
  const sshPane = canSendToSsh(pane && pane.conn);

  listEl.innerHTML = entries
    .map((f) => {
      const p = escapeHtml(f.path);
      const when = f.modified ? new Date(f.modified).toLocaleString() : '';
      const meta = f.is_dir ? when : `${formatBytes(f.size)}${when ? ' • ' + when : ''}`;
      const actions = f.is_dir
        ? ''
        : `<a href="/api/files/download?path=${encodeURIComponent(f.path)}" class="pane-btn" title="Download" download>
             <svg class="icon" viewBox="0 0 24 24"><path fill="currentColor" d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/></svg>
           </a>
           ${sshPane ? `<button class="pane-btn" title="Send to SSH host" data-action="push" data-path="${p}">
             <svg class="icon" viewBox="0 0 24 24"><path fill="currentColor" d="M2.01 21 23 12 2.01 3 2 10l15 2-15 2z"/></svg>
           </button>` : ''}`;
      return `
        <div class="transfer-item files-item${f.is_dir ? ' is-dir' : ''}" ${f.is_dir ? `data-action="open" data-path="${p}"` : ''}>
          <div class="files-item-left">
            ${f.is_dir ? FOLDER_ICON : FILE_ICON}
            <div class="transfer-info">
              <span class="transfer-name">${escapeHtml(f.name)}</span>
              <span class="transfer-meta">${escapeHtml(meta)}</span>
            </div>
          </div>
          <div class="files-item-actions">
            ${actions}
            <button class="pane-btn files-delete" title="Delete" data-action="delete" data-path="${p}" data-dir="${f.is_dir ? '1' : ''}">
              <svg class="icon" viewBox="0 0 24 24"><path fill="currentColor" d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
            </button>
          </div>
        </div>`;
    })
    .join('');
}

/** Mirrors the server's rules for "Send to SSH host" so the button only shows when allowed. */
function canSendToSsh(conn) {
  if (!conn || conn.protocol !== 'ssh' || conn.view_only) return false;
  if (!['full', 'upload_only'].includes(conn.allow_transfer)) return false;
  if (conn.user_permissions && !conn.user_permissions.can_transfer) return false;
  const role = state.currentUser && state.currentUser.role;
  return role === 'admin' || role === 'operator';
}

function onFilesListClick(e) {
  const el = e.target.closest('[data-action]');
  if (!el) return;
  const path = el.dataset.path;
  switch (el.dataset.action) {
    case 'open':
      loadFiles(path);
      break;
    case 'delete':
      e.stopPropagation();
      deleteFilesEntry(path, el.dataset.dir === '1');
      break;
    case 'push': {
      e.stopPropagation();
      const pane = state.panes[state.activePaneIndex];
      if (pane && pane.conn) {
        const name = path.split('/').pop();
        pushFileToSftp(pane.conn.id, path, `./${name}`);
      }
      break;
    }
  }
}

async function uploadFiles(files) {
  const formData = new FormData();
  for (let i = 0; i < files.length; i++) {
    formData.append(`file_${i}`, files[i]);
  }

  showToast(`Uploading ${files.length} file(s)...`);

  try {
    const res = await apiFetch(`/api/files/upload?path=${encodeURIComponent(state.filesPath)}`, {
      method: 'POST',
      body: formData,
    });

    if (!res.ok) {
      showToast(await readError(res, 'Upload failed'), 'danger');
      return;
    }

    const data = await res.json();
    showToast(`Uploaded ${data.files.length} file(s) to Files`, 'success');
    const drawer = document.getElementById('transfer-drawer');
    if (drawer.style.display === 'none') toggleTransferDrawer();
    else loadFiles(state.filesPath);

    // If the active pane is SSH, offer to push the first file to the remote host
    const pane = state.panes[state.activePaneIndex];
    if (pane && canSendToSsh(pane.conn) && data.files.length > 0) {
      const first = data.files[0].path;
      const name = first.split('/').pop();
      if (await uiConfirm(`Push uploaded file "${name}" directly to the remote SSH host?`, { title: 'Send to SSH host', okText: 'Send' })) {
        await pushFileToSftp(pane.conn.id, first, `./${name}`);
      }
    }
  } catch (err) {
    showToast('Upload failed', 'danger');
  }
}

async function createFilesFolder() {
  const name = await uiPrompt('', { title: 'New folder', placeholder: 'Folder name', okText: 'Create' });
  if (!name || !name.trim()) return;
  try {
    const res = await apiFetch('/api/files/mkdir', {
      method: 'POST',
      body: JSON.stringify({ path: state.filesPath, name: name.trim() }),
    });
    if (!res.ok) {
      showToast(await readError(res, 'Could not create folder'), 'danger');
      return;
    }
    loadFiles(state.filesPath);
  } catch (err) {
    showToast('Could not create folder', 'danger');
  }
}

async function deleteFilesEntry(path, isDir) {
  const name = path.split('/').pop();
  const question = isDir
    ? `Delete folder "${name}" and everything in it?`
    : `Delete "${name}"?`;
  if (!(await uiConfirm(question, { title: isDir ? 'Delete folder' : 'Delete file', okText: 'Delete', danger: true }))) return;
  try {
    const res = await apiFetch('/api/files/delete', {
      method: 'POST',
      body: JSON.stringify({ path }),
    });
    if (!res.ok) {
      showToast(await readError(res, 'Delete failed'), 'danger');
      return;
    }
    loadFiles(state.filesPath);
  } catch (err) {
    showToast('Delete failed', 'danger');
  }
}

async function loadSftpDirectory(paneIndex, path) {
  const pane = state.panes[paneIndex];
  if (!pane || !pane.conn || pane.conn.protocol !== 'ssh') return;

  try {
    const res = await fetch(`/api/transfers/sftp/${pane.conn.id}/list`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${state.jwtToken}`,
      },
      body: JSON.stringify({ path: path }),
    });

    if (res.ok) {
      const data = await res.json();
      document.getElementById('sftp-current-path').textContent = path;
      renderSftpFiles(paneIndex, data.entries, path);
    } else {
      showSftpNote(`File browsing unavailable: ${await readError(res, 'unknown error')}`);
    }
  } catch (err) {
    showSftpNote('File browsing unavailable');
  }
}

function showSftpNote(text) {
  const listEl = document.getElementById('sftp-files-list');
  if (listEl) listEl.innerHTML = `<div class="text-muted text-sm p-2 text-center">${escapeHtml(text)}</div>`;
}

/** Ask for an SSH password when the server reports that login needs one. */
async function askSessionPassword(pane, ws, msg) {
  pane.authPending = true;
  const who = msg.username ? `${msg.username}@${msg.host}` : msg.host;
  const password = await uiPassword(msg.reason || '', {
    title: `Password for ${who}`,
    okText: 'Connect',
  });
  if (!pane.authPending) return; // session closed meanwhile
  pane.authPending = false;
  if (ws.readyState !== WebSocket.OPEN) return;
  if (password === null) {
    ws.send(JSON.stringify({ type: 'auth_cancel' }));
  } else {
    ws.send(JSON.stringify({ type: 'auth', password }));
    if (pane.terminal) pane.terminal.focus();
  }
}

function renderSftpFiles(paneIndex, entries, currentPath) {
  const listEl = document.getElementById('sftp-files-list');
  if (!entries || entries.length === 0) {
    listEl.innerHTML = `<div class="text-muted text-sm p-2 text-center">Empty directory</div>`;
    return;
  }

  let html = '';
  if (currentPath !== '.' && currentPath !== '/') {
    const parentPath = currentPath.substring(0, currentPath.lastIndexOf('/')) || '/';
    html += `
      <div class="sftp-item" onclick="loadSftpDirectory(${paneIndex}, '${parentPath}')">
        <div class="sftp-item-left">
          <span>📁</span>
          <strong>.. (Up one level)</strong>
        </div>
      </div>
    `;
  }

  html += entries.map(item => `
    <div class="sftp-item" onclick="${item.is_dir ? `loadSftpDirectory(${paneIndex}, '${item.path}')` : ''}">
      <div class="sftp-item-left">
        <span>${item.is_dir ? '📁' : '📄'}</span>
        <span>${escapeHtml(item.name)}</span>
      </div>
      <div class="text-muted text-sm">
        ${item.is_dir ? 'DIR' : formatBytes(item.size)}
      </div>
    </div>
  `).join('');

  listEl.innerHTML = html;
}

async function pushFileToSftp(connectionId, filePath, remotePath) {
  showToast('Transferring file to remote server via SFTP...');
  try {
    const res = await apiFetch(`/api/transfers/sftp/${connectionId}/upload`, {
      method: 'POST',
      body: JSON.stringify({ file_path: filePath, remote_path: remotePath }),
    });

    if (res.ok) {
      showToast('File transfer to remote SSH host complete!', 'success');
      loadSftpDirectory(state.activePaneIndex, '.');
    } else {
      showToast(await readError(res, 'SFTP transfer failed'), 'danger');
    }
  } catch (err) {
    showToast('Failed to push file via SFTP', 'danger');
  }
}

// ================= Users & RBAC Modal =================
async function openUsersModal() {
  document.getElementById('users-modal').style.display = 'flex';
  try {
    const res = await fetch('/api/users', {
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });
    if (res.ok) {
      const users = await res.json();
      state.usersCache = users;
      renderUsersTable(users);
      await loadGroups();
    }
  } catch (err) {
    console.error('Failed to load users:', err);
  }
}

function closeUsersModal() {
  document.getElementById('users-modal').style.display = 'none';
}

function renderUsersTable(users) {
  const tbody = document.getElementById('users-table-body');
  tbody.innerHTML = users.map(u => {
    const avatarHtml = u.avatar_data && u.avatar_data.startsWith('data:image')
      ? `<img src="${u.avatar_data}" style="width: 24px; height: 24px; border-radius: 50%; object-fit: cover; vertical-align: middle; margin-right: 8px;">`
      : `<span style="width: 24px; height: 24px; border-radius: 50%; background: var(--woofson-bg-active); border: 1px solid var(--woofson-border); display: inline-flex; align-items: center; justify-content: center; font-size: 11px; font-weight: 700; color: var(--woofson-accent); margin-right: 8px; vertical-align: middle;">${(u.username[0] || 'U').toUpperCase()}</span>`;

    const roleName = u.role.toLowerCase() === 'admin' ? 'ADMIN' : u.role.toUpperCase();
    const statusBadge = u.is_active
      ? `<span class="badge" style="background: rgba(16,185,129,0.15); color: #10b981; border: 1px solid rgba(16,185,129,0.3); font-size: 10px; padding: 2px 6px; border-radius: 4px; font-weight: 700;">ACTIVE</span>`
      : `<span class="badge" style="background: rgba(239,68,68,0.15); color: #ef4444; border: 1px solid rgba(239,68,68,0.3); font-size: 10px; padding: 2px 6px; border-radius: 4px; font-weight: 700;">DISABLED</span>`;

    const toggleBtn = u.is_active
      ? `<button class="btn btn-secondary btn-sm" onclick="toggleUserStatus('${u.id}', false)" title="Disable user login">Disable</button>`
      : `<button class="btn btn-sm" style="color: #10b981; border-color: rgba(16,185,129,0.4);" onclick="toggleUserStatus('${u.id}', true)" title="Enable user login">Enable</button>`;

    return `
      <tr>
        <td>
          <div style="display: flex; align-items: center;">
            ${avatarHtml}
            <div>
              <strong>${escapeHtml(u.username)}</strong>
              ${u.display_name && u.display_name !== u.username ? `<span class="text-muted text-sm" style="margin-left: 4px;">(${escapeHtml(u.display_name)})</span>` : ''}
            </div>
          </div>
        </td>
        <td class="text-muted text-sm">${u.email ? escapeHtml(u.email) : '—'}</td>
        <td><span class="user-role-badge">${roleName}</span></td>
        <td><span class="pane-protocol-badge">${u.auth_provider.toUpperCase()}</span></td>
        <td class="text-muted text-sm">${u.groups && u.groups.length ? u.groups.map(escapeHtml).join(', ') : '—'}</td>
        <td>${statusBadge}</td>
        <td class="text-muted">${new Date(u.created_at).toLocaleDateString()}</td>
        <td>
          <div style="display: flex; gap: 4px;">
            ${toggleBtn}
            ${u.auth_provider === 'local' ? `<button class="btn btn-secondary btn-sm" data-reset-user="${escapeHtml(u.id)}" data-reset-name="${escapeHtml(u.username)}" title="Give this user a new password">Reset password</button>` : ''}
            <button class="btn btn-secondary btn-sm text-danger" onclick="deleteUser('${u.id}')">Delete</button>
          </div>
        </td>
      </tr>
    `;
  }).join('');
}

async function toggleUserStatus(userId, newActive) {
  const actionName = newActive ? 'enable' : 'disable';
  if (!(await uiConfirm(`Are you sure you want to ${actionName} this user account?`, { title: newActive ? 'Enable user' : 'Disable user', okText: newActive ? 'Enable' : 'Disable', danger: !newActive }))) return;

  try {
    const res = await apiFetch(`/api/users/${userId}`, {
      method: 'PUT',
      body: JSON.stringify({ is_active: newActive }),
    });
    if (res.ok) {
      showToast(`User account ${newActive ? 'enabled' : 'disabled'}!`, 'success');
      openUsersModal();
    } else {
      const err = await res.json();
      showToast(`Failed to update status: ${err.error || 'Unknown error'}`, 'danger');
    }
  } catch (err) {
    showToast(`Failed: ${err}`, 'danger');
  }
}

function openAddUserModal() {
  document.getElementById('user-edit-form').reset();
  const errEl = document.getElementById('add-user-error');
  if (errEl) errEl.style.display = 'none';
  document.getElementById('user-edit-modal').style.display = 'flex';
}

function closeUserEditModal() {
  document.getElementById('user-edit-modal').style.display = 'none';
}

async function handleSaveUser(e) {
  e.preventDefault();
  const username = document.getElementById('edit-user-username').value.trim();
  const displayName = document.getElementById('edit-user-displayname').value.trim();
  const email = document.getElementById('edit-user-email').value.trim();
  const password = document.getElementById('edit-user-password').value;
  const passwordAgain = document.getElementById('edit-user-password-confirm').value;
  const mustChange = document.getElementById('edit-user-must-change').checked;
  const role = document.getElementById('edit-user-role').value;
  const isActive = document.getElementById('edit-user-active') ? document.getElementById('edit-user-active').value === 'true' : true;
  const errEl = document.getElementById('add-user-error');

  const problem = passwordProblem(password, passwordAgain, username);
  if (problem) {
    errEl.textContent = problem;
    errEl.style.display = 'block';
    return;
  }

  try {
    const res = await fetch('/api/users', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${state.jwtToken}`,
      },
      body: JSON.stringify({
        username,
        display_name: displayName || null,
        email: email || null,
        password,
        role,
        is_active: isActive,
        must_change_password: mustChange,
      }),
    });

    if (res.ok) {
      showToast('User account created successfully!', 'success');
      closeUserEditModal();
      openUsersModal();
    } else {
      const data = await res.json();
      if (errEl) {
        errEl.textContent = `Error: ${data.error || 'Failed to create user'}`;
        errEl.style.display = 'block';
      } else {
        showToast(`Error creating user: ${data.error}`, 'danger');
      }
    }
  } catch (err) {
    if (errEl) {
      errEl.textContent = `Failed: ${err}`;
      errEl.style.display = 'block';
    } else {
      showToast(`Failed: ${err}`, 'danger');
    }
  }
}

async function deleteUser(id) {
  if (!(await uiConfirm('Are you sure you want to delete this user?', { title: 'Delete user', okText: 'Delete', danger: true }))) return;
  try {
    const res = await fetch(`/api/users/${id}`, {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });
    if (res.ok) {
      showToast('User deleted');
      openUsersModal();
    }
  } catch (err) {
    showToast(`Failed: ${err}`, 'danger');
  }
}

// ================= Settings & Audit Modals =================
async function openSettingsModal() {
  document.getElementById('settings-modal').style.display = 'flex';
  try {
    const res = await fetch('/api/settings');
    if (res.ok) {
      const data = await res.json();
      renderSettings(data);
    }
  } catch (e) {}
}

function closeSettingsModal() {
  document.getElementById('settings-modal').style.display = 'none';
}

function renderSettings(s) {
  const oidcEl = document.getElementById('settings-oidc-details');
  oidcEl.innerHTML = `
    <div class="form-group"><label>OIDC SSO Status</label><input class="form-input" disabled value="${s.oidc.enabled ? 'Enabled' : 'Disabled (Configured in config.toml)'}" /></div>
    <div class="form-group"><label>Identity Provider</label><input class="form-input" disabled value="${s.oidc.provider_name}" /></div>
    <div class="form-group"><label>Issuer URL</label><input class="form-input" disabled value="${s.oidc.issuer_url}" /></div>
    <div class="form-group"><label>Redirect URI</label><input class="form-input" disabled value="${s.oidc.redirect_uri}" /></div>
  `;

  const genEl = document.getElementById('settings-general-details');
  genEl.innerHTML = `
    <div class="form-group"><label>Default Clipboard Sync</label><input class="form-input" disabled value="${s.clipboard.default_mode}" /></div>
    <div class="form-group"><label>Staging Directory</label><input class="form-input" disabled value="${s.storage.staging_dir}" /></div>
  `;
}

async function openAuditModal() {
  document.getElementById('audit-modal').style.display = 'flex';
  try {
    const res = await fetch('/api/audit-logs', {
      headers: { Authorization: `Bearer ${state.jwtToken}` },
    });
    if (res.ok) {
      const logs = await res.json();
      const tbody = document.getElementById('audit-table-body');
      tbody.innerHTML = logs.map(l => `
        <tr>
          <td class="text-muted">${new Date(l.timestamp).toLocaleString()}</td>
          <td><strong>${escapeHtml(l.username)}</strong></td>
          <td><span class="pane-protocol-badge">${l.action}</span></td>
          <td>${l.connection_name ? escapeHtml(l.connection_name) : '—'}</td>
          <td><span class="text-muted">${escapeHtml(l.details || '')}</span></td>
        </tr>
      `).join('');
    }
  } catch (e) {}
}

function closeAuditModal() {
  document.getElementById('audit-modal').style.display = 'none';
}

// ================= Keyboard Shortcuts =================
function setupKeyboardShortcuts() {
  document.addEventListener('keydown', (e) => {
    // If session is locked or login modal is visible, don't trigger global navigation shortcuts
    const lockScreen = document.getElementById('session-lock-screen');
    if (lockScreen && (lockScreen.classList.contains('active') || lockScreen.style.display === 'flex')) return;
    const loginModal = document.getElementById('login-modal');
    if (loginModal && (loginModal.classList.contains('active') || loginModal.style.display === 'flex')) return;

    // F10 or Alt+W -> Full Window Toggle
    if (e.key === 'F10' || (e.altKey && (e.key === 'w' || e.key === 'W'))) {
      e.preventDefault();
      toggleFullWindow();
      return;
    }

    // F11 -> Fullscreen Toggle
    if (e.key === 'F11') {
      e.preventDefault();
      toggleFullscreen();
      return;
    }

    // Escape -> Exit Full Window / Fullscreen or Close open modals
    if (e.key === 'Escape') {
      const app = document.getElementById('app');
      if (document.fullscreenElement) {
        document.exitFullscreen().catch(() => {});
        return;
      }
      if (app && app.classList.contains('full-window-mode')) {
        toggleFullWindow();
        return;
      }
      // Close open modals / menus
      document.getElementById('profile-dropdown-menu')?.classList.remove('active');
      const pm = document.getElementById('profile-dropdown-menu');
      if (pm) pm.style.display = 'none';
      closeHelpModal();
      closeAboutModal();
      closeUserProfileModal();
      closeConnectionsModal();
      closeConnectionEditModal();
      closeSettingsModal();
      closeAuditModal();
      closeUsersModal();
      closeUserEditModal();
      document.getElementById('pane-settings-popup')?.remove();
      document.getElementById('pane-kbd-dropdown')?.remove();
      return;
    }

    // Alt+1 to Alt+4 -> Multi-Pane Switching
    if (e.altKey && e.key >= '1' && e.key <= '4') {
      setPaneLayout(parseInt(e.key, 10));
      e.preventDefault();
      return;
    }

    // Ctrl+Shift+V -> Clipboard Drawer
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'v') {
      toggleClipboardDrawer();
      e.preventDefault();
      return;
    }
  });
}

// ================= Toast Notifications & Helpers =================
function showToast(msg, type = 'info') {
  const container = document.getElementById('toast-container');
  const toast = document.createElement('div');
  toast.className = `toast toast-${type}`;
  toast.innerHTML = `<span>${escapeHtml(msg)}</span>`;
  container.appendChild(toast);

  setTimeout(() => {
    toast.style.opacity = '0';
    setTimeout(() => toast.remove(), 200);
  }, 3500);
}

// ================= Groups & Connection Assignments =================
state.usersCache = [];
state.groupsCache = [];

async function loadGroups() {
  try {
    const res = await apiFetch('/api/groups');
    if (!res.ok) return;
    state.groupsCache = await res.json();
    renderGroups();
  } catch (e) {}
}

function renderGroups() {
  const el = document.getElementById('groups-list');
  if (!el) return;
  if (!state.groupsCache.length) {
    el.innerHTML = `<div class="text-muted text-sm">No groups yet.</div>`;
    return;
  }
  const nameOf = (id) => (state.usersCache.find((u) => u.id === id) || {}).username;
  el.innerHTML = state.groupsCache
    .map((g) => {
      const members = (g.member_ids || []).map(nameOf).filter(Boolean);
      return `
        <div class="group-row">
          <div class="group-row-name">${escapeHtml(g.name)}
            <span class="group-row-meta">· ${members.length} member${members.length === 1 ? '' : 's'}${members.length ? ': ' + members.map(escapeHtml).join(', ') : ''}</span>
          </div>
          <button class="btn btn-secondary btn-sm" data-group-action="members" data-group-id="${escapeHtml(g.id)}">Members</button>
          <button class="btn btn-secondary btn-sm" data-group-action="rename" data-group-id="${escapeHtml(g.id)}">Rename</button>
          <button class="btn btn-secondary btn-sm text-danger" data-group-action="delete" data-group-id="${escapeHtml(g.id)}">Delete</button>
        </div>`;
    })
    .join('');
}

document.addEventListener('click', (e) => {
  const btn = e.target.closest('[data-group-action]');
  if (!btn) return;
  const group = state.groupsCache.find((g) => g.id === btn.dataset.groupId);
  if (!group) return;
  if (btn.dataset.groupAction === 'members') openGroupMembers(group);
  if (btn.dataset.groupAction === 'rename') renameGroup(group);
  if (btn.dataset.groupAction === 'delete') deleteGroup(group);
});

async function createGroup() {
  const name = await uiPrompt('', { title: 'New group', placeholder: 'e.g. Finance', okText: 'Create' });
  if (!name || !name.trim()) return;
  const res = await apiFetch('/api/groups', { method: 'POST', body: JSON.stringify({ name: name.trim() }) });
  if (!res.ok) {
    showToast(await readError(res, 'Could not create group'), 'danger');
    return;
  }
  showToast(`Group "${name.trim()}" created`, 'success');
  loadGroups();
}

async function renameGroup(group) {
  const name = await uiPrompt('', { title: 'Rename group', value: group.name, okText: 'Rename' });
  if (!name || !name.trim() || name.trim() === group.name) return;
  const res = await apiFetch(`/api/groups/${group.id}`, {
    method: 'PUT',
    body: JSON.stringify({ name: name.trim(), description: group.description || null }),
  });
  if (!res.ok) {
    showToast(await readError(res, 'Could not rename group'), 'danger');
    return;
  }
  await openUsersModal();
}

async function deleteGroup(group) {
  const ok = await uiConfirm(
    `Delete group "${group.name}"? Its members keep their accounts, but lose access given through this group.`,
    { title: 'Delete group', okText: 'Delete', danger: true },
  );
  if (!ok) return;
  const res = await apiFetch(`/api/groups/${group.id}`, { method: 'DELETE' });
  if (!res.ok) {
    showToast(await readError(res, 'Could not delete group'), 'danger');
    return;
  }
  showToast(`Group "${group.name}" deleted`, 'success');
  await openUsersModal();
}

let membersTarget = null;

function openGroupMembers(group) {
  membersTarget = group;
  document.getElementById('group-members-title').textContent = `Members of ${group.name}`;
  const ids = new Set(group.member_ids || []);
  document.getElementById('group-members-list').innerHTML = state.usersCache
    .map(
      (u) => `
      <label class="pw-check">
        <input type="checkbox" value="${escapeHtml(u.id)}" ${ids.has(u.id) ? 'checked' : ''} />
        <span>${escapeHtml(u.username)}${u.display_name && u.display_name !== u.username ? ` <span class="text-muted">(${escapeHtml(u.display_name)})</span>` : ''}</span>
      </label>`,
    )
    .join('');
  document.getElementById('group-members-box').style.transform = '';
  document.getElementById('group-members').style.display = 'flex';
}

function closeGroupMembers() {
  document.getElementById('group-members').style.display = 'none';
  membersTarget = null;
}

async function saveGroupMembers() {
  if (!membersTarget) return;
  const user_ids = [...document.querySelectorAll('#group-members-list input:checked')].map((i) => i.value);
  const res = await apiFetch(`/api/groups/${membersTarget.id}/members`, {
    method: 'PUT',
    body: JSON.stringify({ user_ids }),
  });
  if (!res.ok) {
    showToast(await readError(res, 'Could not save members'), 'danger');
    return;
  }
  closeGroupMembers();
  showToast('Members saved', 'success');
  await openUsersModal();
}

document.addEventListener('DOMContentLoaded', () => {
  makeDraggable(document.getElementById('group-members-box'), document.getElementById('group-members-header'));
  const save = document.getElementById('group-members-save');
  if (save) save.addEventListener('click', saveGroupMembers);
  const overlay = document.getElementById('group-members');
  if (overlay) {
    overlay.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Escape') closeGroupMembers();
    });
  }
  const list = document.getElementById('conn-assign-list');
  if (list) {
    list.addEventListener('click', (e) => {
      const rm = e.target.closest('.assign-remove');
      if (rm) rm.closest('.assign-row').remove();
    });
  }
});

/** One line under a connection's name: where it comes from (users) or owner/assignees (admins). */
function connectionAccessLine(c) {
  if (c.access === 'admin') {
    const parts = [];
    if (c.owner_name) parts.push(`Owner: ${escapeHtml(c.owner_name)}`);
    if (c.assigned_to && c.assigned_to.length) parts.push(`Assigned: ${c.assigned_to.map(escapeHtml).join(', ')}`);
    return parts.length ? `<span class="conn-access-line">${parts.join(' · ')}</span>` : '';
  }
  const label = {
    own: 'Mine',
    assigned: 'Assigned to me',
    group: `Via group ${escapeHtml(c.access_group || '')}`,
    global: 'Shared with everyone',
  }[c.access];
  return label ? `<span class="conn-access-line">${label}</span>` : '';
}

/** Admins: fill the Owner and Assigned-to section of the connection dialog. */
async function prepareAssignments(conn) {
  const section = document.getElementById('conn-assign-section');
  const list = document.getElementById('conn-assign-list');
  const isAdm = state.currentUser && state.currentUser.role === 'admin';
  list.innerHTML = '';
  section.style.display = isAdm ? '' : 'none';
  if (!isAdm) return;

  try {
    const [uRes, gRes] = await Promise.all([apiFetch('/api/users'), apiFetch('/api/groups')]);
    if (uRes.ok) state.usersCache = await uRes.json();
    if (gRes.ok) state.groupsCache = await gRes.json();
  } catch (e) {}

  const ownerId = conn ? conn.created_by : state.currentUser.id;
  document.getElementById('conn-owner').innerHTML = state.usersCache
    .map((u) => `<option value="${escapeHtml(u.id)}" ${u.id === ownerId ? 'selected' : ''}>${escapeHtml(u.username)}${u.id === state.currentUser.id ? ' (me)' : ''}</option>`)
    .join('');

  if (conn) {
    try {
      const res = await apiFetch(`/api/connections/${conn.id}/permissions`);
      if (res.ok) {
        const d = await res.json();
        d.grants.forEach((g) => addAssignmentRow(g));
      }
    } catch (e) {}
  }
}

function addAssignmentRow(grant) {
  const g = grant || { can_connect: true, can_edit: false, can_clipboard: true, can_transfer: true };
  const target = g.user_id ? `u:${g.user_id}` : g.group_id ? `g:${g.group_id}` : '';
  const userOpts = state.usersCache
    .map((u) => `<option value="u:${escapeHtml(u.id)}" ${target === 'u:' + u.id ? 'selected' : ''}>${escapeHtml(u.username)}</option>`)
    .join('');
  const groupOpts = state.groupsCache
    .map((x) => `<option value="g:${escapeHtml(x.id)}" ${target === 'g:' + x.id ? 'selected' : ''}>${escapeHtml(x.name)}</option>`)
    .join('');
  const row = document.createElement('div');
  row.className = 'assign-row';
  row.innerHTML = `
    <select class="form-input assign-target">
      <option value="">Choose user or group…</option>
      <optgroup label="Users">${userOpts}</optgroup>
      ${groupOpts ? `<optgroup label="Groups">${groupOpts}</optgroup>` : ''}
    </select>
    <label class="pw-check"><input type="checkbox" class="assign-connect" ${g.can_connect ? 'checked' : ''}/>Connect</label>
    <label class="pw-check"><input type="checkbox" class="assign-edit" ${g.can_edit ? 'checked' : ''}/>Edit</label>
    <label class="pw-check"><input type="checkbox" class="assign-clipboard" ${g.can_clipboard ? 'checked' : ''}/>Clipboard</label>
    <label class="pw-check"><input type="checkbox" class="assign-transfer" ${g.can_transfer ? 'checked' : ''}/>Files</label>
    <button type="button" class="assign-remove" title="Remove">
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
    </button>`;
  document.getElementById('conn-assign-list').appendChild(row);
}

async function saveAssignments(connId) {
  const grants = [];
  for (const row of document.querySelectorAll('#conn-assign-list .assign-row')) {
    const target = row.querySelector('.assign-target').value;
    if (!target) continue;
    const [kind, targetId] = [target.slice(0, 1), target.slice(2)];
    grants.push({
      user_id: kind === 'u' ? targetId : null,
      group_id: kind === 'g' ? targetId : null,
      can_connect: row.querySelector('.assign-connect').checked,
      can_edit: row.querySelector('.assign-edit').checked,
      can_clipboard: row.querySelector('.assign-clipboard').checked,
      can_transfer: row.querySelector('.assign-transfer').checked,
    });
  }
  const res = await apiFetch(`/api/connections/${connId}/permissions`, {
    method: 'PUT',
    body: JSON.stringify({ grants }),
  });
  if (!res.ok) {
    showToast(`Connection saved, but assignments failed: ${await readError(res, 'unknown error')}`, 'danger');
    return false;
  }
  return true;
}

// ================= Passwords =================
let minPasswordLength = 12;

async function loadPasswordPolicy() {
  try {
    const res = await fetch('/api/auth/providers');
    if (!res.ok) return;
    const d = await res.json();
    if (d.min_password_length) minPasswordLength = d.min_password_length;
  } catch (e) {}
  document.querySelectorAll('.pw-min-text').forEach((el) => {
    el.textContent = `At least ${minPasswordLength} characters`;
  });
}

function setSessionToken(token) {
  state.jwtToken = token;
  localStorage.setItem('remote_token', token);
  localStorage.setItem('remotedog_token', token);
  document.cookie = `remote_token=${token}; path=/; max-age=86400; SameSite=Lax`;
}

/** Client-side check matching the server's rules. Returns an error message or null. */
function passwordProblem(password, again, username, current) {
  if (!password) return 'Enter a password';
  if (password !== again) return 'Passwords do not match';
  if ([...password].length < minPasswordLength) return `Password must be at least ${minPasswordLength} characters`;
  if (username && password.toLowerCase() === username.toLowerCase()) return 'Password must not be the username';
  if (current && password === current) return 'New password must differ from the current one';
  return null;
}

async function changeOwnPassword(current, next) {
  try {
    const res = await apiFetch('/api/auth/change-password', {
      method: 'POST',
      body: JSON.stringify({ current_password: current, new_password: next }),
    });
    const d = await res.json().catch(() => ({}));
    if (!res.ok) return { ok: false, error: d.error || 'Could not change password' };
    if (d.token) setSessionToken(d.token);
    return { ok: true };
  } catch (e) {
    return { ok: false, error: 'Server connection error' };
  }
}

function setupPasswordDialogs() {
  makeDraggable(document.getElementById('pw-required-box'), document.getElementById('pw-required-header'));
  makeDraggable(document.getElementById('pw-reset-box'), document.getElementById('pw-reset-header'));

  document.getElementById('pw-required-form').addEventListener('submit', (e) => {
    e.preventDefault();
    submitPasswordRequired();
  });
  document.getElementById('pw-required-save').addEventListener('click', submitPasswordRequired);
  document.getElementById('pw-required').addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      submitPasswordRequired();
    }
  });

  document.getElementById('pw-reset-save').addEventListener('click', submitPasswordReset);
  document.getElementById('pw-reset').addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') closePasswordReset();
    if (e.key === 'Enter' && e.target.tagName === 'INPUT' && e.target.type === 'password') {
      e.preventDefault();
      submitPasswordReset();
    }
  });

  // Reset buttons live in the re-rendered users table.
  document.addEventListener('click', (e) => {
    const btn = e.target.closest('[data-reset-user]');
    if (btn) openPasswordReset(btn.dataset.resetUser, btn.dataset.resetName);
  });
}

function showPasswordRequired() {
  const overlay = document.getElementById('pw-required');
  if (!overlay || overlay.style.display === 'flex') return;
  ['pw-required-current', 'pw-required-new', 'pw-required-confirm'].forEach((id) => {
    document.getElementById(id).value = '';
  });
  document.getElementById('pw-required-error').textContent = '';
  document.getElementById('pw-required-current-group').style.display = state.loginPassword ? 'none' : '';
  document.getElementById('pw-required-box').style.transform = '';
  overlay.style.display = 'flex';
  setTimeout(() => {
    document.getElementById(state.loginPassword ? 'pw-required-new' : 'pw-required-current').focus();
  }, 0);
}

async function submitPasswordRequired() {
  const errEl = document.getElementById('pw-required-error');
  const current = state.loginPassword || document.getElementById('pw-required-current').value;
  const next = document.getElementById('pw-required-new').value;
  const again = document.getElementById('pw-required-confirm').value;
  const username = state.currentUser && state.currentUser.username;

  const problem = !current ? 'Enter your current password' : passwordProblem(next, again, username, current);
  if (problem) {
    errEl.textContent = problem;
    return;
  }
  errEl.textContent = '';
  const result = await changeOwnPassword(current, next);
  if (!result.ok) {
    errEl.textContent = result.error;
    if (/current password/i.test(result.error)) {
      state.loginPassword = null;
      document.getElementById('pw-required-current-group').style.display = '';
    }
    return;
  }
  state.loginPassword = null;
  if (state.currentUser) state.currentUser.must_change_password = false;
  document.getElementById('pw-required').style.display = 'none';
  showToast('Password changed', 'success');
  onAuthenticated();
}

let pwResetTarget = null;

function openPasswordReset(userId, username) {
  pwResetTarget = { id: userId, username };
  document.getElementById('pw-reset-title').textContent = `Reset password for ${username}`;
  document.querySelector('input[name="pw-reset-mode"][value="generate"]').checked = true;
  document.getElementById('pw-reset-new').value = '';
  document.getElementById('pw-reset-confirm').value = '';
  document.getElementById('pw-reset-must-change').checked = true;
  document.getElementById('pw-reset-error').textContent = '';
  document.getElementById('pw-reset-box').style.transform = '';
  updatePasswordResetMode();
  document.getElementById('pw-reset').style.display = 'flex';
  setTimeout(() => document.getElementById('pw-reset-save').focus(), 0);
}

function updatePasswordResetMode() {
  const set = document.querySelector('input[name="pw-reset-mode"]:checked').value === 'set';
  document.getElementById('pw-reset-fields').style.display = set ? '' : 'none';
  if (set) setTimeout(() => document.getElementById('pw-reset-new').focus(), 0);
}

function closePasswordReset() {
  document.getElementById('pw-reset').style.display = 'none';
  document.getElementById('pw-reset-new').value = '';
  document.getElementById('pw-reset-confirm').value = '';
  pwResetTarget = null;
}

async function submitPasswordReset() {
  if (!pwResetTarget) return;
  const errEl = document.getElementById('pw-reset-error');
  const set = document.querySelector('input[name="pw-reset-mode"]:checked').value === 'set';
  const mustChange = document.getElementById('pw-reset-must-change').checked;
  const body = { must_change_password: mustChange };
  if (set) {
    const pw = document.getElementById('pw-reset-new').value;
    const problem = passwordProblem(pw, document.getElementById('pw-reset-confirm').value, pwResetTarget.username);
    if (problem) {
      errEl.textContent = problem;
      return;
    }
    body.password = pw;
  }
  const target = pwResetTarget;
  try {
    const res = await apiFetch(`/api/users/${target.id}/reset-password`, {
      method: 'POST',
      body: JSON.stringify(body),
    });
    const d = await res.json().catch(() => ({}));
    if (!res.ok) {
      errEl.textContent = d.error || 'Could not reset password';
      return;
    }
    closePasswordReset();
    if (d.password) {
      await uiCopyDialog(
        `New password for ${target.username}`,
        `Give this password to ${target.username}. It is shown only once.` +
          (mustChange ? ' They will be asked to choose their own at next login.' : ''),
        d.password,
      );
    } else {
      showToast(`Password for ${target.username} has been reset`, 'success');
    }
  } catch (e) {
    errEl.textContent = 'Server connection error';
  }
}

/** Make a dialog box draggable by its header (RULES 9: whole header is a drag handle). */
function makeDraggable(box, header) {
  if (!box || !header) return;
  let drag = null;
  header.addEventListener('mousedown', (e) => {
    if (e.target.closest('button')) return;
    const m = (box.style.transform.match(/-?[\d.]+/g) || [0, 0]).map(Number);
    drag = { x: e.clientX - m[0], y: e.clientY - m[1] };
    e.preventDefault();
  });
  document.addEventListener('mousemove', (e) => {
    if (drag) box.style.transform = `translate(${e.clientX - drag.x}px, ${e.clientY - drag.y}px)`;
  });
  document.addEventListener('mouseup', () => { drag = null; });
}

// ================= In-app Dialog (replaces confirm/prompt/alert) =================
let uiDialogResolve = null;
let uiDialogMode = 'confirm';
let uiDialogCopyValue = '';

function setupUiDialog() {
  const overlay = document.getElementById('ui-dialog');
  if (!overlay || overlay.dataset.ready) return;
  overlay.dataset.ready = '1';
  const box = document.getElementById('ui-dialog-box');
  const header = document.getElementById('ui-dialog-header');
  const input = document.getElementById('ui-dialog-input');

  document.getElementById('ui-dialog-ok').addEventListener('click', uiDialogAccept);
  document.getElementById('ui-dialog-cancel').addEventListener('click', () => closeUiDialog(null));
  document.getElementById('ui-dialog-x').addEventListener('click', () => closeUiDialog(null));
  overlay.addEventListener('mousedown', (e) => {
    if (e.target === overlay) closeUiDialog(null);
  });
  // Keep keys inside the dialog away from pane shortcuts and remote sessions.
  overlay.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      closeUiDialog(null);
    } else if (e.key === 'Enter' && (e.target === input || e.target === overlay || e.target === box)) {
      e.preventDefault();
      uiDialogAccept();
    }
  });

  // While open, keep focus inside the dialog (terminals and canvases try to grab it).
  document.addEventListener('focusin', (e) => {
    if (overlay.style.display !== 'none' && !overlay.contains(e.target)) {
      e.stopPropagation();
      focusUiDialog();
    }
  }, true);

  makeDraggable(box, header);
}

function openUiDialog({ title, message = '', mode = 'confirm', value = '', placeholder = '', okText = 'OK', cancelText = 'Cancel', danger = false }) {
  setupUiDialog();
  if (uiDialogResolve) closeUiDialog(null);

  return new Promise((resolve) => {
    uiDialogResolve = resolve;
    uiDialogMode = mode;
    uiDialogCopyValue = value;

    const overlay = document.getElementById('ui-dialog');
    const input = document.getElementById('ui-dialog-input');
    const ok = document.getElementById('ui-dialog-ok');
    const cancel = document.getElementById('ui-dialog-cancel');

    document.getElementById('ui-dialog-title').textContent = title;
    document.getElementById('ui-dialog-message').textContent = message;
    document.getElementById('ui-dialog-box').style.transform = '';

    const hasInput = mode === 'prompt' || mode === 'copy' || mode === 'password';
    input.type = mode === 'password' ? 'password' : 'text';
    input.autocomplete = mode === 'password' ? 'current-password' : 'off';
    input.style.display = hasInput ? '' : 'none';
    input.value = hasInput ? value : '';
    input.placeholder = placeholder;
    input.readOnly = mode === 'copy';

    ok.textContent = okText;
    ok.className = danger ? 'btn btn-danger' : 'btn btn-accent';
    cancel.textContent = cancelText;

    overlay.style.display = 'flex';
    setTimeout(focusUiDialog, 0);
  });
}

function focusUiDialog() {
  const input = document.getElementById('ui-dialog-input');
  if (input.style.display !== 'none') {
    input.focus();
    if (input.readOnly) input.select();
  } else {
    document.getElementById('ui-dialog-ok').focus();
  }
}

function uiDialogAccept() {
  const input = document.getElementById('ui-dialog-input');
  if (uiDialogMode === 'prompt' || uiDialogMode === 'password') {
    const value = input.value;
    input.value = '';
    closeUiDialog(value);
  } else if (uiDialogMode === 'copy') {
    copyTextToClipboard(uiDialogCopyValue, input);
    closeUiDialog(true);
  } else {
    closeUiDialog(true);
  }
}

function closeUiDialog(result) {
  const overlay = document.getElementById('ui-dialog');
  if (overlay) overlay.style.display = 'none';
  const input = document.getElementById('ui-dialog-input');
  if (input && input.type === 'password') input.value = '';
  const resolve = uiDialogResolve;
  uiDialogResolve = null;
  if (resolve) resolve(result);
}

/** In-app replacement for confirm(). Resolves to true or false. */
async function uiConfirm(message, { title = 'Confirm', okText = 'OK', danger = false } = {}) {
  return (await openUiDialog({ title, message, okText, danger })) === true;
}

/** In-app replacement for prompt(). Resolves to the text, or null when cancelled. */
async function uiPrompt(message, { title = 'Enter a value', value = '', placeholder = '', okText = 'OK' } = {}) {
  const r = await openUiDialog({ title, message, mode: 'prompt', value, placeholder, okText });
  return typeof r === 'string' ? r : null;
}

/** Masked password entry. Resolves to the password, or null when cancelled. */
async function uiPassword(message, { title = 'Password', okText = 'OK' } = {}) {
  const r = await openUiDialog({ title, message, mode: 'password', placeholder: 'Password', okText });
  return typeof r === 'string' ? r : null;
}

/** Shows a value in a read-only field with a Copy button. */
function uiCopyDialog(title, message, value) {
  return openUiDialog({ title, message, mode: 'copy', value, okText: 'Copy', cancelText: 'Close' });
}

function copyTextToClipboard(text, fallbackInput) {
  const done = () => showToast('Copied to clipboard', 'success');
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).then(done).catch(() => legacyCopy(text, fallbackInput, done));
  } else {
    legacyCopy(text, fallbackInput, done);
  }
}

function legacyCopy(text, input, done) {
  try {
    if (input) {
      input.select();
    }
    if (document.execCommand('copy')) {
      done();
      return;
    }
  } catch (e) {}
  showToast('Copy failed: select the text and press Ctrl+C', 'warning');
}

function formatBytes(bytes, decimals = 2) {
  if (!+bytes) return '0 Bytes';
  const k = 1024;
  const dm = decimals < 0 ? 0 : decimals;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
}

function escapeHtml(str) {
  if (!str) return '';
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// ================= Pane Settings & Border/Color Customization (CommanderDog Spirit) =================
const PANE_COLOR_PRESETS = [
  { id: 'default', name: 'Default', hex: 'rgba(255,255,255,0.2)' },
  { id: 'amber', name: 'Amber', hex: '#f59e0b' },
  { id: 'emerald', name: 'Emerald', hex: '#10b981' },
  { id: 'sky', name: 'Sky Blue', hex: '#38bdf8' },
  { id: 'purple', name: 'Purple', hex: '#c084fc' },
  { id: 'rose', name: 'Rose Red', hex: '#f43f5e' },
  { id: 'indigo', name: 'Indigo', hex: '#6366f1' },
  { id: 'teal', name: 'Teal', hex: '#14b8a6' },
  { id: 'orange', name: 'Orange', hex: '#f97316' }
];

const PANE_COLOR_MAP = {
  default: '#3f3f46',
  amber: '#f59e0b',
  emerald: '#10b981',
  sky: '#38bdf8',
  purple: '#c084fc',
  rose: '#f43f5e',
  indigo: '#6366f1',
  teal: '#14b8a6',
  orange: '#f97316'
};

function getPaneColors() {
  try {
    return JSON.parse(localStorage.getItem('rd_pane_colors')) || { 1: 'default', 2: 'default', 3: 'default', 4: 'default' };
  } catch (e) {
    return { 1: 'default', 2: 'default', 3: 'default', 4: 'default' };
  }
}

function getPaneNames() {
  try {
    return JSON.parse(localStorage.getItem('rd_pane_names')) || {};
  } catch (e) {
    return {};
  }
}

function getPaneBorderWidths() {
  try {
    return JSON.parse(localStorage.getItem('rd_pane_border_widths')) || {};
  } catch (e) {
    return {};
  }
}

function setPaneColorPref(paneIndex, color) {
  const colors = getPaneColors();
  colors[paneIndex] = color;
  localStorage.setItem('rd_pane_colors', JSON.stringify(colors));
  applyPaneStyle(paneIndex);
}

function applyPaneRenameFromInput(paneIndex) {
  const input = document.getElementById(`pane-setting-name-input-${paneIndex}`);
  if (!input) return;
  const name = input.value.trim();
  const names = getPaneNames();
  if (name) {
    names[paneIndex] = name;
  } else {
    delete names[paneIndex];
  }
  localStorage.setItem('rd_pane_names', JSON.stringify(names));
  applyPaneStyle(paneIndex);
  showToast(`Pane ${paneIndex} renamed!`);
}

function resetPaneName(paneIndex) {
  const names = getPaneNames();
  delete names[paneIndex];
  localStorage.setItem('rd_pane_names', JSON.stringify(names));
  applyPaneStyle(paneIndex);
}

function cyclePaneColor(paneIndex) {
  const colors = getPaneColors();
  const cur = colors[paneIndex] || 'default';
  const keys = PANE_COLOR_PRESETS.map(p => p.id);
  let idx = keys.indexOf(cur);
  if (idx === -1) idx = 0;
  const next = keys[(idx + 1) % keys.length];
  setPaneColorPref(paneIndex, next);
}

function applyBorderSettings(borderWidth, ringStyle, paneIndex) {
  if (paneIndex) {
    if (borderWidth) {
      const bws = getPaneBorderWidths();
      bws[paneIndex] = borderWidth;
      localStorage.setItem('rd_pane_border_widths', JSON.stringify(bws));
    }
  } else if (borderWidth) {
    localStorage.setItem('rd_border_width', borderWidth);
  }

  if (ringStyle) {
    localStorage.setItem('rd_ring_style', ringStyle);
  }

  initPaneStyles();
}

function applyPaneStyle(paneIndex) {
  const paneEl = document.getElementById(`pane-${paneIndex}`);
  if (!paneEl) return;

  const colors = getPaneColors();
  const names = getPaneNames();
  const bws = getPaneBorderWidths();
  const globalBw = localStorage.getItem('rd_border_width') || '1px';

  const colorKey = colors[paneIndex] || 'default';
  const hex = PANE_COLOR_MAP[colorKey] || colorKey;
  const bw = bws[paneIndex] || globalBw;
  const customName = names[paneIndex] || `[ ${paneIndex} ]`;

  if (colorKey !== 'default') {
    paneEl.style.setProperty('--pane-custom-border', hex);
    paneEl.style.setProperty('--pane-custom-color', hex);
  } else {
    paneEl.style.removeProperty('--pane-custom-border');
    paneEl.style.removeProperty('--pane-custom-color');
  }

  paneEl.style.borderWidth = bw;
  if (bw === '0px') {
    paneEl.classList.add('borderless-pane');
    paneEl.style.borderRadius = '0px';
  } else {
    paneEl.classList.remove('borderless-pane');
    paneEl.style.borderRadius = '';
  }

  const dot = document.getElementById(`pane-dot-${paneIndex}`);
  if (dot) {
    dot.style.background = (colorKey !== 'default') ? hex : 'var(--woofson-accent)';
  }

  const label = document.getElementById(`pane-label-${paneIndex}`);
  if (label) {
    label.textContent = customName.startsWith('[') ? customName : `[ ${customName} ]`;
  }
}

function initPaneStyles() {
  const globalBw = localStorage.getItem('rd_border_width') || '1px';
  const ringStyle = localStorage.getItem('rd_ring_style') || 'subtle';
  const root = document.documentElement;

  root.style.setProperty('--pane-border-width', globalBw);

  if (ringStyle === 'none') {
    root.style.setProperty('--pane-active-ring-width', '0px');
  } else if (ringStyle === 'bold') {
    root.style.setProperty('--pane-active-ring-width', '2.5px');
  } else if (ringStyle === 'glow') {
    root.style.setProperty('--pane-active-ring-width', '1.5px');
  } else {
    root.style.setProperty('--pane-active-ring-width', '1px');
  }

  for (let i = 1; i <= 4; i++) {
    applyPaneStyle(i);
  }
}

function openPaneSettingsMenu(e, paneIndex) {
  if (e) {
    e.preventDefault();
    e.stopPropagation();
  }
  document.getElementById('pane-settings-popup')?.remove();

  const colors = getPaneColors();
  const names = getPaneNames();
  const bws = getPaneBorderWidths();
  const currentColor = colors[paneIndex] || 'default';
  const currentName = names[paneIndex] || `${paneIndex}`;
  const curBorderWidth = bws[paneIndex] || localStorage.getItem('rd_border_width') || '1px';
  const curRingStyle = localStorage.getItem('rd_ring_style') || 'subtle';

  const isCustomHex = currentColor.startsWith('#') || currentColor.startsWith('rgb');
  const currentHexVal = isCustomHex ? currentColor : '#f59e0b';

  const popup = document.createElement('div');
  popup.id = 'pane-settings-popup';
  popup.className = 'pane-settings-dropdown';

  popup.innerHTML = `
    <div style="padding: 8px 12px; font-weight: 700; font-size: 11px; color: var(--woofson-accent); background: var(--woofson-bg-void); border-bottom: 1px solid var(--woofson-border); display: flex; justify-content: space-between; align-items: center;">
      <span style="display: flex; align-items: center; gap: 6px;">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>
        Pane ${paneIndex} Settings
      </span>
      <span style="font-size: 11px; color: var(--woofson-text-dim); cursor: pointer;" onclick="document.getElementById('pane-settings-popup')?.remove();">✕</span>
    </div>
    
    <div style="padding: 10px 12px; max-height: 440px; overflow-y: auto; display: flex; flex-direction: column; gap: 12px;">
      <!-- 1. Renaming -->
      <div>
        <div style="font-size: 10px; color: var(--woofson-text-muted); font-weight: 700; text-transform: uppercase; margin-bottom: 6px;">Pane Name / Label</div>
        <div style="display: flex; gap: 6px;">
          <input type="text" id="pane-setting-name-input-${paneIndex}" value="${escapeHtml(currentName)}" placeholder="${paneIndex}" style="flex: 1; height: 26px; padding: 0 8px; font-size: 11px; background: var(--woofson-bg-void); border: 1px solid var(--woofson-border); border-radius: 4px; color: var(--woofson-text-main);" onkeydown="if(event.key==='Enter'){ applyPaneRenameFromInput(${paneIndex}); }">
          <button type="button" class="btn btn-sm btn-accent" style="height: 26px; padding: 0 8px; font-size: 10px;" onclick="applyPaneRenameFromInput(${paneIndex})">Save</button>
          <button type="button" class="btn btn-sm btn-secondary" style="height: 26px; padding: 0 6px; font-size: 10px;" title="Reset to default ${paneIndex}" onclick="resetPaneName(${paneIndex}); openPaneSettingsMenu(null, ${paneIndex});">Reset</button>
        </div>
      </div>

      <!-- 2. Border & Header Color Palette -->
      <div style="border-top: 1px solid var(--woofson-border); padding-top: 10px;">
        <div style="font-size: 10px; color: var(--woofson-text-muted); font-weight: 700; text-transform: uppercase; margin-bottom: 6px; display: flex; justify-content: space-between;">
          <span>Border & Accent Color</span>
          <span style="color: var(--woofson-accent); cursor: pointer; text-transform: none; font-weight: 600;" onclick="cyclePaneColor(${paneIndex}); openPaneSettingsMenu(null, ${paneIndex});">Cycle ↻</span>
        </div>
        <div style="display: grid; grid-template-columns: repeat(3, 1fr); gap: 4px; margin-bottom: 8px;">
          ${PANE_COLOR_PRESETS.map(p => {
            const isSelected = currentColor === p.id;
            return `
              <button type="button" class="btn btn-sm ${isSelected ? 'active' : ''}" 
                      style="display: flex; align-items: center; gap: 5px; padding: 4px 6px; font-size: 10px; width: 100%; text-align: left; overflow: hidden;"
                      onclick="setPaneColorPref(${paneIndex}, '${p.id}'); openPaneSettingsMenu(null, ${paneIndex});">
                <span style="display: inline-block; width: 9px; height: 9px; border-radius: 50%; background: ${p.hex}; border: 1px solid rgba(255,255,255,0.25); flex-shrink: 0;"></span>
                <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; ${isSelected ? 'font-weight: 700; color: var(--woofson-accent);' : ''}">${p.name}</span>
              </button>
            `;
          }).join('')}
        </div>

        <div style="display: flex; align-items: center; gap: 6px;">
          <input type="color" id="pane-hex-picker-${paneIndex}" value="${currentHexVal}" style="width: 28px; height: 26px; border: 1px solid var(--woofson-border); border-radius: 4px; padding: 0; background: transparent; cursor: pointer;" oninput="document.getElementById('pane-hex-text-${paneIndex}').value = this.value; setPaneColorPref(${paneIndex}, this.value);">
          <input type="text" id="pane-hex-text-${paneIndex}" value="${isCustomHex ? currentColor : ''}" placeholder="#RRGGBB" style="flex: 1; height: 26px; padding: 0 6px; font-family: var(--font-mono); font-size: 11px; background: var(--woofson-bg-void); border: 1px solid var(--woofson-border); border-radius: 4px; color: var(--woofson-text-main);" onchange="if(this.value){ document.getElementById('pane-hex-picker-${paneIndex}').value = this.value; setPaneColorPref(${paneIndex}, this.value); }">
          <button type="button" class="btn btn-sm btn-accent" style="height: 26px; padding: 0 8px; font-size: 10px;" onclick="const val = document.getElementById('pane-hex-text-${paneIndex}').value; if(val){ setPaneColorPref(${paneIndex}, val); openPaneSettingsMenu(null, ${paneIndex}); }">Apply</button>
        </div>
      </div>

      <!-- 3. Border Width & Ring Settings -->
      <div style="border-top: 1px solid var(--woofson-border); padding-top: 10px;">
        <div style="font-size: 10px; color: var(--woofson-text-muted); font-weight: 700; text-transform: uppercase; margin-bottom: 6px;">Border Width (Pane ${paneIndex})</div>
        <div style="display: grid; grid-template-columns: repeat(5, 1fr); gap: 4px; margin-bottom: 8px;">
          ${['0px', '1px', '2px', '3px', '4px'].map(bw => `
            <button type="button" class="btn btn-sm ${curBorderWidth === bw ? 'active' : ''}" style="padding: 2px 2px; font-size: 9.5px; justify-content: center;" onclick="applyBorderSettings('${bw}', null, ${paneIndex}); openPaneSettingsMenu(null, ${paneIndex});">${bw === '0px' ? '0px (None)' : bw}</button>
          `).join('')}
        </div>

        <div style="font-size: 10px; color: var(--woofson-text-muted); font-weight: 700; text-transform: uppercase; margin-bottom: 6px;">Active Focus Ring</div>
        <div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 4px;">
          ${[['subtle', 'Subtle'], ['bold', 'Bold'], ['glow', 'Glow'], ['none', 'None']].map(([rKey, rName]) => `
            <button type="button" class="btn btn-sm ${curRingStyle === rKey ? 'active' : ''}" style="padding: 2px 4px; font-size: 9.5px; justify-content: center;" onclick="applyBorderSettings(null, '${rKey}'); openPaneSettingsMenu(null, ${paneIndex});">${rName}</button>
          `).join('')}
        </div>
      </div>
    </div>
  `;

  document.body.appendChild(popup);

  const btn = document.getElementById(`pane-badge-btn-${paneIndex}`);
  if (btn) {
    const rect = btn.getBoundingClientRect();
    popup.style.position = 'fixed';
    popup.style.top = `${rect.bottom + 4}px`;
    popup.style.left = `${Math.min(window.innerWidth - 280, Math.max(10, rect.left))}px`;
  }

  const closeHandler = (evt) => {
    if (!popup.contains(evt.target) && (!btn || !btn.contains(evt.target))) {
      popup.remove();
      document.removeEventListener('click', closeHandler);
    }
  };
  setTimeout(() => document.addEventListener('click', closeHandler), 10);
}

// ================= Keyboard Layout & Locale Management =================
function getPaneKbdLayout(paneIndex) {
  const pane = state.panes[paneIndex];
  if (pane && pane.conn && pane.conn.settings_json) {
    try {
      const s = JSON.parse(pane.conn.settings_json);
      if (s.keyboard_layout) return s.keyboard_layout;
    } catch (e) {}
  }
  return localStorage.getItem(`rd_pane_kbd_${paneIndex}`) || localStorage.getItem('rd_default_kbd_layout') || 'no';
}

function setPaneKbdLayout(paneIndex, layoutId) {
  localStorage.setItem(`rd_pane_kbd_${paneIndex}`, layoutId);
  localStorage.setItem('rd_default_kbd_layout', layoutId);
  updatePaneKbdBadge(paneIndex);
  const layout = KEYBOARD_LAYOUTS.find(l => l.id === layoutId);
  showToast(`Keyboard layout set to ${layout ? layout.name : layoutId.toUpperCase()}`);
}

function updatePaneKbdBadge(paneIndex) {
  const layoutId = getPaneKbdLayout(paneIndex);
  const layout = KEYBOARD_LAYOUTS.find(l => l.id === layoutId) || KEYBOARD_LAYOUTS[0];
  const labelEl = document.getElementById(`pane-${paneIndex}-kbd-label`);
  const btnEl = document.getElementById(`pane-${paneIndex}-kbd-btn`);
  if (labelEl) {
    labelEl.textContent = `${layout.flag} ${layout.label}`;
  }
  if (btnEl) {
    btnEl.title = `Keyboard Layout: ${layout.name} (${layout.label}) — Click to change`;
  }
}

function initAllPaneKbdBadges() {
  for (let i = 1; i <= 4; i++) {
    updatePaneKbdBadge(i);
  }
}

function openPaneKbdMenu(e, paneIndex) {
  if (e) {
    e.preventDefault();
    e.stopPropagation();
  }
  document.getElementById('pane-kbd-dropdown')?.remove();

  const currentLayout = getPaneKbdLayout(paneIndex);

  const popup = document.createElement('div');
  popup.id = 'pane-kbd-dropdown';
  popup.className = 'pane-settings-dropdown';

  popup.innerHTML = `
    <div style="padding: 8px 12px; font-weight: 700; font-size: 11px; color: var(--woofson-accent); background: var(--woofson-bg-void); border-bottom: 1px solid var(--woofson-border); display: flex; justify-content: space-between; align-items: center;">
      <span style="display: flex; align-items: center; gap: 6px;">
        ⌨️ Keyboard Layout (Pane ${paneIndex})
      </span>
      <span style="font-size: 11px; color: var(--woofson-text-dim); cursor: pointer;" onclick="document.getElementById('pane-kbd-dropdown')?.remove();">✕</span>
    </div>
    <div style="padding: 6px; display: flex; flex-direction: column; gap: 4px; max-height: 320px; overflow-y: auto;">
      ${KEYBOARD_LAYOUTS.map(l => {
        const isSelected = currentLayout === l.id;
        return `
          <button type="button" class="btn btn-sm ${isSelected ? 'active' : ''}" 
                  style="display: flex; align-items: center; gap: 8px; padding: 6px 8px; font-size: 11px; width: 100%; text-align: left; justify-content: flex-start;"
                  onclick="setPaneKbdLayout(${paneIndex}, '${l.id}'); document.getElementById('pane-kbd-dropdown')?.remove();">
            <span style="font-size: 14px;">${l.flag}</span>
            <span style="flex: 1; ${isSelected ? 'font-weight: 700; color: var(--woofson-accent);' : ''}">${l.name}</span>
            <span style="font-family: var(--font-mono); font-size: 10px; color: var(--woofson-text-dim);">${l.label}</span>
          </button>
        `;
      }).join('')}
    </div>
  `;

  document.body.appendChild(popup);

  const btn = document.getElementById(`pane-${paneIndex}-kbd-btn`);
  if (btn) {
    const rect = btn.getBoundingClientRect();
    popup.style.position = 'fixed';
    popup.style.top = `${rect.bottom + 4}px`;
    popup.style.left = `${Math.min(window.innerWidth - 220, Math.max(10, rect.left - 80))}px`;
    popup.style.width = '220px';
    popup.style.zIndex = '1000';
  }

  const closeHandler = (evt) => {
    if (!popup.contains(evt.target) && (!btn || !btn.contains(evt.target))) {
      popup.remove();
      document.removeEventListener('click', closeHandler);
    }
  };
  setTimeout(() => document.addEventListener('click', closeHandler), 10);
}
