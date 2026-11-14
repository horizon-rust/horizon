import MetalKit
import SwiftUI

/// iOS-owned Metal drawable. This is a *host graphics diagnostic*, not
/// Horizon's renderer. The upstream wgpu surface is not connected yet.
struct GameSurfaceView: UIViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> MTKView {
        let view = MTKView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        view.colorPixelFormat = .bgra8Unorm
        view.framebufferOnly = false
        view.clearColor = MTLClearColor(red: 0.07, green: 0.19, blue: 0.28, alpha: 1)
        view.isPaused = false
        view.enableSetNeedsDisplay = false
        view.preferredFramesPerSecond = 30
        view.delegate = context.coordinator
        context.coordinator.queue = view.device?.makeCommandQueue()
        horizon_ios_set_metal_view(Unmanaged.passUnretained(view).toOpaque())
        return view
    }

    func updateUIView(_ uiView: MTKView, context: Context) {
        // The game runtime will eventually consume this surface through a
        // proper native wgpu/winit interface. No gameplay is started here.
    }

    static func dismantleUIView(_ uiView: MTKView, coordinator: Coordinator) {
        uiView.delegate = nil
        horizon_ios_set_metal_view(nil)
        coordinator.queue = nil
    }

    final class Coordinator: NSObject, MTKViewDelegate {
        var queue: MTLCommandQueue?

        func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

        func draw(in view: MTKView) {
            guard let descriptor = view.currentRenderPassDescriptor,
                  let drawable = view.currentDrawable,
                  let buffer = queue?.makeCommandBuffer(),
                  let encoder = buffer.makeRenderCommandEncoder(descriptor: descriptor)
            else { return }
            encoder.endEncoding()
            buffer.present(drawable)
            buffer.commit()
        }
    }
}
