import Foundation
import UIKit
import SwiftUI

/// Transparent overlay over winit's existing UIWindow. Empty areas still
/// receive native winit touches; only visible controls intercept a finger.
private final class NovaPassthroughView: UIView {
    var onResize: (() -> Void)?

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        subviews.contains { view in
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
    private var activeButtons: [NovaOverlayButton] = []
    private var layout: NovaTouchLayout = NovaTouchLayouts.current()
    private let textField = UITextField(frame: CGRect(x: 0, y: 0, width: 2, height: 2))
    private var menuSwipe: UISwipeGestureRecognizer?
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
        window.addSubview(overlay)

        let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swipedLeft))
        swipe.direction = .left
        swipe.cancelsTouchesInView = false
        swipe.delaysTouchesBegan = false
        window.addGestureRecognizer(swipe)
        menuSwipe = swipe

        textField.alpha = 0.02
        textField.autocapitalizationType = .none
        textField.autocorrectionType = .no
        textField.spellCheckingType = .no
        textField.delegate = self
        overlay.addSubview(textField)

        let enabled = UserDefaults.standard.object(forKey: "nova.touch.gestures") as? Bool ?? true
        nova_ios_set_touch_gestures(enabled)
        applyLayout()
        showMenu()
        NovaDiagnostics.record("overlay.install.succeeded; layout=\(layout.name)")
    }

    private func applyLayout() {
        activeButtons.forEach { $0.removeFromSuperview() }
        activeButtons.removeAll()
        layout = NovaTouchLayouts.current()

        for control in layout.controls {
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
            if control.kind == "virtualMouse" {
                let pointer = UIImage(named: "NovaPointer")
                    ?? UIImage(systemName: "cursorarrow")
                button.setImage(pointer?.withRenderingMode(.alwaysTemplate), for: .normal)
                button.tintColor = .white
                button.setTitle(nil, for: .normal)
                let pan = UIPanGestureRecognizer(target: self, action: #selector(movePointer(_:)))
                pan.cancelsTouchesInView = false
                button.addGestureRecognizer(pan)
            }
            button.addTarget(self, action: #selector(buttonDown(_:)), for: .touchDown)
            button.addTarget(self, action: #selector(buttonUp(_:)),
                             for: [.touchUpInside, .touchUpOutside, .touchCancel])
            overlay.addSubview(button)
            activeButtons.append(button)
        }
        layoutButtons()
        refreshSwipe()
    }

    private func layoutButtons() {
        let bounds = overlay.bounds
        guard bounds.width > 10, bounds.height > 10 else { return }
        let mouseScale = UserDefaults.standard.double(forKey: "nova.mouse.scale")
        for button in activeButtons {
            let scalar = button.control.kind == "virtualMouse"
                ? (mouseScale == 0 ? 1 : mouseScale) : 1.0
            let side = CGFloat(button.control.size * scalar)
            button.bounds = CGRect(x: 0, y: 0, width: side, height: side)
            button.center = CGPoint(x: bounds.width * button.control.x,
                                    y: bounds.height * button.control.y)
        }
        overlay.bringSubviewToFront(textField)
        if let panel = panelController?.view { overlay.bringSubviewToFront(panel) }
    }

    private func refreshSwipe() {
        let hasMenuKey = layout.controls.contains(where: { $0.kind == "menu" })
        let enabled = UserDefaults.standard.object(forKey: "nova.touch.gestures") as? Bool ?? true
        menuSwipe?.isEnabled = enabled && !hasMenuKey
        nova_ios_set_touch_gestures(enabled)
    }

    private func setControlsVisible(_ visible: Bool) {
        activeButtons.forEach { $0.isHidden = !visible }
        if !visible { textField.resignFirstResponder() }
    }

    private func showMenu() {
        guard !isEditingLayout else { return }
        isInMenu = true
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
        panelController?.view.removeFromSuperview()
        panelController = nil
        isInMenu = false
        isEditingLayout = false
        setControlsVisible(true)
        refreshSwipe()
    }

    private func present<V: View>(_ view: V) {
        panelController?.view.removeFromSuperview()
        panelController = nil
        let controller = UIHostingController(rootView: view)
        controller.view.backgroundColor = .clear
        controller.view.frame = overlay.bounds
        controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        overlay.addSubview(controller.view)
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
        NovaDiagnostics.record("overlay.quit-requested")
        hidePanel()
        // Original Horizon receives CloseRequested and performs its save path.
        nova_ios_input_close()
    }

    @objc private func swipedLeft(_ gesture: UISwipeGestureRecognizer) {
        guard gesture.state == .ended, !isInMenu && !isEditingLayout else { return }
        showMenu()
    }

    @objc private func buttonDown(_ sender: NovaOverlayButton) {
        switch sender.control.kind {
        case "keyboard":
            nova_ios_input_key(NovaTouchLayouts.keyCode(sender.control.key), true)
        case "mouseLeft":
            nova_ios_input_mouse(0, true)
        case "mouseRight":
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

    @objc private func movePointer(_ gesture: UIPanGestureRecognizer) {
        guard let button = gesture.view as? NovaOverlayButton,
              let window = gameWindow else { return }
        let shift = gesture.translation(in: overlay)
        guard shift != .zero else { return }
        let bounds = overlay.bounds
        let x = min(bounds.maxX - 8, max(8, button.center.x + shift.x))
        let y = min(bounds.maxY - 8, max(8, button.center.y + shift.y))
        button.center = CGPoint(x: x, y: y)
        gesture.setTranslation(.zero, in: overlay)

        let pixelScale = Double(window.screen.scale)
        let defaultSensitivity = UserDefaults.standard.double(forKey: "nova.mouse.sensitivity")
        let sensitivity = defaultSensitivity == 0 ? 1.0 : defaultSensitivity
        nova_ios_input_cursor(Double(x) * pixelScale, Double(y) * pixelScale)
        nova_ios_input_motion(Double(shift.x) * pixelScale * sensitivity,
                              Double(shift.y) * pixelScale * sensitivity)
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
                    Text("Swipe left for this menu, unless your layout has a Menu button.")
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
        .confirmationDialog("Close Horizon?", isPresented: $confirmClose) {
            Button("Close Game", role: .destructive, action: onClose)
            Button("Cancel", role: .cancel) {}
        } message: { Text("Horizon will be asked to save and exit.") }
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
