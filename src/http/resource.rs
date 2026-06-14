use serde::Serialize;
use serde_json::json;

use crate::database::Paginated;

/// Trait for transforming models into API response shapes.
///
/// Implement this trait to control how a model is serialized in API responses:
///
/// ```ignore
/// pub struct UserResource;
///
/// impl ApiResource<User> for UserResource {
///     fn transform(user: &User) -> serde_json::Value {
///         json!({
///             "id": user.id,
///             "email": user.email,
///             "name": user.name,
///             "member_since": user.created_at.format("%Y-%m-%d"),
///         })
///     }
/// }
///
/// // In handler:
/// Ok(Json(UserResource::collection(&users)))
/// Ok(Json(UserResource::make(&user)))
/// ```
pub trait ApiResource<T> {
    /// Transform a single item into its API representation.
    fn transform(item: &T) -> serde_json::Value;

    /// Transform a single item. Alias for `transform`.
    fn make(item: &T) -> serde_json::Value {
        Self::transform(item)
    }

    /// Transform a collection of items.
    fn collection(items: &[T]) -> Vec<serde_json::Value> {
        items.iter().map(Self::transform).collect()
    }

    /// Transform a paginated result set with meta and links.
    fn paginated(paginated: &Paginated<T>, base_url: &str) -> serde_json::Value
    where
        T: Serialize,
    {
        let response = paginated.to_response(base_url);
        json!({
            "data": paginated.data.iter().map(Self::transform).collect::<Vec<_>>(),
            "meta": response.meta,
            "links": response.links,
        })
    }
}
