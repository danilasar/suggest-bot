use std::time::Duration;

use anyhow::anyhow;

pub fn parse_duration(text: &str) -> anyhow::Result<Option<Duration>> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(None);
    }

    let input = parts[1].to_lowercase();
    if input == "permanent" {
        return Ok(None);
    }

    if input.len() < 2 {
        return Err(anyhow!("Некорректный формат времени"));
    }

    let (num, unit) = input.split_at(input.len() - 1);
    let num = num.parse::<u64>()?;

    match unit {
        "m" => Ok(Some(Duration::from_secs(num * 60))),
        "h" => Ok(Some(Duration::from_secs(num * 3600))),
        "d" => Ok(Some(Duration::from_secs(num * 86400))),
        _ => Err(anyhow!("Некорректная единица времени: используйте m, h или d")),
    }
}

