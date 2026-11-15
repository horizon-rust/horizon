import UIKit

/// Left-stick movement. No game-side modifications: its four key states are
/// sent through Nova's existing keyboard mapping bridge, including diagonals.
final class NovaJoystickView: UIView {
    let control: NovaTouchControl

    private let ring = UIView()
    private let thumb = UIView()
    private var held = Set<String>()
    private var activeTouch: UITouch?

    init(control: NovaTouchControl) {
        self.control = control
        super.init(frame: .zero)
        isMultipleTouchEnabled = true
        isExclusiveTouch = false
        backgroundColor = .clear
        ring.isUserInteractionEnabled = false
        ring.layer.borderColor = UIColor.white.withAlphaComponent(0.8).cgColor
        ring.layer.borderWidth = 2
        ring.backgroundColor = UIColor.black.withAlphaComponent(control.opacity * 0.65)
        addSubview(ring)
        thumb.isUserInteractionEnabled = false
        thumb.backgroundColor = UIColor.white.withAlphaComponent(0.8)
        thumb.layer.borderWidth = 2
        thumb.layer.borderColor = UIColor.black.withAlphaComponent(0.5).cgColor
        addSubview(thumb)
    }

    required init?(coder: NSCoder) { fatalError("Use init(control:)") }

    override func layoutSubviews() {
        super.layoutSubviews()
        ring.frame = bounds
        ring.layer.cornerRadius = min(bounds.width, bounds.height) / 2
        let side = min(bounds.width, bounds.height) * 0.37
        thumb.bounds = CGRect(x: 0, y: 0, width: side, height: side)
        thumb.layer.cornerRadius = side / 2
        if activeTouch == nil {
            thumb.center = CGPoint(x: bounds.midX, y: bounds.midY)
        }
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard activeTouch == nil, let touch = touches.first else { return }
        activeTouch = touch
        update(touch.location(in: self))
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = activeTouch, touches.contains(touch) else { return }
        update(touch.location(in: self))
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = activeTouch, touches.contains(touch) else { return }
        releaseAll()
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        releaseAll()
    }

    func releaseAll() {
        activeTouch = nil
        send([])
        thumb.center = CGPoint(x: bounds.midX, y: bounds.midY)
    }

    private func update(_ point: CGPoint) {
        let centre = CGPoint(x: bounds.midX, y: bounds.midY)
        let dx = point.x - centre.x, dy = point.y - centre.y
        let radius = max(1, min(bounds.width, bounds.height) * 0.34)
        let length = hypot(dx, dy)
        let factor = min(1, radius / max(1, length))
        thumb.center = CGPoint(x: centre.x + dx * factor,
                               y: centre.y + dy * factor)

        let nx = dx * factor / radius
        let ny = dy * factor / radius
        let deadzone: CGFloat = 0.27
        var next = Set<String>()
        if ny < -deadzone { next.insert("W") }
        if ny > deadzone { next.insert("S") }
        if nx < -deadzone { next.insert("A") }
        if nx > deadzone { next.insert("D") }
        send(next)
    }

    private func send(_ next: Set<String>) {
        for key in held.subtracting(next) {
            nova_ios_input_key(NovaTouchLayouts.keyCode(key), false)
        }
        for key in next.subtracting(held) {
            nova_ios_input_key(NovaTouchLayouts.keyCode(key), true)
        }
        held = next
    }
}

/// A touch surface for moving a separate desktop-style pointer (menus) or
/// looking around (while in a world). It does not render the cursor itself.
final class NovaTrackpadView: UIView {
    let control: NovaTouchControl
    var onMove: ((CGPoint) -> Void)?
    private var finger: UITouch?

    init(control: NovaTouchControl) {
        self.control = control
        super.init(frame: .zero)
        isMultipleTouchEnabled = false
        backgroundColor = UIColor.black.withAlphaComponent(control.opacity * 0.12)
        layer.borderColor = UIColor.white.withAlphaComponent(0.25).cgColor
        layer.borderWidth = 1
        layer.cornerRadius = 16
        isExclusiveTouch = false
    }

    required init?(coder: NSCoder) { fatalError("Use init(control:)") }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard finger == nil else { return }
        finger = touches.first
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let finger, touches.contains(finger) else { return }
        let here = finger.location(in: self)
        let previous = finger.previousLocation(in: self)
        let delta = CGPoint(x: here.x - previous.x, y: here.y - previous.y)
        if delta != .zero { onMove?(delta) }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        if let finger, touches.contains(finger) { self.finger = nil }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        finger = nil
    }
}
