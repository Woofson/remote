//! Joining a shared session (#32): link details and the viewer's WebSocket. Links are
//! created by the session owner over the session's own connection (see protocols/hub.rs).

use super::*;
use crate::protocols::hub::Guest;

#[derive(Deserialize)]
pub struct ShareWsQuery {
    pub token: Option<String>,
}

fn not_found() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "This share link has expired, was cancelled, or the session has ended" })))
}

/// Signed in: what a share link opens (connection, owner, view-only or control).
pub async fn api_share_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(token): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    extract_claims(&state, &headers)?;
    let link = state.shares.lookup(&token).ok_or_else(not_found)?;
    Ok(Json(link.public_view()))
}

/// Signed in: join a shared session as a viewer. A Remote account is required (#32).
pub async fn ws_share_handler(
    State(state): State<AppState>,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<ShareWsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let claims = if let Some(tok) = &query.token {
        authenticate_token(&state, tok, false)?
    } else {
        extract_claims(&state, &headers)?
    };
    let link = state.shares.lookup(&token).ok_or_else(not_found)?;
    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .ok()
        .flatten()
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Unknown user" }))))?;
    let name = user.display_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| user.username.clone());
    Ok(ws.on_upgrade(move |socket| async move {
        let _ = link.attach(Guest { user_id: user.id, name, ws: socket }).await;
    }))
}
