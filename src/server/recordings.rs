//! Session recordings (#31): listing, playback, download, deletion and sharing.
//!
//! Who may do what:
//! - the user whose session it was: watch and download, share (when the admin rule allows);
//! - admins: everything, including deleting;
//! - people it is shared with (while the admin rule allows sharing): watch, and download
//!   when the share allows it. They cannot delete or share it further.

use super::*;
use crate::db::{RecordingRecord, RecordingShare};

type ApiError = (StatusCode, Json<Value>);

fn err(code: StatusCode, msg: &str) -> ApiError {
    (code, Json(json!({ "error": msg })))
}

fn db_err(e: impl std::fmt::Display) -> ApiError {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

/// At most this many people or groups per recording.
const MAX_SHARES: usize = 50;
/// Content search looks through at most this many recordings.
const MAX_SEARCHED: usize = 500;

struct Rights {
    view: bool,
    download: bool,
    delete: bool,
    share: bool,
    /// "own", "admin" or "shared".
    relation: &'static str,
}

fn rights(state: &AppState, claims: &Claims, rec: &RecordingRecord) -> Rights {
    let sharing = state.policy.read().recording_sharing;
    let own = rec.user_id == claims.sub;
    if claims.role == "admin" {
        return Rights { view: true, download: true, delete: true, share: sharing, relation: if own { "own" } else { "admin" } };
    }
    if own {
        return Rights { view: true, download: true, delete: false, share: sharing, relation: "own" };
    }
    match sharing.then(|| state.db.recording_share_access(&rec.id, &claims.sub).ok().flatten()).flatten() {
        Some(dl) => Rights { view: true, download: dl, delete: false, share: false, relation: "shared" },
        None => Rights { view: false, download: false, delete: false, share: false, relation: "shared" },
    }
}

/// The recording, if this user may see it at all (otherwise "not found", so ids reveal nothing).
fn load(state: &AppState, claims: &Claims, id: &str) -> Result<(RecordingRecord, Rights), ApiError> {
    let rec = state.db.get_recording(id).map_err(db_err)?.ok_or_else(|| err(StatusCode::NOT_FOUND, "Recording not found"))?;
    let r = rights(state, claims, &rec);
    if !r.view {
        return Err(err(StatusCode::NOT_FOUND, "Recording not found"));
    }
    Ok((rec, r))
}

fn folder(state: &AppState) -> std::path::PathBuf {
    let cfg = state.config.read();
    cfg.recording.folder(&cfg.server.data_dir)
}

fn entry(state: &AppState, rec: &RecordingRecord, r: &Rights) -> Value {
    let mut v = json!(rec);
    v["relation"] = json!(r.relation);
    v["can_download"] = json!(r.download);
    v["can_delete"] = json!(r.delete);
    v["can_share"] = json!(r.share);
    // The recording's user and admins see who it is shared with (also after sharing is turned off).
    if r.relation != "shared" {
        v["shared_with"] = json!(state.db.list_recording_shares(&rec.id).unwrap_or_default());
    }
    v
}

/// Decrypt a recording (on a worker thread: it reads the whole file).
async fn read_stream(state: &AppState, rec: &RecordingRecord) -> Result<Vec<u8>, ApiError> {
    let path = folder(state).join(&rec.file_name);
    let key_enc = rec.key_enc.clone();
    let secret = state.config.read().server.jwt_secret.clone();
    tokio::task::spawn_blocking(move || crate::recording::read_recording(&path, &key_enc, &secret))
        .await
        .map_err(db_err)?
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// "all" (admins: every recording) or "mine" (own and shared with me; the default).
    pub scope: Option<String>,
    /// Only recordings whose text contains this (terminal recordings).
    pub content: Option<String>,
}

pub async fn api_list_recordings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let all = q.scope.as_deref() == Some("all");
    if all {
        require_admin(&claims)?;
    }
    let mut list: Vec<(RecordingRecord, Rights)> = if all {
        state.db.list_recordings().map_err(db_err)?.into_iter().map(|r| {
            let rt = rights(&state, &claims, &r);
            (r, rt)
        }).collect()
    } else {
        state
            .db
            .list_recordings_for_user(&claims.sub)
            .map_err(db_err)?
            .into_iter()
            .map(|(r, _)| {
                let rt = rights(&state, &claims, &r);
                (r, rt)
            })
            .filter(|(_, rt)| rt.view)
            .collect()
    };

    if let Some(needle) = q.content.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let needle = needle.to_lowercase();
        let dir = folder(&state);
        let secret = state.config.read().server.jwt_secret.clone();
        let candidates: Vec<(String, String, String)> = list
            .iter()
            .filter(|(r, _)| r.kind == "terminal")
            .take(MAX_SEARCHED)
            .map(|(r, _)| (r.id.clone(), r.file_name.clone(), r.key_enc.clone()))
            .collect();
        let hits: std::collections::HashSet<String> = tokio::task::spawn_blocking(move || {
            candidates
                .into_iter()
                .filter(|(_, file, key)| {
                    crate::recording::read_recording(&dir.join(file), key, &secret)
                        .map(|s| crate::recording::terminal_text(&s).to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .map(|(id, _, _)| id)
                .collect()
        })
        .await
        .map_err(db_err)?;
        list.retain(|(r, _)| hits.contains(&r.id));
    }

    let recordings: Vec<Value> = list.iter().map(|(r, rt)| entry(&state, r, rt)).collect();
    let (sharing, keep_days, max_total_gb) = {
        let cfg = state.config.read();
        (state.policy.read().recording_sharing, cfg.recording.keep_days, cfg.recording.max_total_gb)
    };
    let mut out = json!({ "recordings": recordings, "sharing_enabled": sharing, "keep_days": keep_days });
    if all {
        out["storage"] = json!({
            "total_bytes": crate::recording::total_size(&state.db),
            "max_total_gb": max_total_gb,
            "folder": folder(&state).display().to_string(),
        });
    }
    Ok(Json(out))
}

/// The recording for the player. Watching someone else's recording is logged.
pub async fn api_recording_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, r) = load(&state, &claims, &id)?;
    let stream = read_stream(&state, &rec).await?;
    if r.relation != "own" {
        state.db.log_audit(
            Some(&claims.sub),
            &claims.username,
            "RECORDING_VIEWED",
            rec.connection_id.as_deref(),
            Some(&rec.connection_name),
            Some(&format!("{}'s recording from {}", rec.username, short_time(&rec.started_at))),
            ip.as_opt(),
        );
    }
    Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(stream))
        .map_err(db_err)
}

