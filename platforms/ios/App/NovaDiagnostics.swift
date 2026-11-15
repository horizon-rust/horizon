import Foundation

/// Persistent launch diagnostics are visible in Files → Nova → Diagnostics.
/// The game-owned UIKit process writes the same file through NOVA_DIAGNOSTIC_LOG
/// so startup failures can be inspected without exporting a separate log.
///
/// Do not log game content, player data, accounts, or absolute sandbox paths.
enum NovaDiagnostics {
    private static let lock = NSLock()
    private static let maxBytes: UInt64 = 2_000_000

    static func logURL() -> URL? {
        do {
            let directory = try NovaVisibleStorage.directory(named: "Diagnostics")
            let url = directory.appendingPathComponent("nova-diagnostics.log")
            if !FileManager.default.fileExists(atPath: url.path) {
                guard FileManager.default.createFile(
                    atPath: url.path, contents: Data()
                ) else { return nil }
            }
            return url
        } catch {
            NSLog("Nova: diagnostic log unavailable: %@", error.localizedDescription)
            return nil
        }
    }

    /// Call before the Rust game is started; the previous failed attempt stays
    /// visible until it exceeds the file cap. Never clears logs on relaunch.
    static func record(_ stage: String) {
        lock.lock()
        defer { lock.unlock() }
        guard let url = logURL() else { return }

        do {
            if let size = try FileManager.default
                .attributesOfItem(atPath: url.path)[.size] as? NSNumber,
                size.uint64Value >= maxBytes {
                let previous = url.deletingLastPathComponent()
                    .appendingPathComponent("nova-previous.log")
                try? FileManager.default.removeItem(at: previous)
                try FileManager.default.moveItem(at: url, to: previous)
                _ = FileManager.default.createFile(atPath: url.path, contents: Data())
            }
            let stamp = ISO8601DateFormatter().string(from: Date())
            let sanitized = stage.replacingOccurrences(of: "\n", with: " ")
            let data = Data("[\(stamp)] [swift] \(sanitized)\n".utf8)
            let handle = try FileHandle(forWritingTo: url)
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: data)
        } catch {
            NSLog("Nova: could not write diagnostic log: %@", error.localizedDescription)
        }
    }

    static func tail() -> String {
        guard let url = logURL(), let data = try? Data(contentsOf: url) else {
            return "No diagnostics have been recorded yet."
        }
        let suffix = data.suffix(32_000)
        return String(decoding: suffix, as: UTF8.self)
    }
}
