//! Why a `build-core` operation failed.
//!
//! Same shape as `run_core::RunError` (ADR-0003): a typed variant per
//! failure kind, a stable numeric code in the 200-299 range ADR-0040 claims
//! for this crate out of ADR-0003 §4's headroom, and a `Display` message
//! meant to be shown verbatim.

use std::fmt;

/// Why a build could not be started or could not be understood.
#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    /// The project has no toolchain this crate knows how to build — an
    /// empty directory, or one whose only toolchain is Python, which has no
    /// build step of its own.
    NoBuildableToolchain,
    /// The requested toolchain is not the one the project uses, or has no
    /// build command (`ToolchainId::build_command` answered `None`).
    UnsupportedToolchain(String),
    /// Rebuild was asked for on a toolchain with no clean step of its own,
    /// so "clean, then build" cannot be honoured.
    NoCleanStep(String),
}

impl BuildError {
    pub const CODE_NO_BUILDABLE_TOOLCHAIN: i32 = 200;
    pub const CODE_UNSUPPORTED_TOOLCHAIN: i32 = 201;
    pub const CODE_NO_CLEAN_STEP: i32 = 202;

    /// The variant's stable numeric code. Append-only once this crosses an
    /// FFI seam (ADR-0003): existing numbers must never be renumbered.
    pub fn code(&self) -> i32 {
        match self {
            BuildError::NoBuildableToolchain => Self::CODE_NO_BUILDABLE_TOOLCHAIN,
            BuildError::UnsupportedToolchain(_) => Self::CODE_UNSUPPORTED_TOOLCHAIN,
            BuildError::NoCleanStep(_) => Self::CODE_NO_CLEAN_STEP,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::NoBuildableToolchain => {
                write!(f, "this project has no build tool to run")
            }
            BuildError::UnsupportedToolchain(name) => {
                write!(f, "{name} has no build command")
            }
            BuildError::NoCleanStep(name) => {
                write!(f, "{name} cannot be rebuilt: it has no clean step")
            }
        }
    }
}

impl std::error::Error for BuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_unique_and_inside_the_range() {
        let codes = [
            BuildError::NoBuildableToolchain.code(),
            BuildError::UnsupportedToolchain(String::new()).code(),
            BuildError::NoCleanStep(String::new()).code(),
        ];
        for code in codes {
            assert!(
                (200..=299).contains(&code),
                "{code} left build-core's 200-299 range (ADR-0003 §4)"
            );
        }
        let mut sorted = codes;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), codes.len(), "two variants share a code");
    }
}
