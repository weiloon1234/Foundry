use std::collections::BTreeSet;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;

use crate::auth::lockout::LoginThrottle;
use crate::auth::token::TokenResponse;
use crate::auth::{Actor, CurrentActor};
use crate::config::MfaConfig;
use crate::database::{ComparisonOp, Condition, DbType, DbValue, Expr, FromDbValue, Query, Sql};
use crate::events::Event;
use crate::foundation::{AppContext, Error, Result};
use crate::support::{EventId, GuardId, HashManager, RoleId, Token};

// RFC 6238 TOTP uses HMAC-SHA1; mainstream authenticator apps (Google
// Authenticator, Authy, etc.) ignore the otpauth `algorithm` parameter and
// always compute SHA1, so any other hash breaks code verification.
type HmacSha1 = Hmac<Sha1>;

const TOTP_PERIOD_SECONDS: i64 = 30;
const TOTP_DIGITS: u32 = 6;
const AUTH_MFA_TOTP_FACTORS_TABLE: &str = "auth_mfa_totp_factors";

#[cfg(feature = "webauthn")]
pub mod webauthn {}

#[derive(
    Clone, Debug, Serialize, Deserialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct MfaEnrollChallenge {
    pub secret: String,
    pub otpauth_url: String,
}

pub use MfaEnrollChallenge as EnrollChallenge;

#[derive(
    Clone, Debug, Serialize, Deserialize, ts_rs::TS, foundry_macros::TS, foundry_macros::ApiSchema,
)]
pub struct MfaRecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}

pub use MfaRecoveryCodesResponse as RecoveryCodesResponse;

#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
    foundry_macros::Validate,
)]
pub struct MfaCodeRequest {
    #[validate(required)]
    pub code: String,
}

pub use MfaCodeRequest as CodeRequest;

#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
    foundry_macros::Validate,
)]
pub struct MfaRecoveryCodesRequest {
    #[validate(required)]
    pub current_code: String,
}

pub use MfaRecoveryCodesRequest as RecoveryCodesRequest;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaEnrolledEvent {
    pub actor: Actor,
    pub factor: String,
}

impl Event for MfaEnrolledEvent {
    const ID: EventId = EventId::new("auth.mfa_enrolled");
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaDisabledEvent {
    pub actor: Actor,
    pub factor: String,
}

impl Event for MfaDisabledEvent {
    const ID: EventId = EventId::new("auth.mfa_disabled");
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaVerifiedEvent {
    pub actor: Actor,
    pub factor: String,
}

impl Event for MfaVerifiedEvent {
    const ID: EventId = EventId::new("auth.mfa_verified");
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaFailedEvent {
    pub actor: Actor,
    pub factor: String,
    pub reason: String,
}

impl Event for MfaFailedEvent {
    const ID: EventId = EventId::new("auth.mfa_failed");
}

#[async_trait]
pub trait MfaFactor: Send + Sync + 'static {
    async fn enroll(&self, actor: &Actor) -> Result<EnrollChallenge>;

    async fn confirm(&self, actor: &Actor, response: &str) -> Result<()>;

    async fn verify(&self, actor: &Actor, response: &str) -> Result<()>;

    fn id(&self) -> &str;
}

#[derive(Clone)]
pub struct MfaManager {
    app: AppContext,
    config: MfaConfig,
}

impl MfaManager {
    pub fn new(app: &AppContext) -> Result<Self> {
        Ok(Self {
            app: app.clone(),
            config: app.config().auth()?.mfa,
        })
    }

