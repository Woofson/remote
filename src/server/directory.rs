//! Directory sign-in (#34): LDAP and Active Directory accounts sign in to Remote with their
//! directory password, from the normal login form. The settings live in the database
//! (Admin > Directory); the service account's password is encrypted with the server secret.
//!
//! Sign-in: bind as the service account, find the person with the user filter, bind as that
//! person with the password they typed, then read their groups (as the service account again).
//! The role comes from the groups at every sign-in.

use super::*;
use ldap3::{ldap_escape, Ldap, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry};
use serde::Serialize;
use std::time::Duration;

pub const DIRECTORY_SETTINGS_KEY: &str = "directory_settings";
const TIMEOUT: Duration = Duration::from_secs(10);
/// LDAP result code for a refused password.
const INVALID_CREDENTIALS: u32 = 49;
/// Active Directory: userAccountControl flag for a disabled account.
const AD_ACCOUNT_DISABLED: u64 = 0x2;
/// Active Directory: matching rule that follows groups inside groups.
const AD_IN_CHAIN: &str = "1.2.840.113556.1.4.1941";

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg })))
}

fn db_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

/// Directory settings, stored in the database (the service account password encrypted).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectorySettings {
    pub enabled: bool,
    /// "ad" (Active Directory: groups inside groups, disabled accounts) or "ldap" (others).
    pub kind: String,
    pub host: String,
    /// 0 = the usual port for the connection type.
    pub port: u16,
    /// "ldaps", "starttls" or "none".
    pub security: String,
    /// Check the server's certificate (turn off only for a self-made certificate).
    pub verify_cert: bool,
    /// Service account used to look people up; empty = anonymous.
    pub bind_dn: String,
    /// Encrypted with the server secret; never sent to the browser.
    pub bind_password_enc: Option<String>,
    /// Where to look for people.
    pub user_base: String,
    /// Search filter; `{username}` is replaced by the (escaped) name typed at sign-in.
    pub user_filter: String,
    /// Attribute that becomes the Remote username.
    pub username_attr: String,
    pub name_attr: String,
    pub email_attr: String,
    /// Where to look for groups; empty = the same place as people.
    pub group_base: String,
    /// Directory groups (name or full DN) that make someone Admin or Operator.
    pub admin_group: String,
    pub operator_group: String,
    /// Only members of this group may sign in; empty = everyone the filter finds.
    pub required_group: String,
    /// Role for everyone else.
    pub default_role: String,
}

impl Default for DirectorySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: "ad".into(),
            host: String::new(),
            port: 0,
            security: "ldaps".into(),
            verify_cert: true,
            bind_dn: String::new(),
            bind_password_enc: None,
            user_base: String::new(),
            user_filter: "(&(objectCategory=person)(objectClass=user)(|(sAMAccountName={username})(userPrincipalName={username})))".into(),
            username_attr: "sAMAccountName".into(),
            name_attr: "displayName".into(),
            email_attr: "mail".into(),
            group_base: String::new(),
            admin_group: "Remote-Admins".into(),
            operator_group: "Remote-Operators".into(),
            required_group: String::new(),
            default_role: "viewer".into(),
        }
    }
}

impl DirectorySettings {
    pub fn load(db: &Database) -> Self {
        db.get_setting(DIRECTORY_SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    /// Switched on and complete enough to try.
    pub fn ready(&self) -> bool {
        self.enabled && !self.host.trim().is_empty() && !self.user_base.trim().is_empty()
    }

    fn url(&self) -> String {
        let (scheme, default_port) = if self.security == "ldaps" { ("ldaps", 636) } else { ("ldap", 389) };
        let port = if self.port == 0 { default_port } else { self.port };
        let host = self.host.trim();
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{}]", host) } else { host.to_string() };
        format!("{}://{}:{}", scheme, host, port)
    }

    fn group_base(&self) -> &str {
        if self.group_base.trim().is_empty() { self.user_base.trim() } else { self.group_base.trim() }
    }

    /// The role for these groups, or None when the account may not sign in.
    pub fn role_for(&self, groups: &DirectoryGroups) -> Option<String> {
        if !self.required_group.trim().is_empty() && !groups.contains(&self.required_group) {
            return None;
        }
        Some(if !self.admin_group.trim().is_empty() && groups.contains(&self.admin_group) {
            "admin".into()
        } else if !self.operator_group.trim().is_empty() && groups.contains(&self.operator_group) {
            "operator".into()
        } else {
            self.default_role.clone()
        })
    }
}

/// The groups a person is in, by name (cn) and full DN.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DirectoryGroups {
    pub names: Vec<String>,
    #[serde(skip)]
    pub dns: Vec<String>,
}

