use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use riff_auth::{AuthcodeChallenge, Credentials, OAuthError, PlayerKey, RiffOauthClient};
use tokio::task::JoinHandle;

use super::{Command, Engine, PlayerEvents, SpotifyError};

pub(super) struct Librespot {
    player: Option<UnboundedSender<Command>>,
    start: Box<dyn FnMut() -> UnboundedSender<Command> + Send>,
}

impl Librespot {
    pub(super) fn new(start: impl FnMut() -> UnboundedSender<Command> + Send + 'static) -> Self {
        Self {
            player: None,
            start: Box::new(start),
        }
    }

    fn get_or_start(&mut self) -> UnboundedSender<Command> {
        let start = &mut self.start;
        self.player.get_or_insert_with(start).clone()
    }
}

pub(super) struct Account {
    oauth_client: Arc<RiffOauthClient>,
    auth_challenge: Option<AuthcodeChallenge>,
    commands: UnboundedSender<Command>,
    librespot: Librespot,
    refresh_task: Option<JoinHandle<()>>,
    refreshed_at: Option<std::time::Instant>,
    delegate: PlayerEvents,
}

const REFRESH_AT_MOST_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

impl Account {
    pub(super) fn new(
        oauth_client: Arc<RiffOauthClient>,
        commands: UnboundedSender<Command>,
        librespot: Librespot,
        delegate: PlayerEvents,
    ) -> Self {
        Self {
            oauth_client,
            auth_challenge: None,
            commands,
            librespot,
            refresh_task: None,
            refreshed_at: None,
            delegate,
        }
    }

    pub(super) fn librespot(&self) -> Option<&UnboundedSender<Command>> {
        self.librespot.player.as_ref()
    }

    pub(super) async fn handle(&mut self, command: Command) {
        if let Err(e) = self.try_handle(command).await {
            self.delegate.report_error(e);
        }
    }

    async fn try_handle(&mut self, command: Command) -> Result<(), SpotifyError> {
        match command {
            Command::Restore => {
                info!("Attempting to restore session from stored credentials");
                let credentials =
                    self.oauth_client
                        .get_valid_token()
                        .await
                        .map_err(|e| match e {
                            OAuthError::LoggedOut => {
                                info!("No stored credentials to restore; showing login");
                                SpotifyError::LoggedOut
                            }
                            e => {
                                error!("Failed to get valid token during restore: {e:?}");
                                SpotifyError::LoginFailed
                            }
                        })?;
                info!(
                    "Restoring session (token expires at {:?})",
                    credentials.token_expiry_time
                );
                if !self.oauth_client.restore_session_credentials().await {
                    info!("No stored session token; showing login");
                    return Err(SpotifyError::LoggedOut);
                }
                self.log_in(credentials, None).await
            }
            Command::InitLogin => {
                let auth_url = match self.auth_challenge.as_ref() {
                    Some(challenge) => challenge.auth_url.clone(),
                    None => {
                        let commands = self.commands.clone();
                        let challenge = self
                            .oauth_client
                            .spawn_authcode_listener(move || {
                                let _ = commands.unbounded_send(Command::CompleteLogin);
                            })
                            .await
                            .map_err(|_| SpotifyError::LoginFailed)?;
                        let auth_url = challenge.auth_url.clone();
                        self.auth_challenge = Some(challenge);
                        auth_url
                    }
                };
                self.delegate.login_challenge_started(auth_url);
                Ok(())
            }
            Command::CompleteLogin => {
                let Some(challenge) = self.auth_challenge.take() else {
                    error!("CompleteLogin called but no auth challenge was pending");
                    return Err(SpotifyError::LoginFailed);
                };
                info!("Exchanging auth codes for tokens");
                let login = self
                    .oauth_client
                    .exchange_authcode(challenge)
                    .await
                    .map_err(|e| {
                        error!("Auth code exchange failed: {e:?}");
                        SpotifyError::LoginFailed
                    })?;
                info!(
                    "Login with OAuth2 (token expires at {:?})",
                    login.web_api.token_expiry_time
                );
                self.log_in(login.web_api, Some(login.session)).await
            }
            Command::Logout => {
                self.oauth_client.clear_credentials().await;
                if let Err(e) = PlayerKey::clear().await {
                    warn!("Couldn't remove the playback engine key: {e}");
                }
                if let Some(task) = self.refresh_task.take() {
                    task.abort();
                }
                self.to_player(Command::Logout);
                Ok(())
            }
            Command::RefreshToken => {
                if self
                    .refreshed_at
                    .is_some_and(|at| at.elapsed() < REFRESH_AT_MOST_EVERY)
                {
                    self.delegate.refresh_successful();
                    return Ok(());
                }
                info!("A request was refused for its token; renewing it");
                match self.oauth_client.refresh_now().await {
                    Ok(_) => {
                        self.refreshed_at = Some(std::time::Instant::now());
                        self.to_player(Command::ReconnectSession);
                        self.delegate.refresh_successful();
                        Ok(())
                    }
                    Err(OAuthError::LoggedOut | OAuthError::RefreshFailed { .. }) => {
                        Err(SpotifyError::LoggedOut)
                    }
                    Err(e) => {
                        warn!("Couldn't renew the token: {e}");
                        Ok(())
                    }
                }
            }
            #[cfg(debug_assertions)]
            Command::DevExpireToken => {
                warn!("[dev] Expiring cached OAuth token and forcing a refresh");
                match self.oauth_client.dev_expire_and_refresh().await {
                    Ok(_) => {
                        info!("[dev] Token refresh succeeded");
                        self.delegate.refresh_successful();
                        Ok(())
                    }
                    Err(OAuthError::LoggedOut) => Err(SpotifyError::LoggedOut),
                    Err(e) => {
                        warn!("[dev] Token refresh failed: {e}");
                        Err(SpotifyError::LoginFailed)
                    }
                }
            }
            command => {
                warn!("The account doesn't handle {command:?}");
                Ok(())
            }
        }
    }

