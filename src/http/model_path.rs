use std::ops::Deref;

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::database::TypedPrimaryKey;
use crate::foundation::{AppContext, Error};

/// Resolve a route parameter to a model using the model's typed primary key.
///
/// Malformed primary keys are rejected with 400 Bad Request. A valid key with
/// no matching database record is rejected with 404 Not Found.
#[derive(Debug, Clone)]
pub struct ModelPath<M: TypedPrimaryKey>(pub M);

impl<M: TypedPrimaryKey> ModelPath<M> {
    pub fn into_inner(self) -> M {
        self.0
    }
}

impl<M: TypedPrimaryKey> Deref for ModelPath<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<M> FromRequestParts<AppContext> for ModelPath<M>
where
    M: TypedPrimaryKey,
    M::PrimaryKey: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppContext,
    ) -> std::result::Result<Self, Self::Rejection> {
        let Path(primary_key) = Path::<M::PrimaryKey>::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let database = state.database().map_err(IntoResponse::into_response)?;
        let model = M::model_query()
            .find(database.as_ref(), primary_key)
            .await
            .map_err(IntoResponse::into_response)?
            .ok_or_else(|| {
                Error::not_found(format!("{} model not found", M::table_meta().name()))
                    .into_response()
            })?;

        Ok(Self(model))
    }
}
