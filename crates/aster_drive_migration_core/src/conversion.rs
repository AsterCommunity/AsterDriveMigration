//! Source adapter contracts that do not depend on either database schema.

#[derive(Debug, Clone, Copy, Default)]
pub struct ConversionContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipReason {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Conversion<T> {
    Ready(T),
    Skipped(SkipReason),
}

impl<T> Conversion<T> {
    #[must_use]
    pub fn into_ready(self) -> Option<T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Skipped(_) => None,
        }
    }
}

pub trait SourceConverter<Source> {
    type Output;
    type Error;

    fn convert(
        &self,
        source: Source,
        context: &ConversionContext,
    ) -> Result<Conversion<Self::Output>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_ready_and_skipped_have_explicit_boundaries() {
        let ready = Conversion::Ready(42_u32);
        assert_eq!(ready.clone().into_ready(), Some(42));
        assert_eq!(ready, Conversion::Ready(42));

        let skipped = Conversion::<u32>::Skipped(SkipReason {
            code: "unsupported",
            message: "provider is not supported".to_string(),
        });
        assert_eq!(skipped.clone().into_ready(), None);
        assert_eq!(skipped, skipped.clone());
        assert_eq!(skipped.clone().into_ready(), None);
    }

    #[test]
    fn skip_reason_preserves_static_code_and_owned_message() {
        let reason = SkipReason {
            code: "encrypted",
            message: String::new(),
        };
        assert_eq!(reason.code, "encrypted");
        assert!(reason.message.is_empty());
    }
}
