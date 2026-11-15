import Foundation

/// The Documents directory is exposed in Files → On My iPad → Nova.
/// Keep existing installs intact when moving from hidden Application Support.
enum NovaVisibleStorage {
    private static let manager = FileManager.default
    private static let lock = NSLock()
    private static let folders: Set<String> = ["Game", "Resources", "Diagnostics"]

    static func directory(named name: String) throws -> URL {
        guard folders.contains(name) else {
            throw StorageError.invalidFolder
        }
        lock.lock()
        defer { lock.unlock() }

        let documents = try manager.url(for: .documentDirectory,
                                        in: .userDomainMask,
                                        appropriateFor: nil, create: true)
        let support = try manager.url(for: .applicationSupportDirectory,
                                      in: .userDomainMask,
                                      appropriateFor: nil, create: true)
        let legacy = support.appendingPathComponent("Nova", isDirectory: true)
                            .appendingPathComponent(name, isDirectory: true)
        let visible = documents.appendingPathComponent(name, isDirectory: true)

        if manager.fileExists(atPath: legacy.path) {
            if !manager.fileExists(atPath: visible.path) {
                // A same-container move preserves all existing worlds and
                // prepared game files, without copying gigabytes of resources.
                try manager.moveItem(at: legacy, to: visible)
            } else {
                // Partial migrations must never silently overwrite data.
                // Conflicting files are moved to a visible recovery folder.
                let recovery = documents.appendingPathComponent("Recovered Nova Files",
                                                                isDirectory: true)
                try moveContents(from: legacy, into: visible, recoveringTo: recovery)
            }
        }

        if !manager.fileExists(atPath: visible.path) {
            // Resources are ordinarily created atomically by the ZIP importer.
            if name != "Resources" {
                try manager.createDirectory(at: visible,
                                            withIntermediateDirectories: true)
            }
        }
        if manager.fileExists(atPath: visible.path) {
            // Imported Minecraft data may be large: do not upload it to
            // automatic cloud backups merely because it is visible in Files.
            var local = visible
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try local.setResourceValues(values)
        }
        return visible
    }

    private static func moveContents(from source: URL, into destination: URL,
                                     recoveringTo recovery: URL) throws {
        var isDirectory: ObjCBool = false
        guard manager.fileExists(atPath: source.path, isDirectory: &isDirectory),
              isDirectory.boolValue else { throw StorageError.invalidFolder }
        try manager.createDirectory(at: destination, withIntermediateDirectories: true)
        for item in try manager.contentsOfDirectory(at: source,
                        includingPropertiesForKeys: nil, options: []) {
            let target = destination.appendingPathComponent(item.lastPathComponent)
            if !manager.fileExists(atPath: target.path) {
                try manager.moveItem(at: item, to: target)
                continue
            }
            var sourceDir: ObjCBool = false
            var targetDir: ObjCBool = false
            _ = manager.fileExists(atPath: item.path, isDirectory: &sourceDir)
            _ = manager.fileExists(atPath: target.path, isDirectory: &targetDir)
            if sourceDir.boolValue && targetDir.boolValue {
                try moveContents(from: item, into: target, recoveringTo: recovery)
            } else {
                try manager.createDirectory(at: recovery,
                                            withIntermediateDirectories: true)
                let basename = item.deletingPathExtension().lastPathComponent
                let ext = item.pathExtension
                let unique = "\(basename)-legacy-\(UUID().uuidString)"
                let filename = ext.isEmpty ? unique : "\(unique).\(ext)"
                let copy = recovery.appendingPathComponent(filename)
                try manager.moveItem(at: item, to: copy)
            }
        }
        try manager.removeItem(at: source)
    }

    enum StorageError: Error {
        case invalidFolder
    }
}
