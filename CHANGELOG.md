# Changelog

All notable changes to rustvncserver-android will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2025-12-15

### Changed

- **BREAKING**: Switched from build-time to runtime configuration
  - JNI_OnLoad now reads class names from Java system properties instead of compile-time environment variables
  - Set `rustvnc.main_service_class`, `rustvnc.input_service_class`, and optionally `rustvnc.log_tag` before calling `System.loadLibrary()`
- Removed build.rs compile-time configuration (VNC_PACKAGE, VNC_MAIN_SERVICE, VNC_INPUT_SERVICE, VNC_LOG_TAG environment variables no longer used)
- JNI_OnLoad is now always exported (no longer requires `#[cfg(vnc_standalone)]`)
- Can now be used as a crates.io dependency without any custom JNI code - just re-export and set system properties

### Added

- `read_system_property()` internal function for reading Java system properties from Rust
- Automatic native method registration when system properties are set
- Graceful fallback when properties not set (allows manual registration via `register_vnc_natives()`)

### Migration Guide

**Before (v1.0.x):**
```bash
VNC_PACKAGE="com/mycompany/vnc" cargo ndk build --release
```

**After (v1.1.0):**
```java
// In Java, before System.loadLibrary():
System.setProperty("rustvnc.main_service_class", "com/mycompany/vnc/MainService");
System.setProperty("rustvnc.input_service_class", "com/mycompany/vnc/InputService");
System.loadLibrary("your_lib_name");
```

## [1.0.1] - 2025-12-15

### Changed

- Improved README documentation with readable Java method signatures instead of JNI notation

## [1.0.0] - 2025-12-15

**Initial Release** - Generic Android JNI bindings for rustvncserver.

### Added

**Build-time Configuration:**
- Environment variable configuration for Java package names
  - `VNC_PACKAGE` - Java package path (e.g., `com/mycompany/vnc`)
  - `VNC_MAIN_SERVICE` - Main service class name (default: `MainService`)
  - `VNC_INPUT_SERVICE` - Input service class name (default: `InputService`)
  - `VNC_LOG_TAG` - Android logcat tag (default: `RustVNC`)
- Standalone build mode - compiles directly to `.so` file with no wrapper crate needed

**JNI Method Registration:**
- Dynamic JNI method registration via `RegisterNatives`
- Flexible required vs optional method separation
  - Required methods fail registration if missing
  - Optional methods silently skip if not declared in Java
- Full VNC server lifecycle: init, start, stop, isActive
- Framebuffer management: update, resize, cropped updates
- Connection management: repeater, reverse connections, disconnect
- CopyRect optimization support
- Clipboard (cut text) support

**Required Native Methods:**
- `vncInit()` - Initialize VNC runtime
- `vncStartServer(...)` - Start VNC server
- `vncStopServer()` - Stop VNC server
- `vncIsActive()` - Check server status
- `vncConnectRepeater(...)` - Connect to UltraVNC repeater

**Optional Native Methods:**
- `vncUpdateFramebuffer(ByteBuffer)` - Update framebuffer
- `vncUpdateFramebufferAndSend(ByteBuffer, int, int)` - Update with dimensions
- `vncUpdateFramebufferCropped(...)` - Update cropped region
- `vncNewFramebuffer(int, int)` - Resize framebuffer
- `vncSendCutText(String)` - Send clipboard text
- `vncGetFramebufferWidth()` / `vncGetFramebufferHeight()` - Get dimensions
- `vncConnectReverse(String, int)` - Connect to listening viewer
- `vncGetRemoteHost(long)` / `vncGetDestinationPort(long)` / `vncGetRepeaterId(long)` - Client info
- `vncDisconnect(long)` - Disconnect specific client
- `vncScheduleCopyRect(...)` / `vncDoCopyRect(...)` - CopyRect operations

**Java Callbacks:**
- `MainService.onClientConnected(long clientId)`
- `MainService.onClientDisconnected(long clientId)`
- `MainService.notifyRfbMessageSent(String requestId, boolean success)`
- `MainService.notifyHandshakeComplete(String requestId, boolean success)`
- `InputService.onKeyEvent(int down, long keysym, long clientId)`
- `InputService.onPointerEvent(int buttonMask, int x, int y, long clientId)`
- `InputService.onCutText(String text, long clientId)`

**Features:**
- `turbojpeg` - Enable TurboJPEG for hardware-accelerated JPEG compression
- `debug-logging` - Enable verbose debug logging

**Documentation:**
- Comprehensive README with usage examples
- Gradle integration examples
- Full native method reference

### Dependencies

- `rustvncserver` 2.0 - Pure Rust VNC server library
- `jni` 0.21 - Rust JNI bindings
- `tokio` 1.x - Async runtime with multi-thread support
- `android_logger` 0.15 - Android logcat integration
- `once_cell` 1.19 - Thread-safe lazy initialization
- `log` 0.4 - Logging facade

---

## Release Information

**Initial Release:** v1.0.0 marks the first stable release of rustvncserver-android with complete JNI bindings for Android apps.

**License:** Apache License 2.0

**Repository:** https://github.com/rustvnc/rustvncserver-android
