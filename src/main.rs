use std::convert::TryInto;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::Result;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use axum::Json;
use axum::Router;
use axum_prometheus::PrometheusMetricLayer;
use axum_server::tls_rustls::RustlsConfig;
use json_patch::Patch;
use kube::core::admission::AdmissionRequest;
use kube::core::admission::AdmissionResponse;
use kube::core::admission::AdmissionReview;
use kube::core::DynamicObject;
use serde_json::json;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;
use tracing_subscriber::filter::EnvFilter;

const TLS_CRT_FILE: &str = "/certs/tls.crt";
const TLS_KEY_FILE: &str = "/certs/tls.key";

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"message": "ok"})))
}

async fn mutate(Json(request): Json<AdmissionReview<DynamicObject>>) -> impl IntoResponse {
    let res = (move || -> Result<AdmissionResponse, anyhow::Error> {
        let req: AdmissionRequest<_> = request.try_into()?;
        let mut resp = AdmissionResponse::from(&req);

        let obj = req
            .object
            .ok_or_else(|| anyhow!("could not get object from the request body"))?;

        let patches_str = (|| -> Option<String> {
            obj.metadata
                .annotations?
                .get("ilya-epifanov.github.io/patcher.patches")
                .map(ToOwned::to_owned)
        })();
        let namespace = obj
            .metadata
            .namespace
            .as_ref()
            .map(String::as_str)
            .unwrap_or_default();
        let name = obj
            .metadata
            .name
            .as_ref()
            .map(String::as_str)
            .unwrap_or_default();
        let patches_str = if let Some(patches_str) = patches_str {
            info!("found patches for pod {}/{}, applying", namespace, name);
            patches_str
        } else {
            debug!("no patches found for pod {}/{}, ignoring", namespace, name);
            return Ok(resp);
        };

        let patch: Patch = serde_yaml::from_str(&patches_str)?;

        resp = resp.with_patch(patch)?;
        Ok(resp)
    })();

    return match res {
        Ok(resp) => (StatusCode::OK, Json(resp.into_review())),
        Err(e) => {
            error!("invalid request: {}", e.to_string());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AdmissionResponse::invalid(e.to_string()).into_review()),
            )
        }
    };
}

async fn reload(config: RustlsConfig) {
    loop {
        tokio::time::sleep(Duration::from_secs(60 * 10)).await;
        debug!("reloading rustls configuration");

        let res = config
            .reload_from_pem_file(TLS_CRT_FILE, TLS_KEY_FILE)
            .await;

        match res {
            Ok(()) => debug!("rustls configuration reloaded"),
            Err(e) => warn!(
                "can't reload rustls configuration, will try next time: {}",
                e
            ),
        };
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let filter = EnvFilter::from_default_env();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let addr = SocketAddr::from_str("0.0.0.0:8443")?;

    let config = RustlsConfig::from_pem_file(TLS_CRT_FILE, TLS_KEY_FILE).await?;

    tokio::spawn(reload(config.clone()));

    let router = Router::new()
        .route("/health", get(health))
        .route("/mutate", post(mutate))
        .route("/metrics", get(|| async move { metric_handle.render() }))
        .layer(prometheus_layer);

    info!("starting http server: {:?}", &addr);

    axum_server::bind_rustls(addr, config)
        .serve(router.into_make_service())
        .await
        .unwrap();

    Ok(())
}
