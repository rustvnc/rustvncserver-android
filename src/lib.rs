// Copyright 2025 Dustin McAfee
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Generic Android JNI bindings for rustvncserver.
//!
//! This crate compiles directly to a `.so` file that can be used by any Android app.
//! No wrapper crate needed - just configure your Java package at build time!
//!
//! # Usage
//!
//! Build with environment variables to configure your app's Java package:
//!
//! ```bash
//! VNC_PACKAGE="com/mycompany/vnc" cargo ndk -t arm64-v8a build --release
//! ```
//!
//! Then copy `librustvncserver_android.so` to your app's `jniLibs` folder (rename as needed).
//!
//! # Environment Variables
//!
//! - `VNC_PACKAGE` - Java package path (required), e.g., `"com/example/vnc"`
//! - `VNC_MAIN_SERVICE` - Main service class name (default: `"MainService"`)
//! - `VNC_INPUT_SERVICE` - Input service class name (default: `"InputService"`)
//! - `VNC_LOG_TAG` - Android log tag (default: `"RustVNC"`)
//!
//! # Example Gradle Integration
//!
//! ```groovy
//! def vncPackage = "com/mycompany/vnc"
//!
//! environment "VNC_PACKAGE", vncPackage
//! environment "VNC_LOG_TAG", "MyApp-VNC"
//!
//! commandLine 'cargo', 'ndk', '-t', 'arm64-v8a', 'build', '--release'
//! ```
//!
//! # Java Side Requirements
//!
//! Your Java classes need these native method declarations and callbacks:
//!
//! ```java
//! public class MainService {
//!     static { System.loadLibrary("droidvnc_ng"); }  // or your lib name
//!
//!     // Native methods (registered at runtime via JNI_OnLoad)
//!     public static native void vncInit();
//!     public static native boolean vncStartServer(int w, int h, int port, String name, String pw, String httpDir);
//!     public static native boolean vncStopServer();
//!     public static native boolean vncIsActive();
//!     public static native boolean vncUpdateFramebuffer(ByteBuffer buffer);
//!     public static native boolean vncNewFramebuffer(int width, int height);
//!     public static native long vncConnectRepeater(String host, int port, String id, String requestId);
//!     // ... see full list in source
//!
//!     // Callbacks (called from Rust)
//!     public static void onClientConnected(long clientId) { }
//!     public static void onClientDisconnected(long clientId) { }
//!     public static void notifyRfbMessageSent(String requestId, boolean success) { }
//!     public static void notifyHandshakeComplete(String requestId, boolean success) { }
//! }
//!
//! public class InputService {
//!     public static void onKeyEvent(int down, long keysym, long clientId) { }
//!     public static void onPointerEvent(int buttonMask, int x, int y, long clientId) { }
//!     public static void onCutText(String text, long clientId) { }
//! }
//! ```

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use log::{error, info, warn};
use once_cell::sync::OnceCell;
use rustvncserver::server::{ServerEvent, VncServer};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::{broadcast, mpsc};

// ============================================================================
// Global State
// ============================================================================

/// Global Tokio runtime for the VNC server.
static VNC_RUNTIME: OnceCell<Runtime> = OnceCell::new();
/// Global container for the VNC server instance.
static VNC_SERVER: OnceCell<Arc<Mutex<Option<Arc<VncServer>>>>> = OnceCell::new();
/// Global broadcast sender for shutdown signals.
static SHUTDOWN_SIGNAL: OnceCell<broadcast::Sender<()>> = OnceCell::new();
/// Atomic flag to track if the event handler is running.
static EVENT_HANDLER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Global reference to the Java VM.
static JAVA_VM: OnceCell<jni::JavaVM> = OnceCell::new();
/// Global reference to the InputService Java class.
static INPUT_SERVICE_CLASS: OnceCell<GlobalRef> = OnceCell::new();
/// Global reference to the MainService Java class.
static MAIN_SERVICE_CLASS: OnceCell<GlobalRef> = OnceCell::new();

/// Unique client ID counter (unused but kept for compatibility).
#[allow(dead_code)]
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Flag to prevent concurrent framebuffer updates.
static FRAMEBUFFER_UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

// ============================================================================
// Public API - Registration
// ============================================================================

