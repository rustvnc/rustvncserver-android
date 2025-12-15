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

This crate provides ready-to-use Android JNI bindings for the rustvncserver library. It compiles directly to a `.so` file that can be loaded by any Android app - **no wrapper crate needed**.

**Key Features:**
- Build-time configurable Java package via environment variables
- Flexible JNI method registration (required vs optional methods)
- Full VNC server lifecycle management
- TurboJPEG support for hardware-accelerated JPEG compression
- Works with any Android app package structure

## Quick Start

### 1. Build the Library

```bash
# Set your Java package path
export VNC_PACKAGE="com/mycompany/vnc"
export VNC_MAIN_SERVICE="MainService"      # Optional, defaults to "MainService"
export VNC_INPUT_SERVICE="InputService"    # Optional, defaults to "InputService"
export VNC_LOG_TAG="MyApp-VNC"             # Optional, defaults to "RustVNC"

# Build for Android
cargo ndk -t arm64-v8a build --release --features turbojpeg
```

### 2. Copy to Your App

```bash
cp target/aarch64-linux-android/release/librustvncserver_android.so \
   app/src/main/jniLibs/arm64-v8a/
```

### 3. Add Java Native Methods

```java
public class MainService extends Service {
    static {
        System.loadLibrary("rustvncserver_android");
        vncInit();
    }

    // Required native methods
    private static native void vncInit();
    private native boolean vncStartServer(int width, int height, int port,
                                          String desktopName, String password, String httpRootDir);
    private native boolean vncStopServer();
    private native boolean vncIsActive();
    private native long vncConnectRepeater(String host, int port, String repeaterId, String requestId);

    // Optional native methods (only declare if you use them)
    static native boolean vncUpdateFramebuffer(ByteBuffer buf);
    static native boolean vncNewFramebuffer(int width, int height);
    // ... see full list below

    // Callbacks (called from Rust)
    public static void onClientConnected(long clientId) { /* ... */ }
    public static void onClientDisconnected(long clientId) { /* ... */ }
}

public class InputService {
    public static void onKeyEvent(int down, long keysym, long clientId) { /* ... */ }
    public static void onPointerEvent(int buttonMask, int x, int y, long clientId) { /* ... */ }
    public static void onCutText(String text, long clientId) { /* ... */ }
}
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `VNC_PACKAGE` | Yes | - | Java package path (e.g., `com/mycompany/vnc`) |
| `VNC_MAIN_SERVICE` | No | `MainService` | Main service class name |
| `VNC_INPUT_SERVICE` | No | `InputService` | Input service class name |
| `VNC_LOG_TAG` | No | `RustVNC` | Android logcat tag |

## Native Methods

### Required Methods

These methods **must** be declared in your Java class:

```java
// Initialize the VNC runtime (call once at startup)
private static native void vncInit();

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
    def vncPackage = "com/mycompany/vnc"
    def ndkDir = android.ndkDirectory.absolutePath

    environment "VNC_PACKAGE", vncPackage
    environment "VNC_LOG_TAG", "MyApp-VNC"
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

**Android JNI bindings for rustvncserver - Build-time configurable, production ready**