    pub fn totp(&self) -> TotpFactor {
        TotpFactor::new(self.app.clone(), self.config.clone())
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn requires_mfa(&self, actor: &Actor) -> bool {
        self.requires_mfa_for_roles(&actor.guard, actor.roles.iter())
    }

    pub fn requires_mfa_for_roles<'a, I>(&self, guard: &GuardId, roles: I) -> bool
    where
        I: IntoIterator<Item = &'a RoleId>,
    {
        let Some(required) = self.config.required_roles.get(guard.as_ref()) else {
            return false;
        };
        let role_names = roles
            .into_iter()
            .map(|role| role.as_ref())
            .collect::<BTreeSet<_>>();
        required
            .iter()
            .any(|role| role_names.contains(role.as_str()))
    }

    pub async fn issue_pending_token(
        &self,
        actor: &Actor,
        name: &str,
    ) -> Result<crate::auth::token::TokenPair> {
        self.app
            .tokens()?
            .issue_mfa_pending(actor, name, self.config.pending_token_ttl_minutes)
            .await
    }

    pub async fn issue_full_token(
        &self,
        actor: &Actor,
        name: &str,
    ) -> Result<crate::auth::token::TokenPair> {
        self.app.tokens()?.issue_actor_named(actor, name).await
    }
}

#[derive(Clone)]
pub struct TotpFactor {
    app: AppContext,
    config: MfaConfig,
}

impl TotpFactor {
    pub fn new(app: AppContext, config: MfaConfig) -> Self {
        Self { app, config }
    }

    pub async fn disable(&self, actor: &Actor, response: &str) -> Result<()> {
        self.ensure_enabled()?;
        self.verify_current_code(actor, response).await?;
        let database = self.app.database()?;
        Query::delete_from(AUTH_MFA_TOTP_FACTORS_TABLE)
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .execute(database.as_ref())
            .await?;
        self.dispatch_event(MfaDisabledEvent {
            actor: actor.clone(),
            factor: self.id().to_string(),
        })
        .await?;
        Ok(())
    }

    pub async fn regenerate_recovery_codes(
        &self,
        actor: &Actor,
        current_code: &str,
    ) -> Result<Vec<String>> {
        self.ensure_enabled()?;
        self.verify_current_code(actor, current_code).await?;
        let codes = generate_recovery_codes(self.config.recovery_codes.max(1))?;
        let hashed_codes = hash_recovery_codes(self.app.hash()?.as_ref(), &codes)?;
        self.persist_recovery_codes(actor, &hashed_codes).await?;
        Ok(codes)
    }

    async fn verify_current_code(&self, actor: &Actor, response: &str) -> Result<()> {
        self.verify(actor, response).await
    }

    fn ensure_enabled(&self) -> Result<()> {
        if self.config.enabled {
            Ok(())
        } else {
            Err(Error::http_with_code(
                404,
                "Multi-factor authentication is disabled.",
                "mfa_disabled",
            ))
        }
    }

    async fn load_record(&self, actor: &Actor) -> Result<Option<TotpRecord>> {
        let database = self.app.database()?;
        let rows = Query::table(AUTH_MFA_TOTP_FACTORS_TABLE)
            .select([
                "secret_ciphertext",
                "confirmed_at",
                "recovery_codes",
                "last_used_step",
            ])
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .limit(1)
            .get(database.as_ref())
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };

        Ok(Some(TotpRecord {
            secret_ciphertext: row
                .optional_text("secret_ciphertext")
                .ok_or_else(|| Error::message("missing MFA secret"))?,
            confirmed_at: row
                .get("confirmed_at")
                .and_then(|value| crate::DateTime::from_db_value(value).ok())
                .map(|value| value.as_chrono()),
            recovery_codes: row
                .get("recovery_codes")
                .and_then(|value| serde_json::Value::from_db_value(value).ok())
                .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
                .unwrap_or_default(),
            last_used_step: row
                .get("last_used_step")
                .and_then(|value| i64::from_db_value(value).ok()),
        }))
    }

    async fn upsert_pending_secret(&self, actor: &Actor, secret: &str) -> Result<()> {
        let encrypted = self.app.crypt()?.encrypt_string(secret)?;
        let database = self.app.database()?;
        let empty_recovery_codes = serde_json::json!([]);
        Query::insert_into(AUTH_MFA_TOTP_FACTORS_TABLE)
            .values([
                ("guard", DbValue::Text(actor.guard.to_string())),
                ("actor_id", DbValue::Text(actor.id.clone())),
                ("secret_ciphertext", DbValue::Text(encrypted)),
                (
                    "recovery_codes",
                    DbValue::Json(empty_recovery_codes.clone()),
                ),
            ])
            .on_conflict_columns(["guard", "actor_id"])
            .do_update()
            .set_excluded("secret_ciphertext")
            .set("confirmed_at", DbValue::Null(DbType::TimestampTz))
            .set("recovery_codes", DbValue::Json(empty_recovery_codes))
            .set("last_used_step", DbValue::Null(DbType::Int64))
            .set_expr("updated_at", Sql::now())
            .execute(database.as_ref())
            .await?;
        Ok(())
    }

