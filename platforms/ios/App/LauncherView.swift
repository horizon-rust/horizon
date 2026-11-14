import SwiftUI
import UniformTypeIdentifiers

/// Nova is the thin iOS host for the ORIGINAL Horizon game.
/// The gameplay runtime is linked in the integrated build but cannot yet
/// own an iOS event loop; do not invoke its desktop-style entry from SwiftUI.
struct LauncherView: View {
    @State private var showArchivePicker = false
    @State private var archiveName: String? = NovaGameFiles.stagedArchiveName()
    @State private var archiveMessage: String?
    @State private var importInProgress = false

    private let gameAvailable = horizon_ios_game_is_ready()

    var body: some View {
        ZStack {
            Color(red: 0.06, green: 0.09, blue: 0.13)
                .ignoresSafeArea()

            ScrollView {
                VStack(spacing: 18) {
                    Text("Nova")
                        .font(.system(size: 46, weight: .bold, design: .rounded))
                        .foregroundStyle(.white)
                        .accessibilityAddTraits(.isHeader)

                    Text("Original Horizon for iOS / iPadOS")
                        .font(.headline)
                        .foregroundStyle(.white.opacity(0.75))

                    Button("Play") {
                        // Guarded even if the UI state somehow gets out of sync.
                        guard horizon_ios_game_is_ready() else { return }
                        _ = horizon_ios_launch_game()
                    }
                    .font(.headline)
                    .frame(minWidth: 180, minHeight: 48)
                    .buttonStyle(.borderedProminent)
                    .disabled(!gameAvailable)
                    .accessibilityHint(gameAvailable
                        ? "Launch the original Horizon game"
                        : "The iOS game event loop is not integrated yet")

                    if !gameAvailable {
                        Text("Nova host and original Horizon library are linked. Game startup and Metal window integration are still in development.")
                            .foregroundStyle(.white.opacity(0.75))
                            .multilineTextAlignment(.center)
                    }

                    Divider().overlay(.white.opacity(0.2))

                    Text("Game files")
                        .font(.title3.bold())
                        .foregroundStyle(.white)

                    Text(archiveName.map { "Stored archive: \($0)" }
                         ?? "You can import your previous launcher's ZIP archive for inspection.")
                        .foregroundStyle(.white.opacity(0.75))
                        .multilineTextAlignment(.center)

                    Button(importInProgress ? "Importing…" : "Import game files ZIP") {
                        showArchivePicker = true
                    }
                    .buttonStyle(.bordered)
                    .disabled(importInProgress)

                    Text("Import only stores the ZIP in Nova's Documents folder. It does not extract, validate game versions, or enable Play.")
                        .font(.footnote)
                        .foregroundStyle(.white.opacity(0.55))
                        .multilineTextAlignment(.center)

                    if let archiveMessage {
                        Text(archiveMessage)
                            .font(.footnote)
                            .foregroundStyle(.orange)
                            .multilineTextAlignment(.center)
                    }

                    Text("Host bridge API v\(horizon_ios_host_api_version())")
                        .font(.footnote.monospaced())
                        .foregroundStyle(.white.opacity(0.5))
                }
                .frame(maxWidth: 540)
                .padding(28)
                .frame(maxWidth: .infinity)
            }
        }
        .fileImporter(
            isPresented: $showArchivePicker,
            allowedContentTypes: [.zip],
            allowsMultipleSelection: false
        ) { result in
            switch result {
            case .success(let urls):
                guard let url = urls.first else { return }
                importInProgress = true
                DispatchQueue.global(qos: .userInitiated).async {
                    let result = Result { try NovaGameFiles.stageZip(from: url) }
                    DispatchQueue.main.async {
                        importInProgress = false
                        switch result {
                        case .success(let name):
                            archiveName = name
                            archiveMessage = "Archive stored for later compatibility checking."
                        case .failure(let error):
                            archiveMessage = "Could not import ZIP: \(error.localizedDescription)"
                        }
                    }
                }
            case .failure(let error):
                archiveMessage = "File selection failed: \(error.localizedDescription)"
            }
        }
    }
}
