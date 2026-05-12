// Synapse License Server — minimal axum skeleton.
// Endpoints:
//   POST /activate  { license_key, hw_fp }     -> { jwt, exp }
//   POST /refresh   { jwt }                    -> { jwt, exp }
//   POST /revoke    { license_key, admin_tok } -> { ok }
// JWT: Ed25519 (EdDSA), 72h TTL, claims include hw_fp + license_key + customer_id.

use anyhow::{anyhow, Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use ed25519_dalek::{pkcs8::DecodePrivateKey, SigningKey};
use jsonwebtoken::{encode, decode, DecodingKey, EncodingKey, Header, Validation, Algorithm};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::{Arc, Mutex}};
use time::OffsetDateTime;

const TTL_SECS: i64 = 72 * 3600;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    enc: Arc<EncodingKey>,
    dec: Arc<DecodingKey>,
    admin_token: Arc<String>,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,        // license_key
    cid: String,        // customer_id
    fp: String,         // hw_fp
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)] struct ActivateReq { license_key: String, hw_fp: String }
#[derive(Deserialize)] struct RefreshReq  { jwt: String }
#[derive(Deserialize)] struct RevokeReq   { license_key: String, admin_tok: String }
#[derive(Serialize)]   struct TokenResp   { jwt: String, exp: i64 }

fn db_init(path: &str) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS licenses (
            license_key TEXT PRIMARY KEY,
            customer_id TEXT NOT NULL,
            hw_fp       TEXT,
            revoked     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            last_seen   INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_lic_cust ON licenses(customer_id);
    "#)?;
    Ok(c)
}

fn issue(state: &AppState, license_key: &str, customer_id: &str, hw_fp: &str) -> Result<TokenResp> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let exp = now + TTL_SECS;
    let claims = Claims {
        sub: license_key.into(), cid: customer_id.into(),
        fp: hw_fp.into(), iat: now, exp,
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("synapse-ed25519-1".into());
    let jwt = encode(&header, &claims, &state.enc)?;
    Ok(TokenResp { jwt, exp })
}

async fn activate(State(s): State<AppState>, Json(r): Json<ActivateReq>) -> impl IntoResponse {
    let res: Result<TokenResp> = (|| {
        let db = s.db.lock().unwrap();
        let mut q = db.prepare("SELECT customer_id, hw_fp, revoked FROM licenses WHERE license_key=?1")?;
        let mut rows = q.query(params![r.license_key])?;
        let row = rows.next()?.ok_or_else(|| anyhow!("unknown license"))?;
        let customer_id: String = row.get(0)?;
        let bound: Option<String> = row.get(1)?;
        let revoked: i64 = row.get(2)?;
        if revoked != 0 { return Err(anyhow!("revoked")); }
        if let Some(b) = bound { if b != r.hw_fp { return Err(anyhow!("hw_fp mismatch")); } }
        else {
            db.execute("UPDATE licenses SET hw_fp=?1, last_seen=?2 WHERE license_key=?3",
                params![r.hw_fp, OffsetDateTime::now_utc().unix_timestamp(), r.license_key])?;
        }
        issue(&s, &r.license_key, &customer_id, &r.hw_fp)
    })();
    match res { Ok(t) => (StatusCode::OK, Json(t)).into_response(),
                Err(e) => (StatusCode::UNAUTHORIZED, e.to_string()).into_response() }
}

async fn refresh(State(s): State<AppState>, Json(r): Json<RefreshReq>) -> impl IntoResponse {
    let mut v = Validation::new(Algorithm::EdDSA);
    v.leeway = 30;
    let res: Result<TokenResp> = (|| {
        let data = decode::<Claims>(&r.jwt, &s.dec, &v).context("jwt invalid")?;
        let c = data.claims;
        let db = s.db.lock().unwrap();
        let revoked: i64 = db.query_row("SELECT revoked FROM licenses WHERE license_key=?1",
            params![c.sub], |r| r.get(0)).unwrap_or(1);
        if revoked != 0 { return Err(anyhow!("revoked")); }
        issue(&s, &c.sub, &c.cid, &c.fp)
    })();
    match res { Ok(t) => (StatusCode::OK, Json(t)).into_response(),
                Err(e) => (StatusCode::UNAUTHORIZED, e.to_string()).into_response() }
}

async fn revoke(State(s): State<AppState>, Json(r): Json<RevokeReq>) -> impl IntoResponse {
    if r.admin_tok != *s.admin_token { return (StatusCode::FORBIDDEN, "nope").into_response(); }
    let db = s.db.lock().unwrap();
    let n = db.execute("UPDATE licenses SET revoked=1 WHERE license_key=?1",
        params![r.license_key]).unwrap_or(0);
    (StatusCode::OK, Json(serde_json::json!({"ok": n>0}))).into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into())).init();

    let db_path = std::env::var("LIC_DB").unwrap_or_else(|_| "licenses.db".into());
    let pem_path = std::env::var("LIC_ED25519_PEM")
        .map_err(|_| anyhow!("LIC_ED25519_PEM required"))?;
    let admin_token = std::env::var("LIC_ADMIN_TOKEN")
        .map_err(|_| anyhow!("LIC_ADMIN_TOKEN required"))?;

    let pem = std::fs::read_to_string(&pem_path)?;
    let signing = SigningKey::from_pkcs8_pem(&pem)?;
    let verifying = signing.verifying_key();

    let enc = EncodingKey::from_ed_pem(pem.as_bytes())?;
    let dec = DecodingKey::from_ed_pem(
        pem_to_pub_pem(&verifying).as_bytes()
    )?;

    let state = AppState {
        db: Arc::new(Mutex::new(db_init(&db_path)?)),
        enc: Arc::new(enc), dec: Arc::new(dec),
        admin_token: Arc::new(admin_token),
    };

    let app = Router::new()
        .route("/activate", post(activate))
        .route("/refresh",  post(refresh))
        .route("/revoke",   post(revoke))
        .with_state(state);

    let addr: SocketAddr = std::env::var("LIC_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8443".into()).parse()?;
    tracing::info!("license-server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn pem_to_pub_pem(vk: &ed25519_dalek::VerifyingKey) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    // SubjectPublicKeyInfo for Ed25519 = 12-byte prefix + 32-byte key
    let prefix: [u8; 12] = [0x30,0x2a,0x30,0x05,0x06,0x03,0x2b,0x65,0x70,0x03,0x21,0x00];
    let mut der = Vec::with_capacity(44);
    der.extend_from_slice(&prefix);
    der.extend_from_slice(vk.as_bytes());
    let b64 = STANDARD.encode(&der);
    format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", b64)
}