/// Registers VNC native methods with the specified Java classes.
///
/// This function uses JNI's `RegisterNatives` to dynamically bind Rust functions
/// to Java native methods, allowing the same library to work with any package name.
///
/// # Arguments
///
/// * `env` - The JNI environment
/// * `main_service_class` - Fully qualified class name (e.g., "com/example/app/MainService")
/// * `input_service_class` - Fully qualified class name (e.g., "com/example/app/InputService")
///
/// # Returns
///
/// `Ok(())` if registration succeeds, `Err` with description otherwise.
///
/// # Example
///
/// ```rust,ignore
/// #[no_mangle]
/// pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut c_void) -> jint {
///     let env = vm.get_env().unwrap();
///     register_vnc_natives(
///         &mut env,
///         "com/mycompany/vnc/MainService",
///         "com/mycompany/vnc/InputService",
///     ).expect("Failed to register natives");
///     jni::sys::JNI_VERSION_1_6
/// }
/// ```
pub fn register_vnc_natives(
    env: &mut JNIEnv,
    main_service_class: &str,
    input_service_class: &str,
) -> Result<(), String> {
    // Store Java VM reference
    if let Ok(vm) = env.get_java_vm() {
        let _ = JAVA_VM.set(vm);
    }

    // Find and cache MainService class
    let main_class = env.find_class(main_service_class).map_err(|e| {
        format!(
            "Failed to find MainService class '{}': {}",
            main_service_class, e
        )
    })?;

    let main_global = env
        .new_global_ref(main_class)
        .map_err(|e| format!("Failed to create global ref for MainService: {}", e))?;

    if MAIN_SERVICE_CLASS.set(main_global).is_err() {
        return Err("MAIN_SERVICE_CLASS was already set".to_string());
    }

    // Find and cache InputService class
    let input_class = env.find_class(input_service_class).map_err(|e| {
        format!(
            "Failed to find InputService class '{}': {}",
            input_service_class, e
        )
    })?;

    let input_global = env
        .new_global_ref(input_class)
        .map_err(|e| format!("Failed to create global ref for InputService: {}", e))?;

    if INPUT_SERVICE_CLASS.set(input_global).is_err() {
        return Err("INPUT_SERVICE_CLASS was already set".to_string());
    }

    // Register native methods for MainService
    let main_class_local = env
        .find_class(main_service_class)
        .map_err(|e| format!("Failed to find MainService for registration: {}", e))?;

    register_main_service_natives(env, main_class_local)?;

    info!(
        "VNC natives registered for {} and {}",
        main_service_class, input_service_class
    );

    Ok(())
}

/// Registers native methods for the MainService class.
///
/// Required methods will fail registration if missing.
/// Optional methods are registered individually and silently skipped if missing.
fn register_main_service_natives(env: &mut JNIEnv, class: JClass) -> Result<(), String> {
    use jni::NativeMethod;

    // Required methods - registration fails if any are missing
    let required_methods: Vec<NativeMethod> = vec![
        NativeMethod {
            name: "vncInit".into(),
            sig: "()V".into(),
            fn_ptr: native_vnc_init as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncStartServer".into(),
            sig: "(IIILjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z".into(),
            fn_ptr: native_vnc_start_server as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncStopServer".into(),
            sig: "()Z".into(),
            fn_ptr: native_vnc_stop_server as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncIsActive".into(),
            sig: "()Z".into(),
            fn_ptr: native_vnc_is_active as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncConnectRepeater".into(),
            sig: "(Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;)J".into(),
            fn_ptr: native_vnc_connect_repeater as *mut std::ffi::c_void,
        },
    ];

    env.register_native_methods(&class, &required_methods)
        .map_err(|e| format!("Failed to register required MainService natives: {}", e))?;

    // Optional methods - registered individually, skip if missing
    let optional_methods: Vec<NativeMethod> = vec![
        NativeMethod {
            name: "vncUpdateFramebuffer".into(),
            sig: "(Ljava/nio/ByteBuffer;)Z".into(),
            fn_ptr: native_vnc_update_framebuffer as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncUpdateFramebufferAndSend".into(),
            sig: "(Ljava/nio/ByteBuffer;II)Z".into(),
            fn_ptr: native_vnc_update_framebuffer_and_send as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncUpdateFramebufferCropped".into(),
            sig: "(Ljava/nio/ByteBuffer;IIIIII)Z".into(),
            fn_ptr: native_vnc_update_framebuffer_cropped as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncNewFramebuffer".into(),
            sig: "(II)Z".into(),
            fn_ptr: native_vnc_new_framebuffer as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncSendCutText".into(),
            sig: "(Ljava/lang/String;)V".into(),
            fn_ptr: native_vnc_send_cut_text as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncGetFramebufferWidth".into(),
            sig: "()I".into(),
            fn_ptr: native_vnc_get_framebuffer_width as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncGetFramebufferHeight".into(),
            sig: "()I".into(),
            fn_ptr: native_vnc_get_framebuffer_height as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncConnectReverse".into(),
            sig: "(Ljava/lang/String;I)J".into(),
            fn_ptr: native_vnc_connect_reverse as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncGetRemoteHost".into(),
            sig: "(J)Ljava/lang/String;".into(),
            fn_ptr: native_vnc_get_remote_host as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncGetDestinationPort".into(),
            sig: "(J)I".into(),
            fn_ptr: native_vnc_get_destination_port as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncGetRepeaterId".into(),
            sig: "(J)Ljava/lang/String;".into(),
            fn_ptr: native_vnc_get_repeater_id as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncDisconnect".into(),
            sig: "(J)Z".into(),
            fn_ptr: native_vnc_disconnect as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncScheduleCopyRect".into(),
            sig: "(IIIIII)V".into(),
            fn_ptr: native_vnc_schedule_copy_rect as *mut std::ffi::c_void,
        },
        NativeMethod {
            name: "vncDoCopyRect".into(),
            sig: "(IIIIII)Z".into(),
            fn_ptr: native_vnc_do_copy_rect as *mut std::ffi::c_void,
        },
    ];

    for method in optional_methods {
        let method_name = method.name.to_str().unwrap_or("unknown").to_string();
        if env.register_native_methods(&class, &[method]).is_err() {
            // Clear the exception so we can continue
            let _ = env.exception_clear();
            log::debug!("Optional method {} not found, skipping", method_name);
        }
    }

    Ok(())
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Gets or initializes the global Tokio runtime.
fn get_or_init_vnc_runtime() -> &'static Runtime {
    VNC_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build VNC Tokio runtime")
    })
}

