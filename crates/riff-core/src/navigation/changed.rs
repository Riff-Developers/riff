use bitflags::bitflags;

use super::Transition;

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NavigationFields: u8 {
        const ROUTE = 1 << 0;
        const HIDDEN = 1 << 1;
        const HISTORY = 1 << 2;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationChanged {
    pub changed: NavigationFields,
    pub transition: Option<Transition>,
    pub hidden: bool,
}

impl NavigationChanged {
    pub fn moved(transition: Transition, hidden: bool) -> Self {
        Self {
            changed: NavigationFields::ROUTE,
            transition: Some(transition),
            hidden,
        }
    }

    pub fn hidden(hidden: bool) -> Self {
        Self {
            changed: NavigationFields::HIDDEN,
            transition: None,
            hidden,
        }
    }

    pub fn history(hidden: bool) -> Self {
        Self {
            changed: NavigationFields::HISTORY,
            transition: None,
            hidden,
        }
    }
}
