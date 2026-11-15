import Foundation

/// Nova imports and prepares USER-SUPPLIED assets offline, never distributing them.
enum NovaGameFiles {
    private static let manager = FileManager.default

    private static func importsURL() throws -> URL {
        let caches = try manager.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let folder = caches.appendingPathComponent("Nova/Imports", isDirectory: true)
        try manager.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder.appendingPathComponent("GameFiles.zip", isDirectory: false)
    }

    private static func legacyArchiveURL() throws -> URL {
        let docs = try manager.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        return docs.appendingPathComponent("Nova/Imports/GameFiles.zip")
    }

    private static func storedArchiveURL() throws -> URL? {
        let newer = try importsURL()
        if manager.fileExists(atPath: newer.path) { return newer }
        let old = try legacyArchiveURL()
        return manager.fileExists(atPath: old.path) ? old : nil
    }

    /// Prepared user-imported assets are accessible under Files → Nova → Resources.
    /// Existing private installations migrate without reimporting the ZIP.
    static func resourceDirectory() throws -> URL {
        try NovaVisibleStorage.directory(named: "Resources")
    }

    /// Game saves, options, logs, resourcepacks and shaders are exposed in
    /// Files → Nova → Game. Original game data migrates before Horizon boots.
    static func gameDirectory() throws -> URL {
        try NovaVisibleStorage.directory(named: "Game")
    }

    static func stagedArchiveName() -> String? {
        guard let archive = try? storedArchiveURL() else { return nil }
        return archive.lastPathComponent
    }

    /// Only structural validation, not proof of Horizon version compatibility.
    static func resourcesPresent() -> Bool {
        guard let directory = try? resourceDirectory() else { return false }
        return directory.path.withCString { nova_ios_resources_present($0) }
    }

    /// Copy a selected Files.app ZIP into Nova's temporary cache.
    static func stageZip(from source: URL) throws -> String {
        let access = source.startAccessingSecurityScopedResource()
        defer { if access { source.stopAccessingSecurityScopedResource() } }
        let attrs = try manager.attributesOfItem(atPath: source.path)
        guard (attrs[.size] as? NSNumber)?.int64Value ?? 0 >= 4 else {
            throw ArchiveError.invalidZip
        }

        let handle = try FileHandle(forReadingFrom: source)
        defer { try? handle.close() }
        let magic = try handle.read(upToCount: 4) ?? Data()
        guard magic == Data([0x50, 0x4B, 0x03, 0x04]) else {
            throw ArchiveError.invalidZip
        }
        let destination = try importsURL()
        let temporary = destination.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".zip")
        do {
            try manager.copyItem(at: source, to: temporary)
            if manager.fileExists(atPath: destination.path) {
                try manager.removeItem(at: destination)
            }
            try manager.moveItem(at: temporary, to: destination)
        } catch {
            try? manager.removeItem(at: temporary)
            throw error
        }
        return destination.lastPathComponent
    }

    /// Blocks while extracting. Call on a worker queue, not the main thread.
    static func prepareStoredZip() throws {
        guard let archive = try storedArchiveURL() else { throw ArchiveError.noArchive }
        let destination = try resourceDirectory()
        var errors = [CChar](repeating: 0, count: 1024)
        let result = archive.path.withCString { source in
            destination.path.withCString { target in
                errors.withUnsafeMutableBufferPointer { buffer in
                    nova_ios_install_resources_zip(source, target, buffer.baseAddress, UInt(buffer.count))
                }
            }
        }
        guard result == 0 else {
            let message = errors.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
            throw ArchiveError.installFailed(message)
        }
        guard resourcesPresent() else {
            throw ArchiveError.installFailed("Extracted resources did not pass validation.")
        }
        // Only remove Nova's temporary copy; user's original ZIP is untouched.
        try? manager.removeItem(at: archive)
    }

    enum ArchiveError: LocalizedError {
        case invalidZip
        case noArchive
        case installFailed(String)

        var errorDescription: String? {
            switch self {
            case .invalidZip: return "The selected file is not a valid ZIP archive."
            case .noArchive: return "Import your ZIP first."
            case .installFailed(let message): return message
            }
        }
    }
}
