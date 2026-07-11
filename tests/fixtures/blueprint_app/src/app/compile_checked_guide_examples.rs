use async_trait::async_trait;
use foundry::prelude::*;

use super::datatables::FixtureUser;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GuideJob {
    user_id: String,
}

#[async_trait]
impl Job for GuideJob {
    const ID: JobId = JobId::new("fixture.guide_job");

    async fn handle(&self, _ctx: JobContext) -> Result<()> {
        Ok(())
    }
}

async fn job_dispatch(app: &AppContext) -> Result<()> {
    let jobs = app.jobs()?;
    jobs.dispatch(GuideJob {
        user_id: "123".to_string(),
    })
    .await?;
    jobs.dispatch_on(
        GuideJob {
            user_id: "123".to_string(),
        },
        QueueId::new("mail"),
    )
    .await?;
    jobs.dispatch_later(
        GuideJob {
            user_id: "123".to_string(),
        },
        DateTime::now().add_seconds(60).timestamp_millis(),
    )
    .await?;
    jobs.dispatch_after(
        GuideJob {
            user_id: "123".to_string(),
        },
        std::time::Duration::from_secs(60),
    )
    .await?;
    jobs.dispatch_at(
        GuideJob {
            user_id: "123".to_string(),
        },
        DateTime::now().add_seconds(60),
    )
    .await?;
    jobs.batch("guide")
        .add(GuideJob {
            user_id: "123".to_string(),
        })?
        .dispatch()
        .await?;
    jobs.chain()
        .add(GuideJob {
            user_id: "123".to_string(),
        })?
        .dispatch()
        .await
}

async fn storage_urls(app: &AppContext) -> Result<String> {
    app.storage()?.url("avatars/profile.jpg").await
}

async fn multipart_extractor(form: MultipartForm) -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "title": form.text("title").unwrap_or("Untitled"),
    })))
}

async fn model_cursor(database: &DatabaseManager) -> Result<CursorPaginated<FixtureUser>> {
    FixtureUser::query()
        .cursor_paginate(database, FixtureUser::ID, CursorPagination::new(20))
        .await
}