/// Gets or initializes the shutdown signal broadcaster.
fn get_or_init_shutdown_signal() -> &'static broadcast::Sender<()> {
    SHUTDOWN_SIGNAL.get_or_init(|| {
        let (tx, _) = broadcast::channel(16);
        tx
    })
}

// ============================================================================
// Native Method Implementations
// ============================================================================

extern "system" fn native_vnc_init(env: JNIEnv, _class: JClass) {
    // Initialize Android logger
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("RustVNC"),
    );

    info!("Initializing Rust VNC Server");

    // Initialize runtime and shutdown signal
    get_or_init_vnc_runtime();
    get_or_init_shutdown_signal();

    // Store Java VM if not already stored
    if JAVA_VM.get().is_none() {
        if let Ok(vm) = env.get_java_vm() {
            let _ = JAVA_VM.set(vm);
        }
    }

    // Initialize server container
    VNC_SERVER.get_or_init(|| Arc::new(Mutex::new(None)));

    info!("Rust VNC Server initialized");
}

extern "system" fn native_vnc_start_server(
    mut env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
    port: jint,
    desktop_name: JString,
    password: JString,
    _http_root_dir: JString,
) -> jboolean {
    // Validate dimensions
    const MAX_DIMENSION: i32 = 8192;
    const MIN_DIMENSION: i32 = 1;

    let width = match u16::try_from(width) {
        Ok(w) if w >= MIN_DIMENSION as u16 && w <= MAX_DIMENSION as u16 => w,
        _ => {
            error!(
                "Invalid width: {} (must be {}-{})",
                width, MIN_DIMENSION, MAX_DIMENSION
            );
            return JNI_FALSE;
        }
    };

    let height = match u16::try_from(height) {
        Ok(h) if h >= MIN_DIMENSION as u16 && h <= MAX_DIMENSION as u16 => h,
        _ => {
            error!(
                "Invalid height: {} (must be {}-{})",
                height, MIN_DIMENSION, MAX_DIMENSION
            );
            return JNI_FALSE;
        }
    };

    // Port -1 means "inbound connections disabled"
    let port_opt: Option<u16> = if port == -1 {
        None
    } else {
        match u16::try_from(port) {
            Ok(p) if p > 0 => Some(p),
            _ => {
                error!(
                    "Invalid port: {} (must be -1 for disabled or 1-65535)",
                    port
                );
                return JNI_FALSE;
            }
        }
    };

    let desktop_name_str: String = match env.get_string(&desktop_name) {
        Ok(s) => s.into(),
        Err(e) => {
            error!("Failed to get desktop name: {}", e);
            return JNI_FALSE;
        }
    };

    let password_str: Option<String> = if !password.is_null() {
        match env.get_string(&password) {
            Ok(s) => {
                let pw: String = s.into();
                if pw.is_empty() {
                    None
                } else {
                    Some(pw)
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    if let Some(p) = port_opt {
        info!(
            "Starting Rust VNC Server: {}x{} on port {}",
            width, height, p
        );
    } else {
        info!(
            "Starting Rust VNC Server: {}x{} (inbound connections disabled)",
            width, height
        );
    }

    // Create server and event receiver
    let (server, event_rx) = VncServer::new(width, height, desktop_name_str, password_str);
    let server: Arc<VncServer> = Arc::new(server);

    // Store the server globally
    if let Some(server_container) = VNC_SERVER.get() {
        match server_container.lock() {
            Ok(mut guard) => {
                *guard = Some(server.clone());
            }
            Err(e) => {
                error!("Failed to lock server container: {}", e);
                return JNI_FALSE;
            }
        }
    } else {
        error!("VNC server container not initialized");
        return JNI_FALSE;
    }

    // Start event handler
    spawn_event_handler(event_rx);

    // Start listener if port specified
    if let Some(listen_port) = port_opt {
        let runtime = get_or_init_vnc_runtime();
        let server_clone = server.clone();
        let mut shutdown_rx = get_or_init_shutdown_signal().subscribe();

        runtime.spawn(async move {
            tokio::select! {
                result = server_clone.listen(listen_port) => {
                    if let Err(e) = result {
                        error!("VNC server listen error: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("VNC server received shutdown signal");
                }
            }
        });
    } else {
        info!("VNC server running in outbound-only mode (no listener)");
    }

    info!("Rust VNC Server started successfully");
    JNI_TRUE
}

extern "system" fn native_vnc_stop_server(_env: JNIEnv, _class: JClass) -> jboolean {
    info!("Stopping Rust VNC Server");

    // Step 1: Disconnect all clients
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server_arc) = guard.as_ref() {
                let client_ids = server_arc.get_client_ids().unwrap_or_else(|_| {
                    warn!("Failed to get read lock on client IDs list");
                    Vec::new()
                });

                info!("Found {} client(s) to disconnect", client_ids.len());

                let runtime = get_or_init_vnc_runtime();
                let server_clone = server_arc.clone();

                // Call Java onClientDisconnected for each client
                if let Some(vm) = JAVA_VM.get() {
                    if let Ok(mut env) = vm.attach_current_thread() {
                        if let Some(main_class) = MAIN_SERVICE_CLASS.get() {
                            for client_id in &client_ids {
                                info!("Calling Java onClientDisconnected for client {}", client_id);
                                let args = [JValue::Long(*client_id as jlong)];
                                if let Err(e) = env.call_static_method(
                                    main_class,
                                    "onClientDisconnected",
                                    "(J)V",
                                    &args,
                                ) {
                                    error!(
                                        "Failed to call onClientDisconnected for client {}: {}",
                                        client_id, e
                                    );
                                }
                            }
                        }
                    }
                }

                // Disconnect all clients with timeout
                info!("Disconnecting all clients with 3s timeout");
                let disconnect_result = runtime.block_on(async {
                    tokio::time::timeout(
                        tokio::time::Duration::from_secs(3),
                        server_clone.disconnect_all_clients(),
                    )
                    .await
                });

                match disconnect_result {
                    Ok(_) => info!("All clients disconnected successfully"),
                    Err(_) => warn!("Client disconnect timed out after 3s"),
                }
            }
        }
    }

    // Step 2: Send shutdown signal
    if let Some(shutdown_tx) = SHUTDOWN_SIGNAL.get() {
        info!("Sending shutdown signal to tasks");
        let _ = shutdown_tx.send(());
    }

    // Step 3: Clear server reference
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(mut guard) = server_container.lock() {
            *guard = None;
            info!("Server reference cleared");
        }
    }

    // Step 4: Reset event handler flag
    EVENT_HANDLER_RUNNING.store(false, Ordering::SeqCst);

    info!("Rust VNC Server stopped successfully");
    JNI_TRUE
}

extern "system" fn native_vnc_update_framebuffer(
    env: JNIEnv,
    _class: JClass,
    buffer: JObject,
) -> jboolean {
    let buffer_ptr = match env.get_direct_buffer_address((&buffer).into()) {
        Ok(ptr) => ptr,
        Err(e) => {
            error!("Failed to get buffer address: {}", e);
            return JNI_FALSE;
        }
    };

    let buffer_capacity = match env.get_direct_buffer_capacity((&buffer).into()) {
        Ok(cap) => cap,
        Err(e) => {
            error!("Failed to get buffer capacity: {}", e);
            return JNI_FALSE;
        }
    };

    // Copy buffer immediately
    let buffer_copy = {
        let buffer_slice = unsafe { std::slice::from_raw_parts(buffer_ptr, buffer_capacity) };
        buffer_slice.to_vec()
    };

    // Skip if update already in progress
    if FRAMEBUFFER_UPDATE_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return JNI_TRUE;
    }

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let rt = get_or_init_vnc_runtime();
                let server_clone = server.clone();

                rt.spawn(async move {
                    if let Err(e) = server_clone
                        .framebuffer()
                        .update_from_slice(&buffer_copy)
                        .await
                    {
                        error!("Failed to update framebuffer: {}", e);
                    }
                    FRAMEBUFFER_UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
                });

                return JNI_TRUE;
            }
        }
    }

    FRAMEBUFFER_UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    JNI_FALSE
}

