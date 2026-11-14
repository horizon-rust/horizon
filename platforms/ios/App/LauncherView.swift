import SwiftUI

/// This is only the iOS host. The original Horizon game will be linked here
/// once the iOS window/event-loop integration is ready.
struct LauncherView: View {
    @State private var launchError: String?
    private let gameAvailable = horizon_ios_game_is_ready()

    var body: some View {
        ZStack {
            Color(red: 0.06, green: 0.09, blue: 0.13)
                .ignoresSafeArea()

            VStack(spacing: 18) {
                Text("Horizon")
                    .font(.system(size: 46, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)

                Text("iOS / iPadOS")
                    .font(.headline)
                    .foregroundStyle(.white.opacity(0.75))

                Button {
                    let result = horizon_ios_launch_game()
                    if result != 0 {
                        launchError = "Horizon runtime is not connected yet (code \(result))."
                    }
                } label: {
                    Text("Play")
                        .font(.headline)
                        .frame(minWidth: 180, minHeight: 48)
                }
                .buttonStyle(.borderedProminent)
                .disabled(!gameAvailable)
                .accessibilityHint(gameAvailable ? "Launch Horizon" : "Game runtime integration is not available yet")

                if let launchError {
                    Text(launchError)
                        .foregroundStyle(.orange)
                } else if !gameAvailable {
                    Text("Native host ready. Game integration is the next milestone.")
                        .foregroundStyle(.white.opacity(0.7))
                        .multilineTextAlignment(.center)
                }
                Text("Host bridge API v\(horizon_ios_host_api_version())")
                    .font(.footnote.monospaced())
                    .foregroundStyle(.white.opacity(0.5))
            }
            .padding(28)
        }
    }
}
