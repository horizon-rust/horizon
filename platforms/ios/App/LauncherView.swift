import SwiftUI
import UniformTypeIdentifiers

/// Nova is the thin iOS host for the ORIGINAL Horizon game.
/// The gameplay runtime is linked in the integrated build but cannot yet
/// own an iOS event loop; do not invoke its desktop-style entry from SwiftUI.
struct LauncherView: View {
    @State private var showArchivePicker = false
    @State private var showMetalDiagnostic = false
    @State private var showDiagnostics = false
    @State private var displayedDiagnostics = ""
    @State private var archiveName: String? = NovaGameFiles.stagedArchiveName()
    @State private var archiveMessage: String?
    @State private var importInProgress = false
    @State private var prepareInProgress = false
    @State private var resourcesInstalled = NovaGameFiles.resourcesPresent()

    @State private var nextBootMessage: String?

    // This does NOT mean Horizon's rendering has been tested on-device.
    // It only enables a one-shot cold-start experiment for integrated builds.
    private var canTestHorizonBoot: Bool {
        resourcesInstalled && horizon_ios_host_api_version() >= 2
    }

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

                    Button("Try Horizon (next launch)") {
                        // A second UIApplicationMain inside SwiftUI is illegal.
                        // Save a one-shot request and show explicit restart steps.
                        NovaDiagnostics.record("launcher.game-boot-armed")
                        UserDefaults.standard.set(true, forKey: "nova.launchHorizonOnce")
                        nextBootMessage = "Close Nova completely using the app switcher, then reopen it to test the original Horizon game. The launcher will open again on the following launch."
                    }
                    .font(.headline)
                    .frame(minWidth: 180, minHeight: 48)
                    .buttonStyle(.borderedProminent)
                    .disabled(!canTestHorizonBoot)
                    .accessibilityHint("Start Horizon's winit lifecycle on the next app launch, after closing Nova")

                    if let nextBootMessage {
                        Text(nextBootMessage)
                            .foregroundStyle(.orange)
                            .multilineTextAlignment(.center)
                    } else {
                        Text(canTestHorizonBoot
                             ? "Resources installed. The experimental game entry runs on the next cold launch, not inside the existing SwiftUI lifecycle."
                             : "Prepare your game files to enable the experimental Horizon startup test. The full game build is required.")
                            .foregroundStyle(.white.opacity(0.75))
                            .multilineTextAlignment(.center)
                    }

                    Button("View Logs") {
                        displayedDiagnostics = NovaDiagnostics.tail()
                        showDiagnostics = true
                    }
                    .buttonStyle(.bordered)

                    if let logURL = NovaDiagnostics.logURL() {
                        ShareLink(item: logURL, preview: SharePreview("Nova diagnostics")) {
                            Label("Export Logs", systemImage: "square.and.arrow.up")
                        }
                        .buttonStyle(.bordered)
                    }

                    Button("Test Metal display") {
                        showMetalDiagnostic = true
                    }
                    .buttonStyle(.bordered)
                    .accessibilityHint("Opens a GPU display test, not Horizon gameplay")

                    Divider().overlay(.white.opacity(0.2))

                    Text("Game files")
                        .font(.title3.bold())
                        .foregroundStyle(.white)

                    Text(resourcesInstalled ? "Prepared resources installed locally." : "No validated game resources installed.")
                        .foregroundStyle(resourcesInstalled ? .green : .white.opacity(0.75))
                        .multilineTextAlignment(.center)

                    if let archiveName {
                        Text("Imported ZIP: \(archiveName)")
                            .font(.footnote)
                            .foregroundStyle(.white.opacity(0.7))
                    }

                    Button(importInProgress ? "Importing…" : "Import game files ZIP") {
                        showArchivePicker = true
                    }
                    .buttonStyle(.bordered)
                    .disabled(importInProgress || prepareInProgress)

                    Button(prepareInProgress ? "Preparing resources…" : "Extract and verify game files") {
                        NovaDiagnostics.record("resources.extraction.started")
                        prepareInProgress = true
                        archiveMessage = "Extracting your local ZIP. Keep Nova open."
                        DispatchQueue.global(qos: .userInitiated).async {
                            let result = Result { try NovaGameFiles.prepareStoredZip() }
                            DispatchQueue.main.async {
                                prepareInProgress = false
                                resourcesInstalled = NovaGameFiles.resourcesPresent()
                                archiveName = NovaGameFiles.stagedArchiveName()
                                switch result {
                                case .success:
                                    NovaDiagnostics.record("resources.extraction.succeeded")
                                    archiveMessage = "Game files prepared locally. Horizon startup integration is still in development."
                                case .failure(let error):
                                    NovaDiagnostics.record("resources.extraction.failed")
                                    archiveMessage = "Preparation failed: \(error.localizedDescription)"
                                }
                            }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(archiveName == nil || importInProgress || prepareInProgress)

                    if prepareInProgress {
                        ProgressView().tint(.white)
                    }

                    Text("Files stay on this device. Nova never bundles or downloads Minecraft assets. Structural checks do not enable Play.")
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
        .sheet(isPresented: $showDiagnostics) {
            VStack(spacing: 12) {
                Text("Nova startup diagnostics")
                    .font(.headline)
                Text("Records the launcher, game startup, window creation and rendering stages. Export only if you're comfortable sharing the log.")
                    .font(.footnote)
                    .multilineTextAlignment(.center)
                ScrollView {
                    Text(displayedDiagnostics)
                        .font(.system(size: 11, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                        .padding(12)
                }
                Button("Close") { showDiagnostics = false }
                    .buttonStyle(.borderedProminent)
            }
            .padding()
        }
        .sheet(isPresented: $showMetalDiagnostic) {
            VStack(spacing: 12) {
                Text("Nova Metal surface test")
                    .font(.title3.bold())
                Text("This is a GPU display diagnostic, not the Horizon game.")
                    .font(.footnote)
                GameSurfaceView()
                    .frame(minHeight: 180)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                Button("Close") { showMetalDiagnostic = false }
                    .buttonStyle(.borderedProminent)
            }
            .padding()
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
                            NovaDiagnostics.record("resources.archive-imported")
                            archiveName = name
                            archiveMessage = "ZIP imported locally. Tap Extract and verify game files."
                        case .failure(let error):
                            NovaDiagnostics.record("resources.archive-import-failed")
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