extern "system" fn native_vnc_update_framebuffer_and_send(
    env: JNIEnv,
    class: JClass,
    buffer: JObject,
    _width: jint,
    _height: jint,
) -> jboolean {
    native_vnc_update_framebuffer(env, class, buffer)
}

extern "system" fn native_vnc_update_framebuffer_cropped(
    env: JNIEnv,
    _class: JClass,
    buffer: JObject,
    _width: jint,
    _height: jint,
    crop_x: jint,
    crop_y: jint,
    crop_width: jint,
    crop_height: jint,
) -> jboolean {
    let buffer_ptr = match env.get_direct_buffer_address((&buffer).into()) {
        Ok(ptr) => ptr,
        Err(e) => {
            error!("Failed to get buffer address: {}", e);
            return JNI_FALSE;
        }
    };

    let buffer_capacity = match env.get_direct_buffer_capacity((&buffer).into()) {
        Ok(cap) => cap,
        Err(e) => {
            error!("Failed to get buffer capacity: {}", e);
            return JNI_FALSE;
        }
    };

    // Validate crop dimensions
    const MAX_DIMENSION: i32 = 8192;

    if crop_x < 0 || crop_y < 0 || crop_width <= 0 || crop_height <= 0 {
        error!(
            "Invalid crop parameters: x={}, y={}, w={}, h={}",
            crop_x, crop_y, crop_width, crop_height
        );
        return JNI_FALSE;
    }

    if crop_width > MAX_DIMENSION || crop_height > MAX_DIMENSION {
        error!(
            "Crop dimensions too large: {}x{} (max {})",
            crop_width, crop_height, MAX_DIMENSION
        );
        return JNI_FALSE;
    }

    let expected_size = (crop_width as usize)
        .checked_mul(crop_height as usize)
        .and_then(|s| s.checked_mul(4))
        .unwrap_or(0);

    if expected_size == 0 || buffer_capacity != expected_size {
        error!(
            "Cropped buffer size mismatch: expected {}, got {}",
            expected_size, buffer_capacity
        );
        return JNI_FALSE;
    }

    let buffer_copy = {
        let buffer_slice = unsafe { std::slice::from_raw_parts(buffer_ptr, buffer_capacity) };
        buffer_slice.to_vec()
    };

    if FRAMEBUFFER_UPDATE_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return JNI_TRUE;
    }

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let rt = get_or_init_vnc_runtime();
                let server_clone = server.clone();

                rt.spawn(async move {
                    if let Err(e) = server_clone
                        .framebuffer()
                        .update_cropped(
                            &buffer_copy,
                            crop_x as u16,
                            crop_y as u16,
                            crop_width as u16,
                            crop_height as u16,
                        )
                        .await
                    {
                        error!("Failed to update cropped framebuffer: {}", e);
                    }
                    FRAMEBUFFER_UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
                });

                return JNI_TRUE;
            }
        }
    }

    FRAMEBUFFER_UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    JNI_FALSE
}

