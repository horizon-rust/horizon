import Foundation
import UIKit
import SwiftUI
import Darwin

/// Transparent overlay over winit's existing UIWindow. Empty areas still
/// receive native winit touches; only visible controls intercept a finger.
private final class NovaPassthroughView: UIView {
    var onResize: (() -> Void)?
    var blocksGameplay = false

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        if blocksGameplay { return bounds.contains(point) }
        return subviews.contains { view in
            !view.isHidden && view.alpha > 0.01 && view.isUserInteractionEnabled &&
                view.point(inside: view.convert(point, from: self), with: event)
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        onResize?()
    }
}

private final class NovaOverlayButton: UIButton {
    var control: NovaTouchControl
    init(control: NovaTouchControl) {
        self.control = control
        super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("Use init(control:)") }
}

/// The game still owns UIApplicationMain, Metal, the window and its event loop.
/// Nova only adds UIKit controls and SwiftUI sheets to that existing window.
final class NovaGameOverlay: NSObject, UITextFieldDelegate {
    static let shared = NovaGameOverlay()

    private weak var gameWindow: UIWindow?
    private let overlay = NovaPassthroughView()
    private var panelController: UIViewController?
    private var activeControls: [(view: UIView, control: NovaTouchControl)] = []
    private let cursorView = UIImageView(image: UIImage(named: "NovaPointer")
                                        ?? UIImage(systemName: "cursorarrow"))
    private var cursorPoint: CGPoint?
    private var cursorVisibilityTimer: Timer?
    private var layout: NovaTouchLayout = NovaTouchLayouts.current()
    private let textField = UITextField(frame: CGRect(x: 0, y: 0, width: 2, height: 2))
    private var isEditingLayout = false
    private var isInMenu = false
    private var firstAppearance = true

    /// Run before Horizon enters winit; wait until winit creates its UIWindow.
    static func scheduleInstall() {
        DispatchQueue.main.async { shared.attachWhenWindowExists(remaining: 300) }
    }