    async fn log_in(
        &mut self,
        credentials: Credentials,
        session: Option<Credentials>,
    ) -> Result<(), SpotifyError> {
        let profile = match riff_api::check_user_profile(&credentials.access_token).await {
            Ok(profile) => profile,
            Err(e) => {
                error!("User profile check failed: {e:?}");
                return Err(SpotifyError::LoginFailed);
            }
        };
        if !profile.is_premium {
            warn!("Account is not premium, aborting login");
            return Err(SpotifyError::NotPremium);
        }
        if profile.explicit_filter_enabled {
            info!("Account has explicit content filter enabled");
            self.delegate.set_skip_explicit(true);
        }
        if profile.explicit_filter_locked {
            info!("Account explicit content filter is locked");
        }
        self.delegate
            .set_explicit_filter_locked(profile.explicit_filter_locked);

        self.oauth_client.save_credentials(&credentials).await;
        if let Some(session) = &session {
            self.oauth_client.save_session_credentials(session).await;
        }
        match Engine::chosen() {
            Engine::Librespot => {
                let player = self.librespot.get_or_start();
                self.keep_tokens_fresh(Some(player.clone()));
                let _ = player.unbounded_send(Command::LoggedIn {
                    user_id: profile.user_id,
                    session_token: session.map(|session| session.access_token),
                    explicit_filter_locked: profile.explicit_filter_locked,
                });
            }
            Engine::Soloist => {
                self.keep_tokens_fresh(None);
                info!("Logged in as {}", profile.user_id);
                self.delegate.token_login_successful(profile.user_id);
            }
        }
        Ok(())
    }

    fn keep_tokens_fresh(&mut self, player: Option<UnboundedSender<Command>>) {
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
        let oauth_client = Arc::clone(&self.oauth_client);
        self.refresh_task = Some(tokio::task::spawn(async move {
            loop {
                match oauth_client.refresh_token_at_expiry().await {
                    Ok(_) => {
                        if let Some(player) = &player {
                            if player.unbounded_send(Command::ReconnectSession).is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Token refresh loop stopping: {e}");
                        break;
                    }
                }
            }
        }));
    }

    fn to_player(&self, command: Command) {
        if let Some(player) = self.librespot() {
            let _ = player.unbounded_send(command);
        }
    }
}