extern "system" fn native_vnc_new_framebuffer(
    _env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
) -> jboolean {
    const MAX_DIMENSION: i32 = 8192;
    const MIN_DIMENSION: i32 = 1;

    let width = match u16::try_from(width) {
        Ok(w) if w >= MIN_DIMENSION as u16 && w <= MAX_DIMENSION as u16 => w,
        _ => {
            error!(
                "Invalid width: {} (must be {}-{})",
                width, MIN_DIMENSION, MAX_DIMENSION
            );
            return JNI_FALSE;
        }
    };

    let height = match u16::try_from(height) {
        Ok(h) if h >= MIN_DIMENSION as u16 && h <= MAX_DIMENSION as u16 => h,
        _ => {
            error!(
                "Invalid height: {} (must be {}-{})",
                height, MIN_DIMENSION, MAX_DIMENSION
            );
            return JNI_FALSE;
        }
    };

    info!("Resizing framebuffer to {}x{}", width, height);

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let runtime = get_or_init_vnc_runtime();

                if let Err(e) = runtime.block_on(server.framebuffer().resize(width, height)) {
                    error!("Failed to resize framebuffer: {}", e);
                    return JNI_FALSE;
                }

                info!("Framebuffer resized successfully to {}x{}", width, height);
                return JNI_TRUE;
            }
        }
    }

    error!("VNC server not initialized");
    JNI_FALSE
}

extern "system" fn native_vnc_send_cut_text(mut env: JNIEnv, _class: JClass, text: JString) {
    let text_str: String = match env.get_string(&text) {
        Ok(s) => s.into(),
        Err(e) => {
            error!("Failed to get cut text: {}", e);
            return;
        }
    };

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let runtime = get_or_init_vnc_runtime();
                let server_clone = server.clone();
                let text_clone = text_str;

                runtime.spawn(async move {
                    if let Err(e) = server_clone.send_cut_text_to_all(text_clone).await {
                        error!("Failed to send cut text: {}", e);
                    }
                });
            }
        }
    }
}

extern "system" fn native_vnc_is_active(_env: JNIEnv, _class: JClass) -> jboolean {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if guard.is_some() {
                return JNI_TRUE;
            }
        }
    }
    JNI_FALSE
}

extern "system" fn native_vnc_get_framebuffer_width(_env: JNIEnv, _class: JClass) -> jint {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                return server.framebuffer().width() as jint;
            }
        }
    }
    -1
}

extern "system" fn native_vnc_get_framebuffer_height(_env: JNIEnv, _class: JClass) -> jint {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                return server.framebuffer().height() as jint;
            }
        }
    }
    -1
}

