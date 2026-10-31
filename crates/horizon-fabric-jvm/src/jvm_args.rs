//! JVM configuration and argument building.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Locate a usable JDK/JRE installation on this machine.
///
/// Prefers `JAVA_HOME`, then a Java 21 installation (the version Minecraft 26.3
/// targets), then any other JDK found in the platform's usual install roots.
pub fn find_system_jvm() -> Result<PathBuf, crate::JvmError> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(home) = std::env::var("JAVA_HOME") {
        candidates.push(PathBuf::from(home));
    }

    let roots: &[&str] = if cfg!(windows) {
        &[
            "C:/Program Files/Eclipse Adoptium",
            "C:/Program Files/Java",
            "C:/Program Files/Microsoft/jdk",
            "C:/Program Files/BellSoft",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Library/Java/JavaVirtualMachines"]
    } else {
        &["/usr/lib/jvm", "/usr/java"]
    };

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                candidates.push(path);
            }
        }
    }

    // Prefer a Java 21 install, then anything else that actually has a JVM.
    candidates.sort_by_key(|path| {
        let name = path.to_string_lossy();
        if name.contains("21") {
            0
        } else if name.contains("17") {
            1
        } else {
            2
        }
    });

    for candidate in &candidates {
        if has_jvm_library(candidate) {
            return Ok(candidate.clone());
        }
        let jre = candidate.join("jre");
        if has_jvm_library(&jre) {
            return Ok(jre);
        }
    }

    Err(crate::JvmError::LibraryNotFound(
        "no system JVM found (set JAVA_HOME)".into(),
    ))
}

fn has_jvm_library(path: &Path) -> bool {
    let lib_name = if cfg!(windows) {
        "jvm.dll"
    } else if cfg!(target_os = "macos") {
        "libjvm.dylib"
    } else {
        "libjvm.so"
    };

    path.join("bin").join("server").join(lib_name).exists()
        || path.join("lib").join("server").join(lib_name).exists()
}

/// JVM configuration for the embedded Fabric JVM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JvmConfig {
    /// Path to the JVM installation directory (containing lib/server/jvm.dll or libjvm.so).
    pub jvm_path: PathBuf,
    /// Classpath for the JVM (Fabric Loader, Fabric API, mods, etc.).
    pub classpath: PathBuf,
    /// Minimum heap size (e.g., "64m", "128m").
    pub heap_min: String,
    /// Maximum heap size (e.g., "512m", "1g").
    pub heap_max: String,
    /// Garbage collector configuration.
    pub gc: GcConfig,
    /// Additional JVM options.
    pub extra_options: Vec<String>,
    /// Whether to enable JNI debugging.
    pub debug_jni: bool,
}

/// Garbage collector configuration options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GcConfig {
    /// ZGC - low latency, scalable (JDK 11+)
    Zgc {
        /// Maximum GC pause in milliseconds.
        max_pause_ms: Option<u32>,
    },
    /// Shenandoah - low latency, concurrent (JDK 12+)
    Shenandoah {
        /// Maximum GC pause in milliseconds.
        max_pause_ms: Option<u32>,
    },
    /// G1GC - balanced throughput/latency (JDK 9+)
    G1 {
        /// Maximum GC pause in milliseconds.
        max_pause_ms: Option<u32>,
    },
    /// Serial GC - simple, single-threaded (for small heaps)
    Serial,
}

impl Default for GcConfig {
    fn default() -> Self {
        // Prefer ZGC for low latency in game loop
        Self::Zgc { max_pause_ms: Some(1) }
    }
}

impl JvmConfig {
    /// Build the JVM command line options for this configuration.
    pub fn jvm_options(&self) -> Vec<String> {
        let mut options = vec![format!("-Xms{}", self.heap_min), format!("-Xmx{}", self.heap_max)];

        match &self.gc {
            GcConfig::Zgc { .. } => options.push("-XX:+UseZGC".to_string()),
            GcConfig::Shenandoah { .. } => options.push("-XX:+UseShenandoahGC".to_string()),
            GcConfig::G1 { max_pause_ms } => {
                options.push("-XX:+UseG1GC".to_string());
                if let Some(ms) = max_pause_ms {
                    options.push(format!("-XX:MaxGCPauseMillis={ms}"));
                }
            }
            GcConfig::Serial => options.push("-XX:+UseSerialGC".to_string()),
        }

        if self.debug_jni {
            options.push("-Xcheck:jni".to_string());
        }

        // Make the Fabric loader/API jars visible to `FindClass` as well, so
        // interfaces and JDK-adjacent classes resolve from native code.
        let entries = self.classpath_entries();
        if !entries.is_empty() {
            let joined = entries
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(if cfg!(windows) { ";" } else { ":" });
            options.push(format!("-Djava.class.path={joined}"));
        }

        options.extend(self.extra_options.iter().cloned());
        options
    }

    /// The classpath entries that exist on disk.
    pub fn classpath_entries(&self) -> Vec<PathBuf> {
        let raw = self.classpath.to_string_lossy();
        let separator = if cfg!(windows) { ';' } else { ':' };
        raw.split(separator)
            .filter(|entry| !entry.trim().is_empty())
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .collect()
    }

    /// Create a default configuration for the given Minecraft version.
    pub fn default_for_version(version: &str) -> Result<Self, crate::JvmError> {
        // The JVM is bundled by horizon-prepare in .local/fabric-jvm/
        let base = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("fabric-jvm")
            .join(version);
        
        let jvm_path = base.join("jre");
        
        // Classpath will be set by the loader
        let classpath = base.join("classpath");
        
        Ok(Self {
            jvm_path,
            classpath,
            heap_min: "64m".into(),
            heap_max: "512m".into(),
            gc: GcConfig::default(),
            extra_options: Vec::new(),
            debug_jni: false,
        })
    }
    
    /// Create a configuration for development/testing with a system JVM.
    pub fn for_system_jvm() -> Result<Self, crate::JvmError> {
        let jvm_path = find_system_jvm()?;

        Ok(Self {
            jvm_path,
            classpath: PathBuf::from("."),
            heap_min: "64m".into(),
            heap_max: "512m".into(),
            gc: GcConfig::default(),
            extra_options: Vec::new(),
            debug_jni: false,
        })
    }
}