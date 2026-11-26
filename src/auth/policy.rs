//! Session rules set by admins (login lifetime, idle timeout, 2FA requirements),
//! live session state (idle / locked), and a brake on repeated failed logins.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const ROLES: [&str; 3] = ["admin", "operator", "viewer"];

/// Admin-editable session and sign-in rules (stored in the database).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityPolicy {
    /// After this many hours a login ends and the user signs in again.
    pub max_login_hours: i64,
    /// Minutes without activity before the idle action; 0 = never.
    pub idle_minutes: i64,
    /// "lock" (live sessions keep running behind the lock screen) or "logout".
    pub idle_action: String,
    /// Also ask for a 2FA code when unlocking the lock screen.
    pub lock_requires_2fa: bool,
    /// Roles whose local accounts must use two-factor authentication.
    pub require_2fa_roles: Vec<String>,
    /// Sharing live sessions (#32): "full" (watch or control), "view" (watch-only links) or "off".
    pub sharing: String,
    /// Users may share session recordings with other users and groups (#31). Off by default.
    pub recording_sharing: bool,
    /// Days a browser may skip the 2FA code (sign-in and unlock) after "remember this
    /// device"; 0 = off (the default).
    pub remember_2fa_days: i64,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_login_hours: 24,
            idle_minutes: 0,
            idle_action: "lock".into(),
            lock_requires_2fa: false,
            require_2fa_roles: Vec::new(),
            sharing: "full".into(),
            recording_sharing: false,
            remember_2fa_days: 0,
        }
    }
}

impl SecurityPolicy {
    /// Defaults for a fresh install: the login lifetime comes from config.toml.
    pub fn with_default_lifetime(hours: i64) -> Self {
        Self { max_login_hours: hours.clamp(1, 720), ..Self::default() }
    }

    /// Check values from the admin form; returns a message for the first problem.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=720).contains(&self.max_login_hours) {
            return Err("Login lifetime must be between 1 and 720 hours".into());
        }
        if !(0..=1440).contains(&self.idle_minutes) {
            return Err("Idle timeout must be between 0 (off) and 1440 minutes".into());
        }
        if self.idle_action != "lock" && self.idle_action != "logout" {
            return Err("Idle action must be lock or logout".into());
        }
        if let Some(r) = self.require_2fa_roles.iter().find(|r| !ROLES.contains(&r.as_str())) {
            return Err(format!("Unknown role '{}'", r));
        }
        if !(0..=90).contains(&self.remember_2fa_days) {
            return Err("Remembering devices for 2FA must be between 0 (off) and 90 days".into());
        }
        if !matches!(self.sharing.as_str(), "full" | "view" | "off") {
            return Err("Session sharing must be full, view or off".into());
        }
        Ok(())
    }

    /// May users share their sessions at all, and with control?
    pub fn share_rights(&self) -> (bool, bool) {
        match self.sharing.as_str() {
            "off" => (false, false),
            "view" => (true, false),
            _ => (true, true),
        }
    }

    pub fn requires_2fa(&self, role: &str) -> bool {
        self.require_2fa_roles.iter().any(|r| r == role)
    }
}

/// What the session tracker decided for a request.
#[derive(Debug, PartialEq)]
pub enum SessionStatus {
    Active,
    Locked,
    /// Ended by the idle timeout (idle action "logout").
    Expired,
    /// Ended by logging out (for example in another tab).
    LoggedOut,
}

struct SessionState {
    last_activity: Instant,
    locked: bool,
    /// Ended by the idle timeout; stays ended until pruned.
    expired: bool,
    /// Ended by logging out; stays ended until pruned.
    logged_out: bool,
}

impl SessionState {
    fn fresh(locked: bool) -> Self {
        Self { last_activity: Instant::now(), locked, expired: false, logged_out: false }
    }
}

/// Saved login states are kept this long: the longest login lifetime an admin can set
/// (720 hours), so raising the lifetime later cannot bring an ended login back.
const STORE_KEEP_SECS: i64 = 720 * 3600;

