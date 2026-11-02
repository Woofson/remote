# Remote Project Rules & Development Guidelines

All agents and pair-programming sessions working on **Remote** must strictly adhere to the rules and conventions below. This file is the single source of truth for how work is done in this repository.

---

## 1. Product Vision

* **Remote is a 2026 take on Apache Guacamole**: a browser-based remote access gateway for RDP, VNC, SSH, Telnet and local shells.
* **Three non-negotiables**: **lightweight**, **fast**, and **secure**. Every feature is weighed against memory footprint, latency, and attack surface.
* **Workspace model**: many terminals and remote desktops open at once, arranged in one main pane, split across several panes, in tabs, or as floating windows.
* **Out of scope**: native-client orchestration (launching `xfreerdp3` and similar), mesh discovery, and other "client-first launcher" ideas. Remote is the web gateway.

---

## 2. Naming

* **Product name**: **Remote**. It deliberately takes the everyday verb ("to remote into a machine") as its name.
* **`remote` where possible, `remoted` where taken**: use `remote` wherever available. Use `remoted` only where `remote` is already claimed (the crates.io crate and the binary).
* **No `arf-` prefix**: Remote is a standalone product, not a Brum Chewtoy.
* **No `*Dog` gimmick names**: use clean, professional names (Terminal, Clipboard, File Transfer, Session Recording).
* **Creator & Lab**: Bolt J Woofson @ Woofsons Lab ([www.arf.ac](https://www.arf.ac)).

---

## 3. Versioning & Iteration Rules

* **Standard Semantic Versioning (`MAJOR.MINOR.PATCH`)**, no `-rc` suffixes unless preparing a dedicated release candidate.
  * **Patch (`x.y.Z`)**: bug fixes and regressions.
  * **Minor (`x.Y.0`)**: new features or capabilities.
  * **Major (`X.0.0`)**: breaking changes or architectural overhauls.
  * **Local Scratch Iteration**: local test builds may use letter postfixes (`0.2.1a`–`0.2.1z`); these are never committed or published.
* **Single Source of Truth**: the version is defined only in `Cargo.toml` (`[package] version`). The backend should embed it with `env!("CARGO_PKG_VERSION")` and the frontend should read it from the API.
  * **Known gap**: the login page still shows a hardcoded version badge in `frontend/index.html`. Until that is wired up, flag it rather than editing version strings across files.

---

## 4. Rebuild vs Refresh Notifications

The frontend is embedded into the binary, so **frontend changes also need a rebuild** to show up in a running server.

At the end of every response after making changes, clearly state whether:
1. **Rebuild needed** (`cargo build` / `cargo run`): Rust code, dependencies, or any embedded asset (everything in `frontend/`) changed.
2. **Browser hard refresh only** (Ctrl+F5 / Cmd+Shift+R): only when nothing embedded in the binary changed (e.g. `config.toml` read at startup still needs a restart; say so).

---

## 5. Git Workflow, GitHub Issues & Roadmap

* **GitHub Issues on `Woofson/remote` are the single source of truth** for the roadmap, backlog, and bug tracking. Use the GitHub CLI (`gh issue list/view/create/comment/close`).
* Reference issue numbers in commit messages (e.g. `fix(rdp): restore \\tsclient drive mounting (fixes #5)`).
* **Changelog**: update `CHANGELOG.md` only when finalizing and tagging an official release.
* **Release Cadence**: at most one official release per day, unless fixing a critical emergency bug. Routine work uses "push to git" with SemVer bumps in `Cargo.toml`.
* **Command: `"push to git"`**: commit with a clean conventional commit message and push. **No releases, no publishing.**
* **Command: `"push and release"`**: commit, tag, push, then publish to:
  1. **GitHub Release & Tags**.
  2. **Crates.io** (`cargo publish`, crate `remoted`).
  * Further channels (Docker/GHCR, AUR, Windows) are added here only once they exist.
* Never commit, push, tag, or release without one of the commands above or an explicit request.

---

## 6. Scope Discipline & Feature Planning

* Guard against vibe-coding, spontaneous feature creep, and scope bloat.
* When a new feature or architectural change is proposed:
  1. **Check the backlog**: look at open issues and ask whether existing issues should come first.
  2. **Offer a choice**:
     - **Issue first**: file or update a GitHub issue with requirements before implementation.
     - **Build now**: scope lightly and implement in the current session.
     - **Backlog first**: postpone in favour of open bugs or issues.

---

## 7. Professional Tone & Minimal Emoji Policy

* Keep emojis to a strict minimum in responses, documentation, commit messages, and UI text.
* Sharp, direct, high-signal engineering tone.

---

## 8. Viewport & Workspace Terminology

* **Viewport sizes**:
  * `Phone`: `<600px`, single-pane focus, minimal micro-text.
  * `Tablet`: `600px–1024px`, adaptive single/dual pane layouts, legible touch hierarchy.
  * `PC`: `>1024px`, full multi-pane grid, resizable panes, floating windows.
* **Workspace terms**:
  * **Pane**: one area of the session grid (Single, Dual, Triple, Quad layouts; `Alt+1` to `Alt+4`).
  * **Tab**: a session inside a pane; several sessions can share one pane as tabs. Settings modal tabs are also called tabs.
  * **Floating window**: a session detached from the grid into a draggable, resizable window.
  * **Session**: one live connection (RDP, VNC, SSH, Telnet, local shell).

---

## 9. Design Language (Woofson Design System, shared with Brum)

Whenever creating or modifying windows, modals, panes, or toolbars:

### 1. Primary Window & Modal Headers (`42px` min-height)
* **Full-length drag handle**: the whole header is a drag handle (`cursor: grab;`, `:active { cursor: grabbing; }`).
* **Uniform 28px sizing**: header buttons, selects, and icons are `28px x 28px` (or `28px` high), `border-radius: var(--radius)` (`6px`), `14px` icons.
* **Right-aligned layout switchers**: icon-only, with tooltips (`title="..."`), active state uses the amber glow (`rgba(245, 158, 11, 0.18)` + `var(--accent)`).
* **Flat, stealthy window controls** (Minimize, Maximize, Dock, Float, Close):
  * Default: `border: 1px solid transparent; background: transparent; opacity: 0.7;`
  * Hover: `rgba(255, 255, 255, 0.08); opacity: 1;`
  * Close hover: `rgba(239, 68, 68, 0.2); color: #ef4444;`

### 2. Sub-Headers & Inner Toolbars (`26px` standard)
* Pane headers, secondary toolbars, and action bars use `26px x 26px` buttons, `6px` radius, `13px` icons.
* Float/undock buttons are icon-only (`external-link`), no "Float" text label.

### 3. Pane + Floating Dual Mode
* Every session view and utility should support both **docked in a pane** and **floating window** modes, with state remembered.

### 4. Session & Terminal Lifecycle
* Terminals cleanly handle EOF (`Ctrl+D`), `logout`, and `exit`: close the session and reset state.
* Remember user preferences (layouts, pane settings, keyboard layouts) via `localStorage` or backend storage.
* Terminal uses the bundled **JetBrainsMono Nerd Font** and must render `eza --icons`, `starship`, `fish`, and powerline glyphs without missing boxes.

### 5. Palette
* Woofson Golden Amber palette (see README): accent `#f59e0b`, dark surfaces `#121214` / `#18181b` / `#202024` / `#27272a`.

---

## 10. Lightweight, Fast & Secure Engineering

* **Single binary**: pure Rust backend with the frontend embedded. No Java, no `guacd`, no heavy runtime dependencies.
* **Resource budget**: keep idle memory low (target tens of MB) and startup near-instant. Justify every new dependency.
* **Security defaults**: no secrets committed to the repository, least-privilege default roles, encrypted stored credentials, audit logging. Security regressions are treated as critical bugs.

---

## 11. Authorship, Attribution & AI Anonymity

* **Sole author**: all code, commits, tags, releases, issues, and comments are authored as **Bolt J Woofson** using the repository's configured git identity.
* **No AI attribution, ever**: no `Co-Authored-By` trailers, "Generated with ..." footers, bot signatures, or AI credits anywhere: commits, pull requests, issues, release notes, changelogs.
* **No mentions of AI assistants or their vendors** in source code, comments, documentation, UI text, filenames, or committed configuration.
* **Agent-specific instruction files** (other than this one) stay local and untracked, excluded via `.git/info/exclude`, not `.gitignore`.

---

## 12. Plain-Language Change Explanations

The project owner is not a programmer: they understand concepts but do not read code. For every change, explain in plain language:
1. **What** changed (which feature or behaviour, not just which file).
2. **Why** it changed (the problem it solves or the goal it serves).
3. **Consequences**: side effects, risks, behaviour differences, rebuilds, config changes, or follow-ups.

Avoid code snippets and jargon unless asked; when a technical term is unavoidable, briefly explain it.

---

## 13. Working Discipline & Honesty

* **Ask before changing**: before editing any project file, explain what will change, why, and the consequences (section 12), and wait for approval. Read-only investigation needs no approval. Approval covers only what was described; anything new needs a fresh check-in.
* **No loops**: never ask the owner to repeat the same step or test hoping for a different result. If a fix failed twice, stop trying variations.
* **No repetition**: say things once, clearly; if it did not land, rephrase or ask what is unclear.
* **Say "I don't know"**: separate what was verified from what is believed.
* **Stop, assess, rethink**: when stuck, summarise what is known, what was tried, and what is unclear, then propose a different direction or ask.
* **Ask questions**: when requirements or priorities are unclear, ask short, specific questions before building.
* **Stay current**: verify against the actual code, GitHub issues, and current documentation rather than memory.