/// Desktop recordings: where playback can start, and the length. Watching someone else's
/// recording is logged here (once per opening).
pub async fn api_recording_index(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, r) = load(&state, &claims, &id)?;
    if rec.kind != "graphics" {
        return Err(err(StatusCode::BAD_REQUEST, "Not a desktop recording"));
    }
    let path = folder(&state).join(&rec.file_name);
    let segments = tokio::task::spawn_blocking(move || crate::recording::read_index(&path))
        .await
        .map_err(db_err)?
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    if r.relation != "own" {
        state.db.log_audit(
            Some(&claims.sub),
            &claims.username,
            "RECORDING_VIEWED",
            rec.connection_id.as_deref(),
            Some(&rec.connection_name),
            Some(&format!("{}'s recording from {}", rec.username, short_time(&rec.started_at))),
            ip.as_opt(),
        );
    }
    Ok(Json(json!({ "segments": segments, "duration_ms": rec.duration_ms, "status": rec.status })))
}

/// Desktop recordings: one piece (from a seek point to the next), decrypted but still
/// compressed; the player unpacks it.
pub async fn api_recording_segment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((id, n)): AxumPath<(String, usize)>,
) -> Result<Response, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, _) = load(&state, &claims, &id)?;
    if rec.kind != "graphics" {
        return Err(err(StatusCode::BAD_REQUEST, "Not a desktop recording"));
    }
    let path = folder(&state).join(&rec.file_name);
    let key_enc = rec.key_enc.clone();
    let secret = state.config.read().server.jwt_secret.clone();
    let data = tokio::task::spawn_blocking(move || crate::recording::read_segment(&path, &key_enc, &secret, n))
        .await
        .map_err(db_err)?
        .map_err(|e| err(StatusCode::NOT_FOUND, &e))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(data))
        .map_err(db_err)
}

/// Desktop recordings are downloaded as a video the browser makes; this checks it is
/// allowed and writes it to the activity log before the browser starts.
pub async fn api_recording_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, r) = load(&state, &claims, &id)?;
    if !r.download {
        return Err(err(StatusCode::FORBIDDEN, "Downloading this recording is not allowed"));
    }
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "RECORDING_DOWNLOADED",
        rec.connection_id.as_deref(),
        Some(&rec.connection_name),
        Some(&format!("{}'s recording from {} (saved as video)", rec.username, short_time(&rec.started_at))),
        ip.as_opt(),
    );
    Ok(Json(json!({ "ok": true })))
}

/// Download as a plain (unencrypted) file. Always logged.
pub async fn api_recording_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, r) = load(&state, &claims, &id)?;
    if !r.download {
        return Err(err(StatusCode::FORBIDDEN, "Downloading this recording is not allowed"));
    }
    if rec.kind == "graphics" {
        return Err(err(StatusCode::BAD_REQUEST, "Desktop recordings are saved as a video from the player"));
    }
    let stream = read_stream(&state, &rec).await?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "RECORDING_DOWNLOADED",
        rec.connection_id.as_deref(),
        Some(&rec.connection_name),
        Some(&format!("{}'s recording from {}", rec.username, short_time(&rec.started_at))),
        ip.as_opt(),
    );
    let stamp = chrono::DateTime::parse_from_rfc3339(&rec.started_at)
        .map(|t| t.format("%Y%m%d-%H%M").to_string())
        .unwrap_or_default();
    let name = format!("{}-{}-{}.cast", rec.connection_name, rec.username, stamp);
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, stream.len())
        .header(header::CONTENT_DISPOSITION, content_disposition(&name))
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(stream))
        .map_err(db_err)
}