/// Live per-login state. Idle time is kept in memory (a restart starts it again); locks and
/// ended logins are also saved in the database when a store is given, so they survive a restart.
#[derive(Default)]
pub struct SessionTracker {
    sessions: Mutex<HashMap<String, SessionState>>,
    store: Option<crate::db::Database>,
}

impl SessionTracker {
    /// A tracker that saves locks and ended logins, starting from what was saved before.
    pub fn with_store(db: crate::db::Database) -> Self {
        let mut map = HashMap::new();
        match db.load_login_states(STORE_KEEP_SECS) {
            Ok(rows) => {
                for (sid, state) in rows {
                    let mut s = SessionState::fresh(state == "locked");
                    s.expired = state == "expired";
                    s.logged_out = state == "logged_out";
                    map.insert(sid, s);
                }
            }
            Err(e) => tracing::warn!("Could not read saved login states: {}", e),
        }
        Self { sessions: Mutex::new(map), store: Some(db) }
    }

    /// Save a login's new state (call without holding the map lock).
    fn save(&self, sid: &str, state: Option<&str>) {
        if let Some(db) = &self.store {
            if let Err(e) = db.set_login_state(sid, state) {
                tracing::warn!("Could not save login state: {}", e);
            }
        }
    }

    /// Look up a login and apply the idle rule. Unknown logins start counting now.
    pub fn check(&self, sid: &str, policy: &SecurityPolicy) -> SessionStatus {
        if sid.is_empty() {
            return SessionStatus::Active;
        }
        let (status, changed) = {
            let mut map = self.sessions.lock();
            let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
            if s.logged_out {
                return SessionStatus::LoggedOut;
            }
            if s.expired {
                return SessionStatus::Expired;
            }
            let mut changed = None;
            if policy.idle_minutes > 0
                && s.last_activity.elapsed() > Duration::from_secs(policy.idle_minutes as u64 * 60)
            {
                if policy.idle_action == "logout" {
                    s.expired = true;
                    changed = Some("expired");
                } else if !s.locked {
                    s.locked = true;
                    changed = Some("locked");
                }
            }
            let status = if s.expired {
                SessionStatus::Expired
            } else if s.locked {
                SessionStatus::Locked
            } else {
                SessionStatus::Active
            };
            (status, changed)
        };
        if let Some(state) = changed {
            self.save(sid, Some(state));
        }
        status
    }

    /// The user did something (typing, clicking). Ignored while locked.
    pub fn touch(&self, sid: &str) {
        if let Some(s) = self.sessions.lock().get_mut(sid) {
            if !s.locked && !s.expired && !s.logged_out {
                s.last_activity = Instant::now();
            }
        }
    }

    pub fn lock(&self, sid: &str) {
        let save = {
            let mut map = self.sessions.lock();
            let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(true));
            s.locked = true;
            !s.expired && !s.logged_out
        };
        if save {
            self.save(sid, Some("locked"));
        }
    }

    /// Unlock after a successful password check (an idle-ended login stays ended).
    pub fn unlock(&self, sid: &str) {
        let save = {
            let mut map = self.sessions.lock();
            let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
            if !s.expired && !s.logged_out {
                s.locked = false;
                s.last_activity = Instant::now();
                true
            } else {
                false
            }
        };
        if save {
            self.save(sid, None);
        }
    }

    /// Log out: this login can no longer be used.
    pub fn end(&self, sid: &str) {
        if sid.is_empty() {
            return;
        }
        {
            let mut map = self.sessions.lock();
            let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
            s.logged_out = true;
        }
        self.save(sid, Some("logged_out"));
    }

    /// Drop entries idle for longer than any login can live (keeps memory small).
    pub fn prune(&self, max_age: Duration) {
        self.sessions.lock().retain(|_, s| s.last_activity.elapsed() < max_age);
    }
}

struct Failures {
    count: u32,
    blocked_until: Option<Instant>,
}

/// Slows down guessing: after 5 failed attempts for an account, further attempts are
/// refused for 30 s, doubling with each further failure up to 15 minutes.
#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, Failures>>,
}