    async fn mark_confirmed(&self, actor: &Actor, last_used_step: i64) -> Result<()> {
        let database = self.app.database()?;
        Query::update_table(AUTH_MFA_TOTP_FACTORS_TABLE)
            .set_expr("confirmed_at", Sql::now())
            .value("last_used_step", last_used_step)
            .set_expr("updated_at", Sql::now())
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .execute(database.as_ref())
            .await?;
        Ok(())
    }

    async fn update_last_used_step(&self, actor: &Actor, last_used_step: i64) -> Result<bool> {
        let database = self.app.database()?;
        let affected = Query::update_table(AUTH_MFA_TOTP_FACTORS_TABLE)
            .value("last_used_step", last_used_step)
            .set_expr("updated_at", Sql::now())
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .where_(Expr::column("confirmed_at").is_not_null())
            .where_(Condition::or([
                Expr::column("last_used_step").is_null(),
                Expr::column("last_used_step").compare(
                    ComparisonOp::Lt,
                    Expr::value(DbValue::Int64(last_used_step)),
                ),
            ]))
            .execute(database.as_ref())
            .await?;
        Ok(affected > 0)
    }

    async fn persist_recovery_codes(&self, actor: &Actor, hashes: &[String]) -> Result<()> {
        let recovery_codes = serde_json::to_value(hashes).map_err(Error::other)?;
        let database = self.app.database()?;
        Query::update_table(AUTH_MFA_TOTP_FACTORS_TABLE)
            .value("recovery_codes", DbValue::Json(recovery_codes))
            .set_expr("updated_at", Sql::now())
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .execute(database.as_ref())
            .await?;
        Ok(())
    }

    async fn consume_recovery_code(
        &self,
        actor: &Actor,
        current_hashes: &[String],
        remaining_hashes: &[String],
    ) -> Result<bool> {
        let current = serde_json::to_value(current_hashes).map_err(Error::other)?;
        let remaining = serde_json::to_value(remaining_hashes).map_err(Error::other)?;
        let database = self.app.database()?;
        let affected = Query::update_table(AUTH_MFA_TOTP_FACTORS_TABLE)
            .value("recovery_codes", DbValue::Json(remaining))
            .set_expr("updated_at", Sql::now())
            .where_eq("guard", actor.guard.to_string())
            .where_eq("actor_id", actor.id.clone())
            .where_eq("recovery_codes", DbValue::Json(current))
            .execute(database.as_ref())
            .await?;
        Ok(affected > 0)
    }

    async fn dispatch_event<E>(&self, event: E) -> Result<()>
    where
        E: Event,
    {
        if let Ok(events) = self.app.events() {
            events.dispatch(event).await?;
        }
        Ok(())
    }

    async fn dispatch_failed(&self, actor: &Actor, reason: &str) -> Result<()> {
        self.dispatch_event(MfaFailedEvent {
            actor: actor.clone(),
            factor: self.id().to_string(),
            reason: reason.to_string(),
        })
        .await
    }

