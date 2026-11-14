import Foundation

/// Stores user-provided ZIP archives without extracting untrusted contents.
/// Future resource preparation must validate manifest/version and safe paths.
enum NovaGameFiles {
    private static func archiveURL() throws -> URL {
        let documents = try FileManager.default.url(
            for: .documentDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        let directory = documents.appendingPathComponent("Nova/Imports", isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory.appendingPathComponent("GameFiles.zip", isDirectory: false)
    }

    static func stagedArchiveName() -> String? {
        guard let url = try? archiveURL(),
              FileManager.default.fileExists(atPath: url.path) else { return nil }
        return url.lastPathComponent
    }

    /// Import from Files.app / a security-scoped document provider into the
    /// app's own sandbox. No network, decompression or shell commands are used.
    static func stageZip(from source: URL) throws -> String {
        let access = source.startAccessingSecurityScopedResource()
        defer { if access { source.stopAccessingSecurityScopedResource() } }

        let attributes = try FileManager.default.attributesOfItem(atPath: source.path)
        let fileSize = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        guard fileSize >= 4 else { throw ArchiveError.invalidZip }

        let handle = try FileHandle(forReadingFrom: source)
        defer { try? handle.close() }
        let magic = try handle.read(upToCount: 4) ?? Data()
        guard magic == Data([0x50, 0x4B, 0x03, 0x04]) else {
            throw ArchiveError.invalidZip
        }

        let destination = try archiveURL()
        let temporary = destination.deletingLastPathComponent()
            .appendingPathComponent(UUID().uuidString + ".zip")
        let manager = FileManager.default
        do {
            try manager.copyItem(at: source, to: temporary)
            // Preserve an existing working archive if copy fails.
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

    enum ArchiveError: LocalizedError {
        case invalidZip

        var errorDescription: String? {
            switch self {
            case .invalidZip: return "The selected file is not a supported ZIP archive."
            }
        }
    }
}
