use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{TimeDelta, Utc};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use log::info;

const DB_URL: &str = "sqlite:bans.db";

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new() -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .connect(DB_URL)
            .await
            .context("Failed to connect to database")?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS bans (
                user_id INTEGER PRIMARY KEY,
                expires_at INTEGER NOT NULL
            )"#,
        )
        .execute(&pool)
        .await
        .context("Failed to create bans table")?;

        Ok(Self { pool })
    }

    pub async fn is_banned(&self, user_id: i64) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let result = sqlx::query(
            "SELECT expires_at FROM bans WHERE user_id = ? AND expires_at > ?"
        )
        .bind(user_id)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(result.is_some())
    }

    pub async fn ban_user(&self, user_id: i64, duration: Option<Duration>) -> anyhow::Result<()> {
        let expires_at = match duration {
            Some(d) => Utc::now() + TimeDelta::from_std(d).map_err(|e| anyhow!("Invalid duration: {}", e))?,
            None => Utc::now() + TimeDelta::days(365 * 100), // ~100 years
        };

        sqlx::query(
            "INSERT OR REPLACE INTO bans (user_id, expires_at) VALUES (?, ?)"
        )
        .bind(user_id)
        .bind(expires_at.timestamp())
        .execute(&self.pool)
        .await
        .context("Failed to ban user")?;
        Ok(())
    }

    pub async fn unban_user(&self, user_id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM bans WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .context("Failed to unban user")?;
        Ok(())
    }

    pub async fn cleanup_expired_bans(&self) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        sqlx::query("DELETE FROM bans WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await
            .context("Failed to clean up expired bans")?;
        info!("Очистка устаревших банов выполнена");
        Ok(())
    }
}