    private func attachWhenWindowExists(remaining: Int) {
        guard gameWindow == nil else { return }
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap { $0.windows }
        if let window = windows.first(where: {
            !$0.isHidden && $0.bounds.width > 10 && $0.bounds.height > 10 &&
                $0.rootViewController != nil
        }) {
            install(on: window)
        } else if remaining > 0 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
                self.attachWhenWindowExists(remaining: remaining - 1)
            }
        } else {
            NovaDiagnostics.record("overlay.install.failed; window-not-found")
        }
    }

    private func install(on window: UIWindow) {
        gameWindow = window
        overlay.frame = window.bounds
        overlay.backgroundColor = .clear
        overlay.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        overlay.onResize = { [weak self] in self?.layoutButtons() }
        // Keep the overlay in the winit view-controller hierarchy, above its
        // Metal-backed view, so SwiftUI sheets and alerts can be presented.
        window.rootViewController?.view.addSubview(overlay)

        textField.alpha = 0.02
        textField.autocapitalizationType = .none
        textField.autocorrectionType = .no
        textField.spellCheckingType = .no
        textField.delegate = self
        overlay.addSubview(textField)
        // A PC-style mouse pointer is independent of touch controls. It
        // never receives taps, and left/right buttons use its position.
        cursorView.isUserInteractionEnabled = false
        cursorView.contentMode = .scaleAspectFit
        overlay.addSubview(cursorView)

        let enabled = UserDefaults.standard.object(forKey: "nova.touch.gestures") as? Bool ?? true
        nova_ios_set_touch_gestures(enabled)
        applyLayout()
        showMenu()
        cursorVisibilityTimer = Timer.scheduledTimer(withTimeInterval: 0.15,
                                                     repeats: true) { [weak self] _ in
            self?.updateCursorVisibility()
        }
        NovaDiagnostics.record("overlay.install.succeeded; layout=\(layout.name)")
    }

    private func applyLayout() {
        activeControls.forEach { $0.view.removeFromSuperview() }
        activeControls.removeAll()
        layout = NovaTouchLayouts.current()
        var controls = layout.controls
        // Older layouts can always open the menu without a swipe.
        if !controls.contains(where: { $0.kind == "menu" }) {
            controls.append(NovaTouchControl(name: "Menu", kind: "menu",
                                             x: 0.07, y: 0.12, size: 50,
                                             opacity: 0.8))
        }

        for control in controls {
            if control.kind == "joystick" {
                let joystick = NovaJoystickView(control: control)
                overlay.addSubview(joystick)
                activeControls.append((joystick, control))
                continue
            }
            if control.kind == "virtualMouse" {
                let pad = NovaTrackpadView(control: control)
                pad.onMove = { [weak self] delta in
                    self?.moveCursor(by: delta)
                }
                overlay.addSubview(pad)
                activeControls.append((pad, control))
                continue
            }
            let button = NovaOverlayButton(control: control)
            button.isExclusiveTouch = false
            button.backgroundColor = UIColor.black.withAlphaComponent(control.opacity)
            button.layer.cornerRadius = 12
            button.layer.borderColor = UIColor.white.withAlphaComponent(0.75).cgColor
            button.layer.borderWidth = 1
            button.setTitle(control.name, for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: 12, weight: .semibold)
            button.titleLabel?.adjustsFontSizeToFitWidth = true
            button.titleLabel?.minimumScaleFactor = 0.55
            button.addTarget(self, action: #selector(buttonDown(_:)), for: .touchDown)
            button.addTarget(self, action: #selector(buttonUp(_:)),
                             for: [.touchUpInside, .touchUpOutside, .touchCancel])
            overlay.addSubview(button)
            activeControls.append((button, control))
        }
        layoutButtons()
        refreshTouchGestures()
    }

    private func layoutButtons() {
        let bounds = overlay.bounds
        guard bounds.width > 10, bounds.height > 10 else { return }
        for (view, control) in activeControls {
            let side: CGFloat
            if control.kind == "joystick" {
                side = max(110, CGFloat(control.size))
            } else if control.kind == "virtualMouse" {
                side = max(112, CGFloat(control.size))
            } else {
                side = CGFloat(control.size)
            }
            view.bounds = CGRect(x: 0, y: 0, width: side, height: side)
            view.center = CGPoint(x: bounds.width * control.x,
                                  y: bounds.height * control.y)
        }
        if cursorPoint == nil {
            cursorPoint = CGPoint(x: bounds.midX, y: bounds.midY)
        }
        redrawCursor()
        overlay.bringSubviewToFront(textField)
        overlay.bringSubviewToFront(cursorView)
        if let panel = panelController?.view { overlay.bringSubviewToFront(panel) }
    }

    private func redrawCursor() {
        let scale = UserDefaults.standard.double(forKey: "nova.mouse.scale")
        let factor = CGFloat(scale == 0 ? 1.0 : scale)
        let location = cursorPoint ?? CGPoint(x: overlay.bounds.midX, y: overlay.bounds.midY)
        // Image is the provided pointer PNG, with its click hotspot at top left.
        cursorView.frame = CGRect(x: location.x, y: location.y,
                                  width: 24 * factor, height: 36 * factor)
        updateCursorVisibility()
    }

    private func updateCursorVisibility() {
        // Desktop-style cursor in menus, hidden while the game has its mouse
        // captured for first-person looking; Horizon draws the crosshair.
        cursorView.isHidden = isInMenu || nova_ios_is_playing()
    }

    private func moveCursor(by delta: CGPoint) {
        guard !isInMenu, let window = gameWindow else { return }
        let saved = UserDefaults.standard.double(forKey: "nova.mouse.sensitivity")
        let sensitivity = CGFloat(saved == 0 ? 1 : saved)
        let xShift = delta.x * sensitivity, yShift = delta.y * sensitivity
        let prev = cursorPoint ?? CGPoint(x: overlay.bounds.midX, y: overlay.bounds.midY)
        cursorPoint = CGPoint(
            x: min(overlay.bounds.maxX - 8, max(0, prev.x + xShift)),
            y: min(overlay.bounds.maxY - 8, max(0, prev.y + yShift))
        )
        redrawCursor()
        let pixelScale = Double(window.screen.scale)
        let cursor = cursorPoint ?? prev
        nova_ios_input_cursor(Double(cursor.x) * pixelScale,
                              Double(cursor.y) * pixelScale)
        // Gameplay uses relative motion, whereas GUIs use absolute cursor.
        nova_ios_input_motion(Double(xShift) * pixelScale,
                              Double(yShift) * pixelScale)
    }

    private func refreshTouchGestures() {
        let enabled = UserDefaults.standard.object(forKey: "nova.touch.gestures") as? Bool ?? true
        nova_ios_set_touch_gestures(enabled)
    }

    private func setControlsVisible(_ visible: Bool) {
        activeControls.forEach { $0.view.isHidden = !visible }
        if !visible {
            for item in activeControls {
                (item.view as? NovaJoystickView)?.releaseAll()
            }
        }
        cursorView.isHidden = !visible
        if !visible { textField.resignFirstResponder() }
    }

    private func showMenu() {
        guard !isEditingLayout else { return }
        isInMenu = true
        overlay.blocksGameplay = true
        nova_ios_set_overlay_active(true)
        setControlsVisible(false)
        let view = NovaGameMenuView(
            isFirstAppearance: firstAppearance,
            onPlay: { [weak self] in self?.hidePanel() },
            onEdit: { [weak self] in self?.showEditor() },
            onSettings: { [weak self] in self?.showSettings() },
            onLogs: { [weak self] in self?.showLogs() },
            onClose: { [weak self] in self?.requestClose() }
        )
        firstAppearance = false
        present(view)
    }

    private func hidePanel() {
        removePanel()
        isInMenu = false
        isEditingLayout = false
        overlay.blocksGameplay = false
        nova_ios_set_overlay_active(false)
        setControlsVisible(true)
        refreshTouchGestures()
    }

    private func removePanel() {
        guard let controller = panelController else { return }
        controller.willMove(toParent: nil)
        controller.view.removeFromSuperview()
        controller.removeFromParent()
        panelController = nil
    }

    private func present<V: View>(_ view: V) {
        removePanel()
        let controller = UIHostingController(rootView: view)
        controller.view.backgroundColor = .clear
        controller.view.frame = overlay.bounds
        controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        if let parent = gameWindow?.rootViewController {
            parent.addChild(controller)
            overlay.addSubview(controller.view)
            controller.didMove(toParent: parent)
        } else {
            overlay.addSubview(controller.view)
        }
        panelController = controller
    }

    private func showEditor() {
        isEditingLayout = true
        setControlsVisible(false)
        present(NovaTouchLayoutEditor(
            onDone: { [weak self] in
                self?.isEditingLayout = false
                self?.showMenu()
            },
            onSave: { [weak self] updated in
                self?.layout = updated
                self?.applyLayout()
            }
        ))
    }

    private func showSettings() {
        present(NovaTouchSettingsView(onDone: { [weak self] in
            self?.applyLayout()
            self?.showMenu()
        }))
    }

    private func showLogs() {
        present(NovaGameLogsView(onDone: { [weak self] in self?.showMenu() }))
    }

    private func requestClose() {
        NovaDiagnostics.record("overlay.force-quit")
        // Force Close is immediate and does not imply a save.
        Darwin.exit(EXIT_SUCCESS)
    }

    @objc private func buttonDown(_ sender: NovaOverlayButton) {
        switch sender.control.kind {
        case "keyboard":
            nova_ios_input_key(NovaTouchLayouts.keyCode(sender.control.key), true)
        case "mouseLeft":
            sendCurrentCursor()
            nova_ios_input_mouse(0, true)
        case "mouseRight":
            sendCurrentCursor()
            nova_ios_input_mouse(1, true)
        case "virtualKeyboard":
            textField.becomeFirstResponder()
        case "menu":
            if isInMenu { hidePanel() } else { showMenu() }
        default:
            break
        }
    }

    @objc private func buttonUp(_ sender: NovaOverlayButton) {
        switch sender.control.kind {
        case "keyboard":
            nova_ios_input_key(NovaTouchLayouts.keyCode(sender.control.key), false)
        case "mouseLeft":
            nova_ios_input_mouse(0, false)
        case "mouseRight":
            nova_ios_input_mouse(1, false)
        default:
            break
        }
    }

    private func sendCurrentCursor() {
        guard let window = gameWindow else { return }
        let point = cursorPoint ?? CGPoint(x: overlay.bounds.midX, y: overlay.bounds.midY)
        let scale = Double(window.screen.scale)
        nova_ios_input_cursor(Double(point.x) * scale, Double(point.y) * scale)
    }

    func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange,
                   replacementString string: String) -> Bool {
        if string.isEmpty {
            nova_ios_input_key(NovaTouchLayouts.keyCode("Backspace"), true)
            nova_ios_input_key(NovaTouchLayouts.keyCode("Backspace"), false)
        } else {
            string.withCString { nova_ios_input_text($0) }
        }
        return false
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        nova_ios_input_key(NovaTouchLayouts.keyCode("Enter"), true)
        nova_ios_input_key(NovaTouchLayouts.keyCode("Enter"), false)
        textField.resignFirstResponder()
        return false
    }
}

