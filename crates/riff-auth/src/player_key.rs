use anyhow::Result;

use crate::token_store::TokenStore;

const ATTRS: &[(&str, &str)] = &[("riff-player-key", "yes")];
const LABEL: &str = "Riff Playback Engine Key";

pub struct PlayerKey;

impl PlayerKey {
    pub async fn get() -> Option<String> {
        match Self::find().await {
            Ok(key) => key,
            Err(e) => {
                warn!("Couldn't read the playback engine key: {e}");
                None
            }
        }
    }

    pub async fn set(key: &str) -> Result<()> {
        TokenStore::keyring()
            .await?
            .create_item(LABEL, &ATTRS, key.trim().as_bytes(), true)
            .await?;
        info!("Saved the playback engine key");
        Ok(())
    }

    pub async fn clear() -> Result<()> {
        let keyring = TokenStore::keyring().await?;
        for item in keyring.search_items(&ATTRS).await? {
            item.delete().await?;
        }
        Ok(())
    }

    async fn find() -> Result<Option<String>> {
        let keyring = TokenStore::keyring().await?;
        if let Err(e) = keyring.unlock().await {
            debug!("Couldn't unlock the keyring: {e}");
        }
        let Some(item) = keyring.search_items(&ATTRS).await?.into_iter().next() else {
            return Ok(None);
        };
        let secret = item.secret().await?;
        let key = String::from_utf8_lossy(secret.as_bytes())
            .trim()
            .to_string();
        Ok((!key.is_empty()).then_some(key))
    }
}