impl DirectoryGroups {
    /// Is the person in this group? Accepts a group name or a full DN, any letter case.
    pub fn contains(&self, wanted: &str) -> bool {
        let w = wanted.trim();
        !w.is_empty()
            && (self.names.iter().any(|n| n.eq_ignore_ascii_case(w)) || self.dns.iter().any(|d| d.eq_ignore_ascii_case(w)))
    }

    fn add(&mut self, dn: &str, name: Option<&str>) {
        if self.dns.iter().any(|d| d.eq_ignore_ascii_case(dn)) {
            return;
        }
        self.dns.push(dn.to_string());
        let name = name.map(str::to_string).unwrap_or_else(|| first_rdn_value(dn));
        self.names.push(name);
    }
}

/// "CN=Remote-Admins,OU=Groups,DC=example" -> "Remote-Admins".
fn first_rdn_value(dn: &str) -> String {
    dn.split(',').next().and_then(|r| r.split_once('=')).map(|(_, v)| v.trim().to_string()).unwrap_or_else(|| dn.to_string())
}

/// A person found in the directory whose password was accepted.
#[derive(Debug, Clone, Serialize)]
pub struct DirectoryUser {
    pub dn: String,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub groups: DirectoryGroups,
}

#[derive(Debug)]
pub enum DirError {
    /// Could not connect, or the server stopped answering.
    Unreachable(String),
    /// The settings are wrong (for example the service account was refused).
    Setup(String),
    NotFound,
    Ambiguous,
    Disabled,
    WrongPassword,
}

impl DirError {
    /// Text for the admin's Test button and the activity log.
    pub fn describe(&self) -> String {
        match self {
            DirError::Unreachable(e) => format!("Could not reach the directory server: {}", e),
            DirError::Setup(e) => e.clone(),
            DirError::NotFound => "No account with that name was found".into(),
            DirError::Ambiguous => "More than one account matches that name; make the user filter stricter".into(),
            DirError::Disabled => "The account is disabled in the directory".into(),
            DirError::WrongPassword => "Wrong password".into(),
        }
    }
}

fn ldap_failure(e: LdapError) -> DirError {
    match e {
        LdapError::LdapResult { result } => DirError::Setup(format!("The directory refused the request ({}: {})", result.rc, result.text)),
        other => DirError::Unreachable(other.to_string()),
    }
}

/// Run one directory step with the time limit.
async fn timed<T, F>(f: F) -> Result<T, DirError>
where
    F: std::future::Future<Output = Result<T, LdapError>>,
{
    match tokio::time::timeout(TIMEOUT, f).await {
        Ok(r) => r.map_err(ldap_failure),
        Err(_) => Err(DirError::Unreachable("no answer within 10 seconds".into())),
    }
}

fn service_password(s: &DirectorySettings, secret: &str) -> Result<String, DirError> {
    match &s.bind_password_enc {
        Some(enc) => decrypt_secret(enc, secret).map_err(|_| DirError::Setup("Could not read the saved service account password; enter it again".into())),
        None => Ok(String::new()),
    }
}

/// Connect (with StartTLS when chosen) and sign in as the service account.
async fn connect(s: &DirectorySettings, service_pw: &str) -> Result<Ldap, DirError> {
    let settings = LdapConnSettings::new()
        .set_conn_timeout(TIMEOUT)
        .set_starttls(s.security == "starttls")
        .set_no_tls_verify(!s.verify_cert);
    let (conn, ldap) = match tokio::time::timeout(TIMEOUT + Duration::from_secs(2), LdapConnAsync::with_settings(settings, &s.url())).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(DirError::Unreachable(e.to_string())),
        Err(_) => return Err(DirError::Unreachable("no answer within 10 seconds".into())),
    };
    ldap3::drive!(conn);
    let mut ldap = ldap;
    service_bind(&mut ldap, s, service_pw).await?;
    Ok(ldap)
}