extern "system" fn native_vnc_connect_reverse(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
) -> jlong {
    let host_str: String = match env.get_string(&host) {
        Ok(s) => s.into(),
        Err(e) => {
            error!("Failed to get reverse connection host: {}", e);
            return 0;
        }
    };

    let port_u16 = port as u16;

    info!("Initiating reverse connection to {}:{}", host_str, port_u16);

    if let Some(server_container) = VNC_SERVER.get() {
        let server = match server_container.lock() {
            Ok(guard) => {
                if let Some(s) = guard.as_ref() {
                    s.clone()
                } else {
                    error!("VNC server not started");
                    return 0;
                }
            }
            Err(e) => {
                error!("Failed to lock server container: {}", e);
                return 0;
            }
        };

        let runtime = get_or_init_vnc_runtime();

        return runtime.block_on(async move {
            match server.connect_reverse(host_str, port_u16).await {
                Ok(client_id) => {
                    info!("Reverse connection established, client ID: {}", client_id);
                    client_id as jlong
                }
                Err(e) => {
                    error!("Failed to establish reverse connection: {}", e);
                    0
                }
            }
        });
    }

    error!("VNC server not initialized");
    0
}

extern "system" fn native_vnc_connect_repeater(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    repeater_id: JString,
    request_id: JString,
) -> jlong {
    let host_str: String = match env.get_string(&host) {
        Ok(s) => s.into(),
        Err(e) => {
            error!("Failed to get repeater host: {}", e);
            return 0;
        }
    };

    let repeater_id_str: String = match env.get_string(&repeater_id) {
        Ok(s) => s.into(),
        Err(e) => {
            error!("Failed to get repeater ID: {}", e);
            return 0;
        }
    };

    let request_id_opt: Option<String> = if !request_id.is_null() {
        match env.get_string(&request_id) {
            Ok(s) => {
                let req_id: String = s.into();
                if req_id.is_empty() {
                    None
                } else {
                    Some(req_id)
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let port_u16 = port as u16;

    info!(
        "Connecting to VNC repeater {}:{} with ID: {}, request_id: {:?}",
        host_str, port_u16, repeater_id_str, request_id_opt
    );

    if let Some(server_container) = VNC_SERVER.get() {
        let server = match server_container.lock() {
            Ok(guard) => {
                if let Some(s) = guard.as_ref() {
                    s.clone()
                } else {
                    error!("VNC server not started");
                    return 0;
                }
            }
            Err(e) => {
                error!("Failed to lock server container: {}", e);
                return 0;
            }
        };

        let runtime = get_or_init_vnc_runtime();

        return runtime.block_on(async move {
            match server
                .connect_repeater_with_request_id(
                    host_str,
                    port_u16,
                    repeater_id_str,
                    request_id_opt,
                )
                .await
            {
                Ok(client_id) => {
                    info!("Repeater connection established, client ID: {}", client_id);
                    client_id as jlong
                }
                Err(e) => {
                    error!("Failed to connect to repeater: {}", e);
                    0
                }
            }
        });
    }

    error!("VNC server not initialized");
    0
}

extern "system" fn native_vnc_get_remote_host<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    client_id: jlong,
) -> JString<'local> {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                if let Ok(clients) = server.clients_try_read() {
                    for client_arc in clients.iter() {
                        if let Ok(client_guard) = client_arc.try_read() {
                            if client_guard.get_client_id() == client_id as usize {
                                let host = client_guard.get_remote_host().to_string();
                                if let Ok(jstr) = env.new_string(&host) {
                                    return jstr;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    JString::default()
}

extern "system" fn native_vnc_get_destination_port(
    _env: JNIEnv,
    _class: JClass,
    client_id: jlong,
) -> jint {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                if let Ok(clients) = server.clients_try_read() {
                    for client_arc in clients.iter() {
                        if let Ok(client_guard) = client_arc.try_read() {
                            if client_guard.get_client_id() == client_id as usize {
                                return client_guard.get_destination_port();
                            }
                        }
                    }
                }
            }
        }
    }

    -1
}

extern "system" fn native_vnc_get_repeater_id<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    client_id: jlong,
) -> JString<'local> {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                if let Ok(clients) = server.clients_try_read() {
                    for client_arc in clients.iter() {
                        if let Ok(client_guard) = client_arc.try_read() {
                            if client_guard.get_client_id() == client_id as usize {
                                if let Some(id) = client_guard.get_repeater_id() {
                                    if let Ok(jstr) = env.new_string(id) {
                                        return jstr;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    JString::default()
}

extern "system" fn native_vnc_disconnect(
    _env: JNIEnv,
    _class: JClass,
    client_id: jlong,
) -> jboolean {
    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                if let Ok(mut clients) = server.clients_try_write() {
                    let initial_len = clients.len();

                    clients.retain(|client_arc| {
                        if let Ok(client_guard) = client_arc.try_read() {
                            client_guard.get_client_id() != client_id as usize
                        } else {
                            true
                        }
                    });

                    let removed = clients.len() < initial_len;
                    if removed {
                        info!("Client {} disconnected successfully", client_id);
                        return JNI_TRUE;
                    } else {
                        warn!("Client {} not found for disconnect", client_id);
                        return JNI_FALSE;
                    }
                }
            }
        }
    }

    JNI_FALSE
}

extern "system" fn native_vnc_schedule_copy_rect(
    _env: JNIEnv,
    _class: JClass,
    x: jint,
    y: jint,
    width: jint,
    height: jint,
    dx: jint,
    dy: jint,
) {
    if x < 0 || y < 0 || width <= 0 || height <= 0 {
        error!(
            "Invalid copy rect parameters: x={}, y={}, w={}, h={}",
            x, y, width, height
        );
        return;
    }

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let runtime = get_or_init_vnc_runtime();
                let server_clone = server.clone();

                runtime.spawn(async move {
                    server_clone
                        .schedule_copy_rect(
                            x as u16,
                            y as u16,
                            width as u16,
                            height as u16,
                            dx as i16,
                            dy as i16,
                        )
                        .await;
                });
            }
        }
    }
}

