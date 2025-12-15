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

//! Build script for rustvncserver-android.
//!
//! Reads environment variables to configure the JNI class names at compile time.
//!
//! # Environment Variables
//!
//! - `VNC_PACKAGE` - Java package path (e.g., "com/example/vnc"). Required for standalone build.
//! - `VNC_MAIN_SERVICE` - Main service class name (default: "MainService")
//! - `VNC_INPUT_SERVICE` - Input service class name (default: "InputService")
//! - `VNC_LOG_TAG` - Android log tag (default: "RustVNC")
//!
//! # Example
//!
//! ```bash
//! VNC_PACKAGE="com/example/vnc" cargo build --release
//! ```

fn main() {
    // Tell Cargo/clippy that vnc_standalone is a valid cfg flag
    println!("cargo::rustc-check-cfg=cfg(vnc_standalone)");

    // Re-run if these env vars change
    println!("cargo:rerun-if-env-changed=VNC_PACKAGE");
    println!("cargo:rerun-if-env-changed=VNC_MAIN_SERVICE");
    println!("cargo:rerun-if-env-changed=VNC_INPUT_SERVICE");
    println!("cargo:rerun-if-env-changed=VNC_LOG_TAG");

    // Check if VNC_PACKAGE is set - if so, enable standalone mode
    if let Ok(package) = std::env::var("VNC_PACKAGE") {
        println!("cargo:rustc-cfg=vnc_standalone");
        println!("cargo:rustc-env=VNC_PACKAGE={}", package);

        let main_service =
            std::env::var("VNC_MAIN_SERVICE").unwrap_or_else(|_| "MainService".to_string());
        println!("cargo:rustc-env=VNC_MAIN_SERVICE={}", main_service);

        let input_service =
            std::env::var("VNC_INPUT_SERVICE").unwrap_or_else(|_| "InputService".to_string());
        println!("cargo:rustc-env=VNC_INPUT_SERVICE={}", input_service);

        let log_tag = std::env::var("VNC_LOG_TAG").unwrap_or_else(|_| "RustVNC".to_string());
        println!("cargo:rustc-env=VNC_LOG_TAG={}", log_tag);

        eprintln!(
            "rustvncserver-android: Building standalone .so for {}/{}",
            package, main_service
        );
    }
}