    async fn verify_record(
        &self,
        actor: &Actor,
        record: &TotpRecord,
        response: &str,
    ) -> Result<VerifiedFactor> {
        let throttle = LoginThrottle::new(&self.app)?;
        let identifier = format!("mfa:{}:{}", actor.guard, actor.id);
        throttle
            .before_attempt(&identifier)
            .await
            .map_err(Error::from)?;

        let response = normalize_mfa_response(response);
        let secret = self
            .app
            .crypt()?
            .decrypt_string(&record.secret_ciphertext)?;
        let secret_bytes = decode_base32(&secret)?;
        let current_step = current_totp_step(Utc::now());
        if is_totp_code_candidate(response) {
            for step in (current_step - 1)..=(current_step + 1) {
                if Some(step) <= record.last_used_step {
                    continue;
                }
                let expected = totp_code(&secret_bytes, step)?;
                if crate::support::hmac::constant_time_eq(expected.as_bytes(), response.as_bytes())
                {
                    throttle.record_success(&identifier).await?;
                    return Ok(VerifiedFactor::Totp { step });
                }
            }
        }

        if is_recovery_code_candidate(response) {
            if let Some((_, remaining_hashes)) = consume_matching_recovery_code(
                self.app.hash()?.as_ref(),
                &record.recovery_codes,
                response,
            )? {
                throttle.record_success(&identifier).await?;
                return Ok(VerifiedFactor::RecoveryCode {
                    current_hashes: record.recovery_codes.clone(),
                    remaining_hashes,
                });
            }
        }

        throttle.record_failure(&identifier).await?;
        self.dispatch_failed(actor, "invalid_code").await?;
        Err(invalid_mfa_code_error())
    }
}

#[async_trait]
impl MfaFactor for TotpFactor {
    async fn enroll(&self, actor: &Actor) -> Result<EnrollChallenge> {
        self.ensure_enabled()?;
        let secret = encode_base32(&Token::bytes(20)?);
        self.upsert_pending_secret(actor, &secret).await?;
        let issuer = if self.config.issuer.trim().is_empty() {
            self.app.config().app()?.name
        } else {
            self.config.issuer.clone()
        };
        let label = format!("{}:{}", issuer, actor.id);
        Ok(EnrollChallenge {
            secret: secret.clone(),
            otpauth_url: format!(
                "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
                percent_encode(&label),
                secret,
                percent_encode(&issuer),
                TOTP_DIGITS,
                TOTP_PERIOD_SECONDS,
            ),
        })
    }

    async fn confirm(&self, actor: &Actor, response: &str) -> Result<()> {
        self.ensure_enabled()?;
        let record = self.load_record(actor).await?.ok_or_else(|| {
            Error::http_with_code(404, "MFA enrollment was not started.", "mfa_not_started")
        })?;
        if record.confirmed_at.is_some() {
            return Err(Error::http_with_code(
                409,
                "MFA is already confirmed for this actor.",
                "mfa_already_confirmed",
            ));
        }

        let verified = self.verify_record(actor, &record, response).await?;
        let step = match verified {
            VerifiedFactor::Totp { step } => step,
            VerifiedFactor::RecoveryCode { .. } => {
                return Err(Error::http_with_code(
                    401,
                    "Recovery codes cannot confirm MFA enrollment.",
                    "invalid_mfa_code",
                ))
            }
        };
        self.mark_confirmed(actor, step).await?;
        self.dispatch_event(MfaEnrolledEvent {
            actor: actor.clone(),
            factor: self.id().to_string(),
        })
        .await?;
        Ok(())
    }

    async fn verify(&self, actor: &Actor, response: &str) -> Result<()> {
        self.ensure_enabled()?;
        let record = self.load_record(actor).await?.ok_or_else(|| {
            Error::http_with_code(
                404,
                "MFA is not enrolled for this actor.",
                "mfa_not_enrolled",
            )
        })?;
        if record.confirmed_at.is_none() {
            return Err(Error::http_with_code(
                409,
                "MFA enrollment is not confirmed yet.",
                "mfa_not_confirmed",
            ));
        }

        match self.verify_record(actor, &record, response).await? {
            VerifiedFactor::Totp { step } => {
                if !self.update_last_used_step(actor, step).await? {
                    self.dispatch_failed(actor, "replayed_code").await?;
                    return Err(invalid_mfa_code_error());
                }
            }
            VerifiedFactor::RecoveryCode {
                current_hashes,
                remaining_hashes,
            } => {
                if !self
                    .consume_recovery_code(actor, &current_hashes, &remaining_hashes)
                    .await?
                {
                    self.dispatch_failed(actor, "replayed_code").await?;
                    return Err(invalid_mfa_code_error());
                }
            }
        }

        self.dispatch_event(MfaVerifiedEvent {
            actor: actor.clone(),
            factor: self.id().to_string(),
        })
        .await?;
        Ok(())
    }

