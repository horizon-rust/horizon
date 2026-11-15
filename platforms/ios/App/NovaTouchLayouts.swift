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
    // Independent width/height percentages. Optional for existing saved JSON.
    var sizeX: Double? = nil
    var sizeY: Double? = nil

    var buttonWidth: Double { size * min(300, max(30, sizeX ?? 100)) / 100 }
    var buttonHeight: Double { size * min(300, max(30, sizeY ?? 100)) / 100 }
    var opacity: Double = 0.7

    static let kinds = ["keyboard", "mouseLeft", "mouseRight",
                        "virtualKeyboard", "joystick", "menu"]

    var symbol: String {
        switch kind {
        case "mouseLeft": return "cursorarrow.click"
        case "mouseRight": return "cursorarrow.rays"
        case "virtualMouse": return "hand.draw"
        case "joystick": return "circle.dotted.circle"
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
            NovaTouchControl(name: "Move", kind: "joystick",
                             x: 0.16, y: 0.73, size: 130, opacity: 0.65),
            key("Jump", "Space", 0.88, 0.77),
            key("E", "E", 0.90, 0.58),
            NovaTouchControl(name: "Attack", kind: "mouseLeft", x: 0.75, y: 0.79),
            NovaTouchControl(name: "Use", kind: "mouseRight", x: 0.79, y: 0.63),
            NovaTouchControl(name: "Type", kind: "virtualKeyboard",
                             x: 0.93, y: 0.14),
            NovaTouchControl(name: "Menu", kind: "menu",
                             x: 0.07, y: 0.12, size: 50)
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
        // Obsolete fixed trackpad pads must not reappear after saving.
        layout.controls.removeAll { $0.kind == "virtualMouse" }
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
        var normalized = layout
        normalized.controls.removeAll { $0.kind == "virtualMouse" }
        return normalized
    }

    static func current() -> NovaTouchLayout {
        let name = UserDefaults.standard.string(forKey: currentKey) ?? "default"
        if let selected = load(name) ?? load("default") {
            // Upgrade only an unchanged, automatically created v1 default.
            // Never overwrite a custom layout or a rearranged control.
            let oldCoordinates: [(String, Double, Double)] = [
                ("W", 0.14, 0.64), ("A", 0.07, 0.76),
                ("S", 0.14, 0.76), ("D", 0.21, 0.76)
            ]
            let isOldStarter = selected.name == "default"
                && selected.controls.count == 9
                && zip(selected.controls.prefix(4), oldCoordinates).allSatisfy { pair in
                    let (button, expected) = pair
                    return button.name == expected.0
                        && abs(button.x - expected.1) < 0.001
                        && abs(button.y - expected.2) < 0.001
                }
            if isOldStarter {
                let upgraded = NovaTouchLayout.starter
                try? save(upgraded)
                return upgraded
            }
            return selected
        }
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
