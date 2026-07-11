use std::fs;
use std::sync::OnceLock;

use foundry::prelude::*;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const OWNERS_TABLE: &str = "foundry_attachment_owners";

#[derive(Debug, foundry::Model)]
#[foundry(table = OWNERS_TABLE)]
struct AttachmentOwner {
    id: ModelId<Self>,
    name: String,
    created_at: DateTime,
    updated_at: DateTime,
}

impl HasAttachments for AttachmentOwner {
    fn attachable_type() -> &'static str {
        OWNERS_TABLE
    }

    fn attachable_id(&self) -> String {
        self.id.to_string()
    }

    fn attachment_specs() -> Vec<AttachmentSpec<Self>> {
        vec![
            AttachmentSpec::file("avatar").single(),
            AttachmentSpec::file("gallery"),
        ]
    }
}

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn attachment_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

struct AttachmentRuntime {
    _dir: TempDir,
    app: AppContext,
    database: std::sync::Arc<DatabaseManager>,
}

impl AttachmentRuntime {
    async fn new() -> Option<Self> {
        let url = postgres_url()?;
        let dir = tempfile::tempdir().ok()?;
        let storage_root = dir.path().join("storage");
        fs::create_dir_all(&storage_root).ok()?;
        fs::write(
            dir.path().join("00-runtime.toml"),
            format!(
                r#"
                [database]
                url = "{url}"

                [storage]
                default = "local"

                [storage.disks.local]
                driver = "local"
                root = "{}"
                visibility = "private"
                "#,
                storage_root.display()
            ),
        )
        .ok()?;

        let kernel = App::builder()
            .load_config_dir(dir.path())
            .build_cli_kernel()
            .await
            .ok()?;
        let app = kernel.app().clone();
        let database = app.database().ok()?;
        reset_tables(database.as_ref()).await;
        Some(Self {
            _dir: dir,
            app,
            database,
        })
    }

    fn upload(&self, name: &str, bytes: &[u8]) -> UploadedFile {
        let path = self._dir.path().join(format!("upload-{name}"));
        fs::write(&path, bytes).unwrap();
        UploadedFile {
            field_name: "file".to_string(),
            original_name: Some(name.to_string()),
            content_type: Some("text/plain".to_string()),
            size: bytes.len() as u64,
            temp_path: path,
        }
    }

    async fn cleanup(&self) {
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS attachments", &[])
            .await;
        let _ = self
            .database
            .raw_execute(&format!("DROP TABLE IF EXISTS {OWNERS_TABLE}"), &[])
            .await;
    }
}

async fn reset_tables(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS attachments", &[])
        .await
        .unwrap();
    database
        .raw_execute(&format!("DROP TABLE IF EXISTS {OWNERS_TABLE}"), &[])
        .await
        .unwrap();
    database
        .raw_execute(
            &format!(
                "CREATE TABLE {OWNERS_TABLE} (
                    id UUID PRIMARY KEY,
                    name TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )"
            ),
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE TABLE attachments (
                id UUID PRIMARY KEY,
                attachable_type TEXT NOT NULL,
                attachable_id UUID NOT NULL,
                collection TEXT NOT NULL DEFAULT 'default',
                disk TEXT NOT NULL,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                original_name TEXT,
                mime_type TEXT,
                size BIGINT NOT NULL DEFAULT 0,
                sort_order INT NOT NULL DEFAULT 0,
                custom_properties JSONB NOT NULL DEFAULT '{}',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )",
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE INDEX idx_attachments_poly ON attachments (attachable_type, attachable_id, collection)",
            &[],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn single_collections_are_serialized_and_multi_collections_can_be_reordered() {
    let _guard = attachment_lock().await;
    let Some(runtime) = AttachmentRuntime::new().await else {
        return;
    };
    let owner = AttachmentOwner::create()
        .set(AttachmentOwner::NAME, "Owner")
        .save(&runtime.app)
        .await
        .unwrap();

    let first_upload = runtime.upload("first.txt", b"first");
    let second_upload = runtime.upload("second.txt", b"second");
    let (first_result, second_result) = tokio::join!(
        owner.attach(&runtime.app, "avatar", first_upload),
        owner.attach(&runtime.app, "avatar", second_upload)
    );
    let first = first_result.unwrap();
    let second = second_result.unwrap();
    let avatars = owner.attachments(&runtime.app, "avatar").await.unwrap();
    assert_eq!(avatars.len(), 1);
    assert!(avatars[0].id == first.id || avatars[0].id == second.id);
    let disk = runtime.app.storage().unwrap().disk("local").unwrap();
    assert!(disk.exists(&avatars[0].path).await.unwrap());
    let replaced = if avatars[0].id == first.id {
        &second
    } else {
        &first
    };
    assert!(!disk.exists(&replaced.path).await.unwrap());

    let mut gallery = Vec::new();
    for (name, bytes) in [
        ("one.txt", b"one".as_slice()),
        ("two.txt", b"two".as_slice()),
        ("three.txt", b"three".as_slice()),
    ] {
        gallery.push(
            owner
                .attach(&runtime.app, "gallery", runtime.upload(name, bytes))
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        gallery
            .iter()
            .map(|attachment| attachment.sort_order)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );

    let reversed_ids = gallery
        .iter()
        .rev()
        .map(|attachment| attachment.id.clone())
        .collect::<Vec<_>>();
    let reordered = owner
        .reorder_attachments(&runtime.app, "gallery", &reversed_ids)
        .await
        .unwrap();
    assert_eq!(
        reordered
            .iter()
            .map(|attachment| attachment.id.as_str())
            .collect::<Vec<_>>(),
        reversed_ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(
        owner
            .attachments(&runtime.app, "gallery")
            .await
            .unwrap()
            .iter()
            .map(|attachment| attachment.id.as_str())
            .collect::<Vec<_>>(),
        reversed_ids.iter().map(String::as_str).collect::<Vec<_>>()
    );

    let invalid = vec![reversed_ids[0].clone(), reversed_ids[0].clone()];
    let error = owner
        .reorder_attachments(&runtime.app, "gallery", &invalid)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("more than once"));

    runtime.cleanup().await;
}