const FREE_ATTEMPTS: u32 = 5;
const BASE_BLOCK_SECS: u64 = 30;
const MAX_BLOCK_SECS: u64 = 15 * 60;

impl LoginLimiter {
    fn key(account: &str) -> String {
        account.trim().to_lowercase()
    }

    /// Seconds to wait if the account is currently blocked.
    pub fn blocked_for(&self, account: &str) -> Option<u64> {
        let map = self.failures.lock();
        let until = map.get(&Self::key(account))?.blocked_until?;
        let now = Instant::now();
        (until > now).then(|| (until - now).as_secs().max(1))
    }

    pub fn record_failure(&self, account: &str) {
        let mut map = self.failures.lock();
        let f = map.entry(Self::key(account)).or_insert(Failures { count: 0, blocked_until: None });
        f.count += 1;
        if f.count >= FREE_ATTEMPTS {
            let extra = (f.count - FREE_ATTEMPTS).min(10);
            let secs = (BASE_BLOCK_SECS << extra).min(MAX_BLOCK_SECS);
            f.blocked_until = Some(Instant::now() + Duration::from_secs(secs));
        }
    }

    pub fn record_success(&self, account: &str) {
        self.failures.lock().remove(&Self::key(account));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_rule() {
        let mut p = SecurityPolicy::default();
        assert_eq!(p.share_rights(), (true, true));
        p.sharing = "view".into();
        assert_eq!(p.share_rights(), (true, false));
        p.sharing = "off".into();
        assert_eq!(p.share_rights(), (false, false));
        assert!(p.validate().is_ok());
        p.sharing = "maybe".into();
        assert!(p.validate().is_err());
        // Settings saved before sharing existed load as "full".
        let old: SecurityPolicy = serde_json::from_str(r#"{"max_login_hours":24}"#).unwrap();
        assert_eq!(old.sharing, "full");
    }

    #[test]
    fn limiter_blocks_after_free_attempts() {
        let l = LoginLimiter::default();
        for _ in 0..FREE_ATTEMPTS - 1 {
            l.record_failure("Bob");
        }
        assert!(l.blocked_for("bob").is_none());
        l.record_failure("bob");
        assert!(l.blocked_for("BOB").is_some());
        l.record_success("bob");
        assert!(l.blocked_for("bob").is_none());
    }

    #[test]
    fn tracker_locks_and_unlocks() {
        let t = SessionTracker::default();
        let p = SecurityPolicy::default();
        assert_eq!(t.check("a", &p), SessionStatus::Active);
        t.lock("a");
        assert_eq!(t.check("a", &p), SessionStatus::Locked);
        t.unlock("a");
        assert_eq!(t.check("a", &p), SessionStatus::Active);
        t.end("a");
        assert_eq!(t.check("a", &p), SessionStatus::LoggedOut);
        t.unlock("a");
        assert_eq!(t.check("a", &p), SessionStatus::LoggedOut, "stays ended");
    }

    #[test]
    fn tracker_survives_restart() {
        let db = crate::db::Database::new_in_memory().unwrap();
        let p = SecurityPolicy::default();
        {
            let t = SessionTracker::with_store(db.clone());
            t.end("out");
            t.lock("locked");
            t.lock("unlocked");
            t.unlock("unlocked");
        }
        let t = SessionTracker::with_store(db);
        assert_eq!(t.check("out", &p), SessionStatus::LoggedOut);
        assert_eq!(t.check("locked", &p), SessionStatus::Locked);
        assert_eq!(t.check("unlocked", &p), SessionStatus::Active);
        assert_eq!(t.check("new", &p), SessionStatus::Active);
    }

    #[test]
    fn policy_validation() {
        let mut p = SecurityPolicy::default();
        assert!(p.validate().is_ok());
        p.idle_action = "nap".into();
        assert!(p.validate().is_err());
        p = SecurityPolicy { require_2fa_roles: vec!["root".into()], ..Default::default() };
        assert!(p.validate().is_err());
    }
}
