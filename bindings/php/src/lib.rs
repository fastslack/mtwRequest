// =============================================================================
// mtwRequest — C FFI Binding for PHP
// =============================================================================
//
// This module provides a C ABI interface for mtwRequest, designed to be
// consumed by PHP's FFI extension (PHP 7.4+).
//
// Build: cargo build --release  (produces libmtw_binding_php.so / .dylib / .dll)
//
// PHP usage with FFI:
//   $ffi = FFI::cdef(file_get_contents("mtw_request.h"), "libmtw_binding_php.so");
//   $client = $ffi->mtw_connect("ws://localhost:8080/ws", "token");
//   $ffi->mtw_send($client, "chat.general", "hello");
//   $ffi->mtw_destroy_client($client);
//
// All functions follow these conventions:
//   - Return opaque pointers (void*) for handles (MtwClient*, MtwChannel*, MtwAgent*)
//   - Accept C strings (const char*) for text parameters
//   - Return C strings that must be freed with mtw_free_string()
//   - Return 0 on success, non-zero error code on failure for status functions
//   - Use callbacks (function pointers) for async notification
// =============================================================================

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

// ---------------------------------------------------------------------------
// Opaque handle types
// ---------------------------------------------------------------------------

/// Opaque client handle. PHP sees this as a void pointer.
/// Internally it holds the WebSocket connection and all state.
pub struct MtwClientInner {
    url: String,
    auth_token: Option<String>,
    connected: bool,
    conn_id: Option<String>,
    subscriptions: HashMap<String, bool>,
}

/// Opaque channel subscription handle.
pub struct MtwChannelInner {
    name: String,
    active: bool,
}