/// Admins only.
pub async fn api_delete_recording(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let rec = state.db.get_recording(&id).map_err(db_err)?.ok_or_else(|| err(StatusCode::NOT_FOUND, "Recording not found"))?;
    if rec.status == "recording" {
        return Err(err(StatusCode::CONFLICT, "This session is still being recorded"));
    }
    if !crate::recording::remove_files(&folder(&state), &rec.file_name) {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Could not delete the recording file"));
    }
    state.db.delete_recording(&id).map_err(db_err)?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "RECORDING_DELETED",
        rec.connection_id.as_deref(),
        Some(&rec.connection_name),
        Some(&format!("{}'s recording from {}", rec.username, short_time(&rec.started_at))),
        ip.as_opt(),
    );
    Ok(Json(json!({ "message": "Recording deleted" })))
}

#[derive(Deserialize)]
pub struct SharesPayload {
    pub shares: Vec<RecordingShare>,
}

/// Replace who a recording is shared with (the recording's user or an admin, while allowed).
pub async fn api_set_recording_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
    ip: ClientIp,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<SharesPayload>,
) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    let (rec, r) = load(&state, &claims, &id)?;
    if !state.policy.read().recording_sharing {
        return Err(err(StatusCode::FORBIDDEN, "Sharing recordings is turned off by the administrator"));
    }
    if !r.share {
        return Err(err(StatusCode::FORBIDDEN, "Only the recording's user or an admin can share it"));
    }
    if payload.shares.len() > MAX_SHARES {
        return Err(err(StatusCode::BAD_REQUEST, "Too many people and groups for one recording"));
    }
    let users = state.db.list_users().map_err(db_err)?;
    let groups = state.db.list_groups().map_err(db_err)?;
    let mut clean: Vec<RecordingShare> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for s in payload.shares {
        let (user_id, group_id, name) = match (s.user_id.filter(|v| !v.is_empty()), s.group_id.filter(|v| !v.is_empty())) {
            (Some(u), None) => {
                let found = users.iter().find(|x| x.id == u).ok_or_else(|| err(StatusCode::BAD_REQUEST, "Unknown user"))?;
                if u == rec.user_id {
                    continue; // they have it already
                }
                (Some(u), None, found.username.clone())
            }
            (None, Some(g)) => {
                let found = groups.iter().find(|x| x.id == g).ok_or_else(|| err(StatusCode::BAD_REQUEST, "Unknown group"))?;
                (None, Some(g), format!("group {}", found.name))
            }
            _ => return Err(err(StatusCode::BAD_REQUEST, "Choose one user or one group per line")),
        };
        if clean.iter().any(|c| c.user_id == user_id && c.group_id == group_id) {
            continue;
        }
        names.push(if s.allow_download { format!("{name} (download)") } else { name });
        clean.push(RecordingShare { user_id, group_id, name: None, allow_download: s.allow_download });
    }
    state.db.replace_recording_shares(&rec.id, &clean, &claims.sub).map_err(db_err)?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "RECORDING_SHARED",
        rec.connection_id.as_deref(),
        Some(&rec.connection_name),
        Some(&format!(
            "{}'s recording from {}: {}",
            rec.username,
            short_time(&rec.started_at),
            if names.is_empty() { "no longer shared".to_string() } else { format!("shared with {}", names.join(", ")) }
        )),
        ip.as_opt(),
    );
    let r = rights(&state, &claims, &rec);
    Ok(Json(entry(&state, &rec, &r)))
}

/// People and groups a recording can be shared with (only while sharing is allowed).
pub async fn api_recording_share_targets(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let claims = extract_claims(&state, &headers)?;
    if !state.policy.read().recording_sharing {
        return Err(err(StatusCode::FORBIDDEN, "Sharing recordings is turned off by the administrator"));
    }
    let users: Vec<Value> = state
        .db
        .list_users()
        .map_err(db_err)?
        .into_iter()
        .filter(|u| u.is_active && u.id != claims.sub)
        .map(|u| json!({ "id": u.id, "username": u.username, "display_name": u.display_name }))
        .collect();
    let groups: Vec<Value> =
        state.db.list_groups().map_err(db_err)?.into_iter().map(|g| json!({ "id": g.id, "name": g.name })).collect();
    Ok(Json(json!({ "users": users, "groups": groups })))
}

fn short_time(rfc: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| rfc.to_string())
}
