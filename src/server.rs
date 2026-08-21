//! The Rift proxy: a hyper HTTP/1.1 + HTTP/2 server that routes each request
//! through the compiled DSL and either answers inline or forwards to an upstream
//! over a pooled client connection, streaming the response back.

use crate::rules::{Action, Router};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;
type ResBody = BoxBody<Bytes, BoxErr>;

struct State {
    router: Router,
    client: Client<HttpConnector, Incoming>,
}

fn text(status: StatusCode, body: impl Into<Bytes>) -> Response<ResBody> {
    Response::builder()
        .status(status)
        .body(Full::new(body.into()).map_err(|never| match never {}).boxed())
        .unwrap()
}

pub async fn serve(router: Router, addr: SocketAddr) -> Result<(), BoxErr> {
    let client: Client<HttpConnector, Incoming> =
        Client::builder(TokioExecutor::new()).build_http();
    let state = Arc::new(State { router, client });
    let listener = TcpListener::bind(addr).await?;
    eprintln!("rift listening on http://{addr}");
    loop {
        let (stream, _peer) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let svc = service_fn(move |req| handle(Arc::clone(&state), req));
            // Connection-level errors are usually client hangups; ignore them.
            let _ = auto::Builder::new(TokioExecutor::new())
                .serve_connection(io, svc)
                .await;
        });
    }
}

async fn handle(state: Arc<State>, req: Request<Incoming>) -> Result<Response<ResBody>, BoxErr> {
    // Host from the Host header (HTTP/1.1) or the URI authority (HTTP/2).
    let host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_string())
        .or_else(|| req.uri().host().map(|s| s.to_string()));
    let path = req.uri().path().to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|vs| (k.as_str().to_ascii_lowercase(), vs.to_string())))
        .collect();

    match state.router.route(host.as_deref(), &path, &headers).cloned() {
        Some(Action::Respond(code, body)) => {
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
            Ok(text(status, body))
        }
        Some(Action::Upstream(url)) => Ok(proxy(&state, url, req).await),
        None => Ok(text(StatusCode::BAD_GATEWAY, Bytes::from_static(b"rift: no route matched\n"))),
    }
}

async fn proxy(state: &State, upstream: String, mut req: Request<Incoming>) -> Response<ResBody> {
    let up = match upstream.parse::<Uri>() {
        Ok(u) => u,
        Err(_) => return text(StatusCode::INTERNAL_SERVER_ERROR, Bytes::from_static(b"rift: bad upstream url\n")),
    };
    let scheme = up.scheme_str().unwrap_or("http").to_string();
    let authority = match up.authority() {
        Some(a) => a.clone(),
        None => return text(StatusCode::INTERNAL_SERVER_ERROR, Bytes::from_static(b"rift: upstream has no host\n")),
    };
    let pq = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/").to_string();
    let new_uri = match Uri::builder().scheme(scheme.as_str()).authority(authority.clone()).path_and_query(pq).build() {
        Ok(u) => u,
        Err(_) => return text(StatusCode::INTERNAL_SERVER_ERROR, Bytes::from_static(b"rift: could not build upstream uri\n")),
    };
    *req.uri_mut() = new_uri;
    // Point the Host header at the upstream, not the proxy.
    if let Ok(hv) = authority.as_str().parse() {
        req.headers_mut().insert(hyper::header::HOST, hv);
    }

    match state.client.request(req).await {
        Ok(resp) => resp.map(|b| b.map_err(|e| Box::new(e) as BoxErr).boxed()),
        Err(_) => text(StatusCode::BAD_GATEWAY, Bytes::from_static(b"rift: upstream unreachable\n")),
    }
}
