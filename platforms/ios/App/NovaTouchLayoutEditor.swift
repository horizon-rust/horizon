import SwiftUI
import UIKit

/// The same full-screen layout editor is hosted in the first-run launcher
/// or above the original winit game window. Nothing changes game sources.
struct NovaTouchLayoutEditor: View {
    var onDone: () -> Void
    var onSave: (NovaTouchLayout) -> Void

    @State private var layout = NovaTouchLayouts.current()
    @State private var target: CGPoint = CGPoint(x: 0.5, y: 0.5)
    @State private var canvasOptions = false
    @State private var controlOptions = false
    @State private var selectedID: UUID?
    @State private var editing: NovaTouchControl?
    @State private var savePrompt = false
    @State private var exitPrompt = false
    @State private var loadPrompt = false
    @State private var layoutName = "default"
    @State private var errorMessage: String?

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                NovaBlurBackground(style: .systemUltraThinMaterialLight)
                    .ignoresSafeArea()
                Color.white.opacity(0.28).ignoresSafeArea()

                // The background, not a control, recognizes a long press.
                NovaLongPressCanvas { position in
                    target = CGPoint(x: min(1, max(0, position.x / max(1, geometry.size.width))),
                                     y: min(1, max(0, position.y / max(1, geometry.size.height))))
                    canvasOptions = true
                }

                Text("long press to add button")
                    .font(.system(size: 24, weight: .medium, design: .rounded))
                    .foregroundStyle(.black)
                    .allowsHitTesting(false)

                ForEach(layout.controls) { control in
                    NovaEditingButton(control: control) {
                        selectedID = control.id
                        controlOptions = true
                    } onMove: { translation in
                        // Save one final position. Dragging itself only changes
                        // the local button view: no full-layout diff per frame.
                        guard let index = layout.controls.firstIndex(where: { $0.id == control.id }) else {
                            return
                        }
                        let canvas = geometry.size
                        let proposed = CGPoint(
                            x: control.x * canvas.width + translation.width,
                            y: control.y * canvas.height + translation.height
                        )
                        let snapped = NovaButtonSnap.position(
                            for: control, at: proposed,
                            among: layout.controls, canvas: canvas
                        )
                        layout.controls[index].x = min(0.98, max(0.02,
                            snapped.x / max(1, canvas.width)))
                        layout.controls[index].y = min(0.98, max(0.02,
                            snapped.y / max(1, canvas.height)))
                    }
                    .position(x: control.x * geometry.size.width,
                              y: control.y * geometry.size.height)
                }

                VStack {
                    HStack {
                        Text("Touch Layout • \(layout.name)")
                            .font(.headline)
                            .foregroundStyle(.black)
                        Spacer()
                        Button("Save") {
                            layoutName = layout.name
                            savePrompt = true
                        }
                        .buttonStyle(.borderedProminent)
                        Button("Exit") { exitPrompt = true }
                            .buttonStyle(.bordered)
                    }
                    .padding(18)
                    Spacer()
                }
            }
            .contentShape(Rectangle())
        }
        .confirmationDialog("Touch layout", isPresented: $canvasOptions) {
            Button("Save") {
                layoutName = layout.name
                savePrompt = true
            }
            Button("Exit Without Saving", role: .destructive) { exitPrompt = true }
            Button("Add") {
                layout.controls.append(
                    NovaTouchControl(x: target.x, y: target.y)
                )
            }
            Button("Load") { loadPrompt = true }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Button", isPresented: $controlOptions) {
            Button("Options") {
                editing = layout.controls.first(where: { $0.id == selectedID })
            }
            Button("Delete", role: .destructive) {
                layout.controls.removeAll(where: { $0.id == selectedID })
            }
            Button("Copy") {
                guard var copy = layout.controls.first(where: { $0.id == selectedID }) else {
                    return
                }
                copy.id = UUID()
                copy.x = min(0.95, copy.x + 0.05)
                copy.y = min(0.95, copy.y + 0.05)
                layout.controls.append(copy)
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(item: $editing) { control in
            NovaControlOptions(control: control) { updated in
                if let index = layout.controls.firstIndex(where: { $0.id == updated.id }) {
                    layout.controls[index] = updated
                }
                editing = nil
            } onCancel: {
                editing = nil
            }
        }
        .alert("Save Layout", isPresented: $savePrompt) {
            TextField("Layout name", text: $layoutName)
            Button("Save") {
                do {
                    layout.name = layoutName.trimmingCharacters(in: .whitespacesAndNewlines)
                    try NovaTouchLayouts.save(layout)
                    onSave(layout)
                    onDone()
                } catch { errorMessage = error.localizedDescription }
            }
            Button("Cancel", role: .cancel) {}
        } message: { Text("Save a JSON file in Files → Nova → TouchLayouts.") }
        .alert("Exit without saving?", isPresented: $exitPrompt) {
            Button("Yes, Exit", role: .destructive) { onDone() }
            Button("Cancel", role: .cancel) {}
        } message: { Text("Unsaved button changes will be lost.") }
        .alert("Layout error", isPresented: Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK") { errorMessage = nil }
        } message: { Text(errorMessage ?? "") }
        .sheet(isPresented: $loadPrompt) {
            NavigationStack {
                List(NovaTouchLayouts.names(), id: \.self) { name in
                    Button(name) {
                        if let loaded = NovaTouchLayouts.load(name) {
                            layout = loaded
                        }
                        loadPrompt = false
                    }
                }
                .navigationTitle("Load Touch Layout")
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { loadPrompt = false }
                    }
                }
            }
        }
    }
}

