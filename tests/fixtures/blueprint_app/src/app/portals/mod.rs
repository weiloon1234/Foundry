use async_trait::async_trait;
use foundry::prelude::*;

use crate::app::ids;

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub phone: String,
}

#[async_trait]
impl RequestValidator for CreateUser {
    async fn validate(&self, validator: &mut Validator) -> Result<()> {
        validator
            .field("email", self.email.clone())
            .required()
            .email()
            .apply()
            .await?;
        validator
            .field("phone", self.phone.clone())
            .required()
            .rule(ids::MOBILE_RULE)
            .apply()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl foundry::validation::FromMultipart for CreateUser {
    async fn from_multipart(
        multipart: &mut foundry::validation::Multipart,
    ) -> foundry::foundation::Result<Self> {
        let mut email = None;
        let mut phone = None;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| foundry::foundation::Error::message(format!("multipart error: {e}")))?
        {
            match field.name().unwrap_or("") {
                "email" => {
                    email = Some(field.text().await.map_err(|e| {
                        foundry::foundation::Error::message(format!("field error: {e}"))
                    })?)
                }
                "phone" => {
                    phone = Some(field.text().await.map_err(|e| {
                        foundry::foundation::Error::message(format!("field error: {e}"))
                    })?)
                }
                _ => {}
            }
        }
        Ok(Self {
            email: email.unwrap_or_default(),
            phone: phone.unwrap_or_default(),
        })
    }
}

pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route_named_with_options(
        ids::Route::Health,
        "/health",
        get(health),
        HttpRouteOptions::new().action_name("GetHealthStatus"),
    );
    registrar.route_named_with_options(
        ids::Route::UsersStore,
        "/users",
        post(create_user),
        HttpRouteOptions::new()
            .action_name("CreateUser")
            .guard(ids::AuthGuard::Api)
            .permission(ids::Ability::DashboardView),
    );
    Ok(())
}

async fn health(State(app): State<AppContext>) -> impl IntoResponse {
    let entries = app.resolve::<std::sync::Mutex<Vec<String>>>().unwrap();
    Json(serde_json::json!({
        "entries": entries.lock().unwrap().clone(),
    }))
}

async fn create_user(
    _actor: CurrentActor,
    Validated(payload): Validated<CreateUser>,
) -> impl IntoResponse {
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "email": payload.email,
            "phone": payload.phone,
        })),
    )
}
