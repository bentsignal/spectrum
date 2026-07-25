use std::{error::Error, fmt};

/// A fail-closed reason that a text run could not be shaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError {
    message: String,
}

impl ShapeError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ShapeError {}

/// A fail-closed reason that a font was not emitted as a subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetError {
    message: String,
}

impl SubsetError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn from_shape(error: ShapeError) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for SubsetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SubsetError {}