/// Lightweight edge snapping performed once at drag end, never per frame.
/// Works for different button widths/heights and all four neighboring sides.
private enum NovaButtonSnap {
    static func size(_ control: NovaTouchControl) -> CGSize {
        CGSize(
            width: control.kind == "joystick" ? max(100, control.buttonWidth) : control.buttonWidth,
            height: control.kind == "joystick" ? max(100, control.buttonHeight) : control.buttonHeight
        )
    }

    static func position(for moving: NovaTouchControl, at point: CGPoint,
                         among controls: [NovaTouchControl], canvas: CGSize) -> CGPoint {
        let movingSize = size(moving)
        let threshold: CGFloat = 20
        let alignment: CGFloat = 16
        var best: (point: CGPoint, score: CGFloat)?

        for neighbor in controls where neighbor.id != moving.id {
            let otherSize = size(neighbor)
            let center = CGPoint(x: neighbor.x * canvas.width, y: neighbor.y * canvas.height)
            let halfWidth = (movingSize.width + otherSize.width) / 2
            let halfHeight = (movingSize.height + otherSize.height) / 2
            let dx = point.x - center.x
            let dy = point.y - center.y

            // A significant intentional overlap is not snapped away.
            // The orthogonal axis must overlap for these to be neighbors.
            let horizontalGap = abs(dx) - halfWidth
            if horizontalGap >= -6 && horizontalGap <= threshold
                && abs(dy) < halfHeight {
                let edgeY = [
                    center.y,
                    center.y + (otherSize.height - movingSize.height) / 2,
                    center.y - (otherSize.height - movingSize.height) / 2
                ]
                let alignedY = edgeY.min(by: { abs($0 - point.y) < abs($1 - point.y) })!
                let newY = abs(alignedY - point.y) <= alignment ? alignedY : point.y
                let candidate = CGPoint(
                    x: center.x + (dx >= 0 ? halfWidth : -halfWidth), y: newY
                )
                let score = abs(horizontalGap) + abs(newY - point.y) * 0.3
                if best == nil || score < best!.score {
                    best = (candidate, score)
                }
            }

            let verticalGap = abs(dy) - halfHeight
            if verticalGap >= -6 && verticalGap <= threshold
                && abs(dx) < halfWidth {
                let edgeX = [
                    center.x,
                    center.x + (otherSize.width - movingSize.width) / 2,
                    center.x - (otherSize.width - movingSize.width) / 2
                ]
                let alignedX = edgeX.min(by: { abs($0 - point.x) < abs($1 - point.x) })!
                let newX = abs(alignedX - point.x) <= alignment ? alignedX : point.x
                let candidate = CGPoint(
                    x: newX, y: center.y + (dy >= 0 ? halfHeight : -halfHeight)
                )
                let score = abs(verticalGap) + abs(newX - point.x) * 0.3
                if best == nil || score < best!.score {
                    best = (candidate, score)
                }
            }
        }
        return best?.point ?? point
    }
}

