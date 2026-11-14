import Foundation
import SwiftUI

/// Exactly ONE system lifecycle per process. SwiftUI initializes UIKit for
/// Nova's launcher, OR winit initializes UIKit for the original Horizon game.
/// We never call winit's UIApplicationMain from an already-running SwiftUI app.
@main
enum NovaEntryPoint {
    private static let nextLaunchKey = "nova.launchHorizonOnce"

    static func main() {
        let defaults = UserDefaults.standard
        let shouldStartHorizon = defaults.bool(forKey: nextLaunchKey)
        // Clear before entering Rust: a crash must not cause a boot loop.
        defaults.removeObject(forKey: nextLaunchKey)

        if shouldStartHorizon && horizon_ios_host_api_version() >= 2
            && NovaGameFiles.resourcesPresent() {
            do {
                let resources = try NovaGameFiles.resourceDirectory()
                let gameHome = try NovaGameFiles.gameDirectory()
                // The iOS game entry owns UIApplicationMain and does not return
                // during normal operation. Do NOT invoke on a background thread.
                let code = resources.path.withCString { resourcePath in
                    gameHome.path.withCString { homePath in
                        horizon_ios_start_original_game(resourcePath, homePath)
                    }
                }
                NSLog("Nova: Horizon returned code %d; opening launcher", code)
            } catch {
                NSLog("Nova: unable to prepare game boot: %@", error.localizedDescription)
            }
        }

        // The launcher remains the default, including after a failed game boot.
        HorizonIOSApp.main()
    }
}
