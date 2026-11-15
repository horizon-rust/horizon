import Foundation
import SwiftUI
import Darwin

/// Exactly ONE system lifecycle per process. SwiftUI initializes UIKit for
/// Nova's launcher, OR winit initializes UIKit for the original Horizon game.
/// We never call winit's UIApplicationMain from an already-running SwiftUI app.
@main
enum NovaEntryPoint {
    private static let nextLaunchKey = "nova.launchHorizonOnce"

    static func main() {
        NovaDiagnostics.startSession()
        NovaDiagnostics.captureConsole()
        let defaults = UserDefaults.standard
        // Once game files are installed, Horizon owns UIKit from app startup.
        // Nova's launcher is then a UIKit overlay over the SAME live game window:
        // Play hides it immediately, without a second UIApplicationMain/restart.
        let shouldStartHorizon = horizon_ios_host_api_version() >= 2 &&
                                 NovaGameFiles.resourcesPresent()
        let info = Bundle.main.infoDictionary ?? [:]
        let build = info["NovaBuildCommit"] as? String ?? "unknown"
        let run = info["NovaBuildRun"] as? String ?? "unknown"
        NovaDiagnostics.record("process.started; boot-request=\(shouldStartHorizon); bridge=\(horizon_ios_host_api_version()); commit=\(build); run=\(run)")
        // Clear before entering Rust: a crash must not cause a boot loop.
        defaults.removeObject(forKey: nextLaunchKey)

        if shouldStartHorizon && horizon_ios_host_api_version() >= 2
            && NovaGameFiles.resourcesPresent() {
            NovaDiagnostics.record("boot.preflight.passed; preparing private resource and game paths")
            do {
                let resources = try NovaGameFiles.resourceDirectory()
                let gameHome = try NovaGameFiles.gameDirectory()
                // The iOS game entry owns UIApplicationMain and does not return
                // during normal operation. Do NOT invoke on a background thread.
                if let diagnosticFile = NovaDiagnostics.logURL() {
                    setenv("NOVA_DIAGNOSTIC_LOG", diagnosticFile.path, 1)
                    NovaDiagnostics.record("boot.runtime-diagnostics-enabled")
                } else {
                    NovaDiagnostics.record("boot.runtime-diagnostics-unavailable")
                }
                NovaAudio.configure()
                NovaGameOverlay.scheduleInstall()
                NovaDiagnostics.record("boot.calling-original-game")
                let code = resources.path.withCString { resourcePath in
                    gameHome.path.withCString { homePath in
                        horizon_ios_start_original_game(resourcePath, homePath)
                    }
                }
                NovaDiagnostics.record("boot.original-game-returned code=\(code)")
                NSLog("Nova: Horizon returned code %d; opening launcher", code)
            } catch {
                NovaDiagnostics.record("boot.preflight.error: \(error.localizedDescription)")
                NSLog("Nova: unable to prepare game boot: %@", error.localizedDescription)
            }
        }

        if !shouldStartHorizon {
            NovaDiagnostics.record("launcher.opening; resources-setup-mode")
        }
        NovaDiagnostics.record("launcher.opening")
        // The launcher remains the default, including after a failed game boot.
        HorizonIOSApp.main()
    }
}