private struct NovaEditingButton: View {
    let control: NovaTouchControl
    let onTap: () -> Void
    let onMove: (CGSize) -> Void
    @GestureState private var dragOffset = CGSize.zero

    var body: some View {
        Text(control.name)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(.white)
            .lineLimit(1)
            .frame(width: control.kind == "joystick"
                            ? max(100, control.buttonWidth) : control.buttonWidth,
                   height: control.kind == "joystick"
                            ? max(100, control.buttonHeight) : control.buttonHeight)
            .background(.black.opacity(control.opacity), in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14)
                .strokeBorder(.white.opacity(0.85), lineWidth: 1.5))
            .offset(dragOffset)
            .onTapGesture(perform: onTap)
            .gesture(DragGesture(minimumDistance: 5)
                .updating($dragOffset) { value, state, _ in
                    state = value.translation
                }
                .onEnded { value in
                    onMove(value.translation)
                })
    }
}

private struct NovaControlOptions: View {
    @State var control: NovaTouchControl
    var onSave: (NovaTouchControl) -> Void
    var onCancel: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                TextField("Button name", text: $control.name)
                    .autocorrectionDisabled()
                Picker("Key type", selection: $control.kind) {
                    Text("Keyboard key").tag("keyboard")
                    Text("Mouse left").tag("mouseLeft")
                    Text("Mouse right").tag("mouseRight")
                    Text("Movement joystick").tag("joystick")
                    Text("Virtual keyboard").tag("virtualKeyboard")
                    Text("Menu").tag("menu")
                }
                if control.kind == "keyboard" {
                    Picker("Keyboard key", selection: $control.key) {
                        ForEach(NovaTouchLayouts.keyboardKeys, id: \.self) {
                            Text($0).tag($0)
                        }
                    }
                }
                VStack(alignment: .leading) {
                    Text("Opacity: \(Int(control.opacity * 100))%")
                    Slider(value: $control.opacity, in: 0.1...1.0)
                }
                Section("Button Size") {
                    Text("Set width (X) and height (Y) independently, from 30% to 300%.")
                        .font(.footnote)
                    HStack {
                        Text("X width (%)")
                        Spacer()
                        TextField("100", value: Binding(
                            get: { control.sizeX ?? 100 },
                            set: { control.sizeX = min(300, max(30, $0)) }
                        ), format: .number)
                        .keyboardType(.decimalPad)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                    }
                    HStack {
                        Text("Y height (%)")
                        Spacer()
                        TextField("100", value: Binding(
                            get: { control.sizeY ?? 100 },
                            set: { control.sizeY = min(300, max(30, $0)) }
                        ), format: .number)
                        .keyboardType(.decimalPad)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                    }
                    Text("100% keeps the original size (\(Int(control.size)) pt).")
                        .font(.footnote)
                }
            }
            .navigationTitle("Button Options")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        control.name = String(control.name.prefix(32))
                        onSave(control)
                    }
                }
            }
        }
    }
}

private struct NovaLongPressCanvas: UIViewRepresentable {
    let onLongPress: (CGPoint) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(onLongPress: onLongPress) }
    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.backgroundColor = .clear
        let press = UILongPressGestureRecognizer(
            target: context.coordinator,
            action: #selector(Coordinator.held(_:))
        )
        press.minimumPressDuration = 0.5
        view.addGestureRecognizer(press)
        return view
    }
    func updateUIView(_ view: UIView, context: Context) {
        context.coordinator.onLongPress = onLongPress
    }
    final class Coordinator: NSObject {
        var onLongPress: (CGPoint) -> Void
        init(onLongPress: @escaping (CGPoint) -> Void) {
            self.onLongPress = onLongPress
        }
        @objc func held(_ recognizer: UILongPressGestureRecognizer) {
            guard recognizer.state == .began, let view = recognizer.view else { return }
            onLongPress(recognizer.location(in: view))
        }
    }
}

struct NovaBlurBackground: UIViewRepresentable {
    var style: UIBlurEffect.Style
    func makeUIView(context: Context) -> UIVisualEffectView {
        UIVisualEffectView(effect: UIBlurEffect(style: style))
    }
    func updateUIView(_ view: UIVisualEffectView, context: Context) {
        view.effect = UIBlurEffect(style: style)
    }
}
