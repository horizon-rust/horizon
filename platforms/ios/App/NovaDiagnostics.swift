import Foundation
import Darwin

/// Per-process diagnostics visible in Files → Nova → Diagnostics.
///
/// The live session is lastlog.text. At the next cold start, that complete
/// file is renamed to its creation-time timestamp, even if the previous
/// session crashed or was force-closed. Do not clear the file while running.
///
/// These files can contain player/server information from Horizon's own logs.
/// Nothing is uploaded automatically.
enum NovaDiagnostics {
    private static let lock = NSLock()
    private static var currentURL: URL?

    /// Initialize before recording the very first process event. Calling this
    /// again in the same process must not rotate the current session.
    static func startSession() {
        _ = logURL()
    }

    static func logURL() -> URL? {
        lock.lock()
        defer { lock.unlock() }
        return logURLLocked()
    }

    private static func logURLLocked() -> URL? {
        if let currentURL { return currentURL }
        do {
            let folder = try NovaVisibleStorage.directory(named: "Diagnostics")
            let url = folder.appendingPathComponent("lastlog.text")
            if FileManager.default.fileExists(atPath: url.path) {
                // The archived name describes when the previous session began,
                // not when the new session was started.
                let attrs = try FileManager.default.attributesOfItem(atPath: url.path)
                let started = (attrs[.creationDate] as? Date)
                    ?? (attrs[.modificationDate] as? Date) ?? Date()
                let format = DateFormatter()
                format.locale = Locale(identifier: "en_US_POSIX")
                format.dateFormat = "yyyy-MM-dd_HH-mm-ss"
                let name = format.string(from: started)
                var archive = folder.appendingPathComponent("\(name).text")
                var suffix = 2
                while FileManager.default.fileExists(atPath: archive.path) {
                    archive = folder.appendingPathComponent("\(name)-\(suffix).text")
                    suffix += 1
                }
                try FileManager.default.moveItem(at: url, to: archive)
            }
            guard FileManager.default.createFile(atPath: url.path, contents: Data()) else {
                return nil
            }
            currentURL = url
            return url
        } catch {
            NSLog("Nova: diagnostic session unavailable: %@", error.localizedDescription)
            return nil
        }
    }

    /// Capture native stdout/stderr (including early Rust and renderer errors)
    /// in the SAME file, without interfering with Horizon's own latest.log.
    /// Call once, at process startup, before invoking the Rust game.
    static func captureConsole() {
        guard let url = logURL() else { return }
        let descriptor = url.path.withCString { Darwin.open($0, O_WRONLY | O_APPEND) }
        guard descriptor >= 0 else { return }
        let stdoutOK = Darwin.dup2(descriptor, STDOUT_FILENO) >= 0
        let stderrOK = Darwin.dup2(descriptor, STDERR_FILENO) >= 0
        if descriptor != STDOUT_FILENO && descriptor != STDERR_FILENO {
            _ = Darwin.close(descriptor)
        }
        record("diagnostics.console-capture; stdout=\(stdoutOK); stderr=\(stderrOK)")
    }

    static func record(_ stage: String) {
        lock.lock()
        defer { lock.unlock() }
        guard let url = logURLLocked() else { return }
        let stamp = ISO8601DateFormatter().string(from: Date())
        let sanitized = stage.replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
        let bytes = Array("[\(stamp)] [swift] \(sanitized)\n".utf8)
        // O_APPEND prevents concurrent Rust/Swift writes from seeking to an
        // outdated end-of-file offset and overwriting one another.
        let descriptor = url.path.withCString { Darwin.open($0, O_WRONLY | O_APPEND) }
        guard descriptor >= 0 else {
            NSLog("Nova: could not open diagnostic session for writing")
            return
        }
        defer { _ = Darwin.close(descriptor) }
        let complete = bytes.withUnsafeBytes { buffer -> Bool in
            guard let base = buffer.baseAddress else { return false }
            var offset = 0
            while offset < buffer.count {
                let result = Darwin.write(descriptor, base.advanced(by: offset),
                                          buffer.count - offset)
                if result > 0 {
                    offset += result
                } else if result == -1 && errno == EINTR {
                    continue
                } else {
                    return false
                }
            }
            return true
        }
        if !complete {
            NSLog("Nova: could not complete diagnostic session write")
        }
    }

    /// Read only the last 32 KB, even when a long gameplay session generated
    /// a large log. The full file remains available in Files.
    static func tail() -> String {
        guard let url = logURL() else {
            return "No diagnostics have been recorded yet."
        }
        do {
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            let end = try handle.seekToEnd()
            try handle.seek(toOffset: end > 32_000 ? end - 32_000 : 0)
            let data = try handle.readToEnd() ?? Data()
            return String(decoding: data, as: UTF8.self)
        } catch {
            return "Could not read diagnostics: \(error.localizedDescription)"
        }
    }
}
