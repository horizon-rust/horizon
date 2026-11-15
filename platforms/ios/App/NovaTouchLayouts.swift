import Foundation

/// Normalized positions keep button layouts portable between iPad sizes.
struct NovaTouchControl: Identifiable, Codable, Equatable {
    var id = UUID()
    var name: String = "new"
    var kind: String = "keyboard"
    var key: String = "Space"
    var x: Double = 0.5
    var y: Double = 0.5
    var size: Double = 64
    var opacity: Double = 0.7

    static let kinds = ["keyboard", "mouseLeft", "mouseRight",
                        "virtualMouse", "virtualKeyboard", "menu"]

    var symbol: String {
        switch kind {
        case "mouseLeft": return "cursorarrow.click"
        case "mouseRight": return "cursorarrow.rays"
        case "virtualMouse": return "cursorarrow"
        case "virtualKeyboard": return "keyboard"
        case "menu": return "line.3.horizontal"
        default: return "keyboard"
        }
    }
}

struct NovaTouchLayout: Codable {
    var version = 1
    var name: String = "default"
    var controls: [NovaTouchControl] = []

    static var starter: NovaTouchLayout {
        func key(_ label: String, _ binding: String, _ x: Double, _ y: Double) -> NovaTouchControl {
            NovaTouchControl(name: label, kind: "keyboard", key: binding,
                             x: x, y: y, size: 58, opacity: 0.6)
        }
        return NovaTouchLayout(controls: [
            key("W", "W", 0.14, 0.64),
            key("A", "A", 0.07, 0.76),
            key("S", "S", 0.14, 0.76),
            key("D", "D", 0.21, 0.76),
            key("Jump", "Space", 0.88, 0.73),
            key("E", "E", 0.89, 0.57),
            NovaTouchControl(name: "Attack", kind: "mouseLeft", x: 0.73, y: 0.76),
            NovaTouchControl(name: "Use", kind: "mouseRight", x: 0.8, y: 0.63),
            NovaTouchControl(name: "Mouse", kind: "virtualMouse", x: 0.55, y: 0.66),
            NovaTouchControl(name: "Type", kind: "virtualKeyboard", x: 0.93, y: 0.14)
        ])
    }
}

/// JSON lives in Documents/TouchLayouts and is exposed by Files sharing.
enum NovaTouchLayouts {
    static let currentKey = "nova.touch.current"
    static let keyboardKeys: [String] = [
        "W", "A", "S", "D", "Space", "Shift", "Control", "Escape",
        "Enter", "Tab", "Backspace", "E", "Q", "R", "F", "T", "G",
        "C", "V", "X", "Z", "B", "N", "M", "P", "I", "O", "L", "K",
        "J", "H", "U", "Y", "1", "2", "3", "4", "5", "6", "7",
        "8", "9", "0", "F1", "F2", "F3", "F5", "F11",
        "Up", "Down", "Left", "Right", "Slash"
    ]

    static func keyCode(_ key: String) -> UInt32 {
        let names = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        if key.count == 1, let a = key.uppercased().first,
           let scalar = a.asciiValue,
           names.contains(a) || "0123456789".contains(a) {
            return UInt32(scalar)
        }
        let special: [String: UInt32] = [
            "Space": 1001, "Shift": 1002, "Control": 1003,
            "Escape": 1004, "Enter": 1005, "Tab": 1006,
            "Backspace": 1007, "Up": 1008, "Down": 1009,
            "Left": 1010, "Right": 1011, "F1": 1012,
            "F2": 1013, "F3": 1014, "F5": 1015, "F11": 1016,
            "Slash": 1017
        ]
        return special[key] ?? 1001
    }

    static func directory() throws -> URL {
        let docs = try FileManager.default.url(for: .documentDirectory,
                                                in: .userDomainMask,
                                                appropriateFor: nil, create: true)
        let url = docs.appendingPathComponent("TouchLayouts", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private static func file(_ name: String) throws -> URL {
        let characters = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " -_"))
        let sanitized = String(name.unicodeScalars.map {
            characters.contains($0) ? Character($0) : Character("_")
        }).trimmingCharacters(in: .whitespaces)
        guard !sanitized.isEmpty else { throw LayoutError.emptyName }
        return try directory().appendingPathComponent(String(sanitized.prefix(64)) + ".json")
    }

    static func save(_ value: NovaTouchLayout) throws {
        var layout = value
        layout.name = String(value.name.prefix(64))
        let data = try JSONEncoder.pretty.encode(layout)
        try data.write(to: file(layout.name), options: [.atomic])
        UserDefaults.standard.set(layout.name, forKey: currentKey)
    }

    static func names() -> [String] {
        guard let folder = try? directory(),
              let urls = try? FileManager.default.contentsOfDirectory(
                  at: folder, includingPropertiesForKeys: nil)
        else { return [] }
        return urls.filter { $0.pathExtension.lowercased() == "json" }
            .map { $0.deletingPathExtension().lastPathComponent }
            .sorted()
    }

    static func load(_ name: String) -> NovaTouchLayout? {
        guard let url = try? file(name), let data = try? Data(contentsOf: url),
              let layout = try? JSONDecoder().decode(NovaTouchLayout.self, from: data),
              layout.version == 1 else { return nil }
        return layout
    }

    static func current() -> NovaTouchLayout {
        let name = UserDefaults.standard.string(forKey: currentKey) ?? "default"
        if let selected = load(name) ?? load("default") { return selected }
        let starter = NovaTouchLayout.starter
        // Put default.json in the Files-visible folder from the first launch.
        try? save(starter)
        return starter
    }

    enum LayoutError: LocalizedError {
        case emptyName
        var errorDescription: String? { "Enter a layout name." }
    }
}

private extension JSONEncoder {
    static var pretty: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }
}
