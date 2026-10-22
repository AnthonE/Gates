//! The browser's asset reader: Bevy's, with retries, and with no `unwrap` on
//! the network.
//!
//! Bevy's `HttpWasmAssetReader` (bevy_asset 0.18 `io/wasm.rs:80`) unwraps the
//! body read, so a download that breaks after its headers arrive panics the
//! whole module. Seen on the public page on 2026-10-13: Cloudflare's HTTP/3
//! answered one file `net::ERR_QUIC_PROTOCOL_ERROR` mid-body, and the session
//! died 116 s in with `Result::unwrap()` on `TypeError: Failed to fetch`.
//!
//! This one tries a file up to four times when the failure looks transient (a
//! dropped connection, a reset body, 408, 429 or a 5xx), waiting longer each
//! time, and otherwise hands Bevy an error, which fails that one asset and
//! keeps the session alive. 403 and 404 are still "not found", as in Bevy's.
//!
//! [`retry_after_ms`] is the whole policy, pure and on every target, so
//! `tests/web.rs` gates it natively. The reader itself is wasm-only and
//! `client-web` registers it as the default source before `AssetPlugin`.

/// How one try at a file failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failed {
    /// No complete response: the request or its body never arrived.
    Network,
    /// The server answered with this status.
    Status(u16),
}

/// The wait before each retry, in order. Three retries, four tries in all,
/// about 4.6 s of waiting before a file is given up on.
const BACKOFF_MS: [u32; 3] = [400, 1_200, 3_000];

/// How long to wait before trying again after try `attempt` (0-based) failed
/// this way, or `None` to give up.
pub fn retry_after_ms(attempt: u32, failed: Failed) -> Option<u32> {
    let transient = match failed {
        Failed::Network => true,
        Failed::Status(s) => s == 408 || s == 429 || (500..=599).contains(&s),
    };
    if !transient {
        return None;
    }
    BACKOFF_MS.get(attempt as usize).copied()
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{bytes, source};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use bevy::asset::io::{
        AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, Reader, VecReader,
    };
    use bevy::log::warn;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use super::{retry_after_ms, Failed};
    use crate::net::web::js_err;

    /// The default asset source for a page: files under `root`, relative to
    /// the page's own URL, as Bevy's reader resolves them.
    pub fn source(root: &'static str) -> AssetSourceBuilder {
        AssetSourceBuilder::new(move || {
            Box::new(RetryingReader {
                root: PathBuf::from(root),
            })
        })
    }

    struct RetryingReader {
        root: PathBuf,
    }

    /// One try: the whole body, or how it failed and why, in words.
    async fn once(url: &str) -> Result<Vec<u8>, (Failed, String)> {
        let window = web_sys::window().ok_or((Failed::Network, "no window".to_string()))?;
        let resp = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(|e| (Failed::Network, js_err("fetch", &e)))?;
        let resp: web_sys::Response = resp
            .dyn_into()
            .map_err(|e| (Failed::Network, js_err("response", &e)))?;
        let status = resp.status();
        if status != 200 {
            return Err((Failed::Status(status), format!("HTTP {status}")));
        }
        let body = resp
            .array_buffer()
            .map_err(|e| (Failed::Network, js_err("body", &e)))?;
        let buf = JsFuture::from(body)
            .await
            .map_err(|e| (Failed::Network, js_err("body", &e)))?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    async fn sleep_ms(ms: u32) {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let armed = web_sys::window().and_then(|w| {
                w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
                    .ok()
            });
            if armed.is_none() {
                let _ = resolve.call0(&JsValue::NULL);
            }
        });
        let _ = JsFuture::from(wait).await;
    }

    /// One file's whole body, with the reader's retries: `None` once it has
    /// failed for good. For the bytes the game reads itself rather than
    /// through Bevy (the sound bank's recordings, `render::audio`).
    pub async fn bytes(url: String) -> Option<Vec<u8>> {
        let mut attempt = 0;
        loop {
            let (failed, why) = match once(&url).await {
                Ok(bytes) => return Some(bytes),
                Err(e) => e,
            };
            let Some(ms) = retry_after_ms(attempt, failed) else {
                warn!("{url}: {why} ({} tries)", attempt + 1);
                return None;
            };
            sleep_ms(ms).await;
            attempt += 1;
        }
    }

    impl RetryingReader {
        async fn fetch(&self, path: PathBuf) -> Result<VecReader, AssetReaderError> {
            let url = path.to_string_lossy().into_owned();
            let mut attempt = 0;
            loop {
                let (failed, why) = match once(&url).await {
                    Ok(bytes) => return Ok(VecReader::new(bytes)),
                    Err(e) => e,
                };
                let Some(ms) = retry_after_ms(attempt, failed) else {
                    return Err(match failed {
                        Failed::Status(403 | 404) => AssetReaderError::NotFound(path),
                        Failed::Status(s) => AssetReaderError::HttpError(s),
                        Failed::Network => AssetReaderError::Io(Arc::new(std::io::Error::other(
                            format!("{url}: {why} ({} tries)", attempt + 1),
                        ))),
                    });
                };
                warn!("asset {url}: {why}; retrying in {ms} ms");
                sleep_ms(ms).await;
                attempt += 1;
            }
        }
    }

    impl AssetReader for RetryingReader {
        async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
            self.fetch(self.root.join(path)).await
        }

        /// No `.meta` files ship and the page sets `AssetMetaCheck::Never`,
        /// so this is never asked; answered without a request if it is.
        async fn read_meta<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<impl Reader + 'a, AssetReaderError> {
            Err::<VecReader, _>(AssetReaderError::NotFound(self.root.join(path)))
        }

        async fn read_directory<'a>(
            &'a self,
            _path: &'a Path,
        ) -> Result<Box<PathStream>, AssetReaderError> {
            // Bevy's answer too: HTTP has no listing, so a folder is empty.
            Ok(Box::new(bevy::tasks::futures_lite::stream::empty()))
        }

        async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
            Ok(false)
        }
    }
}
