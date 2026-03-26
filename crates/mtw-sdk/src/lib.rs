pub mod builder;
pub mod prelude;
pub mod testing;

// Re-export crates for direct access
pub use mtw_codec;
pub use mtw_core;
pub use mtw_protocol;
pub use mtw_router;

// Re-export the most fundamental types at the top level
pub use mtw_core::error::MtwError;
pub use mtw_core::module::{
    HealthStatus, ModuleContext, ModuleDep, ModuleManifest, ModuleType, MtwModule, Permission,
};
pub use mtw_protocol::message::{ConnId, MsgType, MtwMessage, Payload, TransportEvent};
pub use mtw_router::middleware::{MiddlewareAction, MiddlewareContext, MtwMiddleware};
pub use mtw_codec::MtwCodec;