async fn service_bind(ldap: &mut Ldap, s: &DirectorySettings, service_pw: &str) -> Result<(), DirError> {
    let res = timed(ldap.simple_bind(s.bind_dn.trim(), service_pw)).await?;
    if res.rc == INVALID_CREDENTIALS {
        return Err(DirError::Setup("The directory refused the service account (check its name and password)".into()));
    }
    if res.rc != 0 {
        return Err(DirError::Setup(format!("The directory refused the service account ({}: {})", res.rc, res.text)));
    }
    Ok(())
}

fn attr(entry: &SearchEntry, name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return None;
    }
    entry
        .attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name.trim()))
        .and_then(|(_, v)| v.first())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn attr_all(entry: &SearchEntry, name: &str) -> Vec<String> {
    entry.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// Find exactly one person for the name typed at sign-in.
async fn find_user(ldap: &mut Ldap, s: &DirectorySettings, username: &str) -> Result<SearchEntry, DirError> {
    let filter = s.user_filter.trim().replace("{username}", &ldap_escape(username));
    let attrs = vec![
        s.username_attr.trim(),
        s.name_attr.trim(),
        s.email_attr.trim(),
        "memberOf",
        "userAccountControl",
    ];
    let attrs: Vec<&str> = attrs.into_iter().filter(|a| !a.is_empty()).collect();
    let res = timed(ldap.search(s.user_base.trim(), Scope::Subtree, &filter, attrs)).await?;
    // Active Directory answers searches at the top of the domain with referrals as well: skip them.
    let entries: Vec<SearchEntry> = res.0.into_iter().filter(|e| !e.is_ref()).map(SearchEntry::construct).collect();
    if res.1.rc != 0 && entries.is_empty() {
        return Err(DirError::Setup(format!(
            "Searching for people failed ({}: {}). Check where to look for people and the user filter",
            res.1.rc, res.1.text
        )));
    }
    match entries.len() {
        0 => Err(DirError::NotFound),
        1 => Ok(entries.into_iter().next().unwrap()),
        _ => Err(DirError::Ambiguous),
    }
}

/// The person's groups: in Active Directory including groups inside groups.
async fn find_groups(ldap: &mut Ldap, s: &DirectorySettings, entry: &SearchEntry, username: &str) -> Result<DirectoryGroups, DirError> {
    let mut groups = DirectoryGroups::default();
    let dn = ldap_escape(&entry.dn).into_owned();
    let filter = if s.kind == "ad" {
        format!("(&(objectClass=group)(member:{}:={}))", AD_IN_CHAIN, dn)
    } else {
        // memberOf (where the server keeps it) is read from the person below.
        format!(
            "(|(&(objectClass=groupOfNames)(member={dn}))(&(objectClass=groupOfUniqueNames)(uniqueMember={dn}))(&(objectClass=posixGroup)(memberUid={u})))",
            dn = dn,
            u = ldap_escape(username)
        )
    };
    let res = timed(ldap.search(s.group_base(), Scope::Subtree, &filter, vec!["cn"])).await?;
    for e in res.0.into_iter().filter(|e| !e.is_ref()).map(SearchEntry::construct) {
        let name = attr(&e, "cn");
        groups.add(&e.dn, name.as_deref());
    }
    if s.kind != "ad" {
        for g in attr_all(entry, "memberOf") {
            groups.add(&g, None);
        }
    }
    Ok(groups)
}

/// Check a username and password against the directory. On success, the person's details
/// and groups. An empty password is always refused (LDAP would treat it as anonymous).
pub async fn authenticate(s: &DirectorySettings, secret: &str, username: &str, password: &str) -> Result<DirectoryUser, DirError> {
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err(DirError::WrongPassword);
    }
    let service_pw = service_password(s, secret)?;
    let mut ldap = connect(s, &service_pw).await?;
    let entry = find_user(&mut ldap, s, username).await?;
    if s.kind == "ad" {
        let flags: u64 = attr(&entry, "userAccountControl").and_then(|v| v.parse().ok()).unwrap_or(0);
        if flags & AD_ACCOUNT_DISABLED != 0 {
            let _ = ldap.unbind().await;
            return Err(DirError::Disabled);
        }
    }
    // The password check: sign in as the person.
    let res = timed(ldap.simple_bind(&entry.dn, password)).await?;
    if res.rc != 0 {
        let _ = ldap.unbind().await;
        return Err(if res.rc == INVALID_CREDENTIALS { DirError::WrongPassword } else { DirError::Setup(format!("The directory refused the sign-in ({}: {})", res.rc, res.text)) });
    }
    // Groups are read as the service account (people may not be allowed to read them).
    service_bind(&mut ldap, s, &service_pw).await?;
    let groups = find_groups(&mut ldap, s, &entry, username).await?;
    let _ = ldap.unbind().await;
    Ok(DirectoryUser {
        username: attr(&entry, &s.username_attr).unwrap_or_else(|| username.to_string()),
        display_name: attr(&entry, &s.name_attr),
        email: attr(&entry, &s.email_attr),
        dn: entry.dn,
        groups,
    })
}

