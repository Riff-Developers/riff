use url::Url;

use crate::events::{
    EngineStatus, Event, LoginMethod, SessionChanged, SessionFields, SessionStatus,
};

#[derive(Clone, Debug)]
pub enum SessionAction {
    TryLogin(LoginMethod),
    OpenLoginUrl(Url),
    LoginSucceeded(String),
    LoginFailed,
    NotPremium,
    TokenRefreshed,
    TokenRejected,
    Logout,
    SetConnectionLost(bool),
    SetEngine(EngineStatus),
}

#[derive(Debug, Default)]
pub struct Session {
    pub user: Option<String>,
    pub status: SessionStatus,
    pub connection_lost: bool,
    pub engine: EngineStatus,
}

impl Session {
    pub fn apply(&mut self, action: SessionAction) -> Vec<Event> {
        match action {
            SessionAction::TryLogin(method) => self.set_status(SessionStatus::LoggingIn(method)),
            SessionAction::OpenLoginUrl(url) => {
                self.set_status(SessionStatus::WaitingForBrowser(url))
            }
            SessionAction::LoginSucceeded(user) => {
                self.user = Some(user);
                self.set_status(SessionStatus::LoggedIn)
            }
            SessionAction::LoginFailed => self.set_status(SessionStatus::Failed),
            SessionAction::NotPremium => self.set_status(SessionStatus::NotPremium),
            SessionAction::TokenRefreshed => vec![Event::TokenRefreshed],
            SessionAction::TokenRejected => vec![Event::TokenRejected],
            SessionAction::Logout => {
                self.user = None;
                self.set_status(SessionStatus::LoggedOut)
            }
            SessionAction::SetConnectionLost(lost) if lost != self.connection_lost => {
                self.connection_lost = lost;
                vec![self.changed(SessionFields::CONNECTION)]
            }
            SessionAction::SetConnectionLost(_) => vec![],
            SessionAction::SetEngine(engine) if engine != self.engine => {
                self.engine = engine;
                vec![self.changed(SessionFields::ENGINE)]
            }
            SessionAction::SetEngine(_) => vec![],
        }
    }

    fn set_status(&mut self, status: SessionStatus) -> Vec<Event> {
        self.status = status;
        vec![self.changed(SessionFields::STATUS)]
    }

    fn changed(&self, changed: SessionFields) -> Event {
        SessionChanged {
            changed,
            status: self.status.clone(),
            connection_lost: self.connection_lost,
            engine: self.engine.clone(),
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_changes_are_told_once() {
        let mut session = Session::default();
        let needs_key = || {
            SessionAction::SetEngine(EngineStatus::NeedsKey {
                key_page: "https://example.com/keys".to_string(),
                refused: false,
            })
        };
        let events = session.apply(needs_key());
        assert!(matches!(
            &events[..],
            [Event::Session(changed)] if changed.changed == SessionFields::ENGINE
                && matches!(changed.engine, EngineStatus::NeedsKey { .. })
        ));
        assert!(session.apply(needs_key()).is_empty());
    }

    #[test]
    fn logging_in_and_out() {
        let mut session = Session::default();
        session.apply(SessionAction::TryLogin(LoginMethod::Saved));
        assert_eq!(session.status, SessionStatus::LoggingIn(LoginMethod::Saved));

        let events = session.apply(SessionAction::LoginSucceeded("me".into()));
        assert!(matches!(&events[..], [Event::Session(s)] if s.became(&SessionStatus::LoggedIn)));
        assert_eq!(session.user.as_deref(), Some("me"));

        session.apply(SessionAction::Logout);
        assert_eq!(session.user, None);
        assert_eq!(session.status, SessionStatus::LoggedOut);
    }

    #[test]
    fn connection_changes_are_only_sent_once() {
        let mut session = Session::default();
        assert_eq!(
            session.apply(SessionAction::SetConnectionLost(true)).len(),
            1
        );
        assert!(session
            .apply(SessionAction::SetConnectionLost(true))
            .is_empty());
        assert!(session.connection_lost);
    }
}