extern "system" fn native_vnc_do_copy_rect(
    _env: JNIEnv,
    _class: JClass,
    x: jint,
    y: jint,
    width: jint,
    height: jint,
    dx: jint,
    dy: jint,
) -> jboolean {
    if x < 0 || y < 0 || width <= 0 || height <= 0 {
        error!(
            "Invalid copy rect parameters: x={}, y={}, w={}, h={}",
            x, y, width, height
        );
        return JNI_FALSE;
    }

    if let Some(server_container) = VNC_SERVER.get() {
        if let Ok(guard) = server_container.lock() {
            if let Some(server) = guard.as_ref() {
                let runtime = get_or_init_vnc_runtime();

                let result = runtime.block_on(server.do_copy_rect(
                    x as u16,
                    y as u16,
                    width as u16,
                    height as u16,
                    dx as i16,
                    dy as i16,
                ));

                match result {
                    Ok(()) => return JNI_TRUE,
                    Err(e) => {
                        error!("Failed to perform copy rect: {}", e);
                        return JNI_FALSE;
                    }
                }
            }
        }
    }

    error!("VNC server not initialized");
    JNI_FALSE
}

// ============================================================================
// Event Handler
// ============================================================================

/// Spawns the event handler task.
fn spawn_event_handler(mut event_rx: mpsc::UnboundedReceiver<ServerEvent>) {
    if EVENT_HANDLER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        warn!("Event handler already running");
        return;
    }

    let runtime = get_or_init_vnc_runtime();
    let mut shutdown_rx = get_or_init_shutdown_signal().subscribe();

    runtime.spawn(async move {
        info!("VNC event handler started");

        loop {
            tokio::select! {
                Some(event) = event_rx.recv() => {
                    handle_server_event(event);
                }
                _ = shutdown_rx.recv() => {
                    info!("Event handler received shutdown signal");
                    break;
                }
            }
        }

        EVENT_HANDLER_RUNNING.store(false, Ordering::SeqCst);
        info!("VNC event handler stopped");
    });
}