/// Check the password of a Remote directory account (lock screen, confirming 2FA changes).
/// Ok(false) for a wrong password; Err with a message when the directory cannot answer.
pub async fn check_password(state: &AppState, user: &User, password: &str) -> Result<bool, String> {
    let s = DirectorySettings::load(&state.db);
    if !s.ready() {
        return Err("Directory sign-in is switched off. Use Switch User, or ask an admin.".into());
    }
    let secret = state.config.read().server.jwt_secret.clone();
    match authenticate(&s, &secret, &user.username, password).await {
        // The directory must still know this person under the same name.
        Ok(person) => Ok(person.username.eq_ignore_ascii_case(&user.username)),
        Err(DirError::Unreachable(e)) | Err(DirError::Setup(e)) => {
            tracing::warn!("Directory password check for {}: {}", user.username, e);
            Err("The directory server could not be reached. Try again later, or ask an admin.".into())
        }
        Err(_) => Ok(false),
    }
}

// ---------------- Signing in to Remote ----------------

fn refused() -> (StatusCode, Json<Value>) {
    err(StatusCode::UNAUTHORIZED, "Invalid username or password")
}

/// Sign in with a directory account: check the password, then create or update the Remote
/// account (auth_provider "ldap") with the role from the directory groups.
pub async fn sign_in(state: &AppState, s: &DirectorySettings, typed: &str, password: &str, ip: &ClientIp) -> ApiResult<User> {
    let secret = state.config.read().server.jwt_secret.clone();
    let fail = |reason: &str, user_id: Option<&str>| {
        state.limiter.record_failure(typed);
        state.limiter.record_failure(&ip.limiter_key());
        state.db.log_audit(user_id, typed, "LOGIN_FAILED", None, None, Some(&format!("Directory: {}", reason)), ip.as_opt());
    };
    let person = match authenticate(s, &secret, typed, password).await {
        Ok(p) => p,
        Err(e @ (DirError::Unreachable(_) | DirError::Setup(_))) => {
            tracing::warn!("Directory sign-in for {}: {}", typed, e.describe());
            state.db.log_audit(None, typed, "LOGIN_FAILED", None, None, Some(&format!("Directory: {}", e.describe())), ip.as_opt());
            return Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "The directory server could not be reached or is not set up correctly. Try again later, or ask an admin.",
            ));
        }
        Err(e) => {
            fail(&e.describe(), None);
            return Err(refused());
        }
    };
    let Some(role) = s.role_for(&person.groups) else {
        fail("not a member of the group required to sign in", None);
        return Err(err(StatusCode::FORBIDDEN, "Your directory account is not allowed to sign in to Remote. Ask an admin."));
    };

    let existing = state.db.get_user_by_username(&person.username).map_err(db_err)?;
    let user = match existing {
        Some(mut u) if u.auth_provider == "ldap" => {
            if !u.is_active {
                fail("the Remote account is disabled", Some(&u.id));
                return Err(err(StatusCode::FORBIDDEN, "Account is disabled"));
            }
            let role_changed = u.role != role;
            if role_changed {
                state.db.log_audit(
                    Some(&u.id),
                    &u.username,
                    "ROLE_CHANGED",
                    None,
                    None,
                    Some(&format!("Directory groups: {} -> {}", u.role, role)),
                    ip.as_opt(),
                );
            }
            u.role = role;
            if person.email.is_some() {
                u.email = person.email.clone();
            }
            if person.display_name.is_some() {
                u.display_name = person.display_name.clone();
            }
            state.db.update_user(&u).map_err(db_err)?;
            if role_changed {
                // Logins made with the old role end.
                let _ = state.db.bump_session_generation(&u.id);
            }
            u
        }
        Some(u) => {
            fail("the name belongs to a Remote account that signs in another way", Some(&u.id));
            return Err(err(
                StatusCode::CONFLICT,
                "A Remote account with this name already exists and signs in another way. Ask an admin.",
            ));
        }
        None => {
            let new_user = User {
                id: Uuid::new_v4().to_string(),
                username: person.username.clone(),
                // Never used: directory accounts sign in with their directory password.
                password_hash: hash_password(&Uuid::new_v4().to_string()).unwrap_or_default(),
                email: person.email.clone(),
                display_name: person.display_name.clone(),
                role,
                is_active: true,
                auth_provider: "ldap".to_string(),
                oidc_sub: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_login: None,
                avatar_data: None,
            };
            state.db.create_user(&new_user).map_err(db_err)?;
            state.db.log_audit(
                Some(&new_user.id),
                &new_user.username,
                "USER_CREATED",
                None,
                None,
                Some(&format!("First sign-in with a directory account, role {}", new_user.role)),
                ip.as_opt(),
            );
            new_user
        }
    };
    sync_linked_groups(state, &user, &person.groups, ip);
    state.limiter.record_success(typed);
    state.limiter.record_success(&ip.limiter_key());
    Ok(user)
}

