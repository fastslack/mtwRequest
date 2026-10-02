//! Prelude module — import everything a module developer typically needs.
//!
//! ```
//! use mtw_sdk::prelude::*;
//! ```

// Core module system
pub use mtw_core::error::MtwError;
pub use mtw_core::module::{
    HealthStatus, ModuleContext, ModuleDep, ModuleManifest, ModuleType, MtwModule, Permission,
};

// Protocol types
pub use mtw_protocol::message::{ConnId, MsgType, MtwMessage, Payload, TransportEvent};

// Middleware
pub use mtw_router::middleware::{MiddlewareAction, MiddlewareContext, MtwMiddleware};

// Codec
pub use mtw_codec::MtwCodec;

// Builder
pub use crate::builder::{create_manifest, default_manifest, ModuleManifestBuilder};

// Common external re-exports
pub use async_trait::async_trait;
pub use serde::{Deserialize, Serialize};
pub use std::collections::HashMap;
pub use std::sync::Arc;