/// Opaque agent interaction handle.
pub struct MtwAgentInner {
    name: String,
    streaming: bool,
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// Error codes returned by status functions.
/// PHP can check these against constants defined in the header file.
pub const MTW_OK: i32 = 0;
pub const MTW_ERR_NULL_PTR: i32 = -1;
pub const MTW_ERR_INVALID_UTF8: i32 = -2;
pub const MTW_ERR_NOT_CONNECTED: i32 = -3;
pub const MTW_ERR_CONNECTION_FAILED: i32 = -4;
pub const MTW_ERR_TIMEOUT: i32 = -5;
pub const MTW_ERR_SEND_FAILED: i32 = -6;
pub const MTW_ERR_SUBSCRIBE_FAILED: i32 = -7;
pub const MTW_ERR_AGENT_ERROR: i32 = -8;
pub const MTW_ERR_UNKNOWN: i32 = -99;

// ---------------------------------------------------------------------------
// Callback types
// ---------------------------------------------------------------------------

/// Callback for receiving messages.
///
/// C signature:
///   typedef void (*mtw_message_callback)(const char* channel, const char* payload, void* user_data);
///
/// Parameters:
///   - channel:   the channel name the message was received on
///   - payload:   JSON-encoded MtwMessage
///   - user_data: opaque pointer passed during registration
pub type MtwMessageCallback =
    Option<extern "C" fn(channel: *const c_char, payload: *const c_char, user_data: *mut std::ffi::c_void)>;

/// Callback for agent streaming chunks.
///
/// C signature:
///   typedef void (*mtw_stream_callback)(const char* chunk_text, int done, void* user_data);
///
/// Parameters:
///   - chunk_text: the text content of this chunk
///   - done:       1 if this is the final chunk, 0 otherwise
///   - user_data:  opaque pointer passed during registration
pub type MtwStreamCallback =
    Option<extern "C" fn(chunk_text: *const c_char, done: i32, user_data: *mut std::ffi::c_void)>;

/// Callback for agent tool calls.
///
/// C signature:
///   typedef const char* (*mtw_tool_callback)(const char* tool_name, const char* params_json, void* user_data);
///
/// The callback must return a C string with the tool result (JSON-encoded).
/// The caller is responsible for freeing the returned string with mtw_free_string().
pub type MtwToolCallback = Option<
    extern "C" fn(
        tool_name: *const c_char,
        params_json: *const c_char,
        user_data: *mut std::ffi::c_void,
    ) -> *const c_char,
>;

// ---------------------------------------------------------------------------
// Helper: convert C string to Rust String safely
// ---------------------------------------------------------------------------

unsafe fn cstr_to_string(s: *const c_char) -> Result<String, i32> {
    if s.is_null() {
        return Err(MTW_ERR_NULL_PTR);
    }
    CStr::from_ptr(s)
        .to_str()
        .map(|s| s.to_owned())
        .map_err(|_| MTW_ERR_INVALID_UTF8)
}

fn string_to_cstr(s: &str) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

// ---------------------------------------------------------------------------
// Connection lifecycle
// ---------------------------------------------------------------------------

/// Create a new client and connect to an mtwRequest server.
///
/// C signature:
///   MtwClient* mtw_connect(const char* url, const char* token);
///
/// Parameters:
///   - url:   WebSocket endpoint URL (e.g. "ws://localhost:8080/ws")
///   - token: Authentication token, or NULL for unauthenticated
///
/// Returns:
///   - Opaque pointer to client handle, or NULL on failure
///
/// The caller must eventually call mtw_destroy_client() to free resources.
#[no_mangle]
pub unsafe extern "C" fn mtw_connect(url: *const c_char, token: *const c_char) -> *mut MtwClientInner {
    let url = match cstr_to_string(url) {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let token = if token.is_null() {
        None
    } else {
        cstr_to_string(token).ok()
    };

    // In real implementation:
    // 1. Initialize tokio runtime (or use a shared static runtime)
    // 2. Connect via tokio-tungstenite
    // 3. Send Connect message and await Ack
    let client = Box::new(MtwClientInner {
        url,
        auth_token: token,
        connected: true,
        conn_id: None,
        subscriptions: HashMap::new(),
    });

    Box::into_raw(client)
}

/// Close the connection and free the client handle.
///
/// C signature:
///   void mtw_destroy_client(MtwClient* client);
///
/// After calling this, the pointer is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn mtw_destroy_client(client: *mut MtwClientInner) {
    if !client.is_null() {
        let _ = Box::from_raw(client);
    }
}

/// Check if the client is connected.
///
/// C signature:
///   int mtw_is_connected(const MtwClient* client);
///
/// Returns: 1 if connected, 0 if not, MTW_ERR_NULL_PTR if client is NULL
#[no_mangle]
pub unsafe extern "C" fn mtw_is_connected(client: *const MtwClientInner) -> i32 {
    if client.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    if (*client).connected { 1 } else { 0 }
}

/// Get the server-assigned connection ID.
///
/// C signature:
///   const char* mtw_connection_id(const MtwClient* client);
///
/// Returns: connection ID string (caller must free with mtw_free_string),
///          or NULL if not connected.
#[no_mangle]
pub unsafe extern "C" fn mtw_connection_id(client: *const MtwClientInner) -> *const c_char {
    if client.is_null() {
        return ptr::null();
    }
    match &(*client).conn_id {
        Some(id) => string_to_cstr(id),
        None => ptr::null(),
    }
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

/// Send a message on a channel.
///
/// C signature:
///   int mtw_send(MtwClient* client, const char* channel, const char* payload);
///
/// Parameters:
///   - client:  client handle from mtw_connect()
///   - channel: target channel name
///   - payload: message content (text or JSON string)
///
/// Returns: MTW_OK on success, error code on failure
#[no_mangle]
pub unsafe extern "C" fn mtw_send(
    client: *mut MtwClientInner,
    channel: *const c_char,
    payload: *const c_char,
) -> i32 {
    if client.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    let _channel = match cstr_to_string(channel) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let _payload = match cstr_to_string(payload) {
        Ok(s) => s,
        Err(e) => return e,
    };

    if !(*client).connected {
        return MTW_ERR_NOT_CONNECTED;
    }

    // Build MtwMessage { msg_type: Publish, channel, payload: Text(payload) }
    // Encode and send via the WebSocket connection
    MTW_OK
}

/// Send a request and block until a response is received.
///
/// C signature:
///   const char* mtw_request(MtwClient* client, const char* channel, const char* payload, int timeout_ms);
///
/// Returns: JSON-encoded response message (caller must free with mtw_free_string),
///          or NULL on error/timeout.
#[no_mangle]
pub unsafe extern "C" fn mtw_request(
    client: *mut MtwClientInner,
    channel: *const c_char,
    payload: *const c_char,
    _timeout_ms: i32,
) -> *const c_char {
    if client.is_null() || channel.is_null() || payload.is_null() {
        return ptr::null();
    }
    // Build Request message, send, block until Response with matching ref_id
    ptr::null()
}

// ---------------------------------------------------------------------------
// Channel operations
// ---------------------------------------------------------------------------

/// Subscribe to a channel.
///
/// C signature:
///   MtwChannel* mtw_subscribe(MtwClient* client, const char* channel);
///
/// Returns: opaque channel handle, or NULL on failure.
/// The caller must free with mtw_destroy_channel().
#[no_mangle]
pub unsafe extern "C" fn mtw_subscribe(
    client: *mut MtwClientInner,
    channel: *const c_char,
) -> *mut MtwChannelInner {
    if client.is_null() {
        return ptr::null_mut();
    }
    let channel_name = match cstr_to_string(channel) {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    (*client).subscriptions.insert(channel_name.clone(), true);

    let ch = Box::new(MtwChannelInner {
        name: channel_name,
        active: true,
    });
    Box::into_raw(ch)
}

/// Unsubscribe from a channel and free the handle.
///
/// C signature:
///   int mtw_unsubscribe(MtwClient* client, MtwChannel* channel);
///
/// Returns: MTW_OK on success, error code on failure
#[no_mangle]
pub unsafe extern "C" fn mtw_unsubscribe(
    client: *mut MtwClientInner,
    channel: *mut MtwChannelInner,
) -> i32 {
    if client.is_null() || channel.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    let ch = Box::from_raw(channel);
    (*client).subscriptions.remove(&ch.name);
    MTW_OK
}

/// Register a message callback on a channel.
///
/// C signature:
///   int mtw_on_message(MtwChannel* channel, mtw_message_callback callback, void* user_data);
///
/// The callback is invoked on the event loop thread whenever a message
/// arrives on the subscribed channel.
#[no_mangle]
pub unsafe extern "C" fn mtw_on_message(
    channel: *mut MtwChannelInner,
    _callback: MtwMessageCallback,
    _user_data: *mut std::ffi::c_void,
) -> i32 {
    if channel.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    // Store the callback and user_data pointer. The message dispatch loop
    // calls the callback with the channel name and JSON payload.
    MTW_OK
}

/// Publish a message to a channel.
///
/// C signature:
///   int mtw_publish(MtwChannel* channel, const char* payload);
#[no_mangle]
pub unsafe extern "C" fn mtw_publish(channel: *mut MtwChannelInner, payload: *const c_char) -> i32 {
    if channel.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    let _payload = match cstr_to_string(payload) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // Build Publish message and send through the client's connection
    MTW_OK
}

/// Free a channel handle without unsubscribing.
///
/// C signature:
///   void mtw_destroy_channel(MtwChannel* channel);
#[no_mangle]
pub unsafe extern "C" fn mtw_destroy_channel(channel: *mut MtwChannelInner) {
    if !channel.is_null() {
        let _ = Box::from_raw(channel);
    }
}

// ---------------------------------------------------------------------------
// Agent operations
// ---------------------------------------------------------------------------

/// Create an agent interaction handle.
///
/// C signature:
///   MtwAgent* mtw_create_agent(MtwClient* client, const char* agent_name);
///
/// Returns: opaque agent handle, or NULL on failure.
/// The caller must free with mtw_destroy_agent().
#[no_mangle]
pub unsafe extern "C" fn mtw_create_agent(
    client: *const MtwClientInner,
    agent_name: *const c_char,
) -> *mut MtwAgentInner {
    if client.is_null() {
        return ptr::null_mut();
    }
    let name = match cstr_to_string(agent_name) {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let agent = Box::new(MtwAgentInner {
        name,
        streaming: false,
    });
    Box::into_raw(agent)
}

/// Send a task to an agent and block until the complete response.
///
/// C signature:
///   const char* mtw_agent_send(MtwAgent* agent, const char* content, int timeout_ms);
///
/// Returns: JSON-encoded AgentResponse (caller must free with mtw_free_string),
///          or NULL on error/timeout.
#[no_mangle]
pub unsafe extern "C" fn mtw_agent_send(
    agent: *mut MtwAgentInner,
    content: *const c_char,
    _timeout_ms: i32,
) -> *const c_char {
    if agent.is_null() || content.is_null() {
        return ptr::null();
    }
    // Build AgentTask, send, collect AgentChunk messages until AgentComplete
    ptr::null()
}

/// Stream a response from an agent using a callback.
///
/// C signature:
///   int mtw_agent_stream(MtwAgent* agent, const char* content,
///                        mtw_stream_callback callback, void* user_data);
///
/// The callback is invoked for each chunk. The `done` parameter is 1
/// when the stream is complete.
///
/// Returns: MTW_OK if streaming started, error code on failure
#[no_mangle]
pub unsafe extern "C" fn mtw_agent_stream(
    agent: *mut MtwAgentInner,
    content: *const c_char,
    _callback: MtwStreamCallback,
    _user_data: *mut std::ffi::c_void,
) -> i32 {
    if agent.is_null() || content.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    (*agent).streaming = true;
    // Send AgentTask, then for each AgentChunk received, invoke the callback.
    // When AgentComplete arrives, invoke callback with done=1.
    MTW_OK
}

/// Register a tool handler for an agent.
///
/// C signature:
///   int mtw_agent_on_tool_call(MtwAgent* agent, const char* tool_name,
///                               mtw_tool_callback callback, void* user_data);
///
/// When the agent requests the named tool, the callback is invoked with
/// the tool parameters as a JSON string. The callback must return the
/// result as a JSON string (freed by the caller).
#[no_mangle]
pub unsafe extern "C" fn mtw_agent_on_tool_call(
    agent: *mut MtwAgentInner,
    tool_name: *const c_char,
    _callback: MtwToolCallback,
    _user_data: *mut std::ffi::c_void,
) -> i32 {
    if agent.is_null() || tool_name.is_null() {
        return MTW_ERR_NULL_PTR;
    }
    // Store the callback. When AgentToolCall arrives with matching tool name:
    // 1. Invoke callback(tool_name, params_json, user_data)
    // 2. Send AgentToolResult with the returned string
    MTW_OK
}

/// Free an agent handle.
///
/// C signature:
///   void mtw_destroy_agent(MtwAgent* agent);
#[no_mangle]
pub unsafe extern "C" fn mtw_destroy_agent(agent: *mut MtwAgentInner) {
    if !agent.is_null() {
        let _ = Box::from_raw(agent);
    }
}

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------

/// Free a string returned by any mtw_* function.
///
/// C signature:
///   void mtw_free_string(const char* s);
///
/// Must be called on every non-NULL string returned by the library.
#[no_mangle]
pub unsafe extern "C" fn mtw_free_string(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// Get the last error message for the current thread.
///
/// C signature:
///   const char* mtw_last_error();
///
/// Returns: error message string (caller must free with mtw_free_string),
///          or NULL if no error.
#[no_mangle]
pub unsafe extern "C" fn mtw_last_error() -> *const c_char {
    // In real implementation: use thread-local storage to track the last error
    ptr::null()
}

// ---------------------------------------------------------------------------
// C Header (for reference — would be generated or shipped alongside the .so)
// ---------------------------------------------------------------------------
//
// /* mtw_request.h — C header for PHP FFI */
// #ifndef MTW_REQUEST_H
// #define MTW_REQUEST_H
//
// #include <stdint.h>
//
// /* Error codes */
// #define MTW_OK                  0
// #define MTW_ERR_NULL_PTR       -1
// #define MTW_ERR_INVALID_UTF8   -2
// #define MTW_ERR_NOT_CONNECTED  -3
// #define MTW_ERR_CONNECTION_FAILED -4
// #define MTW_ERR_TIMEOUT        -5
// #define MTW_ERR_SEND_FAILED    -6
// #define MTW_ERR_SUBSCRIBE_FAILED -7
// #define MTW_ERR_AGENT_ERROR    -8
// #define MTW_ERR_UNKNOWN        -99
//
// /* Opaque types */
// typedef struct MtwClientInner MtwClient;
// typedef struct MtwChannelInner MtwChannel;
// typedef struct MtwAgentInner MtwAgent;
//
// /* Callbacks */
// typedef void (*mtw_message_callback)(const char* channel, const char* payload, void* user_data);
// typedef void (*mtw_stream_callback)(const char* chunk_text, int done, void* user_data);
// typedef const char* (*mtw_tool_callback)(const char* tool_name, const char* params_json, void* user_data);
//
// /* Connection */
// MtwClient* mtw_connect(const char* url, const char* token);
// void mtw_destroy_client(MtwClient* client);
// int mtw_is_connected(const MtwClient* client);
// const char* mtw_connection_id(const MtwClient* client);
//
// /* Messaging */
// int mtw_send(MtwClient* client, const char* channel, const char* payload);
// const char* mtw_request(MtwClient* client, const char* channel, const char* payload, int timeout_ms);
//
// /* Channels */
// MtwChannel* mtw_subscribe(MtwClient* client, const char* channel);
// int mtw_unsubscribe(MtwClient* client, MtwChannel* channel);
// int mtw_on_message(MtwChannel* channel, mtw_message_callback callback, void* user_data);
// int mtw_publish(MtwChannel* channel, const char* payload);
// void mtw_destroy_channel(MtwChannel* channel);
//
// /* Agents */
// MtwAgent* mtw_create_agent(const MtwClient* client, const char* agent_name);
// const char* mtw_agent_send(MtwAgent* agent, const char* content, int timeout_ms);
// int mtw_agent_stream(MtwAgent* agent, const char* content, mtw_stream_callback callback, void* user_data);
// int mtw_agent_on_tool_call(MtwAgent* agent, const char* tool_name, mtw_tool_callback callback, void* user_data);
// void mtw_destroy_agent(MtwAgent* agent);
//
// /* Memory */
// void mtw_free_string(const char* s);
// const char* mtw_last_error(void);
//
// #endif /* MTW_REQUEST_H */
