use crate::{lsp, tools};
use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

fn get_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, (StatusCode, Json<ErrorResponse>)> {
    v.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
        let s = format!("missing field: {}", key);
        (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: s }))
    })
}

pub fn router() -> Router {
    Router::new()
        .route("/read", post(read_handler))
        .route("/bash", post(bash_handler))
        .route("/edit", post(edit_handler))
        .route("/write", post(write_handler))
        .route("/grep", post(grep_handler))
        .route("/find", post(find_handler))
        .route("/ls", post(ls_handler))
        .route("/spawn", post(spawn_handler))
        .route("/logs", post(logs_handler))
        .route("/status", post(status_handler))
        .route("/kill", post(kill_handler))
        .route("/sessions", get(sessions_handler))
        .route("/pkg/install", post(pkg_install_handler))
        .route("/pkg/search", post(pkg_search_handler))
        .route("/pkg/list", post(pkg_list_handler))
        .route("/pkg/remove", post(pkg_remove_handler))
        .route("/lsp/diagnose", post(lsp_diagnose_handler))
        .route("/lsp/sessions", get(lsp_sessions_handler))
        .route("/lsp/request", post(lsp_request_handler))
}

async fn read_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::ReadResult>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    tools::read_file(path)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn bash_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::BashResult>, (StatusCode, Json<ErrorResponse>)> {
    let command = get_str(&req, "command")?;
    Ok(Json(tools::run_bash(command, None).await))
}

async fn edit_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::EditResult>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    let edits: Vec<tools::EditOp> = serde_json::from_value(
        req.get("edits")
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "missing edits"))?
            .clone(),
    )
    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("invalid edits: {}", e)))?;
    tools::edit_file(path, &edits)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn write_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::WriteResult>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    let content = get_str(&req, "content")?;
    tools::write_file(path, content)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn grep_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::GrepResult>, (StatusCode, Json<ErrorResponse>)> {
    let pattern = get_str(&req, "pattern")?;
    let path = req.get("path").and_then(|v| v.as_str());
    tools::grep_files(pattern, path)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn find_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::FindResult>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    let name = req.get("name").and_then(|v| v.as_str());
    tools::find_files(path, name)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn ls_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::LsResult>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    tools::list_dir(path)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn spawn_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::SpawnResult>, (StatusCode, Json<ErrorResponse>)> {
    let command = get_str(&req, "command")?;
    tools::spawn_bash(command)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn logs_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::LogsResult>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = get_str(&req, "session_id")?;
    tools::get_logs(session_id)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn status_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::StatusResult>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = get_str(&req, "session_id")?;
    tools::get_status(session_id)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn kill_handler(
    Json(req): Json<Value>,
) -> Result<Json<tools::KillResult>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = get_str(&req, "session_id")?;
    tools::kill_session(session_id)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e.0))
}

async fn sessions_handler() -> Json<Vec<tools::StatusResult>> {
    Json(tools::list_sessions().await)
}

// --- Package management handlers ---

async fn pkg_install_handler(
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let package = get_str(&req, "package")?;
    match crate::nix::install(package) {
        Ok(msg) => Ok(Json(serde_json::json!({ "ok": true, "msg": msg }))),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, e)),
    }
}

async fn pkg_search_handler(
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let query = get_str(&req, "query")?;
    match crate::nix::search(query) {
        Ok(results) => Ok(Json(serde_json::json!({ "packages": results }))),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, e)),
    }
}

async fn pkg_list_handler(
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    match crate::nix::list() {
        Ok(results) => Ok(Json(serde_json::json!({ "packages": results }))),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, e)),
    }
}

async fn pkg_remove_handler(
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let package = get_str(&req, "package")?;
    match crate::nix::remove(package) {
        Ok(msg) => Ok(Json(serde_json::json!({ "ok": true, "msg": msg }))),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, e)),
    }
}

// --- LSP handlers ---

async fn lsp_diagnose_handler(
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let path = get_str(&req, "path")?;
    match lsp::diagnose_file(path).await {
        Ok(diags) => Ok(Json(serde_json::to_value(diags).unwrap_or_default())),
        Err(e) => Err(err(StatusCode::BAD_REQUEST, e)),
    }
}

async fn lsp_sessions_handler() -> Json<Value> {
    Json(serde_json::json!(lsp::list_sessions()))
}

async fn lsp_request_handler(
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let method = get_str(&req, "method")?;
    let path = get_str(&req, "path")?;
    let line = req.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let col = req.get("column").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let params = serde_json::json!({
        "textDocument": { "uri": format!("file://{}", path) },
        "position": { "line": line, "character": col },
    });
    let params = if method.ends_with("/references") {
        serde_json::json!({
            "textDocument": { "uri": format!("file://{}", path) },
            "position": { "line": line, "character": col },
            "context": { "includeDeclaration": true },
        })
    } else {
        params
    };
    lsp::lsp_request(path, method, params)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))
}
