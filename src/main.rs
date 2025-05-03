mod db;
mod handlers;
mod utils;

use std::sync::Arc;
use std::time::Duration;

use db::Database;
use governor::{Quota, RateLimiter};
use governor::DefaultKeyedRateLimiter;
use nonzero_ext::nonzero;
use teloxide::prelude::*;
use log::{info, error};

pub(crate) struct Config {
    target_chat_id: i64,
    admin_ids: Vec<i64>,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            target_chat_id: std::env::var("TARGET_CHAT_ID")?.parse()?,
            admin_ids: std::env::var("ADMIN_IDS")
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
        })
    }

    fn is_admin(&self, user_id: i64) -> bool {
        self.admin_ids.contains(&user_id)
    }
}

struct RateLimits {
    per_second: Arc<DefaultKeyedRateLimiter<i64>>,
    per_minute: Arc<DefaultKeyedRateLimiter<i64>>,
}

impl RateLimits {
    fn new() -> Self {
        Self {
            per_second: Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(1u32)))),
            per_minute: Arc::new(RateLimiter::keyed(Quota::per_minute(nonzero!(10u32)))),
        }
    }
}


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    //env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    env_logger::Builder::from_default_env().init();

    let config = Arc::new(Config::from_env()?);
    let db: Arc<Database> = Arc::new(Database::new().await?);
    let limits = Arc::new(RateLimits::new());
    let bot = Bot::from_env();

    info!("🚀 Бот запущен");

    let db_clone = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            if let Err(e) = db_clone.cleanup_expired_bans().await {
                error!("Ошибка при очистке банов: {}", e);
            }
        }
    });

    Dispatcher::builder(bot, Update::filter_message().endpoint(move |bot, msg| {
        let config = config.clone();
        let db = db.clone();
        let limits = limits.clone();
        async move {
            if let Err(e) = handlers::handle_message(bot, msg, config, db, limits).await {
                error!("Ошибка обработки сообщения: {}", e);
            }
            respond(())
        }
    }))
    .enable_ctrlc_handler()
    .build()
    .dispatch()
    .await;

    Ok(())
}