/// Remote groups that follow a directory group: put the person in or take them out.
fn sync_linked_groups(state: &AppState, user: &User, groups: &DirectoryGroups, ip: &ClientIp) {
    let linked: Vec<(String, String, bool)> = state
        .db
        .list_groups()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|g| g.directory_group.as_deref().map(|d| (g.id.clone(), g.name.clone(), groups.contains(d))))
        .collect();
    if linked.is_empty() {
        return;
    }
    match state.db.sync_user_groups(&user.id, &linked) {
        Ok(changed) if !changed.is_empty() => {
            let text = changed
                .iter()
                .map(|(name, member)| format!("{} '{}'", if *member { "added to" } else { "removed from" }, name))
                .collect::<Vec<_>>()
                .join(", ");
            state.db.log_audit(Some(&user.id), &user.username, "GROUP_MEMBERSHIP", None, None, Some(&format!("Directory groups: {}", text)), ip.as_opt());
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("Directory sign-in: could not update groups of {}: {}", user.username, e),
    }
}

// ---------------- Admin ----------------

fn public_view(s: &DirectorySettings) -> Value {
    let mut v = json!(s);
    if let Some(o) = v.as_object_mut() {
        o.remove("bind_password_enc");
        o.insert("has_password".into(), json!(s.bind_password_enc.is_some()));
    }
    v
}

pub async fn api_get_directory(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    Ok(Json(public_view(&DirectorySettings::load(&state.db))))
}

#[derive(Deserialize)]
pub struct DirectoryPayload {
    pub enabled: bool,
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub security: String,
    pub verify_cert: bool,
    pub bind_dn: String,
    /// New password; omitted or null keeps the saved one, "" removes it.
    pub bind_password: Option<String>,
    pub user_base: String,
    pub user_filter: String,
    pub username_attr: String,
    pub name_attr: String,
    pub email_attr: String,
    pub group_base: String,
    pub admin_group: String,
    pub operator_group: String,
    pub required_group: String,
    pub default_role: String,
}

/// Check the form and build the settings (the saved password is kept unless a new one is given).
fn settings_from(p: DirectoryPayload, old: &DirectorySettings, secret: &str) -> ApiResult<DirectorySettings> {
    let bad = |m: &str| err(StatusCode::BAD_REQUEST, m);
    if !["ad", "ldap"].contains(&p.kind.as_str()) {
        return Err(bad("The directory type must be Active Directory or LDAP"));
    }
    if !["ldaps", "starttls", "none"].contains(&p.security.as_str()) {
        return Err(bad("The connection must be LDAPS, StartTLS or none"));
    }
    if !["admin", "operator", "viewer"].contains(&p.default_role.as_str()) {
        return Err(bad("Unknown role for everyone else"));
    }
    let filter = p.user_filter.trim().to_string();
    if !filter.contains("{username}") {
        return Err(bad("The user filter must contain {username}"));
    }
    if !(filter.starts_with('(') && filter.ends_with(')')) {
        return Err(bad("The user filter must be in brackets, for example (uid={username})"));
    }
    if p.username_attr.trim().is_empty() {
        return Err(bad("Fill in which attribute holds the username"));
    }
    if p.enabled && (p.host.trim().is_empty() || p.user_base.trim().is_empty()) {
        return Err(bad("To switch directory sign-in on, fill in the server and where to look for people"));
    }
    let bind_password_enc = match p.bind_password {
        None => old.bind_password_enc.clone(),
        Some(pw) if pw.is_empty() => None,
        Some(pw) => Some(encrypt_secret(&pw, secret).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?),
    };
    Ok(DirectorySettings {
        enabled: p.enabled,
        kind: p.kind,
        host: p.host.trim().to_string(),
        port: p.port,
        security: p.security,
        verify_cert: p.verify_cert,
        bind_dn: p.bind_dn.trim().to_string(),
        bind_password_enc,
        user_base: p.user_base.trim().to_string(),
        user_filter: filter,
        username_attr: p.username_attr.trim().to_string(),
        name_attr: p.name_attr.trim().to_string(),
        email_attr: p.email_attr.trim().to_string(),
        group_base: p.group_base.trim().to_string(),
        admin_group: p.admin_group.trim().to_string(),
        operator_group: p.operator_group.trim().to_string(),
        required_group: p.required_group.trim().to_string(),
        default_role: p.default_role,
    })
}

