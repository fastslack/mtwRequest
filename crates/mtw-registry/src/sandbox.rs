//! WASM sandbox for running untrusted modules.
//!
//! Built on top of `wasmtime`. Each [`WasmSandbox`] owns a shared [`Engine`];
//! [`load_module`](WasmSandbox::load_module) parses and compiles a wasm binary,
//! and [`execute`](WasmSandbox::execute) creates a fresh [`Store`] per call
//! with fuel and memory limits wired through the [`SandboxConfig`].
//!
//! The guest ABI is intentionally narrow and pragmatic:
//!
//! * The guest module must export a linear `memory`.
//! * The guest module must export `mtw_alloc(size: i32) -> i32` returning a
//!   pointer into guest memory for `size` bytes.
//! * The function being invoked must be exported with signature
//!   `(ptr: i32, len: i32) -> i64`. The return value packs a result
//!   `(ptr: i32, len: i32)` as `(hi << 32) | lo`.
//!
//! This is enough to shuttle serialized payloads in and out. Richer
//! host bindings (component model, WASI preview 2, async) are future work.

use std::sync::Mutex;

use mtw_core::module::Permission;
use serde::{Deserialize, Serialize};
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder};

/// Configuration for the WASM sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Maximum memory in bytes
    pub max_memory: u64,
    /// Maximum execution time in milliseconds
    pub max_execution_time: u64,
    /// Whether to enable WASI
    pub enable_wasi: bool,
    /// Fuel units to grant per call. Wasmtime charges ~1 unit per instruction,
    /// so 1_000_000_000 is in the ballpark of "a few seconds on a modern CPU".
    #[serde(default = "default_fuel")]
    pub fuel_per_call: u64,
}

fn default_fuel() -> u64 {
    1_000_000_000
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory: 64 * 1024 * 1024,
            max_execution_time: 30_000,
            enable_wasi: false,
            fuel_per_call: default_fuel(),
        }
    }
}

/// Sandbox permissions — controls what a WASM module can access
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxPermissions {
    /// Allowed network hosts (`*` = any)
    pub allowed_hosts: Vec<String>,
    /// Allowed filesystem paths
    pub allowed_paths: Vec<String>,
    /// Allowed environment variables
    pub allowed_env_vars: Vec<String>,
    /// Whether subprocess spawning is allowed
    pub allow_subprocess: bool,
}

impl SandboxPermissions {
    /// Create permissions from a list of Permission enums
    pub fn from_permissions(permissions: &[Permission]) -> Self {
        let mut sandbox = Self::default();
        for perm in permissions {
            match perm {
                Permission::Network => sandbox.allowed_hosts.push("*".into()),
                Permission::FileSystem => sandbox.allowed_paths.push("*".into()),
                Permission::Environment => sandbox.allowed_env_vars.push("*".into()),
                Permission::Subprocess => sandbox.allow_subprocess = true,
                Permission::Database => sandbox.allowed_hosts.push("*".into()),
                Permission::Custom(_) => {}
            }
        }
        sandbox
    }

    fn allows(&self, requested: &Permission) -> bool {
        match requested {
            Permission::Network | Permission::Database => !self.allowed_hosts.is_empty(),
            Permission::FileSystem => !self.allowed_paths.is_empty(),
            Permission::Environment => !self.allowed_env_vars.is_empty(),
            Permission::Subprocess => self.allow_subprocess,
            Permission::Custom(_) => true,
        }
    }
}

/// Errors specific to sandbox operations
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox engine error: {0}")]
    Engine(String),
    #[error("module not loaded — call load_module first")]
    NotLoaded,
    #[error("module load error: {0}")]
    LoadError(String),
    #[error("permission denied: {0:?}")]
    PermissionDenied(Permission),
    #[error("missing export: {0}")]
    MissingExport(&'static str),
    #[error("guest trap: {0}")]
    Trap(String),
    #[error("wasi is enabled but was not compiled into this build")]
    WasiUnsupported,
}

struct SandboxState {
    limits: StoreLimits,
}

/// WASM sandbox for running untrusted modules
pub struct WasmSandbox {
    engine: Engine,
    config: SandboxConfig,
    permissions: SandboxPermissions,
    module: Mutex<Option<Module>>,
}

impl std::fmt::Debug for WasmSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loaded = self.module.lock().map(|g| g.is_some()).unwrap_or(false);
        f.debug_struct("WasmSandbox")
            .field("config", &self.config)
            .field("permissions", &self.permissions)
            .field("module_loaded", &loaded)
            .finish()
    }
}

impl WasmSandbox {
    /// Create a new WASM sandbox with the given configuration
    pub fn new(config: SandboxConfig, permissions: SandboxPermissions) -> Result<Self, SandboxError> {
        if config.enable_wasi {
            // WASI support is intentionally deferred — see the module-level docs.
            return Err(SandboxError::WasiUnsupported);
        }
        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true).epoch_interruption(false);
        let engine = Engine::new(&engine_config).map_err(|e| SandboxError::Engine(e.to_string()))?;

