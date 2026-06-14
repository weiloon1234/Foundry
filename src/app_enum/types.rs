use crate::support::Collection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumKey {
    String(String),
    Int(i32),
}

impl EnumKey {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Int(_) => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Self::String(_) => None,
            Self::Int(value) => Some(*value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumKeyKind {
    String,
    Int,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumOption {
    pub value: EnumKey,
    pub label_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumMeta {
    pub id: String,
    pub key_kind: EnumKeyKind,
    pub options: Collection<EnumOption>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_key_string_serialization() {
        let key = EnumKey::String("pending".into());
        let json = serde_json::to_string(&key).unwrap();
        let back: EnumKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, key);
    }

    #[test]
    fn enum_key_int_serialization() {
        let key = EnumKey::Int(42);
        let json = serde_json::to_string(&key).unwrap();
        let back: EnumKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, key);
    }

    #[test]
    fn enum_meta_serialization() {
        let meta = EnumMeta {
            id: "status".into(),
            key_kind: EnumKeyKind::String,
            options: Collection::from(vec![
                EnumOption {
                    value: EnumKey::String("pending".into()),
                    label_key: "enum.status.pending".into(),
                },
                EnumOption {
                    value: EnumKey::String("active".into()),
                    label_key: "enum.status.active".into(),
                },
            ]),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: EnumMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
    }
}
