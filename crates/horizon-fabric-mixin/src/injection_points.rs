//! Mixin injection point definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An injection point in a Mixin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionPoint {
    /// The target method (intermediary name).
    pub method: String,
    
    /// The target method descriptor.
    pub descriptor: String,
    
    /// The injection point kind.
    #[serde(flatten)]
    pub kind: InjectionPointKind,
}

/// The kind of injection point.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InjectionPointKind {
    /// @Inject - inject code at a location.
    #[serde(rename = "INJECT")]
    Inject {
        /// Where to inject (HEAD, TAIL, INVOKE, etc.).
        at: At,
        /// Optional slice (for INVOKE).
        slice: Option<String>,
        /// Whether this injection is cancellable.
        cancellable: bool,
        /// Locals to capture.
        locals: Option<Locals>,
    },
    
    /// @ModifyVariable - modify a local variable.
    #[serde(rename = "MODIFY_VARIABLE")]
    ModifyVariable {
        /// Where to modify.
        at: At,
        /// Variable index or name.
        index: Option<i32>,
        name: Option<String>,
        /// Variable type descriptor.
        type_desc: Option<String>,
    },
    
    /// @Redirect - redirect a method call/field access.
    #[serde(rename = "REDIRECT")]
    Redirect {
        /// Where to redirect.
        at: At,
        /// The target method to redirect to.
        target: String,
        /// Target method descriptor.
        target_descriptor: String,
    },
    
    /// @Overwrite - overwrite a method entirely.
    #[serde(rename = "OVERWRITE")]
    Overwrite,
}

/// Where to inject/modify/redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum At {
    /// At the start of the method.
    Head,
    /// At the end of the method.
    Tail,
    /// At a method invocation.
    Invoke,
    /// At a field read/write.
    Field,
    /// At a new instance creation.
    New,
    /// At a return.
    Return,
    /// At a specific line number (requires line number table).
    Line(i32),
}

/// Locals capture configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Locals {
    /// Capture all locals.
    #[serde(default)]
    pub capture_all: bool,
    /// Specific locals to capture (by index or name).
    pub locals: HashMap<String, LocalCapture>,
}

/// A local variable capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCapture {
    /// Variable index.
    pub index: Option<i32>,
    /// Variable name.
    pub name: Option<String>,
    /// Variable type.
    pub type_desc: Option<String>,
    /// Whether to capture as final (for lambdas).
    #[serde(default)]
    pub final_: bool,
}

/// @ModifyVariable annotation data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyVariable {
    /// Target method name.
    pub method: String,
    /// Target method descriptor.
    pub descriptor: String,
    /// Where in the method to apply the modification.
    pub at: At,
    /// Local variable slot index.
    pub index: Option<i32>,
    /// Local variable name.
    pub name: Option<String>,
    /// Local variable type descriptor.
    pub type_desc: Option<String>,
}

/// @Redirect annotation data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redirect {
    /// Target method name.
    pub method: String,
    /// Target method descriptor.
    pub descriptor: String,
    /// Where in the method to redirect.
    pub at: At,
    /// Owner of the redirected call.
    pub target: String,
    /// Descriptor of the redirected call.
    pub target_descriptor: String,
}

/// @Inject annotation data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inject {
    /// Target method name.
    pub method: String,
    /// Target method descriptor.
    pub descriptor: String,
    /// Where in the method to inject.
    pub at: At,
    /// Optional slice selecting the instruction range.
    pub slice: Option<String>,
    /// Whether the injected code may cancel the target.
    #[serde(default)]
    pub cancellable: bool,
    /// Locals exposed to the handler.
    pub locals: Option<Locals>,
}