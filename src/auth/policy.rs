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
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_login_hours: 24,
            idle_minutes: 0,
            idle_action: "lock".into(),
            lock_requires_2fa: false,
            require_2fa_roles: Vec::new(),
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
        Ok(())
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
}

struct SessionState {
    last_activity: Instant,
    locked: bool,
    /// Ended by the idle timeout; stays ended until pruned.
    expired: bool,
}

impl SessionState {
    fn fresh(locked: bool) -> Self {
        Self { last_activity: Instant::now(), locked, expired: false }
    }
}

/// Live per-login state, kept in memory (a server restart forgets idle time and locks).
#[derive(Default)]
pub struct SessionTracker {
    sessions: Mutex<HashMap<String, SessionState>>,
}

impl SessionTracker {
    /// Look up a login and apply the idle rule. Unknown logins start counting now.
    pub fn check(&self, sid: &str, policy: &SecurityPolicy) -> SessionStatus {
        if sid.is_empty() {
            return SessionStatus::Active;
        }
        let mut map = self.sessions.lock();
        let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
        if s.expired {
            return SessionStatus::Expired;
        }
        if policy.idle_minutes > 0
            && s.last_activity.elapsed() > Duration::from_secs(policy.idle_minutes as u64 * 60)
        {
            if policy.idle_action == "logout" {
                s.expired = true;
                return SessionStatus::Expired;
            }
            s.locked = true;
        }
        if s.locked { SessionStatus::Locked } else { SessionStatus::Active }
    }

    /// The user did something (typing, clicking). Ignored while locked.
    pub fn touch(&self, sid: &str) {
        if let Some(s) = self.sessions.lock().get_mut(sid) {
            if !s.locked && !s.expired {
                s.last_activity = Instant::now();
            }
        }
    }

    pub fn lock(&self, sid: &str) {
        let mut map = self.sessions.lock();
        let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(true));
        s.locked = true;
    }

    /// Unlock after a successful password check (an idle-ended login stays ended).
    pub fn unlock(&self, sid: &str) {
        let mut map = self.sessions.lock();
        let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
        if !s.expired {
            s.locked = false;
            s.last_activity = Instant::now();
        }
    }

    /// Log out: this login can no longer be used.
    pub fn end(&self, sid: &str) {
        if sid.is_empty() {
            return;
        }
        let mut map = self.sessions.lock();
        let s = map.entry(sid.to_string()).or_insert_with(|| SessionState::fresh(false));
        s.expired = true;
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
        assert_eq!(t.check("a", &p), SessionStatus::Expired);
        assert_eq!(t.check("a", &p), SessionStatus::Expired, "stays ended");
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
