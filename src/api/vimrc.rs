//! Serve one explicitly configured Vimrc as text; evaluating it belongs to the editor.

use super::AppState;
use crate::{Error, Result};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rustix::fs::{Mode, OFlags, open};
use serde::Serialize;
use std::{fs::File, io::Read, path::Path};

const MAX_VIMRC_BYTES: u64 = 64 * 1024;

#[derive(Debug, Serialize)]
struct VimrcDocument {
    path: String,
    text: String,
    exists: bool,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/notes/vimrc", get(read))
}

async fn read(State(state): State<AppState>) -> Response {
    let result = tokio::task::spawn_blocking(move || read_file(&state.vimrc))
        .await
        .map_err(|error| Error::Task(format!("could not read Vimrc: {error}")))
        .and_then(std::convert::identity);
    let mut response = match result {
        Ok(document) => Json(document).into_response(),
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn read_file(path: &Path) -> Result<VimrcDocument> {
    // NONBLOCK makes FIFOs safe to reject before any reads. Configured symlinks
    // are supported, but the opened destination must be a regular file.
    let file = match open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => File::from(file),
        Err(error) if error == rustix::io::Errno::NOENT => {
            return Ok(VimrcDocument {
                path: path.to_string_lossy().into_owned(),
                text: String::new(),
                exists: false,
            });
        }
        Err(error) => return Err(Error::io(path, error.into())),
    };
    let metadata = file.metadata().map_err(|error| Error::io(path, error))?;
    if !metadata.is_file() {
        return Err(Error::InvalidRequest(format!(
            "Vimrc {} must be a regular file",
            path.display()
        )));
    }
    if metadata.len() > MAX_VIMRC_BYTES {
        return Err(too_large(path));
    }
    let mut bytes = Vec::new();
    file.take(MAX_VIMRC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(path, error))?;
    if bytes.len() as u64 > MAX_VIMRC_BYTES {
        return Err(too_large(path));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        Error::InvalidRequest(format!("Vimrc {} must contain UTF-8 text", path.display()))
    })?;
    Ok(VimrcDocument {
        path: path.to_string_lossy().into_owned(),
        text,
        exists: true,
    })
}

fn too_large(path: &Path) -> Error {
    Error::InvalidRequest(format!("Vimrc {} exceeds 64 KiB", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn missing_vimrc_is_optional_and_edits_are_read_without_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".vimrc");
        let missing = read_file(&path).unwrap();
        assert!(!missing.exists);
        assert!(missing.text.is_empty());
        assert!(!path.exists());
        std::fs::write(&path, "nnoremap U <C-r>\n").unwrap();
        let first = read_file(&path).unwrap();
        assert!(first.exists);
        assert_eq!(first.text, "nnoremap U <C-r>\n");
        std::fs::write(&path, "nnoremap <leader>w :w<CR>\n").unwrap();
        assert_eq!(
            read_file(&path).unwrap().text,
            "nnoremap <leader>w :w<CR>\n"
        );
        std::fs::write(&path, "").unwrap();
        assert!(read_file(&path).unwrap().exists);
    }

    #[test]
    fn vimrc_rejects_oversize_non_utf8_and_non_file_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".vimrc");
        std::fs::write(&path, vec![b' '; 65_536]).unwrap();
        assert_eq!(read_file(&path).unwrap().text.len(), 65_536);
        std::fs::write(&path, vec![b' '; 65_537]).unwrap();
        assert!(read_file(&path).unwrap_err().to_string().contains("64 KiB"));
        std::fs::write(&path, [0xff]).unwrap();
        assert!(read_file(&path).unwrap_err().to_string().contains("UTF-8"));
        assert!(
            read_file(dir.path())
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        let fifo = dir.path().join("fifo");
        rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        assert!(
            read_file(&fifo)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
    }

    #[tokio::test]
    async fn api_serves_only_configured_file_as_uncached_json_and_reloads_it() {
        let library = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let path = config.path().join(".vimrc");
        let other_path = config.path().join("other.vim");
        std::fs::write(&other_path, "not configured").unwrap();
        let state = AppState::new(library.path(), data.path())
            .await
            .unwrap()
            .with_vimrc(&path);
        let app = crate::build_router(state, None);
        let request = || {
            Request::builder()
                .uri("/api/notes/vimrc?path=other.vim")
                .body(Body::empty())
                .unwrap()
        };
        for text in [None, Some("nnoremap U <C-r>\n"), Some("set ignorecase\n")] {
            if let Some(text) = text {
                std::fs::write(&path, text).unwrap();
            }
            let response = app.clone().oneshot(request()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let document: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(document["path"], path.to_string_lossy().as_ref());
            assert_eq!(document["text"], text.unwrap_or_default());
            assert_eq!(document["exists"], text.is_some());
        }
        std::fs::write(&path, vec![b' '; 65_537]).unwrap();
        let response = app.oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    }
}