private struct NovaGameMenuView: View {
    let isFirstAppearance: Bool
    let onPlay: () -> Void
    let onEdit: () -> Void
    let onSettings: () -> Void
    let onLogs: () -> Void
    let onClose: () -> Void
    @State private var confirmClose = false

    var body: some View {
        ZStack {
            NovaBlurBackground(style: .systemMaterialDark).ignoresSafeArea()
            Color.black.opacity(0.5).ignoresSafeArea()
            ScrollView {
                VStack(spacing: 16) {
                    Text("Nova").font(.system(size: 46, weight: .bold, design: .rounded))
                    Text("Horizon for iOS / iPadOS")
                        .font(.subheadline)
                        .foregroundStyle(.white.opacity(0.7))
                    Button(isFirstAppearance ? "Play" : "Return to Game", action: onPlay)
                        .buttonStyle(.borderedProminent)
                    Button("Touch Layout", action: onEdit).buttonStyle(.bordered)
                    Button("Mouse & Touch Settings", action: onSettings).buttonStyle(.bordered)
                    Button("Show Logs", action: onLogs).buttonStyle(.bordered)
                    Button("Force Close") { confirmClose = true }
                        .buttonStyle(.bordered)
                        .tint(.red)
                    Text("Open Nova using your Menu touch button.")
                        .font(.caption)
                        .foregroundStyle(.white.opacity(0.6))
                }
                .font(.headline)
                .frame(maxWidth: 480)
                .frame(maxWidth: .infinity)
                .padding(32)
            }
        }
        .foregroundStyle(.white)
        .confirmationDialog("Force close Nova?", isPresented: $confirmClose) {
            Button("Force Close App", role: .destructive, action: onClose)
            Button("Cancel", role: .cancel) {}
        } message: { Text("The entire app exits immediately. Unsaved world changes can be lost.") }
    }
}

