use actix_web::{get, http, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use anyhow::anyhow;
use anyhow::Result;
use itertools::Itertools;
use json_patch::Patch;
use kube::core::{
    admission::{AdmissionRequest, AdmissionResponse, AdmissionReview},
    DynamicObject,
};
use rustls::internal::pemfile::{certs, rsa_private_keys};
use rustls::{NoClientAuth, ServerConfig};
use serde_json::json;
use std::convert::TryInto;
use std::fs::File;
use std::io::BufReader;
use std::net::ToSocketAddrs;
use tracing::debug;
use tracing::{error, info, warn};
use tracing_subscriber::filter::EnvFilter;

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok()
        .header(http::header::CONTENT_TYPE, "application/json")
        .json(json!({"message": "ok"}))
}

#[post("/mutate")]
async fn mutate(
    reqst: HttpRequest,
    body: web::Json<AdmissionReview<DynamicObject>>,
) -> impl Responder {
    if let Some(content_type) = reqst.head().headers.get("content-type") {
        if content_type != "application/json" {
            let msg = format!("invalid content-type: {:?}", content_type);
            warn!("Warn: {}, Code: {}", msg, http::StatusCode::BAD_REQUEST);
            return HttpResponse::BadRequest().json(msg);
        }
    }

    let res = (move || -> Result<AdmissionResponse, anyhow::Error> {
        let req: AdmissionRequest<_> = body.into_inner().try_into()?;
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
        let patches_str = if let Some(patches_str) = patches_str {
            debug!("Found patches, applying");
            patches_str
        } else {
            debug!("No patches found");
            return Ok(resp);
        };

        let patch: Patch = serde_yaml::from_str(&patches_str)?;

        resp = resp.with_patch(patch)?;
        Ok(resp)
    })();

    return match res {
        Ok(resp) => HttpResponse::Ok().json(resp.into_review()),
        Err(e) => {
            error!("invalid request: {}", e.to_string());
            HttpResponse::InternalServerError()
                .json(&AdmissionResponse::invalid(e.to_string()).into_review())
        }
    };
}

#[actix_web::main]
async fn main() -> Result<(), anyhow::Error> {
    let filter = EnvFilter::from_default_env();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let addrs = "0.0.0.0:8443".to_socket_addrs()?.collect_vec();

    info!("Started http server: {:?}", &addrs);
    let mut config = ServerConfig::new(NoClientAuth::new());

    let cert_file = &mut BufReader::new(File::open("/certs/tls.crt")?);
    let cert_chain = certs(cert_file).map_err(|_| anyhow::anyhow!("can't read certificates"))?;

    let key_file = &mut BufReader::new(File::open("/certs/tls.key")?);
    let mut keys =
        rsa_private_keys(key_file).map_err(|_| anyhow::anyhow!("can't read private keys"))?;

    config.set_single_cert(cert_chain, keys.swap_remove(0))?;

    let router = || App::new().service(mutate).service(health);

    HttpServer::new(router)
        .bind_rustls(&addrs[..], config)?
        .run()
        .await?;
    Ok(())
}
