use anyhow::{Context, Result};
use std::process::Command;

fn open_command() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

/// Open a list of URLs in the default browser.
pub fn open(urls: Option<&Vec<String>>) -> Result<()> {
    let Some(urls) = urls.filter(|u| !u.is_empty()) else {
        return Ok(());
    };

    let cmd = open_command();
    for url in urls {
        Command::new(cmd)
            .arg(url)
            .spawn()
            .with_context(|| format!("Failed to open URL '{url}'"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn none_urls_is_ok() {
        assert!(open(None).is_ok());
    }

    #[test]
    fn empty_urls_is_ok() {
        let urls = vec![];
        assert!(open(Some(&urls)).is_ok());
    }
}
