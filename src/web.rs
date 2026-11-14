//! Embedded web UI and direct Rust controller. No Python or HTTP proxy process.
use crate::control::Controller;
use anyhow::{Result, ensure};
use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

#[derive(Clone)]
struct Policy {
    host: String,
    localhost: Option<String>,
}
fn local(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_loopback() || ip.is_link_local()
}
pub fn validate_bind(bind: SocketAddrV4) -> Result<()> {
    ensure!(
        local(*bind.ip()) && !bind.ip().is_unspecified() && bind.port() >= 1024,
        "use a specific private IPv4 (or loopback) and a port from 1024 to 65535"
    );
    Ok(())
}
fn protected(router: Router, bind: SocketAddrV4) -> Result<Router> {
    validate_bind(bind)?;
    let policy = Policy {
        host: bind.to_string(),
        localhost: bind
            .ip()
            .is_loopback()
            .then(|| format!("localhost:{}", bind.port())),
    };
    Ok(router.layer(middleware::from_fn_with_state(policy, guard)))
}
async fn guard(
    State(policy): State<Policy>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let host_ok =
        host.is_some_and(|host| host == policy.host || policy.localhost.as_deref() == Some(host));
    let peer_ok = matches!(peer.ip(), std::net::IpAddr::V4(ip) if local(ip));
    let write = !matches!(*req.method(), Method::GET | Method::HEAD);
    let origin_ok = req.headers().get(header::ORIGIN).is_none_or(|origin| {
        origin
            .to_str()
            .ok()
            .is_some_and(|origin| host.is_some_and(|host| origin == format!("http://{host}")))
    });
    let csrf_ok = req
        .headers()
        .get("x-vinyl-control")
        .is_some_and(|h| h == "1");
    let mut response = if !host_ok || !peer_ok || (write && (!origin_ok || !csrf_ok)) {
        (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error":"Local control request rejected"})),
        )
            .into_response()
    } else if req.uri().to_string().len() > 1024 {
        StatusCode::URI_TOO_LONG.into_response()
    } else {
        next.run(req).await
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https://statici.icloud.com; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}
fn assets() -> Router {
    Router::new()
        .route(
            "/",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    include_str!("../web/index.html"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
            }),
        )
}
async fn terminate() -> Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = term.recv() => {} }
    }
    #[cfg(windows)]
    {
        let mut breaks = tokio::signal::windows::ctrl_break()?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = breaks.recv() => {} }
    }
    Ok(())
}
pub async fn serve(controller: Controller, listener: tokio::net::TcpListener) -> Result<()> {
    let SocketAddr::V4(bind) = listener.local_addr()? else {
        anyhow::bail!("web launcher requires IPv4");
    };
    let router = protected(controller.routes().merge(assets()), bind)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = rx.await;
    });
    use std::future::IntoFuture;
    let server = server.into_future();
    tokio::pin!(server);
    let (result, finished) = tokio::select! {
        result = &mut server => (result.map_err(anyhow::Error::from), true),
        result = terminate() => (result, false),
    };
    let _ = tx.send(());
    let stop = controller.stop().await;
    // An idle HTTP client must not keep the launcher alive after audio has stopped.
    if !finished {
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut server).await;
    }
    result?;
    stop
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, extract::DefaultBodyLimit, http::Request as HttpRequest};
    use tower::ServiceExt;
    fn router() -> Router {
        let router =
            assets()
                .route(
                    "/api/config",
                    axum::routing::post(
                        |axum::Json(value): axum::Json<serde_json::Value>| async move {
                            axum::Json(value)
                        },
                    ),
                )
                .layer(DefaultBodyLimit::max(4096));
        protected(router, "127.0.0.1:8787".parse().unwrap()).unwrap()
    }
    fn request(method: &str, path: &str, host: &str) -> axum::http::request::Builder {
        HttpRequest::builder()
            .method(method)
            .uri(path)
            .header("host", host)
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
    }
    #[tokio::test]
    async fn embedded_assets_and_only_known_paths() {
        for (path, mime) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/style.css", "text/css"),
        ] {
            let response = router()
                .oneshot(
                    request("GET", path, "localhost:8787")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            assert!(
                response.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with(mime)
            );
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(
                response.headers()["content-security-policy"]
                    .to_str()
                    .unwrap()
                    .contains("frame-ancestors 'none'")
            );
        }
        for path in ["/api/video", "/api/anything", "/../../LICENSE"] {
            assert_eq!(
                router()
                    .oneshot(
                        request("GET", path, "127.0.0.1:8787")
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                404
            );
        }
    }
    #[tokio::test]
    async fn rebinding_and_public_peers_are_rejected() {
        for host in ["attacker.example", "localhost:9999", "127.0.0.1"] {
            assert_eq!(
                router()
                    .oneshot(request("GET", "/", host).body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                403
            );
        }
        let req = request("GET", "/", "127.0.0.1:8787")
            .extension(ConnectInfo("8.8.8.8:1234".parse::<SocketAddr>().unwrap()))
            .body(Body::empty())
            .unwrap();
        assert_eq!(router().oneshot(req).await.unwrap().status(), 403);
    }
    #[tokio::test]
    async fn writes_require_local_origin_and_custom_header_and_bounded_body() {
        for req in [
            request("POST", "/api/config", "127.0.0.1:8787"),
            request("POST", "/api/config", "127.0.0.1:8787")
                .header("x-vinyl-control", "1")
                .header("origin", "http://attacker.example"),
        ] {
            assert_eq!(
                router()
                    .oneshot(req.body(Body::from("{}")).unwrap())
                    .await
                    .unwrap()
                    .status(),
                403
            );
        }
        let valid = || {
            request("POST", "/api/config", "localhost:8787")
                .header("x-vinyl-control", "1")
                .header("origin", "http://localhost:8787")
                .header("content-type", "application/json")
        };
        assert_eq!(
            router()
                .oneshot(valid().body(Body::from("{}")).unwrap())
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            router()
                .oneshot(
                    valid()
                        .body(Body::from(format!("\"{}\"", "x".repeat(4096))))
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            413
        );
    }
    #[test]
    fn bind_does_not_expose_all_interfaces_or_public_addresses() {
        for bind in [
            "0.0.0.0:8787",
            "8.8.8.8:8787",
            "224.0.0.1:8787",
            "127.0.0.1:80",
        ] {
            assert!(validate_bind(bind.parse().unwrap()).is_err());
        }
        for bind in ["127.0.0.1:8787", "192.168.50.2:8787", "169.254.1.2:8787"] {
            assert!(validate_bind(bind.parse().unwrap()).is_ok());
        }
    }
}