pub async fn api_set_directory(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<DirectoryPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let secret = state.config.read().server.jwt_secret.clone();
    let s = settings_from(p, &DirectorySettings::load(&state.db), &secret)?;
    state.db.set_setting(DIRECTORY_SETTINGS_KEY, &serde_json::to_string(&s).map_err(db_err)?).map_err(db_err)?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "DIRECTORY_SETTINGS_CHANGED",
        None,
        None,
        Some(&format!(
            "directory sign-in {}, {} {} ({}{})",
            if s.enabled { "on" } else { "off" },
            if s.kind == "ad" { "Active Directory" } else { "LDAP" },
            s.url(),
            s.security,
            if s.verify_cert { "" } else { ", certificate not checked" }
        )),
        None,
    );
    Ok(Json(public_view(&s)))
}

#[derive(Deserialize)]
pub struct DirectoryTestPayload {
    /// The form as it is now (not yet saved).
    pub settings: DirectoryPayload,
    /// Optional: try signing in as this person.
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Admin: try the settings in the form. Reports each step, and with a username (and password)
/// the account that is found, its groups and the role it would get.
pub async fn api_test_directory(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<DirectoryTestPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let secret = state.config.read().server.jwt_secret.clone();
    let mut form = p.settings;
    form.enabled = false; // only checked for completeness when switching on
    let s = settings_from(form, &DirectorySettings::load(&state.db), &secret)?;
    if s.host.is_empty() || s.user_base.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Fill in the server and where to look for people first"));
    }
    let mut steps: Vec<Value> = Vec::new();
    let step = |steps: &mut Vec<Value>, ok: bool, text: String| steps.push(json!({ "ok": ok, "text": text }));
    let service_pw = match service_password(&s, &secret) {
        Ok(pw) => pw,
        Err(e) => {
            step(&mut steps, false, e.describe());
            return Ok(Json(json!({ "ok": false, "steps": steps })));
        }
    };
    let mut ldap = match connect(&s, &service_pw).await {
        Ok(l) => {
            step(&mut steps, true, format!("Connected to {} and signed in as {}", s.url(), if s.bind_dn.is_empty() { "anonymous" } else { &s.bind_dn }));
            l
        }
        Err(e) => {
            let hint = match (&e, s.security.as_str(), s.verify_cert) {
                (DirError::Unreachable(m), "ldaps" | "starttls", true) if m.to_lowercase().contains("certificate") => {
                    " The server's certificate is not trusted; if it is a self-made certificate, turn off the certificate check."
                }
                (DirError::Unreachable(m), _, _) if m.contains("channel closed") || m.contains("reset") || m.contains("eof") => {
                    " The server closed the connection: check that the connection type (LDAPS, StartTLS or none) matches the port."
                }
                _ => "",
            };
            step(&mut steps, false, format!("{}{}", e.describe(), hint));
            return Ok(Json(json!({ "ok": false, "steps": steps })));
        }
    };
    let username = p.username.as_deref().map(str::trim).unwrap_or("");
    if username.is_empty() {
        let _ = ldap.unbind().await;
        step(&mut steps, true, "Settings work. Enter a username to also check finding a person and their groups.".into());
        return Ok(Json(json!({ "ok": true, "steps": steps })));
    }
    let entry = match find_user(&mut ldap, &s, username).await {
        Ok(e) => {
            step(&mut steps, true, format!("Found {}", e.dn));
            e
        }
        Err(e) => {
            let _ = ldap.unbind().await;
            step(&mut steps, false, e.describe());
            return Ok(Json(json!({ "ok": false, "steps": steps })));
        }
    };
    let mut ok = true;
    if s.kind == "ad" {
        let flags: u64 = attr(&entry, "userAccountControl").and_then(|v| v.parse().ok()).unwrap_or(0);
        if flags & AD_ACCOUNT_DISABLED != 0 {
            ok = false;
            step(&mut steps, false, DirError::Disabled.describe());
        }
    }
    let password = p.password.unwrap_or_default();
    if !password.is_empty() {
        match timed(ldap.simple_bind(&entry.dn, &password)).await {
            Ok(r) if r.rc == 0 => step(&mut steps, true, "Password accepted".into()),
            Ok(r) if r.rc == INVALID_CREDENTIALS => {
                ok = false;
                step(&mut steps, false, "Wrong password (or the account is locked or disabled)".into());
            }
            Ok(r) => {
                ok = false;
                step(&mut steps, false, format!("The directory refused the sign-in ({}: {})", r.rc, r.text));
            }
            Err(e) => {
                ok = false;
                step(&mut steps, false, e.describe());
            }
        }
        if let Err(e) = service_bind(&mut ldap, &s, &service_pw).await {
            step(&mut steps, false, e.describe());
            return Ok(Json(json!({ "ok": false, "steps": steps })));
        }
    }
    let groups = match find_groups(&mut ldap, &s, &entry, username).await {
        Ok(g) => g,
        Err(e) => {
            let _ = ldap.unbind().await;
            step(&mut steps, false, format!("Reading groups failed: {}", e.describe()));
            return Ok(Json(json!({ "ok": false, "steps": steps })));
        }
    };
    let _ = ldap.unbind().await;
    step(
        &mut steps,
        true,
        if groups.names.is_empty() { "Groups: none".into() } else { format!("Groups: {}", groups.names.join(", ")) },
    );
    let role = s.role_for(&groups);
    match &role {
        Some(r) => step(&mut steps, true, format!("Would sign in as {} with role {}", attr(&entry, &s.username_attr).unwrap_or_else(|| username.to_string()), r)),
        None => {
            ok = false;
            step(&mut steps, false, format!("Not allowed to sign in: not a member of {}", s.required_group));
        }
    }
    Ok(Json(json!({
        "ok": ok,
        "steps": steps,
        "user": {
            "dn": entry.dn,
            "username": attr(&entry, &s.username_attr),
            "name": attr(&entry, &s.name_attr),
            "email": attr(&entry, &s.email_attr),
            "groups": groups.names,
            "role": role,
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(names: &[&str], dns: &[&str]) -> DirectoryGroups {
        DirectoryGroups { names: names.iter().map(|s| s.to_string()).collect(), dns: dns.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn roles_from_groups() {
        let s = DirectorySettings { required_group: String::new(), ..Default::default() };
        assert_eq!(s.role_for(&groups(&["remote-admins"], &[])).as_deref(), Some("admin"), "any letter case");
        assert_eq!(s.role_for(&groups(&["Remote-Operators"], &[])).as_deref(), Some("operator"));
        assert_eq!(s.role_for(&groups(&["Staff"], &[])).as_deref(), Some("viewer"));
        let by_dn = DirectorySettings { admin_group: "CN=Remote-Admins,OU=Groups,DC=test,DC=lan".into(), ..Default::default() };
        assert_eq!(by_dn.role_for(&groups(&["x"], &["cn=remote-admins,ou=groups,dc=test,dc=lan"])).as_deref(), Some("admin"));
        let strict = DirectorySettings { required_group: "Remote-Users".into(), ..Default::default() };
        assert_eq!(strict.role_for(&groups(&["Remote-Admins"], &[])), None, "required group missing");
        assert_eq!(strict.role_for(&groups(&["Remote-Users"], &[])).as_deref(), Some("viewer"));
    }

    #[test]
    fn urls_and_names() {
        let mut s = DirectorySettings { host: "dc1.test.lan".into(), ..Default::default() };
        assert_eq!(s.url(), "ldaps://dc1.test.lan:636");
        s.security = "starttls".into();
        assert_eq!(s.url(), "ldap://dc1.test.lan:389");
        s.port = 3268;
        s.host = "fd00::5".into();
        assert_eq!(s.url(), "ldap://[fd00::5]:3268");
        assert_eq!(first_rdn_value("CN=Remote-Admins,OU=Groups,DC=x"), "Remote-Admins");
    }
}
