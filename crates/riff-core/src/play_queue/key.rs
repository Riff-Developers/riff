use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKey {
    Current,
    Next(usize),
}

impl fmt::Display for EntryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Current => write!(f, "queue:now"),
            Self::Next(i) => write!(f, "queue:n:{i}"),
        }
    }
}

impl FromStr for EntryKey {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix("queue:").ok_or(())? {
            "now" => Ok(Self::Current),
            rest => rest
                .strip_prefix("n:")
                .and_then(|i| i.parse().ok())
                .map(Self::Next)
                .ok_or(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_key_round_trip() {
        for key in [EntryKey::Current, EntryKey::Next(3)] {
            assert_eq!(key.to_string().parse::<EntryKey>(), Ok(key));
        }
        assert!("abc".parse::<EntryKey>().is_err());
        assert!("queue:x:1".parse::<EntryKey>().is_err());
    }
}