    fn id(&self) -> &str {
        "totp"
    }
}

pub mod routes {
    use super::*;

    pub async fn enroll(
        State(app): State<AppContext>,
        CurrentActor(actor): CurrentActor,
    ) -> Result<Json<EnrollChallenge>> {
        let manager = MfaManager::new(&app)?;
        Ok(Json(manager.totp().enroll(&actor).await?))
    }

    pub async fn confirm(
        State(app): State<AppContext>,
        CurrentActor(actor): CurrentActor,
        Json(body): Json<CodeRequest>,
    ) -> Result<StatusCode> {
        let manager = MfaManager::new(&app)?;
        manager.totp().confirm(&actor, &body.code).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    pub async fn verify(
        State(app): State<AppContext>,
        CurrentActor(actor): CurrentActor,
        Json(body): Json<CodeRequest>,
    ) -> Result<Json<TokenResponse>> {
        let manager = MfaManager::new(&app)?;
        let totp = manager.totp();
        totp.verify(&actor, &body.code).await?;
        let tokens = manager.issue_full_token(&actor, "").await?;
        Ok(Json(TokenResponse::new(tokens)))
    }

    pub async fn disable(
        State(app): State<AppContext>,
        CurrentActor(actor): CurrentActor,
        Json(body): Json<CodeRequest>,
    ) -> Result<StatusCode> {
        let manager = MfaManager::new(&app)?;
        manager.totp().disable(&actor, &body.code).await?;
        Ok(StatusCode::NO_CONTENT)
    }

    pub async fn recovery(
        State(app): State<AppContext>,
        CurrentActor(actor): CurrentActor,
        Json(body): Json<RecoveryCodesRequest>,
    ) -> Result<Json<RecoveryCodesResponse>> {
        let manager = MfaManager::new(&app)?;
        let recovery_codes = manager
            .totp()
            .regenerate_recovery_codes(&actor, &body.current_code)
            .await?;
        Ok(Json(RecoveryCodesResponse { recovery_codes }))
    }
}

#[derive(Clone, Debug)]
struct TotpRecord {
    secret_ciphertext: String,
    confirmed_at: Option<chrono::DateTime<Utc>>,
    recovery_codes: Vec<String>,
    last_used_step: Option<i64>,
}

enum VerifiedFactor {
    Totp {
        step: i64,
    },
    RecoveryCode {
        current_hashes: Vec<String>,
        remaining_hashes: Vec<String>,
    },
}

fn invalid_mfa_code_error() -> Error {
    Error::http_with_code(
        401,
        "Invalid multi-factor authentication code.",
        "invalid_mfa_code",
    )
}

fn normalize_mfa_response(response: &str) -> &str {
    response.trim()
}

fn is_totp_code_candidate(response: &str) -> bool {
    response.len() == TOTP_DIGITS as usize && response.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_recovery_code_candidate(response: &str) -> bool {
    const PART_LEN: usize = 5;
    let bytes = response.as_bytes();
    bytes.len() == PART_LEN * 2 + 1
        && bytes[PART_LEN] == b'-'
        && bytes[..PART_LEN]
            .iter()
            .chain(bytes[PART_LEN + 1..].iter())
            .all(|byte| byte.is_ascii_alphanumeric())
}

fn current_totp_step(now: chrono::DateTime<Utc>) -> i64 {
    now.timestamp() / TOTP_PERIOD_SECONDS
}

fn totp_code(secret: &[u8], step: i64) -> Result<String> {
    let counter = (step as u64).to_be_bytes();
    let mut mac = HmacSha1::new_from_slice(secret).map_err(Error::other)?;
    mac.update(&counter);
    let result = mac.finalize().into_bytes();
    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let binary = ((u32::from(result[offset]) & 0x7f) << 24)
        | (u32::from(result[offset + 1]) << 16)
        | (u32::from(result[offset + 2]) << 8)
        | u32::from(result[offset + 3]);
    Ok(format!("{:06}", binary % 10_u32.pow(TOTP_DIGITS)))
}

fn encode_base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer = 0u16;
    let mut bits_left = 0u8;

    for &byte in bytes {
        buffer = (buffer << 8) | u16::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            output.push(ALPHABET[index] as char);
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        output.push(ALPHABET[index] as char);
    }

    output
}

fn decode_base32(value: &str) -> Result<Vec<u8>> {
    let mut buffer = 0u32;
    let mut bits_left = 0u8;
    let mut output = Vec::new();

    for byte in value.bytes().filter(|byte| *byte != b'=') {
        let normalized = byte.to_ascii_uppercase();
        let digit = match normalized {
            b'A'..=b'Z' => normalized - b'A',
            b'2'..=b'7' => normalized - b'2' + 26,
            _ => {
                return Err(Error::message(
                    "MFA secret contains an invalid base32 character.",
                ))
            }
        };

        buffer = (buffer << 5) | u32::from(digit);
        bits_left += 5;
        if bits_left >= 8 {
            output.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }

    Ok(output)
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn generate_recovery_codes(count: usize) -> Result<Vec<String>> {
    (0..count)
        .map(|_| {
            Ok(format!(
                "{}-{}",
                HashManager::random_string(5)?,
                HashManager::random_string(5)?
            ))
        })
        .collect()
}

fn hash_recovery_codes(
    hash_manager: &crate::support::HashManager,
    codes: &[String],
) -> Result<Vec<String>> {
    codes.iter().map(|code| hash_manager.hash(code)).collect()
}

fn consume_matching_recovery_code(
    hash_manager: &crate::support::HashManager,
    hashes: &[String],
    response: &str,
) -> Result<Option<(usize, Vec<String>)>> {
    for (index, hash) in hashes.iter().enumerate() {
        if hash_manager.check(response, hash)? {
            let mut remaining = hashes.to_vec();
            remaining.remove(index);
            return Ok(Some((index, remaining)));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HashingConfig;

    #[test]
    fn base32_roundtrip_preserves_secret_bytes() {
        let bytes = b"foundry-mfa-secret";
        let encoded = encode_base32(bytes);
        let decoded = decode_base32(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn totp_code_is_six_digits() {
        let secret = decode_base32(&encode_base32(b"foundry-secret-seed")).unwrap();
        let code = totp_code(&secret, 1_000).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn totp_code_matches_rfc_6238_sha1_vectors() {
        // RFC 6238 Appendix B vectors (SHA1 secret, 8-digit codes truncated
        // to the framework's 6 digits). Guards against drifting off the
        // algorithm that real authenticator apps implement.
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59i64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            let step = time / TOTP_PERIOD_SECONDS;
            assert_eq!(totp_code(secret, step).unwrap(), expected);
        }
    }

    #[test]
    fn mfa_response_candidates_are_shape_limited() {
        assert!(is_totp_code_candidate("123456"));
        assert!(!is_totp_code_candidate("12345"));
        assert!(!is_totp_code_candidate("1234567"));
        assert!(!is_totp_code_candidate("１２３４５６"));
        assert!(!is_totp_code_candidate("12345a"));

        assert!(is_recovery_code_candidate("ABCDE-12345"));
        assert!(is_recovery_code_candidate("abcde-12345"));
        assert!(!is_recovery_code_candidate("ABCDE12345"));
        assert!(!is_recovery_code_candidate("ABCDE-1234"));
        assert!(!is_recovery_code_candidate("ABCDE-12345-extra"));
        assert!(!is_recovery_code_candidate("ABCDE_12345"));
        assert_eq!(normalize_mfa_response(" 123456\n"), "123456");
    }

    #[test]
    fn recovery_codes_are_consumed_once() {
        let hash = HashManager::from_config(&HashingConfig::default()).unwrap();
        let codes = vec!["ABCDE-12345".to_string(), "FGHIJ-67890".to_string()];
        let hashes = hash_recovery_codes(&hash, &codes).unwrap();

        let consumed = consume_matching_recovery_code(&hash, &hashes, &codes[0])
            .unwrap()
            .unwrap();
        assert_eq!(consumed.0, 0);
        assert_eq!(consumed.1.len(), 1);
        assert!(
            consume_matching_recovery_code(&hash, &consumed.1, &codes[0])
                .unwrap()
                .is_none()
        );
    }
}