/// Handles a server event by calling the appropriate Java method.
fn handle_server_event(event: ServerEvent) {
    let vm = match JAVA_VM.get() {
        Some(vm) => vm,
        None => {
            error!("Java VM not available");
            return;
        }
    };

    let mut env = match vm.attach_current_thread() {
        Ok(env) => env,
        Err(e) => {
            error!("Failed to attach to Java thread: {}", e);
            return;
        }
    };

    match event {
        ServerEvent::ClientConnected { client_id } => {
            info!("Client {} connected", client_id);
            if let Some(main_class) = MAIN_SERVICE_CLASS.get() {
                let args = [JValue::Long(client_id as jlong)];
                if let Err(e) =
                    env.call_static_method(main_class, "onClientConnected", "(J)V", &args)
                {
                    error!("Failed to call onClientConnected: {}", e);
                }
            }
        }
        ServerEvent::ClientDisconnected { client_id } => {
            info!("Client {} disconnected", client_id);
            if let Some(main_class) = MAIN_SERVICE_CLASS.get() {
                let args = [JValue::Long(client_id as jlong)];
                if let Err(e) =
                    env.call_static_method(main_class, "onClientDisconnected", "(J)V", &args)
                {
                    error!("Failed to call onClientDisconnected: {}", e);
                }
            }
        }
        ServerEvent::KeyPress {
            client_id,
            down,
            key,
        } => {
            if let Some(input_class) = INPUT_SERVICE_CLASS.get() {
                let args = [
                    JValue::Int(if down { 1 } else { 0 }),
                    JValue::Long(key as jlong),
                    JValue::Long(client_id as jlong),
                ];
                if let Err(e) = env.call_static_method(input_class, "onKeyEvent", "(IJJ)V", &args) {
                    error!("Failed to call onKeyEvent: {}", e);
                }
            }
        }
        ServerEvent::PointerMove {
            client_id,
            x,
            y,
            button_mask,
        } => {
            if let Some(input_class) = INPUT_SERVICE_CLASS.get() {
                let args = [
                    JValue::Int(button_mask as jint),
                    JValue::Int(x as jint),
                    JValue::Int(y as jint),
                    JValue::Long(client_id as jlong),
                ];
                if let Err(e) =
                    env.call_static_method(input_class, "onPointerEvent", "(IIIJ)V", &args)
                {
                    error!("Failed to call onPointerEvent: {}", e);
                }
            }
        }
        ServerEvent::CutText { client_id, text } => {
            if let Some(input_class) = INPUT_SERVICE_CLASS.get() {
                if let Ok(jtext) = env.new_string(&text) {
                    let args = [JValue::Object(&jtext), JValue::Long(client_id as jlong)];
                    if let Err(e) = env.call_static_method(
                        input_class,
                        "onCutText",
                        "(Ljava/lang/String;J)V",
                        &args,
                    ) {
                        error!("Failed to call onCutText: {}", e);
                    }
                }
            }
        }
        ServerEvent::RfbMessageSent {
            client_id: _,
            request_id,
            success,
        } => {
            info!(
                "RFB message sent: request_id={:?}, success={}",
                request_id, success
            );
            if let Some(main_class) = MAIN_SERVICE_CLASS.get() {
                let request_id_obj: JObject = match &request_id {
                    Some(id) => match env.new_string(id) {
                        Ok(jstr) => JObject::from(jstr),
                        Err(_) => JObject::null(),
                    },
                    None => JObject::null(),
                };

                let args = [
                    JValue::Object(&request_id_obj),
                    JValue::Bool(u8::from(success)),
                ];
                if let Err(e) = env.call_static_method(
                    main_class,
                    "notifyRfbMessageSent",
                    "(Ljava/lang/String;Z)V",
                    &args,
                ) {
                    error!("Failed to call notifyRfbMessageSent: {}", e);
                }
            }
        }
        ServerEvent::HandshakeComplete {
            client_id: _,
            request_id,
            success,
        } => {
            info!(
                "Handshake complete: request_id={:?}, success={}",
                request_id, success
            );
            if let Some(main_class) = MAIN_SERVICE_CLASS.get() {
                let request_id_obj: JObject = match &request_id {
                    Some(id) => match env.new_string(id) {
                        Ok(jstr) => JObject::from(jstr),
                        Err(_) => JObject::null(),
                    },
                    None => JObject::null(),
                };

                let args = [
                    JValue::Object(&request_id_obj),
                    JValue::Bool(u8::from(success)),
                ];
                if let Err(e) = env.call_static_method(
                    main_class,
                    "notifyHandshakeComplete",
                    "(Ljava/lang/String;Z)V",
                    &args,
                ) {
                    error!("Failed to call notifyHandshakeComplete: {}", e);
                }
            }
        }
    }
}

// ============================================================================
// Standalone JNI_OnLoad (when VNC_PACKAGE env var is set at build time)
// ============================================================================

/// JNI_OnLoad for standalone builds.
///
/// This is automatically included when building with `VNC_PACKAGE` environment variable.
/// No wrapper crate needed - just build this crate directly and use the .so file.
///
/// # Example Build
///
/// ```bash
/// VNC_PACKAGE="com/example/vnc" cargo build --release --target aarch64-linux-android
/// ```
///
/// Or with custom class names:
///
/// ```bash
/// VNC_PACKAGE="com/myapp" VNC_MAIN_SERVICE="VncService" cargo build --release
/// ```
#[cfg(vnc_standalone)]
#[no_mangle]
pub extern "system" fn JNI_OnLoad(
    vm: jni::JavaVM,
    _reserved: *mut std::ffi::c_void,
) -> jni::sys::jint {
    use jni::sys::{JNI_ERR, JNI_VERSION_1_6};

    // Initialize Android logger with configured tag
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag(env!("VNC_LOG_TAG")),
    );

    info!("JNI_OnLoad: Registering VNC native methods");

    // Get JNI environment
    let mut env = match vm.get_env() {
        Ok(env) => env,
        Err(e) => {
            error!("Failed to get JNI environment: {}", e);
            return JNI_ERR;
        }
    };

    // Build full class names from compile-time env vars
    let main_class = concat!(env!("VNC_PACKAGE"), "/", env!("VNC_MAIN_SERVICE"));
    let input_class = concat!(env!("VNC_PACKAGE"), "/", env!("VNC_INPUT_SERVICE"));

    info!(
        "Registering for classes: {} and {}",
        main_class, input_class
    );

    // Register native methods
    match register_vnc_natives(&mut env, main_class, input_class) {
        Ok(()) => {
            info!("VNC native methods registered successfully");
            JNI_VERSION_1_6
        }
        Err(e) => {
            error!("Failed to register VNC native methods: {}", e);
            JNI_ERR
        }
    }
}
