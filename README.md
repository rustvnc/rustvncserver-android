# rustvncserver-android

Generic Android JNI bindings for [rustvncserver](https://github.com/rustvnc/rustvncserver) - a pure Rust VNC server library.

[![Crates.io](https://img.shields.io/crates/v/rustvncserver-android.svg)](https://crates.io/crates/rustvncserver-android)
[![Documentation](https://docs.rs/rustvncserver-android/badge.svg)](https://docs.rs/rustvncserver-android)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Build Status](https://github.com/rustvnc/rustvncserver-android/workflows/CI/badge.svg)](https://github.com/rustvnc/rustvncserver-android/actions)

[![LinkedIn](https://img.shields.io/badge/LinkedIn-Dustin%20McAfee-blue?style=flat&logo=linkedin)](https://www.linkedin.com/in/dustinmcafee/)

**Support this project:**

[![GitHub Sponsors](https://img.shields.io/badge/Sponsor-❤-red?style=flat&logo=github-sponsors)](https://github.com/sponsors/dustinmcafee)
[![PayPal](https://img.shields.io/badge/PayPal-Donate-blue?style=flat&logo=paypal)](https://paypal.me/dustinmcafee)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee-☕-yellow?style=flat&logo=buy-me-a-coffee)](https://buymeacoffee.com/dustinmcafee)

## Overview

This crate provides ready-to-use Android JNI bindings for the rustvncserver library. It uses **runtime configuration** via Java system properties - just set properties before loading the library and everything is configured automatically.

**Key Features:**
- Runtime configurable Java package via system properties (no build-time config needed)
- Use as a crates.io dependency - no custom JNI code required
- Flexible JNI method registration (required vs optional methods)
- Full VNC server lifecycle management
- TurboJPEG support for hardware-accelerated JPEG compression
- Works with any Android app package structure

## Quick Start

### Option 1: As a Dependency (Recommended)

Add to your `Cargo.toml`:

```toml
[package]
name = "my-vnc-app"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
rustvncserver-android = { version = "1.0", features = ["turbojpeg"] }
```

Create a minimal `src/lib.rs`:

```rust
// Re-export everything including JNI_OnLoad
pub use rustvncserver_android::*;
```

In your Java code, set system properties BEFORE loading the library:

```java
public class MainService extends Service {
    static {
        // Configure class names (use / instead of . in package path)
        System.setProperty("rustvnc.main_service_class", "com/mycompany/vnc/MainService");
        System.setProperty("rustvnc.input_service_class", "com/mycompany/vnc/InputService");
        System.setProperty("rustvnc.log_tag", "MyApp-VNC");  // Optional

        // Load the library - JNI_OnLoad registers natives automatically
        System.loadLibrary("my_vnc_app");
    }

    // Native methods are now available!
    private native boolean vncStartServer(int width, int height, int port,
                                          String desktopName, String password, String httpRootDir);
    // ... see full list below
}
```

### Option 2: Manual Registration

If you need more control, skip the system properties and call `register_vnc_natives()` from your own `JNI_OnLoad`:

```rust
use rustvncserver_android::register_vnc_natives;
use jni::JavaVM;
use jni::sys::{jint, JNI_VERSION_1_6, JNI_ERR};
use std::ffi::c_void;

#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut c_void) -> jint {
    let mut env = match vm.get_env() {
        Ok(env) => env,
        Err(_) => return JNI_ERR,
    };

    match register_vnc_natives(
        &mut env,
        "com/mycompany/vnc/MainService",
        "com/mycompany/vnc/InputService",
    ) {
        Ok(()) => JNI_VERSION_1_6,
        Err(_) => JNI_ERR,
    }
}
```

## System Properties

Set these properties in Java **before** calling `System.loadLibrary()`:

| Property | Required | Default | Description |
|----------|----------|---------|-------------|
| `rustvnc.main_service_class` | Yes | - | Full class path (e.g., `com/mycompany/vnc/MainService`) |
| `rustvnc.input_service_class` | Yes | - | Full class path (e.g., `com/mycompany/vnc/InputService`) |
| `rustvnc.log_tag` | No | `RustVNC` | Android logcat tag |

## Native Methods

### Required Methods

These methods **must** be declared in your Java class:

```java
// Start VNC server with given dimensions, port, name, password, and HTTP root dir
private native boolean vncStartServer(int width, int height, int port,
                                      String desktopName, String password, String httpRootDir);

// Stop the VNC server
private native boolean vncStopServer();

// Check if the server is currently running
private native boolean vncIsActive();

// Connect to a UltraVNC repeater
private native long vncConnectRepeater(String host, int port, String repeaterId, String requestId);
```

### Optional Methods

These methods are registered only if declared in your Java class:

```java
// Framebuffer updates
static native boolean vncUpdateFramebuffer(ByteBuffer buf);
static native boolean vncUpdateFramebufferAndSend(ByteBuffer buf, int width, int height);
static native boolean vncUpdateFramebufferCropped(ByteBuffer buf, int width, int height,
                                                   int cropX, int cropY, int cropWidth, int cropHeight);
static native boolean vncNewFramebuffer(int width, int height);

// Framebuffer info
static native int vncGetFramebufferWidth();
static native int vncGetFramebufferHeight();

// Clipboard
static native void vncSendCutText(String text);

// Connections
static native long vncConnectReverse(String host, int port);

// Client info (pass clientId from callbacks)
static native String vncGetRemoteHost(long clientId);
static native int vncGetDestinationPort(long clientId);
static native String vncGetRepeaterId(long clientId);
static native boolean vncDisconnect(long clientId);

// CopyRect optimization (for scrolling/window dragging)
static native void vncScheduleCopyRect(int srcX, int srcY, int width, int height, int dstX, int dstY);
static native boolean vncDoCopyRect(int srcX, int srcY, int width, int height, int dstX, int dstY);
```

## Callbacks

The library calls these static methods on your Java classes:

### MainService Callbacks

```java
public static void onClientConnected(long clientId) { }
public static void onClientDisconnected(long clientId) { }
public static void notifyRfbMessageSent(String requestId, boolean success) { }
public static void notifyHandshakeComplete(String requestId, boolean success) { }
```

### InputService Callbacks

```java
public static void onKeyEvent(int down, long keysym, long clientId) { }
public static void onPointerEvent(int buttonMask, int x, int y, long clientId) { }
public static void onCutText(String text, long clientId) { }
```

## Gradle Integration

Example `build.gradle` task for building the Rust library:

```groovy
tasks.register('buildRust', Exec) {
    def ndkDir = android.ndkDirectory.absolutePath

    environment "ANDROID_NDK_ROOT", ndkDir

    commandLine 'cargo', 'ndk', '-t', 'arm64-v8a', '-o',
                'app/src/main/jniLibs', 'build', '--release', '--features', 'turbojpeg'
}

tasks.whenTaskAdded { task ->
    if (task.name == 'assembleDebug' || task.name == 'assembleRelease') {
        task.dependsOn buildRust
    }
}
```

## Features

| Feature | Description |
|---------|-------------|
| `turbojpeg` | Enable TurboJPEG for hardware-accelerated JPEG compression |
| `debug-logging` | Enable verbose debug logging |

### TurboJPEG Setup

For `turbojpeg` support, you need to build libjpeg-turbo for Android. See the [libjpeg-turbo Android build guide](https://github.com/nickkuk/libjpeg-turbo-android-gradle) for details.

## Platform Support

- Android arm64-v8a (primary target)
- Android armeabi-v7a
- Android x86_64
- Android x86

## Dependencies

This crate depends on:
- [rustvncserver](https://github.com/rustvnc/rustvncserver) - Pure Rust VNC server library
- [jni](https://crates.io/crates/jni) - Rust JNI bindings
- [tokio](https://crates.io/crates/tokio) - Async runtime
- [android_logger](https://crates.io/crates/android_logger) - Android logcat integration

## License

Apache-2.0 - See [LICENSE](LICENSE) file for details.

## Credits

- **Author**: Dustin McAfee
- **VNC Server**: [rustvncserver](https://github.com/rustvnc/rustvncserver)
- **Encodings**: [rfb-encodings](https://github.com/rustvnc/rfb-encodings)

## See Also

- [rustvncserver](https://github.com/rustvnc/rustvncserver) - The underlying VNC server library
- [RustVNC](https://github.com/nickkuk/RustVNC) - Example Android VNC server app using this library
- [RFC 6143](https://datatracker.ietf.org/doc/html/rfc6143) - RFB Protocol Specification

---

**Android JNI bindings for rustvncserver - Runtime configurable, production ready**