private struct NovaTouchSettingsView: View {
    let onDone: () -> Void
    @State private var mouseScale =
        UserDefaults.standard.double(forKey: "nova.mouse.scale") == 0
        ? 1.0 : UserDefaults.standard.double(forKey: "nova.mouse.scale")
    @State private var sensitivity =
        UserDefaults.standard.double(forKey: "nova.mouse.sensitivity") == 0
        ? 1.0 : UserDefaults.standard.double(forKey: "nova.mouse.sensitivity")
    @State private var touchGestures =
        UserDefaults.standard.object(forKey: "nova.touch.gestures") as? Bool ?? true

    var body: some View {
        NavigationStack {
            Form {
                Section("Virtual Mouse") {
                    Text("Mouse scale: \(mouseScale, specifier: "%.2f")×")
                    Slider(value: $mouseScale, in: 0.5...3.0)
                    Text("Mouse sensitivity: \(sensitivity, specifier: "%.2f")×")
                    Slider(value: $sensitivity, in: 0.2...4.0)
                }
                Section("Touch") {
                    Toggle("Touch gestures", isOn: $touchGestures)
                    Text("Disable to ignore direct screen taps and swipes in Horizon. Touch buttons remain available.")
                        .font(.footnote)
                }
                Section("Layouts") {
                    Text("JSON layouts are in Files → On My iPad → Nova → TouchLayouts.")
                        .font(.footnote)
                }
            }
            .navigationTitle("Nova Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        UserDefaults.standard.set(mouseScale, forKey: "nova.mouse.scale")
                        UserDefaults.standard.set(sensitivity, forKey: "nova.mouse.sensitivity")
                        UserDefaults.standard.set(touchGestures, forKey: "nova.touch.gestures")
                        nova_ios_set_touch_gestures(touchGestures)
                        onDone()
                    }
                }
            }
        }
    }
}

private struct NovaGameLogsView: View {
    let onDone: () -> Void
    @State private var logs = NovaDiagnostics.tail()
    var body: some View {
        NavigationStack {
            ScrollView {
                Text(logs)
                    .font(.system(size: 12, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .navigationTitle("Nova Logs")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: onDone)
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh") { logs = NovaDiagnostics.tail() }
                }
            }
        }
    }
}
