import Foundation
import AVFoundation
import UIKit

/// Configure the system output session before CPAL opens Horizon's audio device.
enum NovaAudio {
    private static var observers: [NSObjectProtocol] = []

    static func configure() {
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.playback, mode: .default)
            try session.setActive(true)
            NovaDiagnostics.record(
                "audio.session.active; sampleRate=\(Int(session.sampleRate)); outputs=\(session.currentRoute.outputs.count)"
            )
        } catch {
            NovaDiagnostics.record("audio.session.failed: \(error.localizedDescription)")
        }
        if observers.isEmpty {
            observers.append(NotificationCenter.default.addObserver(
                forName: AVAudioSession.interruptionNotification,
                object: session, queue: .main
            ) { notification in
                guard let value = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                      value == AVAudioSession.InterruptionType.ended.rawValue else { return }
                configure()
            })
            observers.append(NotificationCenter.default.addObserver(
                forName: UIApplication.didBecomeActiveNotification,
                object: nil, queue: .main
            ) { _ in configure() })
        }
    }
}
