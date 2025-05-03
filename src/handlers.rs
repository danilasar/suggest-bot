use std::sync::Arc;

use anyhow::{anyhow, Context};
use governor::clock::Clock;
use teloxide::{prelude::*, utils::command::BotCommands};
use teloxide::types::ParseMode;
use log::{error, warn, debug, info};
use super::{Config, db::Database, RateLimits, utils::parse_duration};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub(crate) enum Command {
    #[command(description = "показать автора сообщения")]
    Who,
    #[command(description = "заблокировать пользователя (пример: /ban 30m)")]
    Ban,
    #[command(description = "разблокировать пользователя")]
    Unban,
}

pub async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<Config>,
    db: Arc<Database>,
    limits: Arc<RateLimits>,
) -> anyhow::Result<()> {
    if msg.chat.id == ChatId(config.target_chat_id) {
        if let Ok(command) = Command::parse(msg.text().unwrap_or_default(), "") {
           info!("Command received: {:?} from {}", command, msg.chat.id);
            return handle_command(bot, msg, command, config, db).await;
        } else {
           return handle_reply_to_forwarded_message(bot, msg).await;
        }
    }

    if msg.chat.is_private() {
        let user_id = msg.chat.id.0;
        debug!("Processing message from user {}", user_id);

        match db.is_banned(user_id).await {
            Ok(true) => {
                warn!("User {} is banned", user_id);
                bot.send_message(msg.chat.id, "⛔ Вы заблокированы").await?;
                return Ok(());
            }
            Err(e) => return Err(e),
            _ => {}
        }

        if let Err(until) = limits.per_second.check_key(&user_id) {
            let clock = governor::clock::QuantaClock::default();
            let wait = until.wait_time_from(clock.now());
            warn!("Rate limit (1s) exceeded for {}", user_id);
            bot.send_message(msg.chat.id, format!("⚠ Подождите {} секунд", wait.as_secs()))
                .await?;
            return Ok(());
        }

        if let Err(until) = limits.per_minute.check_key(&user_id) {
            let clock = governor::clock::QuantaClock::default();
            let wait = until.wait_time_from(clock.now());
            warn!("Rate limit (10m) exceeded for {}", user_id);
            bot.send_message(msg.chat.id, format!("⚠ Подождите {} секунд", wait.as_secs()))
                .await?;
            return Ok(());
        }

        info!("Forwarding message from {}", user_id);
        bot.forward_message(ChatId(config.target_chat_id), msg.chat.id, msg.id)
            .await
            .context("Failed to forward message")?;
    }

    Ok(())
}

macro_rules! ok_or_tg {
    ($prerequisite:expr, $bot: ident, $source_msg: ident, $text: literal) => {
        {
            let tmp = || async {
                let result = $prerequisite.ok_or_else( || async {
                    let _ = $bot.send_message($source_msg.chat.id, $text).await;
                    anyhow!($text)
                });
                match result {
                    Ok(val) => Ok(val),
                    Err(err) => Err(err.await)
                }
            };
            tmp().await
        }
    };
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    command: Command,
    _: Arc<Config>,
    db: Arc<Database>,
) -> anyhow::Result<()> {
    let user = msg.from().ok_or_else(|| anyhow!("Missing user information"))?;

    let reply_msg = ok_or_tg!(msg.reply_to_message(), bot, msg, "Ответьте на сообщение пользователя")?;
    let target_user = ok_or_tg!(reply_msg.forward_from(), bot, msg, "Не удалось определить пользователя")?;

    match command {
        Command::Who => {
            if let teloxide::types::ForwardedFrom::User(target_user) = target_user {
                info!("Showing info for {}", target_user.id);
                let link = format!("tg://user?id={}", target_user.id);
                bot.send_message(msg.chat.id, format!("👤 [{}]({})", target_user.first_name, link))
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
        }
        Command::Ban => {
            if let teloxide::types::ForwardedFrom::User(target_user) = target_user {
                let text = msg.text().unwrap_or_default();
                let parts: Vec<&str> = text.split_whitespace().collect();

                if parts.len() < 2 {
                    bot.send_message(msg.chat.id, "ℹ️ Формат команды: /ban <время> [m/h/d] или /ban permanent")
                        .await?;
                    return Ok(());
                }

                let duration = parse_duration(text)?;
                db.ban_user(target_user.id.0 as i64, duration).await.unwrap_or_else(|error| {
                    let _ = bot.send_message(msg.chat.id, error.to_string());
                });

                let message = match duration {
                    Some(d) => format!("⏳ Пользователь {} заблокирован на {} минут", 
                                      target_user.first_name, d.as_secs() / 60),
                    None => format!("🔒 Пользователь {} заблокирован навсегда", 
                                   target_user.first_name),
                };

                info!("Banned user {}: {}", target_user.id, message);
                bot.send_message(msg.chat.id, message).await?;
            }
        }
        Command::Unban => {
            if let teloxide::types::ForwardedFrom::User(target_user) = target_user {
                db.unban_user(target_user.id.0 as i64).await?;
                info!("Unbanned user {}", target_user.id);
                let result = bot.send_message(msg.chat.id, format!("🔓 Пользователь {} разблокирован", 
                                                     target_user.first_name))
                    .await;
                if let Err(error) = result {
                    let _ = bot.send_message(msg.chat.id, error.to_string());
                }
            }
        }
    }

    Ok(())
}


async fn handle_reply_to_forwarded_message(
    bot: Bot,
    msg: Message
) -> anyhow::Result<()> {
    info!("Обработка ответа в целевом чате {}", msg.chat.id);
    if let Some(reply_msg) = msg.reply_to_message() {
        if let Some(teloxide::types::ForwardedFrom::User(target_user)) = reply_msg.forward_from() {
            info!("Отправка ответа пользователю {}", target_user.id);
            match bot.copy_message(target_user.id, msg.chat.id, msg.id).await {
                Ok(_) => info!("Сообщение скопировано успешно"),
                Err(e) => {
                    error!("Ошибка копирования: {}", e);
                    bot.send_message(target_user.id, "Тип сообщения не поддерживается. Постараюсь отправить текст из него.").await?;
                    if let Some(text) = msg.text() {
                        bot.send_message(target_user.id, text).await?;
                    }
                }
            }
        }
    }
    Ok(())
}
