//! Connection pools (#39): several equivalent machines behind one entry in the connections
//! list. A pool is a connection with protocol "pool"; its settings name the member
//! connections. Connecting to it sends the user to the member with the fewest open Remote
//! sessions, skipping machines that do not answer, and back to the same machine when
//! "keep me on the same machine" is on. Only admins create and change pools.

use super::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Duration;

/// How long a member may take to answer before it counts as off.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// A pool's settings (`settings_json["pool"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct PoolSettings {
    /// Protocol of the members (all the same).
    pub protocol: String,
    pub members: Vec<String>,
    pub sticky: bool,
}

pub fn pool_settings(settings_json: &str) -> Option<PoolSettings> {
    let v: Value = serde_json::from_str(settings_json).ok()?;
    let p = v.get("pool")?;
    let protocol = p.get("protocol")?.as_str()?.to_string();
    let members = p
        .get("members")?
        .as_array()?
        .iter()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let sticky = p.get("sticky").and_then(|s| s.as_bool()).unwrap_or(protocol == "rdp");
    Some(PoolSettings { protocol, members, sticky })
}

/// Check a pool before it is saved: members exist, are not pools and share one protocol.
pub fn validate_pool(state: &AppState, settings_json: &str) -> Result<PoolSettings, String> {
    let p = pool_settings(settings_json).ok_or_else(|| "Choose the machines that belong to the pool".to_string())?;
    if !matches!(p.protocol.as_str(), "rdp" | "vnc" | "ssh" | "telnet" | "local_pty") {
        return Err("Choose the protocol of the machines in the pool".into());
    }
    if p.members.is_empty() {
        return Err("A pool needs at least one machine".into());
    }
    for id in &p.members {
        let m = state
            .db
            .get_connection_raw(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "One of the chosen machines no longer exists".to_string())?;
        if m.protocol == "pool" {
            return Err(format!("{} is a pool itself; pools cannot contain pools", m.name));
        }
        if m.protocol != p.protocol {
            return Err(format!("{} uses another protocol than the pool", m.name));
        }
    }
    Ok(p)
}

/// Open Remote sessions per connection: the "how busy" of pool members.
#[derive(Default)]
pub struct ActiveSessions(Mutex<HashMap<String, usize>>);

pub struct ActiveGuard {
    owner: Arc<ActiveSessions>,
    id: String,
}

impl ActiveSessions {
    pub fn count(&self, id: &str) -> usize {
        self.0.lock().get(id).copied().unwrap_or(0)
    }

    /// Count a session until the returned guard is dropped (the session ends).
    pub fn start(self: &Arc<Self>, id: &str) -> ActiveGuard {
        *self.0.lock().entry(id.to_string()).or_insert(0) += 1;
        ActiveGuard { owner: self.clone(), id: id.to_string() }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut map = self.owner.0.lock();
        if let Some(n) = map.get_mut(&self.id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                map.remove(&self.id);
            }
        }
    }
}

/// Where a pool sends the user.
pub enum PoolChoice {
    /// This member answers.
    Member(ConnectionRecord),
    /// Nothing answers: wake this member first (it has Wake-on-LAN set up).
    Wake(ConnectionRecord, crate::protocols::wol::WakeSettings),
}

/// Pick a member: the remembered one when it answers and "same machine" is on, otherwise the
/// answering member with the fewest open sessions (ties: the first in the pool's order).
pub async fn choose_member(state: &AppState, pool: &ConnectionRecord, user_id: &str) -> Result<PoolChoice, String> {
    let settings = pool_settings(&pool.settings_json).ok_or_else(|| "This pool has no machines".to_string())?;
    let members: Vec<ConnectionRecord> = settings
        .members
        .iter()
        .filter_map(|id| state.db.get_connection_raw(id).ok().flatten())
        .filter(|m| m.protocol == settings.protocol)
        .collect();
    if members.is_empty() {
        return Err("This pool has no machines".into());
    }
    // Ask all members at once whether they answer.
    let probes = members.iter().map(|m| {
        let (host, port, local) = (m.host.clone(), m.port, m.protocol == "local_pty");
        async move { local || crate::protocols::wol::reachable(&host, port, PROBE_TIMEOUT).await }
    });
    let online: Vec<bool> = futures_util::future::join_all(probes).await;

    if settings.sticky {
        if let Ok(Some(last)) = state.db.pool_sticky_member(&pool.id, user_id) {
            if let Some(i) = members.iter().position(|m| m.id == last) {
                if online[i] {
                    return Ok(PoolChoice::Member(members[i].clone()));
                }
            }
        }
    }
    let best = members
        .iter()
        .enumerate()
        .filter(|(i, _)| online[*i])
        .min_by_key(|(i, m)| (state.active.count(&m.id), *i))
        .map(|(_, m)| m.clone());
    if let Some(m) = best {
        return Ok(PoolChoice::Member(m));
    }
    for m in &members {
        if let Some(w) = crate::protocols::wol::WakeSettings::from_settings(&m.settings_json) {
            return Ok(PoolChoice::Wake(m.clone(), w));
        }
    }
    Err(format!("None of the {} machines in \"{}\" can be reached right now.", members.len(), pool.name))
}

/// Remember the member for "keep me on the same machine".
pub fn remember(state: &AppState, pool: &ConnectionRecord, user_id: &str, member_id: &str) {
    if pool_settings(&pool.settings_json).is_some_and(|s| s.sticky) {
        let _ = state.db.set_pool_sticky_member(&pool.id, user_id, member_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_counts() {
        let s = pool_settings(r#"{"pool":{"protocol":"rdp","members":["a","b"]}}"#).unwrap();
        assert_eq!(s.members, vec!["a", "b"]);
        assert!(s.sticky, "RDP pools keep users on the same machine by default");
        let s = pool_settings(r#"{"pool":{"protocol":"ssh","members":["a"]}}"#).unwrap();
        assert!(!s.sticky, "terminal pools do not");
        assert!(pool_settings(r#"{"wol_mac":"x"}"#).is_none());

        let active = Arc::new(ActiveSessions::default());
        let g1 = active.start("a");
        let g2 = active.start("a");
        assert_eq!(active.count("a"), 2);
        drop(g1);
        assert_eq!(active.count("a"), 1);
        drop(g2);
        assert_eq!(active.count("a"), 0);
    }
}
