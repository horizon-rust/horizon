import MetalKit
import SwiftUI

/// An iOS-owned Metal drawable for the future upstream wgpu surface.
/// Not shown by the launcher until the original game has been connected.
struct GameSurfaceView: UIViewRepresentable {
    func makeUIView(context: Context) -> MTKView {
        let view = MTKView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        view.colorPixelFormat = .bgra8Unorm
        view.framebufferOnly = false
        view.isPaused = true
        view.enableSetNeedsDisplay = false
        return view
    }

    func updateUIView(_ uiView: MTKView, context: Context) {
        // Horizon will own frame presentation after the native game is linked.
    }
}