        Ok(Self {
            engine,
            config,
            permissions,
            module: Mutex::new(None),
        })
    }

    /// Get the sandbox configuration
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Get the sandbox permissions
    pub fn permissions(&self) -> &SandboxPermissions {
        &self.permissions
    }

    /// Load a WASM module into the sandbox
    pub fn load_module(&self, wasm_bytes: &[u8]) -> Result<(), SandboxError> {
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::LoadError(e.to_string()))?;
        *self
            .module
            .lock()
            .expect("sandbox module lock poisoned") = Some(module);
        Ok(())
    }

    /// Validate that a module's requested permissions are satisfied by the
    /// sandbox's permission set.
    pub fn validate_permissions(&self, requested: &[Permission]) -> Result<(), SandboxError> {
        for perm in requested {
            if !self.permissions.allows(perm) {
                return Err(SandboxError::PermissionDenied(perm.clone()));
            }
        }
        Ok(())
    }

    /// Execute a guest function. See the module-level docs for the expected
    /// guest ABI. `args` are copied into guest memory via `mtw_alloc`; the
    /// return value is the bytes the guest writes at the returned pointer.
    pub fn execute(&self, function: &str, args: &[u8]) -> Result<Vec<u8>, SandboxError> {
        let module_guard = self
            .module
            .lock()
            .expect("sandbox module lock poisoned");
        let module = module_guard.as_ref().ok_or(SandboxError::NotLoaded)?;

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory as usize)
            .build();
        let mut store = Store::new(&self.engine, SandboxState { limits });
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(self.config.fuel_per_call)
            .map_err(|e| SandboxError::Engine(e.to_string()))?;

        let linker: Linker<SandboxState> = Linker::new(&self.engine);
        let instance = linker
            .instantiate(&mut store, module)
            .map_err(|e| SandboxError::LoadError(e.to_string()))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(SandboxError::MissingExport("memory"))?;

        let args_ptr = write_bytes(&mut store, instance, memory, args)?;
        let func = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, function)
            .map_err(|_| SandboxError::MissingExport("<function>"))?;
        let packed = func
            .call(&mut store, (args_ptr, args.len() as i32))
            .map_err(|e| SandboxError::Trap(e.to_string()))?;

        let ret_ptr = (packed >> 32) as i32;
        let ret_len = (packed & 0xffff_ffff) as u32 as i32;
        read_bytes(&store, memory, ret_ptr, ret_len)
    }
}

fn write_bytes(
    store: &mut Store<SandboxState>,
    instance: Instance,
    memory: Memory,
    bytes: &[u8],
) -> Result<i32, SandboxError> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "mtw_alloc")
        .map_err(|_| SandboxError::MissingExport("mtw_alloc"))?;
    let ptr = alloc
        .call(&mut *store, bytes.len() as i32)
        .map_err(|e| SandboxError::Trap(e.to_string()))?;
    memory
        .write(&mut *store, ptr as usize, bytes)
        .map_err(|e| SandboxError::Engine(e.to_string()))?;
    Ok(ptr)
}

fn read_bytes(
    store: &Store<SandboxState>,
    memory: Memory,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, SandboxError> {
    if len <= 0 {
        return Ok(Vec::new());
    }
    let mut out = vec![0u8; len as usize];
    memory
        .read(store, ptr as usize, &mut out)
        .map_err(|e| SandboxError::Engine(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasi_enabled_is_rejected() {
        let cfg = SandboxConfig {
            enable_wasi: true,
            ..Default::default()
        };
        let err = WasmSandbox::new(cfg, SandboxPermissions::default()).unwrap_err();
        assert!(matches!(err, SandboxError::WasiUnsupported));
    }

    #[test]
    fn execute_without_load_errors() {
        let sandbox =
            WasmSandbox::new(SandboxConfig::default(), SandboxPermissions::default()).unwrap();
        let err = sandbox.execute("noop", &[]).unwrap_err();
        assert!(matches!(err, SandboxError::NotLoaded));
    }

    #[test]
    fn validate_permissions_denies_missing() {
        let sandbox =
            WasmSandbox::new(SandboxConfig::default(), SandboxPermissions::default()).unwrap();
        let err = sandbox
            .validate_permissions(&[Permission::Network])
            .unwrap_err();
        assert!(matches!(err, SandboxError::PermissionDenied(Permission::Network)));
    }

    #[test]
    fn validate_permissions_allows_granted() {
        let sandbox = WasmSandbox::new(
            SandboxConfig::default(),
            SandboxPermissions::from_permissions(&[Permission::Network]),
        )
        .unwrap();
        sandbox
            .validate_permissions(&[Permission::Network])
            .unwrap();
    }

    #[test]
    fn load_invalid_wasm_errors() {
        let sandbox =
            WasmSandbox::new(SandboxConfig::default(), SandboxPermissions::default()).unwrap();
        let err = sandbox.load_module(b"not wasm").unwrap_err();
        assert!(matches!(err, SandboxError::LoadError(_)));
    }

    #[test]
    fn load_trivial_wasm_succeeds() {
        // Minimal valid wasm: just the magic header + version.
        let wasm = wat::parse_str("(module)").expect("valid wat");
        let sandbox =
            WasmSandbox::new(SandboxConfig::default(), SandboxPermissions::default()).unwrap();
        sandbox.load_module(&wasm).unwrap();
    }

    #[test]
    fn execute_echo_round_trip() {
        // A tiny guest that echoes its input back unchanged. It exports memory,
        // `mtw_alloc` (bump allocator from offset 0x1000), and `echo`.
        let wasm = wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (global $bump (mut i32) (i32.const 4096))
              (func (export "mtw_alloc") (param $n i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $n)))
                (local.get $ptr))
              (func (export "echo") (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                  (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                  (i64.extend_i32_u (local.get $len))))
            )
        "#,
        )
        .expect("valid wat");

        let sandbox =
            WasmSandbox::new(SandboxConfig::default(), SandboxPermissions::default()).unwrap();
        sandbox.load_module(&wasm).unwrap();
        let out = sandbox.execute("echo", b"hello").unwrap();
        assert_eq!(out, b"hello");
    }
}
